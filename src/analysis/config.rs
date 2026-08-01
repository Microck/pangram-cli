//! Rate, retry, polling, and clock policy for the analysis module.
//!
//! The clock and every wait are injectable. Protocol tests use paused Tokio
//! time inside one runtime, so retry, `Retry-After`, polling, and timeout
//! behavior is proven without flaky wall-clock assumptions.

pub use tokio::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::config::MAX_REQUESTS_PER_SECOND;

/// The wall-clock implementation used by production analyzers.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

/// Time source owned by [`crate::config::MAX_REQUESTS_PER_SECOND`]-bounded
/// analysis policy so every wait is injectable. The default implementation
/// is the Tokio runtime clock; tests run the same code against paused
/// runtime time inside one multi-thread runtime.
pub trait Clock: std::fmt::Debug + Send + Sync + Copy + 'static {
    fn now(&self) -> Instant;

    /// Sleeps unless `cancel` fires first. Returns `true` when the full
    /// duration elapsed and `false` when cancellation interrupted the wait.
    fn sleep_until(
        &self,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> impl Future<Output = bool> + Send;
}

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    async fn sleep_until(&self, deadline: Instant, cancel: &CancellationToken) -> bool {
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => true,
            () = cancel.cancelled() => false,
        }
    }
}

/// Bounds one safe-GET retry chain. Delays are decorrelated full-jitter
/// values inside `[base, min(cap, 3 * previous)]`; a server `Retry-After`
/// hint replaces the computed delay and is bounded by `max_delay`.
///
/// A chain also has a cumulative retry-time `budget`: the combined sleep
/// planned across retry attempts may not exceed it, so large
/// `Retry-After` hints cannot keep postponing a caller deadline
/// indefinitely even when each individual hint is bounded.
///
/// A zero `base_delay` makes every backoff delay zero, one `max_attempts`
/// disables retries, and a zero `max_retry_after` ignores server hints; this
/// is the deterministic policy injected by protocol tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryPolicy {
    /// Total tries for one safe GET, including the first. Within 1..=10.
    pub max_attempts: u32,
    /// The first backoff delay and the lower bound of every later delay.
    pub base_delay: Duration,
    /// The ceiling for computed backoff and honored `Retry-After` hints.
    pub max_delay: Duration,
    /// Cumulative ceiling on the total sleep planned across the retry chain.
    /// `None` selects the documented default (`max_delay * max_attempts`).
    pub cumulative_retry_budget: Option<Duration>,
}

impl RetryPolicy {
    /// The production policy: five attempts over a bounded 250 ms to 5 s
    /// decorrelated window, with a cumulative 12 s retry-time budget so an
    /// adversarial `Retry-After` sequence cannot keep a bounded chain alive
    /// indefinitely.
    pub const PRODUCTION: Self = Self {
        max_attempts: 5,
        base_delay: Duration::from_millis(250),
        max_delay: Duration::from_secs(5),
        cumulative_retry_budget: Some(Duration::from_secs(12)),
    };

    /// A deterministic policy for tests: no wait between attempts, no
    /// cumulative budget (sleep is already zero).
    pub const OFF: Self = Self {
        max_attempts: 1,
        base_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
        cumulative_retry_budget: None,
    };

    /// The effective cumulative retry-time budget. When unset, the bound is
    /// `max_delay * max_attempts`, which is the largest legitimate total and
    /// never under-counts a deterministic chain.
    #[must_use]
    pub fn cumulative_retry_budget(&self) -> Duration {
        self.cumulative_retry_budget
            .unwrap_or_else(|| self.max_delay.saturating_mul(self.max_attempts))
    }

    /// Validates the bounds that production callers rely on: at least one
    /// attempt, and `max_delay >= base_delay` (clamping `Retry-After`
    /// behavior requires it).
    pub fn validate(self) -> Result<Self, String> {
        if self.max_attempts == 0 {
            return Err("max_attempts must be at least 1".into());
        }
        if self.max_delay < self.base_delay {
            return Err("retry max_delay must not be below base_delay".into());
        }
        Ok(self)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::PRODUCTION
    }
}

/// How often an observation loop re-polls after a non-terminal state.
/// Independent of retry backoff.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PollPolicy {
    pub interval: Duration,
    pub min_interval: Duration,
}

impl PollPolicy {
    /// The production interval matches Pangram's documented client cadence.
    pub const PRODUCTION: Self = Self {
        interval: Duration::from_millis(500),
        min_interval: Duration::from_millis(100),
    };

    pub const fn new(interval: Duration, min_interval: Duration) -> Self {
        Self {
            interval,
            min_interval,
        }
    }

