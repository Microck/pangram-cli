//! Pangram 4 text protocol integration tests against a real loopback
//! Axum fixture. No mocks, no live Pangram, no credentials.
//!
//! One `multi_thread` runtime with `start_paused` drives everything: all
//! waits (retry backoff, Retry-After, poll intervals, timeouts) advance in
//! virtual time, so the assertions are deterministic evidence, not
//! wall-clock races.

#![cfg(feature = "dev-tools")]

#[path = "support/protocol_loopback/mod.rs"]
mod fixture;

use microck_pangram_cli::analysis::{
    Analyzer, Duration, PollPolicy, RetryPolicy, StopObserving, WaitOptions,
};
use microck_pangram_cli::domain::{AnalysisStatus, TextOrigin, UpstreamTaskId};
use microck_pangram_cli::output::ErrorCode;

use fixture::{
    ProtocolFixture, SYNTHETIC_KEY, SYNTHETIC_TEXT, Step, TASK_ID, pangram4_failure,
    pangram4_success,
};
use microck_pangram_cli::analysis::AnalysisRequest;

const KEY_FRAGMENT: &str = "synthetic_key_0000";

fn request(text: &str) -> AnalysisRequest {
    AnalysisRequest::new(text, TextOrigin::Literal, None, 8, false, false)
}

fn assert_scrubbed(error: &microck_pangram_cli::output::CanonicalError) {
    let rendered = format!("{error:?}");
    let serialized = serde_json::to_string(error).expect("canonical error serializes");
    for surface in [&rendered, &serialized] {
        assert!(
            !surface.contains(SYNTHETIC_KEY),
            "the synthetic key must never appear: {surface}"
        );
        assert!(
            !surface.contains(KEY_FRAGMENT),
            "even a key fragment must never appear"
        );
        assert!(
            !surface.contains(SYNTHETIC_TEXT),
            "submitted content must never appear"
        );
        assert!(
            !surface.to_ascii_lowercase().contains("x-api-key"),
            "header names stay out of errors"
        );
    }
}

/// 1. Exact request grammar: one POST issue with the pinned Pangram 4 body,
/// the right header, no `Authorization`, then a terminal GET.
#[tokio::test(flavor = "current_thread")]
async fn submits_exact_pangram_4_document_and_reads_terminal_result() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(SYNTHETIC_TEXT)));

    let analyzer = Analyzer::from_client(fixture.client());
    let accepted = analyzer
        .start(
            request(SYNTHETIC_TEXT),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance succeeds");
    let microck_pangram_cli::analysis::Accepted::Task(input) = accepted else {
        panic!("the fixture returns an asynchronous acceptance");
    };
    assert_eq!(input.task_id.as_str(), TASK_ID);

    let first = &fixture.requests()[0];
    assert_eq!(first.method, "POST");
    assert_eq!(first.path, "/task");
    assert!(first.header_equals("x-api-key", SYNTHETIC_KEY));
    assert!(!first.header_present("authorization"));
    let body = first.body_json();
    assert_eq!(body["text"], SYNTHETIC_TEXT);
    assert_eq!(body["model"], "pangram-4");
    assert_eq!(body["public_dashboard_link"], false);
    assert_eq!(body.as_object().expect("an object").len(), 3);

    let running = analyzer.running(input);
    let outcome = running
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption")
        .expect("terminal success");

    assert_eq!(outcome.status(), AnalysisStatus::Succeeded);
    assert_eq!(fixture.requests()[1].path, format!("/task/{TASK_ID}"));
    assert_eq!(fixture.get_count(), 1);

    let check = &outcome.checks()[0];
    let microck_pangram_cli::domain::Check::AiDetection(state) = check else {
        panic!("the only check is AI detection");
    };
    let microck_pangram_cli::domain::CheckState::Succeeded { result, .. } = state else {
        panic!("a terminal success carries a result");
    };
    assert_eq!(
        result.classification,
        microck_pangram_cli::domain::AiClassification::Human
    );
    assert_eq!(result.segments.len(), 1);
    assert!(!result.segments[0].is_humanized);
    assert_eq!(
        outcome.provenance().upstream_version.as_deref(),
        Some("4.0")
    );

    fixture.shutdown().await;
}

