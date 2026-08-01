//! One process-wide time-based issue gate for Pangram requests.
//!
//! A single shared pacemaker enforces the documented hard maximum of 5
//! requests per second on request **issue** timing: every request is issued
//! only after the gate releases it, so consecutive issue events are spaced at
//! least `1/rate` apart and no burst can exceed the pacing. This is a GCRA
//! (Generic Cell Rate Algorithm) style gate rather than a concurrency
//! semaphore: it does not wait for in-flight requests to finish, it schedules
//! when the next request may start.
//!
//! The configured `network.max_requests_per_second` (built-in default 5.0,
//! validated `0 < rate <= 5`) may only lower the effective pacing; it can
//! never raise the rate above the hard ceiling. The schedule is shared by
//! every caller through one [`Pacemaker`] state, so submit and poll paths
//! share the same envelope.
//!
//! The clock and every wait are injectable so protocol tests run the same
//! gate against paused runtime time.

use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use super::config::{Clock, Duration, Instant};

/// The shared pacing state behind one client's request gate.
struct Inner {
    /// The next instant at which a request may issue. Behind `Mutex`, the
    /// read-modify-write reservation is atomic per caller.
    next_issue: Instant,
    /// Minimum interval between consecutive issue events (`1/rate`).
    interval: Duration,
}

/// The outcome of one gated acquisition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// The slot opened; the request may issue now.
    Released,
    /// The caller's cancellation token fired while waiting for the slot.
    Cancelled,
    /// No cancellation fired, but the caller's wait deadline passed before
    /// the slot opened. Distinct from `Cancelled` so the observe loop maps
    /// it to the canonical wait timeout, not to local interruption.
    DeadlinePassed,
}

/// One copy of the pacing gate. Cloning shares the inner schedule; every
/// request issued through this gate observes the same cumulative cadence.
#[derive(Clone)]
pub struct Pacemaker<C> {
    inner: Arc<Mutex<Inner>>,
    clock: C,
}

