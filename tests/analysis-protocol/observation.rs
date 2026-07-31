//! Observation-path protocol tests: wait timeouts, bounded safe-GET retries,
//! `Retry-After` honoring, deadline clamping, and local cancellation.
//!
//! These scenarios use real short wall-clock bounds plus recorded fixture
//! requests as their determinism evidence (see the timed assertions), and a
//! `multi_thread` runtime only where a hanging fixture must be polled while
//! cancellation lands.

use super::support::*;

/// 8. Wait timeouts exit through canonical wait_timeout carrying the local
/// ID, upstream task ID, and last observed stage.
#[tokio::test(flavor = "current_thread")]
async fn wait_timeout_reports_identity_and_last_stage() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    // Polls keep returning an in-progress document faster than the
    // per-request timeout, so only the local wait deadline can end the
    // observation. The scripted queue is generous because the poll
    // interval is zero.
    for _ in 0..1024 {
        fixture.on_poll(Step::Json(serde_json::json!({
            "task_id": TASK_ID,
            "stage": "STAGE_INFERENCE"
        })));
    }

    let analyzer = Analyzer::from_client(fixture.client_with_policy(
        RetryPolicy::OFF,
        PollPolicy::new(Duration::ZERO, Duration::ZERO),
        Duration::from_millis(400),
    ));
    let accepted = analyzer
        .start(
            request(SYNTHETIC_TEXT),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance succeeds");
    let microck_pangram_cli::analysis::Accepted::Task(input) = accepted else {
        panic!();
    };
    let analysis_id = input.request.id();
    // The shared pacing gate spaces the first poll one 200 ms interval
    // behind the submit, so the wait budget must exceed that interval for
    // at least one stage-bearing observation to land before the deadline.
    let result = analyzer
        .running(input)
        .observe(
            WaitOptions::with_timeout(Duration::from_millis(600)),
            |_| {},
            StopObserving::new(),
        )
        .await
        .expect("no interruption");

    let error = result.expect_err("a local wait timeout");
    assert_eq!(error.error().code(), ErrorCode::WaitTimeout);
    assert!(error.error().retryable());
    let details = serde_json::to_string(
        &serde_json::to_value(error.error().details().expect("details"))
            .expect("details serialize"),
    )
    .expect("details text");
    assert!(details.contains(&analysis_id.to_string()), "{details}");
    assert!(details.contains(TASK_ID), "{details}");
    assert!(details.contains("STAGE_INFERENCE"), "{details}");
    assert_scrubbed(error.error());
    fixture.shutdown().await;
}