    #[must_use]
    pub fn effective_interval(&self) -> Duration {
        self.interval.max(self.min_interval)
    }
}

impl Default for PollPolicy {
    fn default() -> Self {
        Self::PRODUCTION
    }
}

/// Shared analysis behavior configuration: immutable for one process.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisConfig<C = SystemClock> {
    retry: RetryPolicy,
    polling: PollPolicy,
    per_request_timeout: Duration,
    max_requests_per_second: f64,
    clock: C,
}

impl AnalysisConfig<SystemClock> {
    /// Production configuration from the effective `network` config value.
    /// `max_requests_per_second` comes pre-validated by the config layer
    /// (`0 < rate <= 5`); passing `None` selects the built-in ceiling.
    pub fn production(max_requests_per_second: Option<f64>) -> Self {
        let rate = max_requests_per_second.unwrap_or(MAX_REQUESTS_PER_SECOND);
        Self::new(
            RetryPolicy::PRODUCTION,
            PollPolicy::PRODUCTION,
            Duration::from_secs(10),
            rate,
            SystemClock,
        )
        .expect("production analysis config is valid")
    }

    /// Test-only configuration for loopback fixtures; in-crate unit tests
    /// exercise it alongside the injected clocks used by the dev-tools
    /// protocol tests.
    #[cfg(any(test, feature = "dev-tools", doctest))]
    #[doc(hidden)]
    pub fn for_test(
        retry: RetryPolicy,
        polling: PollPolicy,
        per_request_timeout: Duration,
        max_requests_per_second: f64,
    ) -> Self {
        Self::new(
            retry,
            polling,
            per_request_timeout,
            max_requests_per_second,
            SystemClock,
        )
        .expect("test analysis config must be valid")
    }
}

impl<C: Clock> AnalysisConfig<C> {
    fn new(
        retry: RetryPolicy,
        polling: PollPolicy,
        per_request_timeout: Duration,
        max_requests_per_second: f64,
        clock: C,
    ) -> Result<Self, String> {
        retry.validate()?;
        if !max_requests_per_second.is_finite()
            || max_requests_per_second <= 0.0
            || max_requests_per_second > MAX_REQUESTS_PER_SECOND
        {
            return Err(format!(
                "max_requests_per_second must be greater than 0 and at most {MAX_REQUESTS_PER_SECOND}"
            ));
        }
        if per_request_timeout.is_zero() {
            return Err("per-request timeout must be positive".into());
        }
        Ok(Self {
            retry,
            polling,
            per_request_timeout,
            max_requests_per_second,
            clock,
        })
    }

    #[must_use]
    pub const fn retry(&self) -> RetryPolicy {
        self.retry
    }

    #[must_use]
    pub const fn polling(&self) -> PollPolicy {
        self.polling
    }

    #[must_use]
    pub const fn per_request_timeout(&self) -> Duration {
        self.per_request_timeout
    }

    /// The effective throughput ceiling. Configuration may lower it toward
    /// (but never below a positive) value and never above 5 QPS.
    #[must_use]
    pub const fn max_requests_per_second(&self) -> f64 {
        self.max_requests_per_second
    }

    pub(crate) const fn clock(&self) -> C {
        self.clock
    }
}

/// Per-wait options supplied by adapters. A `None` timeout waits until the
/// operation is terminal or locally cancelled. Each CLI analysis command maps
/// its `--timeout` flag onto this: an absent flag is `UNBOUNDED` (wait for
/// terminal), and a supplied duration bounds the local wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WaitOptions {
    pub timeout: Option<Duration>,
}

impl WaitOptions {
    pub const UNBOUNDED: Self = Self { timeout: None };

    pub const fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout: Some(timeout),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_config_clamps_only_downward() {
        let config = AnalysisConfig::production(Some(2.0));
        assert_eq!(config.max_requests_per_second(), 2.0);

        let default = AnalysisConfig::production(None);
        assert_eq!(default.max_requests_per_second(), MAX_REQUESTS_PER_SECOND);
    }

    #[test]
    fn config_rejects_rates_above_the_hard_ceiling() {
        let error = AnalysisConfig::new(
            RetryPolicy::PRODUCTION,
            PollPolicy::PRODUCTION,
            Duration::from_secs(1),
            5.000_000_1,
            SystemClock,
        )
        .unwrap_err();
        assert!(error.contains("at most 5"), "{error}");
    }

    #[test]
    fn retry_policy_rejects_an_inverted_window() {
        let policy = RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_secs(5),
            max_delay: Duration::from_secs(1),
            cumulative_retry_budget: None,
        };
        assert!(policy.validate().is_err());
    }
}
