//! Submission-path protocol tests: exact Pangram 4 request grammar, terminal
//! mapping, and the billable-submission cancellation boundary (F3).
//!
//! A `start_paused` runtime is not required here: these scenarios are
//! request/response boundaries proven by recorded fixture requests, not by
//! deadline timing.

use super::support::*;

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

/// 3b. A terminal provider message loaded with terminal control sequences,
/// non-ASCII bytes, and overlong content is reduced before it can appear in
/// canonical details: controls gone, non-ASCII scalars removed, hyperlinks
/// neutralized, and the prefix bounded. The detail surfaces in `Debug` and
/// serialized output carry only the reduced form, never raw provider text.
#[tokio::test(flavor = "current_thread")]
async fn terminal_failure_message_is_sanitized_before_canonical_details() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    let hostile = format!(
        "\u{1b}[31mFAIL\u{1b}[0m plain\u{7}\u{1b}]8;;https://evil.example\u{7}\\\u{3b4}\u{2603} {}",
        "x".repeat(400)
    );
    fixture.on_poll(Step::Json(pangram4_failure(&hostile)));

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

    let microck_pangram_cli::domain::Check::AiDetection(state) = &outcome.checks()[0] else {
        panic!("the only check is AI detection");
    };
    let microck_pangram_cli::domain::CheckState::Failed { error, .. } = state else {
        panic!("a terminal provider failure");
    };
    assert_eq!(error.code(), ErrorCode::UpstreamAnalysisFailed);
    let message = match error.details() {
        Some(microck_pangram_cli::output::CanonicalErrorDetails::Fields(fields)) => fields
            .get("upstream_message")
            .and_then(serde_json::Value::as_str)
            .expect("a sanitized upstream_message survives"),
        other => panic!("a provider failure carries field details, not {other:?}"),
    };
    assert!(
        message.chars().count() <= 200,
        "the retained message is bounded: {}",
        message.chars().count()
    );
    assert!(
        !message.chars().any(|ch| ch.is_ascii_control()),
        "no control character may survive sanitization"
    );
    assert!(
        message.is_ascii(),
        "non-ASCII scalars are removed: {message:?}"
    );
    assert!(
        !message.contains("\u{1b}")
            && !message.contains('\u{3b4}')
            && !message.contains('\u{2603}'),
        "raw provider sequences never cross the boundary"
    );
    // The reduced message is the only provider text anywhere in the
    // canonical error: no raw escape introducer survives into Debug or the
    // serialized form.
    for surface in [
        format!("{error:?}"),
        serde_json::to_string(error).expect("serializes"),
    ] {
        assert!(
            !surface.contains('\u{1b}'),
            "no escape introducer may surface: {surface:?}"
        );
        assert!(
            !surface.contains('\u{3b4}'),
            "non-ASCII is stripped: {surface:?}"
        );
    }
    assert_scrubbed(error);
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

/// 7b. Cancellation after the billable POST is issued but before acceptance
/// is ambiguous, never a definite "no remote action": the send may have
/// reached Pangram, so the canonical outcome is submission_outcome_unknown
/// with reconciliation identity. The POST is issued exactly once and never
/// replayed. Uses a multi-thread runtime because the hanging fixture must be
/// polled while `start` is in flight.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_after_issue_reports_ambiguous_acceptance() {
    let fixture = ProtocolFixture::start().await;
    // The submit hangs: it reaches the fixture and is recorded, but produces
    // no acceptance response.
    fixture.on_submit(Step::Hang);

    let analyzer = Analyzer::from_client(fixture.client());
    let stop = StopObserving::new();
    let cancel = stop.token().clone();
    let submit = tokio::spawn({
        let stop = stop.clone();
        async move { analyzer.start(request(SYNTHETIC_TEXT), stop.token()).await }
    });

    // Wait until the POST actually reaches the fixture: the send is
    // unambiguously issued, so cancellation now is post-issue.
    fixture.wait_for_posts(1).await;
    cancel.cancel();

    let error = submit
        .await
        .expect("the submit task completes")
        .expect_err("post-issue cancellation fails ambiguously");
    assert_eq!(error.error().code(), ErrorCode::SubmissionOutcomeUnknown);
    assert!(!error.error().retryable());
    let recovery = error.error().recovery().expect("fixed recovery");
    assert_eq!(
        recovery.message(),
        "A manual retry may create a second billable operation."
    );
    let details =
        serde_json::to_value(error.error().details().expect("details")).expect("details serialize");
    let payload = serde_json::to_string(&details).expect("details to string");
    assert!(payload.contains("anl_"), "{payload}");
    assert!(payload.contains("request_sha256"), "{payload}");
    assert!(payload.contains("last_status"), "{payload}");
    assert_eq!(
        fixture.post_count(),
        1,
        "the ambiguous send is never replayed: duplicate billing is forbidden"
    );
    assert_scrubbed(error.error());
    fixture.shutdown().await;
}

/// 7c. Cancellation before the submit is issued completes no remote action:
/// the stop token is already cancelled when `start` runs, so no POST is ever
/// sent and the outcome is the local-stop network error, not the ambiguous
/// acceptance. This pins the pre-issue boundary (F3).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_before_issue_completes_no_remote_action() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));

    let analyzer = Analyzer::from_client(fixture.client());
    let stop = StopObserving::new();
    // Cancel before `start` runs: the token is already tripped.
    stop.token().cancel();

    let result = analyzer.start(request(SYNTHETIC_TEXT), stop.token()).await;
    let error = result.expect_err("pre-issue cancellation stops submission");
    // Pre-issue cancellation is the definite local stop: no POST was issued,
    // so it is the network-unavailable local-stop code, not
    // submission_outcome_unknown.
    assert_eq!(error.error().code(), ErrorCode::NetworkUnavailable);
    assert_eq!(
        fixture.post_count(),
        0,
        "a pre-issue cancellation never sends the billable POST"
    );
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