/// 8a. A tiny wait deadline interrupts a bounded safe-GET retry chain
/// promptly, not after the full `Retry-After` hint schedule. The loopback
/// serves repeated 503/429 with a long hint; the caller's 250 ms wait
/// timeout must end the chain near 250 ms, never through the 30 s hint.
#[tokio::test(flavor = "current_thread")]
async fn tiny_wait_deadline_interrupts_retry_after_chain_promptly() {
    let fixture = ProtocolFixture::start().await;
    let retry = RetryPolicy {
        max_attempts: 5,
        base_delay: Duration::ZERO,
        // A long hint window so the honored 30 s Retry-After is not the
        // constraint; the deadline must be.
        max_delay: Duration::from_secs(30),
        cumulative_retry_budget: None,
    }
    .validate()
    .expect("valid policy");
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    // A long Retry-After on every transient response. Without deadline
    // clamping the chain would sleep roughly 30 s between attempts, far
    // beyond a 250 ms wait deadline.
    for _ in 0..5 {
        fixture.on_poll(Step::Status(503, Some(30), None));
    }

    let analyzer = Analyzer::from_client(fixture.client_with_policy(
        retry,
        PollPolicy::new(Duration::ZERO, Duration::ZERO),
        Duration::from_millis(400),
    ));
    let accepted = analyzer
        .start(
            request(SYNTHETIC_TEXT),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance succeeds");
    let microck_pangram_cli::analysis::Accepted::Task(input) = accepted else {
        panic!();
    };
    let started = std::time::Instant::now();
    let result = analyzer
        .running(input)
        .observe(
            WaitOptions::with_timeout(Duration::from_millis(250)),
            |_| {},
            StopObserving::new(),
        )
        .await
        .expect("no interruption");
    let elapsed = started.elapsed();

    let error = result.expect_err("a tiny wait deadline must end observation");
    assert_eq!(error.error().code(), ErrorCode::WaitTimeout);
    // The wall-clock bound proves interruption was prompt: every sleep is
    // clamped to the remaining deadline instead of riding out the 30 s
    // hint. The first poll's real 200 ms pace plus the 250 ms budget leave
    // generous headroom under one second, while the hint would demand many
    // seconds.
    assert!(
        elapsed < Duration::from_secs(1),
        "a 30 s Retry-After chain must not delay a 250 ms deadline: {elapsed:?}"
    );
    assert!(
        fixture.get_count() <= 2,
        "the deadline clamps the first retry sleep to nothing: {}",
        fixture.get_count()
    );
    assert_scrubbed(error.error());
    fixture.shutdown().await;
}

/// 8b. Local cancellation interrupts a pending retry sleep: a 503/429
/// storm with a long `Retry-After` is abandoned as soon as the stop token
/// fires, never after the hinted delay.
#[tokio::test(flavor = "current_thread")]
async fn cancellation_interrupts_a_pending_retry_sleep() {
    let fixture = ProtocolFixture::start().await;
    let retry = RetryPolicy {
        max_attempts: 5,
        base_delay: Duration::ZERO,
        max_delay: Duration::from_secs(30),
        cumulative_retry_budget: None,
    }
    .validate()
    .expect("valid policy");
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Status(503, Some(30), None));

    let analyzer = Analyzer::from_client(fixture.client_with_policy(
        retry,
        PollPolicy::new(Duration::ZERO, Duration::ZERO),
        Duration::from_millis(400),
    ));
    let accepted = analyzer
        .start(
            request(SYNTHETIC_TEXT),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance succeeds");
    let microck_pangram_cli::analysis::Accepted::Task(input) = accepted else {
        panic!();
    };

    let stop = StopObserving::new();
    let stopper = stop.clone();
    let observation = tokio::spawn(async move {
        analyzer
            .running(input)
            .observe(WaitOptions::UNBOUNDED, |_| {}, stop)
            .await
    });

    // cancel inside the 30 s Retry-After sleep, not before the poll fires.
    fixture.wait_for_gets(1).await;
    let started = std::time::Instant::now();
    stopper.stop();
    let interrupted = observation
        .await
        .expect("the observation task completes")
        .expect_err("cancellation interrupts the retry sleep");
    assert!(interrupted.identity.task_id.is_some());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the 30 s hint must not hold the cancelled chain: {:?}",
        started.elapsed()
    );
    assert_eq!(
        fixture.get_count(),
        1,
        "the cancelled retry never issues a second request"
    );
    fixture.shutdown().await;
}

/// 9. Cancellation stops local observation only and never reaches upstream.
#[tokio::test(flavor = "current_thread")]
async fn cancellation_stops_local_observation_only() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Hang);

    let analyzer = Analyzer::from_client(fixture.client());
    let accepted = analyzer
        .start(
            request(SYNTHETIC_TEXT),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance succeeds");
    let microck_pangram_cli::analysis::Accepted::Task(input) = accepted else {
        panic!();
    };
    let analysis_id = input.request.id();

    let stop = StopObserving::new();
    let stopper = stop.clone();
    let observation = tokio::spawn(async move {
        analyzer
            .running(input)
            .observe(WaitOptions::UNBOUNDED, |_| {}, stop)
            .await
    });

    // Cancel while the first poll hangs. Nothing else is scripted: any
    // further HTTP request would hit the unscripted-queue panic.
    fixture.wait_for_gets(1).await;
    stopper.stop();

    let interrupted = observation
        .await
        .expect("the observation task completes")
        .expect_err("cancellation interrupts observation");
    assert_eq!(interrupted.identity.analysis_id, analysis_id);
    assert_eq!(
        interrupted
            .identity
            .task_id
            .as_ref()
            .map(UpstreamTaskId::as_str),
        Some(TASK_ID)
    );
    let posts = fixture.post_count();
    let gets = fixture.get_count();
    assert_eq!(posts, 1);
    assert!(
        gets <= 1,
        "cancellation must not produce further remote calls: {gets}"
    );
    let recorded = fixture.requests();
    let paths: Vec<_> = recorded
        .iter()
        .map(|request| request.path.as_str())
        .collect();
    assert!(
        !paths.iter().any(|path| path.contains("cancel")),
        "no remote cancellation route may be called: {paths:?}"
    );
    fixture.shutdown().await;
}