/// 2. One in-progress poll transitions to running, then terminal success.
#[tokio::test(flavor = "current_thread")]
async fn in_progress_poll_then_terminal_success_preserves_stage_provenance() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(serde_json::json!({
        "task_id": TASK_ID,
        "stage": "STAGE_PREPROCESSING"
    })));
    fixture.on_poll(Step::Json(pangram4_success(SYNTHETIC_TEXT)));

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

    let running = analyzer.running(input);
    let mut stages = Vec::new();
    let outcome = running
        .observe(
            WaitOptions::UNBOUNDED,
            |progress| stages.push(progress.last_stage.as_str().to_owned()),
            StopObserving::new(),
        )
        .await
        .expect("no interruption")
        .expect("terminal success");

    assert_eq!(stages, ["STAGE_PREPROCESSING"]);
    assert_eq!(outcome.status(), AnalysisStatus::Succeeded);
    assert_eq!(fixture.get_count(), 2);
    fixture.shutdown().await;
}

/// 3. A terminal provider failure becomes a failed analysis with a canonical
/// upstream_analysis_failed error, never a panic.
#[tokio::test(flavor = "current_thread")]
async fn terminal_task_failure_maps_to_upstream_analysis_failed() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_failure("the input was too short")));

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
    let outcome = analyzer
        .running(input)
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption")
        .expect("a failed analysis is still an analysis value");

    assert_eq!(outcome.status(), AnalysisStatus::Failed);
    let microck_pangram_cli::domain::Check::AiDetection(state) = &outcome.checks()[0] else {
        panic!("the only check is AI detection");
    };
    let microck_pangram_cli::domain::CheckState::Failed { error, .. } = state else {
        panic!();
    };
    assert_eq!(error.code(), ErrorCode::UpstreamAnalysisFailed);
    assert!(!error.retryable());
    assert_scrubbed(error);
    fixture.shutdown().await;
}

/// 4. The documented HTTP failure matrix maps onto canonical codes, and
/// transient classes retry exactly a bounded number of times.
#[tokio::test(flavor = "current_thread")]
async fn http_status_matrix_maps_to_canonical_codes() {
    let cases: &[(u16, ErrorCode, bool)] = &[
        (400, ErrorCode::UpstreamError, false),
        (401, ErrorCode::InvalidApiKey, false),
        (402, ErrorCode::PaymentRequired, false),
        (403, ErrorCode::PermissionDenied, false),
        (413, ErrorCode::UnsupportedInput, false),
        (415, ErrorCode::UpstreamContractChanged, false),
        (422, ErrorCode::UpstreamContractChanged, false),
        (429, ErrorCode::RateLimited, true),
        (500, ErrorCode::UpstreamError, true),
        (503, ErrorCode::UpstreamError, true),
    ];

    for (status, code, retryable) in cases {
        let fixture = ProtocolFixture::start().await;
        let analyzer = Analyzer::from_client(fixture.client_with_policy(
            RetryPolicy::OFF,
            PollPolicy::new(Duration::ZERO, Duration::ZERO),
            Duration::from_millis(400),
        ));
        // The terminal non-retry classes fail the first poll.
        fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
        fixture.on_poll(Step::Status(*status, None, None));

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
        let result = analyzer
            .running(input)
            .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
            .await
            .expect("no interruption");

        // With policy OFF, transient statuses exhaust immediately to the
        // classified failure; 429 surfaces as RateLimited.
        let error = result.expect_err("a classified failure");
        assert_eq!(error.error().code(), *code, "status {status}");
        assert_eq!(error.error().retryable(), *retryable, "status {status}");
        assert_scrubbed(error.error());
        fixture.shutdown().await;
    }
}

/// 5. Safe GET retries honor Retry-After within bounds and stop at the
/// configured maximum attempts.
#[tokio::test(flavor = "current_thread")]
async fn safe_get_retries_are_bounded_and_honor_retry_after() {
    let fixture = ProtocolFixture::start().await;
    let retry = RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::ZERO,
        max_delay: Duration::from_secs(30),
    }
    .validate()
    .expect("valid policy");
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Status(429, Some(1), None));
    fixture.on_poll(Step::Status(503, None, None));
    fixture.on_poll(Step::Json(pangram4_success(SYNTHETIC_TEXT)));

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
    let outcome = analyzer
        .running(input)
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption")
        .expect("the third poll succeeds");

    assert_eq!(outcome.status(), AnalysisStatus::Succeeded);
    assert_eq!(
        fixture.get_count(),
        3,
        "two transient failures then one success"
    );
    fixture.shutdown().await;
}

