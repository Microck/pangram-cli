//! Contract-matrix protocol tests: the HTTP failure mapping, bounded
//! safe-GET retries, upstream document validation, and endpoint hygiene.

use super::support::*;

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
        (501, ErrorCode::UpstreamError, false),
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
        cumulative_retry_budget: None,
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
        cumulative_retry_budget: None,
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