impl<C: Clock> Pacemaker<C> {
    /// Builds a gate pacing requests at `max_requests_per_second`. The
    /// configured rate is validated at the config boundary (`0 < rate <= 5`),
    /// which remains the single documented enforcement seam. As release-path
    /// defense-in-depth (a `debug_assert!` is compiled out), the effective
    /// rate is clamped into the documented ceiling and any non-finite or
    /// non-positive input falls back to the safe maximum rather than letting
    /// `Duration::from_secs_f64` panic or a too-large rate exceed the hard
    /// 5-requests-per-second ceiling. The first request may issue immediately.
    #[must_use]
    pub fn new(max_requests_per_second: f64, clock: C) -> Self {
        let rate = if max_requests_per_second.is_finite() && max_requests_per_second > 0.0 {
            max_requests_per_second.min(crate::config::MAX_REQUESTS_PER_SECOND)
        } else {
            // Fail closed to the documented ceiling pacing, never a burst.
            crate::config::MAX_REQUESTS_PER_SECOND
        };
        let interval = Duration::from_secs_f64(1.0 / rate);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                next_issue: clock.now(),
                interval,
            })),
            clock,
        }
    }

    /// Reserves the next issue slot, sleeping until the gate releases it.
    /// The schedule is advanced exactly once per call regardless of the
    /// outcome, so a cancelled or deadline-cut caller only ever delays
    /// (never accelerates) later issues. Holding the lock across the whole
    /// read-modify-write keeps concurrent callers from choosing the same
    /// slot. When `deadline` would pass before the slot opens, the wait
    /// stops at the deadline and reports `DeadlinePassed` so the caller can
    /// surface the canonical wait timeout instead of a local interruption.
    pub async fn hurdle(&self, cancel: &CancellationToken, deadline: Option<Instant>) -> Gate {
        let now = self.clock.now();
        // An already-lapsed wait deadline must gate request issue even on the
        // immediate path: a caller whose budget ran out issues no request at
        // all, and the shared schedule is not advanced for a stop it never
        // performs. (CodeRabbit stability finding; the sleep path already
        // honors the deadline, but the released-immediately branch did not.)
        if deadline.is_some_and(|deadline| now >= deadline) {
            return Gate::DeadlinePassed;
        }
        let (wait_until, immediate) = {
            let mut inner = self.inner.lock().expect("pacemaker poisoned");
            let scheduled = inner.next_issue;
            if scheduled <= now {
                inner.next_issue = now + inner.interval;
                (now, true)
            } else {
                inner.next_issue = scheduled + inner.interval;
                (scheduled, false)
            }
        };
        if immediate {
            // First call (or a lull) releases without sleeping. We still
            // yield so a cancelled sibling cannot starve on a single-thread
            // runtime.
            tokio::task::yield_now().await;
            return if cancel.is_cancelled() {
                Gate::Cancelled
            } else {
                Gate::Released
            };
        }
        let wake = deadline.map_or(wait_until, |deadline| wait_until.min(deadline));
        if !self.clock.sleep_until(wake, cancel).await {
            return Gate::Cancelled;
        }
        if wake < wait_until {
            // The deadline woke us before the slot opened: the caller's wait
            // budget is exhausted, so this is a wait timeout, not a cancel.
            return Gate::DeadlinePassed;
        }
        Gate::Released
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::analysis::config::SystemClock;

    /// The default 5-per-second pacing (200 ms between issues).
    const FIVE_PER_SECOND: f64 = 5.0;
    const INTERVAL_MS: u128 = 200;

    const NO_DEADLINE: Option<Instant> = None;

    #[tokio::test(start_paused = true)]
    async fn first_issue_is_immediate_and_subsequent_issues_are_paced() {
        let clock = SystemClock;
        let gate = Pacemaker::new(FIVE_PER_SECOND, clock);
        let cancel = CancellationToken::new();

        let t0 = clock.now();
        assert_eq!(gate.hurdle(&cancel, NO_DEADLINE).await, Gate::Released);
        assert_eq!(clock.now().checked_duration_since(t0), Some(Duration::ZERO));

        for expected_ms in [INTERVAL_MS, 2 * INTERVAL_MS, 3 * INTERVAL_MS] {
            assert_eq!(gate.hurdle(&cancel, NO_DEADLINE).await, Gate::Released);
            let elapsed = clock.now().checked_duration_since(t0).expect("monotonic");
            assert_eq!(elapsed.as_millis(), expected_ms, "issue spacing is 1/rate");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn concurrently_issued_requests_are_paced_with_no_burst() {
        let clock = SystemClock;
        let gate = Pacemaker::new(FIVE_PER_SECOND, clock);
        let cancel = CancellationToken::new();
        let start = clock.now();

        let mut joins = Vec::new();
        for _ in 0..8 {
            let gate = gate.clone();
            let cancel = cancel.clone();
            joins.push(tokio::spawn(async move {
                assert_eq!(gate.hurdle(&cancel, NO_DEADLINE).await, Gate::Released);
                gate.clock.now() // capture issue instant
            }));
        }
        let mut issues = Vec::new();
        for join in joins {
            issues.push(join.await.expect("pacing task completes"));
        }
        issues.sort_unstable();
        assert_eq!(issues.len(), 8);
        // First is immediate; each later issue is at least 1 interval after
        // the previous one.
        assert_eq!(issues[0], start);
        for window in issues.windows(2) {
            let gap = window[1]
                .checked_duration_since(window[0])
                .expect("monotonic");
            assert!(
                gap.as_millis() >= INTERVAL_MS,
                "concurrent issues stayed at least 1/rate apart: {gap:?}"
            );
        }
        // No burst: 8 issue events span at least 7 full intervals.
        let total = issues[7]
            .checked_duration_since(issues[0])
            .expect("monotonic");
        assert!(total >= Duration::from_millis(7 * INTERVAL_MS as u64));
    }

    #[tokio::test(start_paused = true)]
    async fn a_lowered_fractional_rate_paces_to_its_own_interval() {
        let clock = SystemClock;
        // 2.5 requests/second -> 400 ms between issues (longer, never more).
        let gate = Pacemaker::new(2.5, clock);
        let cancel = CancellationToken::new();
        let t0 = clock.now();
        assert_eq!(gate.hurdle(&cancel, NO_DEADLINE).await, Gate::Released);
        assert_eq!(gate.hurdle(&cancel, NO_DEADLINE).await, Gate::Released);
        let elapsed = clock.now().checked_duration_since(t0).expect("monotonic");
        assert_eq!(elapsed.as_millis(), 400, "a lowered rate lowers pacing");
        // And a lowered rate is never faster than the hard 5-per-second
        // ceiling's 200 ms interval would allow.
        assert!(elapsed >= Duration::from_millis(200));
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_interrupts_a_pending_gated_wait() {
        let clock = SystemClock;
        let gate = Pacemaker::new(FIVE_PER_SECOND, clock);
        let cancel = CancellationToken::new();
        let t0 = clock.now();
        assert_eq!(gate.hurdle(&cancel, NO_DEADLINE).await, Gate::Released);

        let stopper = cancel.clone();
        let gate = gate.clone();
        let waiting = tokio::spawn(async move { gate.hurdle(&cancel, NO_DEADLINE).await });
        // Cancellation lands before the 200 ms slot opens, so the
        // waiting hurdle returns promptly instead of sleeping to time.
        stopper.cancel();
        let outcome = waiting.await.expect("task completes");
        assert_eq!(outcome, Gate::Cancelled, "cancellation interrupts the wait");
        assert_eq!(
            clock.now().checked_duration_since(t0),
            Some(Duration::ZERO),
            "the cancelled wait must never block for its slot"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_wait_deadline_passing_before_the_slot_reports_deadline_not_cancel() {
        let clock = SystemClock;
        let gate = Pacemaker::new(FIVE_PER_SECOND, clock);
        let cancel = CancellationToken::new();
        let t0 = clock.now();
        assert_eq!(gate.hurdle(&cancel, NO_DEADLINE).await, Gate::Released);

        // The next slot opens at +200 ms; a deadline at +50 ms must wake the
        // wait there and report the deadline, not a cancellation.
        let deadline = clock.now() + Duration::from_millis(50);
        let outcome = gate.hurdle(&cancel, Some(deadline)).await;
        assert_eq!(outcome, Gate::DeadlinePassed);
        assert_eq!(
            clock.now().checked_duration_since(t0),
            Some(Duration::from_millis(50)),
            "the wait stops at the deadline, not at the slot"
        );
        assert!(
            !cancel.is_cancelled(),
            "a deadline wake is not a cancellation"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_lapsed_deadline_stops_immediate_issue_without_advancing_the_schedule() {
        let clock = SystemClock;
        let gate = Pacemaker::new(FIVE_PER_SECOND, clock);
        let cancel = CancellationToken::new();

        // First call is normally immediate; a deadline that has already
        // passed must prevent that immediate issue entirely.
        let past_deadline = clock.now() - Duration::from_millis(1);
        let outcome = gate.hurdle(&cancel, Some(past_deadline)).await;
        assert_eq!(
            outcome,
            Gate::DeadlinePassed,
            "a lapsed deadline gates the immediate issue path"
        );
        assert!(
            !cancel.is_cancelled(),
            "the deadline stop is not a cancellation"
        );

        // Because the stopped call performed no issue, the schedule is not
        // advanced: a subsequent hurdle with a live deadline still releases
        // immediately (the first true issue).
        let outcome = gate
            .hurdle(&cancel, Some(clock.now() + Duration::from_secs(60)))
            .await;
        assert_eq!(
            outcome,
            Gate::Released,
            "the schedule was not advanced by the lapsed-deadline stop"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_rate_above_the_ceiling_is_clamped_to_the_documented_pacing() {
        // Release-path defense-in-depth: a caller that bypasses config
        // validation with 10 rps must not exceed the hard 5 rps ceiling.
        let clock = SystemClock;
        let gate = Pacemaker::new(10.0, clock);
        let cancel = CancellationToken::new();
        let start = clock.now();

        assert_eq!(gate.hurdle(&cancel, NO_DEADLINE).await, Gate::Released);
        assert_eq!(gate.hurdle(&cancel, NO_DEADLINE).await, Gate::Released);
        let elapsed = clock
            .now()
            .checked_duration_since(start)
            .expect("monotonic");
        assert_eq!(
            elapsed.as_millis(),
            INTERVAL_MS,
            "an over-ceiling rate is clamped to the 200 ms ceiling spacing"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn non_finite_or_non_positive_rates_fail_closed_to_the_ceiling() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
            let clock = SystemClock;
            // Must not panic in `Duration::from_secs_f64`; falls back to the
            // documented 200 ms ceiling spacing.
            let gate = Pacemaker::new(bad, clock);
            let cancel = CancellationToken::new();
            let start = clock.now();
            assert_eq!(gate.hurdle(&cancel, NO_DEADLINE).await, Gate::Released);
            assert_eq!(gate.hurdle(&cancel, NO_DEADLINE).await, Gate::Released);
            let elapsed = clock
                .now()
                .checked_duration_since(start)
                .expect("monotonic");
            assert_eq!(
                elapsed.as_millis(),
                INTERVAL_MS,
                "rate {bad} fails closed to the ceiling pacing"
            );
        }
    }
}