/// 6. Retry exhaustion surfaces the classified error (no infinite loop).
#[tokio::test(flavor = "current_thread")]
async fn safe_get_retry_exhaustion_returns_the_classified_error() {
    let fixture = ProtocolFixture::start().await;
    let retry = RetryPolicy {
        max_attempts: 2,
        base_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
    }
    .validate()
    .expect("valid policy");
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Status(503, None, None));
    fixture.on_poll(Step::Status(503, None, None));

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
    let result = analyzer
        .running(input)
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption");

    let error = result.expect_err("exhaustion");
    assert_eq!(error.error().code(), ErrorCode::UpstreamError);
    assert_eq!(fixture.get_count(), 2, "exactly max_attempts polls");
    fixture.shutdown().await;
}

/// 7. A billable POST is issued exactly once even when the acceptance never
/// arrives (connection hang -> request timeout). The outcome is the fixed
/// non-retryable submission_outcome_unknown.
#[tokio::test(flavor = "current_thread")]
async fn ambiguous_submission_issues_one_post_and_reports_outcome_unknown() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Hang);

    let analyzer = Analyzer::from_client(fixture.client());
    let result = analyzer
        .start(
            request(SYNTHETIC_TEXT),
            &StopObserving::new().token().clone(),
        )
        .await;

    let error = result.expect_err("an ambiguous submission fails");
    assert_eq!(error.error().code(), ErrorCode::SubmissionOutcomeUnknown);
    assert!(!error.error().retryable());
    let recovery = error.error().recovery().expect("fixed recovery");
    assert_eq!(
        recovery.message(),
        "A manual retry may create a second billable operation."
    );
    assert!(recovery.command().is_none());
    let details =
        serde_json::to_value(error.error().details().expect("details")).expect("details serialize");
    let payload = serde_json::to_string(&details).expect("details to string");
    assert!(payload.contains("anl_"), "{payload}");
    assert!(payload.contains("request_sha256"), "{payload}");
    assert!(payload.contains("last_status"), "{payload}");
    assert_eq!(
        fixture.post_count(),
        1,
        "exactly one POST may be issued: duplicate billing is forbidden"
    );
    assert_scrubbed(error.error());
    fixture.shutdown().await;
}

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
    let result = analyzer
        .running(input)
        .observe(
            WaitOptions::with_timeout(Duration::from_millis(50)),
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
    tokio::task::yield_now().await;
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

/// 10. Contract-change matrix: wrong version, unknown stage/label/
/// confidence/classification, missing required fields (incl. humanizer), and
/// out-of-range scores all map to upstream_contract_changed.
#[tokio::test(flavor = "current_thread")]
async fn panged_documents_map_to_upstream_contract_changed() {
    let good = pangram4_success(SYNTHETIC_TEXT);

    let mut variants: Vec<(&str, serde_json::Value)> = Vec::new();
    let mut wrong_version = good.clone();
    wrong_version["version"] = serde_json::json!("3.1");
    variants.push(("wrong version", wrong_version));

    let mut missing_version = good.clone();
    missing_version
        .as_object_mut()
        .expect("object")
        .remove("version");
    variants.push(("missing version", missing_version));

    let mut unknown_stage = good.clone();
    unknown_stage["stage"] = serde_json::json!("STAGE_TELEPORTING");
    variants.push(("unknown stage", unknown_stage));

    let mut unknown_classification = good.clone();
    unknown_classification["prediction_short"] = serde_json::json!("Robot");
    variants.push(("unknown classification", unknown_classification));

    let mut missing_humanizer = good.clone();
    missing_humanizer["windows"][0]
        .as_object_mut()
        .expect("object")
        .remove("humanizer_score");
    variants.push(("missing humanizer_score", missing_humanizer));

    let mut missing_is_humanized = good.clone();
    missing_is_humanized["windows"][0]
        .as_object_mut()
        .expect("object")
        .remove("is_humanized");
    variants.push(("missing is_humanized", missing_is_humanized));

    let mut high_score = good.clone();
    high_score["windows"][0]["humanizer_score"] = serde_json::json!(1.2);
    variants.push(("humanizer score above 1", high_score));

    let mut fraction_nan = good.clone();
    fraction_nan["fraction_ai"] = serde_json::json!("not-a-number");
    variants.push(("fraction with wrong type", fraction_nan));

    let mut unknown_confidence = good.clone();
    unknown_confidence["windows"][0]["confidence"] = serde_json::json!("Certain");
    variants.push(("unknown confidence", unknown_confidence));

    let mut missing_windows = good.clone();
    missing_windows
        .as_object_mut()
        .expect("object")
        .remove("windows");
    variants.push(("missing windows", missing_windows));

    for (name, document) in variants {
        let fixture = ProtocolFixture::start().await;
        fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
        fixture.on_poll(Step::Json(document));

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
        let result = analyzer
            .running(input)
            .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
            .await
            .expect("no interruption");

        assert!(result.is_err(), "{name} must fail the pinned contract");
        let error = match result {
            Err(error) => error,
            Ok(_) => unreachable!(),
        };
        assert_eq!(
            error.error().code(),
            ErrorCode::UpstreamContractChanged,
            "{name}"
        );
        assert!(!error.error().retryable(), "{name}");
        assert_scrubbed(error.error());
        fixture.shutdown().await;
    }
}

/// 11. A malformed (non-JSON) poll body is a contract change, not a panic.
#[tokio::test(flavor = "current_thread")]
async fn malformed_poll_body_maps_to_contract_changed() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Status(200, None, None));

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
    let result = analyzer
        .running(input)
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption");
    let error = result.expect_err("malformed body");
    assert_eq!(error.error().code(), ErrorCode::UpstreamContractChanged);
    assert_scrubbed(error.error());
    fixture.shutdown().await;
}

/// 12. Additive optional fields (extra members at every level) are ignored.
#[tokio::test(flavor = "current_thread")]
async fn additive_fields_are_ignored() {
    let fixture = ProtocolFixture::start().await;
    let mut doc = pangram4_success(SYNTHETIC_TEXT);
    doc["new_upstream_field"] = serde_json::json!({"nested": [1, 2, 3]});
    doc["windows"][0]["extra_window_field"] = serde_json::json!("ignore me");
    fixture.on_submit(Step::Json(
        serde_json::json!({"task_id": TASK_ID, "extra": true}),
    ));
    fixture.on_poll(Step::Json(doc));

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
    let outcome = analyzer
        .running(input)
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption")
        .expect("additive fields still normalize");
    assert_eq!(outcome.status(), AnalysisStatus::Succeeded);
    fixture.shutdown().await;
}

/// 13. A dashboard link appears only when requested and returned.
#[tokio::test(flavor = "current_thread")]
async fn dashboard_link_passes_through_only_when_returned() {
    let fixture = ProtocolFixture::start().await;
    let mut doc = pangram4_success(SYNTHETIC_TEXT);
    doc["dashboard_link"] = serde_json::json!("https://dashboard.pangram.com/fixture-example");
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(doc));

    let analyzer = Analyzer::from_client(fixture.client());
    let link_request =
        AnalysisRequest::new(SYNTHETIC_TEXT, TextOrigin::Literal, None, 8, false, true);
    let accepted = analyzer
        .start(link_request, &StopObserving::new().token().clone())
        .await
        .expect("acceptance succeeds");
    let microck_pangram_cli::analysis::Accepted::Task(input) = accepted else {
        panic!();
    };
    let first = &fixture.requests()[0];
    assert_eq!(first.body_json()["public_dashboard_link"], true);

    let outcome = analyzer
        .running(input)
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption")
        .expect("terminal success");
    let microck_pangram_cli::domain::Check::AiDetection(state) = &outcome.checks()[0] else {
        panic!("the only check is AI detection");
    };
    let microck_pangram_cli::domain::CheckState::Succeeded { result, .. } = state else {
        panic!();
    };
    assert_eq!(
        result.dashboard_link.as_deref(),
        Some("https://dashboard.pangram.com/fixture-example")
    );
    fixture.shutdown().await;
}

/// 14. The loopback constructor rejects non-loopback and non-root URLs.
#[test]
fn loopback_constructor_rejects_non_loopback_targets() {
    use microck_pangram_cli::analysis::UpstreamEndpoints;

    assert!(UpstreamEndpoints::loopback("https://pangram.com").is_err());
    assert!(UpstreamEndpoints::loopback("http://10.0.0.4:8080").is_err());
    assert!(UpstreamEndpoints::loopback("http://127.0.0.1:8080/api").is_err());
    assert!(UpstreamEndpoints::loopback("ftp://127.0.0.1").is_err());
    assert!(UpstreamEndpoints::loopback("http://[::1]:8080").is_ok());
    assert!(UpstreamEndpoints::loopback("http://localhost").is_ok());
}
