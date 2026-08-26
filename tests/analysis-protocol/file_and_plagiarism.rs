use tokio_util::sync::CancellationToken;

use microck_pangram_cli::analysis::CombinedAnalysisObservation;

use super::support::*;

#[tokio::test]
async fn file_submission_uses_the_verified_ordered_multipart_contract() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_file(Step::Json(serde_json::json!([
        file_success("sample.pdf", "first synthetic file"),
        file_success("sample.docx", "second synthetic file")
    ])));
    let files = vec![
        FileUpload::new("sample.pdf", FileFormat::Pdf, b"%PDF-1.4\n%%EOF".to_vec()).unwrap(),
        FileUpload::new("sample.docx", FileFormat::Docx, b"PK\x03\x04".to_vec()).unwrap(),
    ];

    let normalized = fixture
        .client()
        .submit_files(&files, &CancellationToken::new())
        .await
        .expect("verified file response normalizes");

    assert_eq!(normalized.len(), 2);
    assert_eq!(normalized[0].filename(), "sample.pdf");
    assert_eq!(normalized[1].filename(), "sample.docx");
    let requests = fixture.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/file");
    assert!(request.header_equals("x-api-key", SYNTHETIC_KEY));
    assert!(request.header_starts_with("content-type", "multipart/form-data; boundary="));
    let body = String::from_utf8_lossy(&request.body);
    assert_eq!(body.matches("name=\"files\"").count(), 2);
    assert!(body.contains("filename=\"sample.pdf\""));
    assert!(body.contains("Content-Type: application/pdf"));
    assert!(body.contains("filename=\"sample.docx\""));
    assert!(body.contains(
        "Content-Type: application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    ));
    assert!(body.contains("name=\"public_dashboard_link\""));
    assert!(body.contains("\r\n\r\nfalse\r\n"));
    assert!(!body.contains("name=\"model\""));
    fixture.shutdown().await;
}

#[tokio::test]
async fn file_response_order_or_shape_drift_is_rejected_without_a_retry() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_file(Step::Json(serde_json::json!([
        file_success("second.rtf", "second synthetic file"),
        file_success("first.rtf", "first synthetic file")
    ])));
    let files = vec![
        FileUpload::new("first.rtf", FileFormat::Rtf, b"{\\rtf1 first}".to_vec()).unwrap(),
        FileUpload::new("second.rtf", FileFormat::Rtf, b"{\\rtf1 second}".to_vec()).unwrap(),
    ];

    let outcome = fixture
        .client()
        .submit_files(&files, &CancellationToken::new())
        .await;

    assert!(outcome.is_err());
    assert_eq!(fixture.post_count(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn plagiarism_submission_uses_exact_json_and_numeric_sentence_count() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_plagiarism(Step::Json(plagiarism_success()));

    let result = fixture
        .client()
        .submit_plagiarism(SYNTHETIC_TEXT, &CancellationToken::new())
        .await
        .expect("verified plagiarism response normalizes");

    assert!(!result.plagiarism_detected);
    assert_eq!(result.plagiarized_sentence_count, 0);
    assert_eq!(result.total_sentences, 2);
    let requests = fixture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/plagiarism");
    assert!(requests[0].header_equals("x-api-key", SYNTHETIC_KEY));
    assert_eq!(
        requests[0].body_json(),
        serde_json::json!({"text": SYNTHETIC_TEXT})
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn plagiarism_list_sentence_count_is_contract_drift_without_a_retry() {
    let fixture = ProtocolFixture::start().await;
    let mut response = plagiarism_success();
    response["plagiarized_sentences"] = serde_json::json!([]);
    fixture.on_plagiarism(Step::Json(response));

    let outcome = fixture
        .client()
        .submit_plagiarism(SYNTHETIC_TEXT, &CancellationToken::new())
        .await;

    assert!(outcome.is_err());
    assert_eq!(fixture.post_count(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn analyzer_projects_a_terminal_file_analysis_without_task_identity() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_file(Step::Json(serde_json::json!([file_success(
        "sample.rtf",
        "synthetic extracted text"
    )])));
    let upload = FileUpload::new(
        "sample.rtf",
        FileFormat::Rtf,
        b"{\\rtf1 synthetic}".to_vec(),
    )
    .unwrap();
    let request = FileAnalysisRequest::new(upload, Some("/tmp/sample.rtf".to_owned()), true);

    let analysis = Analyzer::from_client(fixture.client())
        .detect_file(request, WaitOptions::UNBOUNDED, &CancellationToken::new())
        .await
        .expect("file analysis succeeds");

    assert_eq!(analysis.status(), AnalysisStatus::Succeeded);
    assert!(analysis.provenance().upstream_task_ids.is_none());
    assert_eq!(
        analysis.provenance().upstream_version.as_deref(),
        Some("4.0")
    );
    let Some(AnalysisInput::File(input)) = analysis.input() else {
        panic!("file input expected");
    };
    assert_eq!(input.path.as_deref(), Some("/tmp/sample.rtf"));
    assert_eq!(
        input.extracted_text.as_deref(),
        Some("synthetic extracted text")
    );
    assert!(matches!(
        &analysis.checks()[0],
        Check::AiDetection(CheckState::Succeeded { .. })
    ));
    fixture.shutdown().await;
}

#[tokio::test]
async fn file_timeout_bounds_a_post_issue_synchronous_wait() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_file(Step::Hang);
    let analyzer = Analyzer::from_client(fixture.client_with_policy(
        RetryPolicy::OFF,
        PollPolicy::new(Duration::ZERO, Duration::ZERO),
        Duration::from_secs(10),
    ));
    let upload = FileUpload::new(
        "sample.rtf",
        FileFormat::Rtf,
        b"{\\rtf1 synthetic}".to_vec(),
    )
    .unwrap();

    let error = tokio::time::timeout(
        Duration::from_secs(5),
        analyzer.detect_file(
            FileAnalysisRequest::new(upload, Some("/tmp/sample.rtf".to_owned()), false),
            WaitOptions::with_timeout(Duration::from_millis(500)),
            &CancellationToken::new(),
        ),
    )
    .await
    .expect("the local deadline must win over the transport timeout")
    .expect_err("the local deadline stops the stalled file response wait");

    assert_eq!(error.error().code(), ErrorCode::SubmissionOutcomeUnknown);
    assert_eq!(fixture.post_count(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn analyzer_projects_a_terminal_plagiarism_analysis() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_plagiarism(Step::Json(plagiarism_success()));

    let analysis = Analyzer::from_client(fixture.client())
        .plagiarism(
            request(SYNTHETIC_TEXT),
            WaitOptions::UNBOUNDED,
            &CancellationToken::new(),
        )
        .await
        .expect("plagiarism analysis succeeds");

    assert_eq!(analysis.status(), AnalysisStatus::Succeeded);
    assert!(analysis.provenance().upstream_task_ids.is_none());
    assert!(matches!(
        &analysis.checks()[0],
        Check::Plagiarism(CheckState::Succeeded { .. })
    ));
    assert_eq!(fixture.post_count(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn plagiarism_timeout_bounds_a_post_issue_synchronous_wait() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_plagiarism(Step::Hang);
    let analyzer = Analyzer::from_client(fixture.client_with_policy(
        RetryPolicy::OFF,
        PollPolicy::new(Duration::ZERO, Duration::ZERO),
        Duration::from_secs(10),
    ));

    let error = tokio::time::timeout(
        Duration::from_secs(5),
        analyzer.plagiarism(
            request(SYNTHETIC_TEXT),
            WaitOptions::with_timeout(Duration::from_millis(500)),
            &CancellationToken::new(),
        ),
    )
    .await
    .expect("the local deadline must win over the transport timeout")
    .expect_err("the local deadline stops the stalled response wait");

    assert_eq!(error.error().code(), ErrorCode::SubmissionOutcomeUnknown);
    assert_eq!(fixture.post_count(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn plagiarism_timeout_before_issue_sends_no_second_request() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_plagiarism(Step::Json(plagiarism_success()));
    let client = fixture.client_with_policy(
        RetryPolicy::OFF,
        PollPolicy::new(Duration::ZERO, Duration::ZERO),
        Duration::from_secs(2),
    );
    client
        .submit_plagiarism(SYNTHETIC_TEXT, &CancellationToken::new())
        .await
        .expect("the first request reserves the immediate pacing slot");
    let analyzer = Analyzer::from_client(client);

    let error = analyzer
        .plagiarism(
            request(SYNTHETIC_TEXT),
            WaitOptions::with_timeout(Duration::from_millis(50)),
            &CancellationToken::new(),
        )
        .await
        .expect_err("the deadline passes before the next pacing slot opens");

    assert_eq!(error.error().code(), ErrorCode::WaitTimeout);
    assert_eq!(
        fixture.post_count(),
        1,
        "a pre-issue timeout must not send another billable request"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn combined_analysis_shares_one_deadline_with_stalled_plagiarism() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({ "task_id": TASK_ID })));
    fixture.on_poll(Step::Json(pangram4_success(SYNTHETIC_TEXT)));
    fixture.on_plagiarism(Step::Hang);
    let analyzer = Analyzer::from_client(fixture.client_with_policy(
        RetryPolicy::OFF,
        PollPolicy::new(Duration::ZERO, Duration::ZERO),
        Duration::from_secs(10),
    ));

    let analysis = tokio::time::timeout(
        Duration::from_secs(5),
        analyzer.analyze_combined(
            request(SYNTHETIC_TEXT),
            WaitOptions::with_timeout(Duration::from_millis(500)),
            |_| {},
            StopObserving::new(),
        ),
    )
    .await
    .expect("the shared local deadline must win over the transport timeout")
    .expect("a deadline is not a user interruption")
    .expect("combined deadline failures remain canonical checks");

    assert!(
        analysis.checks().iter().any(|check| matches!(
            check,
            Check::Plagiarism(CheckState::Failed { error, .. })
                if error.code() == ErrorCode::SubmissionOutcomeUnknown
        )),
        "the issued stalled plagiarism request remains ambiguous"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn combined_analysis_preserves_ai_success_when_plagiarism_fails() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({ "task_id": TASK_ID })));
    fixture.on_poll(Step::Json(pangram4_success(SYNTHETIC_TEXT)));
    fixture.on_plagiarism(Step::Status(402, None, None));
    let analyzer = Analyzer::from_client(fixture.client());

    let analysis = analyzer
        .analyze_combined(
            request(SYNTHETIC_TEXT),
            WaitOptions::UNBOUNDED,
            |_| {},
            StopObserving::new(),
        )
        .await
        .expect("observation is not interrupted")
        .expect("combined analysis returns its partial result");

    assert_eq!(analysis.status(), AnalysisStatus::Partial);
    assert_eq!(analysis.checks().len(), 2);
    assert!(matches!(
        &analysis.checks()[0],
        Check::AiDetection(CheckState::Succeeded { .. })
    ));
    let Check::Plagiarism(CheckState::Failed { error, .. }) = &analysis.checks()[1] else {
        panic!("plagiarism failure expected");
    };
    assert_eq!(error.code(), ErrorCode::PaymentRequired);
    let requests = fixture.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/task")
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/task/task-123")
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/plagiarism")
            .count(),
        1
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn combined_analysis_reports_acceptance_before_first_progress() {
    #[derive(Debug, PartialEq, Eq)]
    enum Observation {
        Accepted(AnalysisStatus),
        Progress(String),
    }

    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({ "task_id": TASK_ID })));
    fixture.on_poll(Step::Json(serde_json::json!({
        "task_id": TASK_ID,
        "stage": "STAGE_PREPROCESSING"
    })));
    fixture.on_poll(Step::Json(pangram4_success(SYNTHETIC_TEXT)));
    fixture.on_plagiarism(Step::Json(plagiarism_success()));
    let analyzer = Analyzer::from_client(fixture.client_with_policy(
        RetryPolicy::OFF,
        PollPolicy::new(Duration::ZERO, Duration::ZERO),
        Duration::from_secs(2),
    ));
    let observations = std::cell::RefCell::new(Vec::new());

    let analysis = analyzer
        .analyze_combined(
            request(SYNTHETIC_TEXT),
            WaitOptions::UNBOUNDED,
            |observation| match observation {
                CombinedAnalysisObservation::Accepted(running) => observations
                    .borrow_mut()
                    .push(Observation::Accepted(running.snapshot().status())),
                CombinedAnalysisObservation::Progress(progress) => {
                    observations.borrow_mut().push(Observation::Progress(
                        progress.last_stage.as_str().to_owned(),
                    ));
                }
            },
            StopObserving::new(),
        )
        .await
        .expect("observation is not interrupted")
        .expect("combined analysis succeeds");

    assert_eq!(analysis.status(), AnalysisStatus::Succeeded);
    assert_eq!(
        observations.into_inner(),
        [
            Observation::Accepted(AnalysisStatus::Queued),
            Observation::Progress("STAGE_PREPROCESSING".to_owned()),
        ]
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn combined_analysis_submits_plagiarism_while_ai_observation_is_stalled() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({ "task_id": TASK_ID })));
    fixture.on_poll(Step::Hang);
    fixture.on_plagiarism(Step::Json(plagiarism_success()));
    let analyzer = Analyzer::from_client(fixture.client_with_policy(
        RetryPolicy::OFF,
        PollPolicy::new(Duration::ZERO, Duration::ZERO),
        Duration::from_secs(2),
    ));
    let stop = StopObserving::new();
    let analysis_stop = stop.clone();

    let (outcome, ()) = tokio::join!(
        analyzer.analyze_combined(
            request(SYNTHETIC_TEXT),
            WaitOptions::UNBOUNDED,
            |_| {},
            analysis_stop,
        ),
        async {
            tokio::time::timeout(Duration::from_millis(250), fixture.wait_for_posts(2))
                .await
                .expect("both billable submissions happen before the stalled poll times out");
            stop.stop();
        }
    );

    // The fixture records a POST before its response reaches the client. The
    // stop can therefore win either before or after the plagiarism response
    // is acknowledged. Both outcomes must stop the combined analysis, and an
    // unacknowledged issued request must keep its reconciliation error.
    match outcome {
        Err(_) => {}
        Ok(Err(error)) => assert_eq!(
            error.error().code(),
            ErrorCode::SubmissionOutcomeUnknown,
            "only an ambiguous issued submission may outrank interruption"
        ),
        Ok(Ok(_)) => panic!("local cancellation must not assemble a completed analysis"),
    }
    let requests = fixture.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.path == "/plagiarism")
            .count(),
        1
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn combined_cancellation_preserves_an_ambiguous_plagiarism_submission() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({ "task_id": TASK_ID })));
    fixture.on_poll(Step::Hang);
    fixture.on_plagiarism(Step::Hang);
    let analyzer = Analyzer::from_client(fixture.client_with_policy(
        RetryPolicy::OFF,
        PollPolicy::new(Duration::ZERO, Duration::ZERO),
        Duration::from_secs(10),
    ));
    let stop = StopObserving::new();
    let analysis_stop = stop.clone();

    let (outcome, ()) = tokio::join!(
        analyzer.analyze_combined(
            request(SYNTHETIC_TEXT),
            WaitOptions::UNBOUNDED,
            |_| {},
            analysis_stop,
        ),
        async {
            tokio::time::timeout(Duration::from_secs(5), fixture.wait_for_posts(2))
                .await
                .expect("both billable submissions are issued before cancellation");
            stop.stop();
        }
    );

    let error = outcome
        .expect("submission ambiguity takes precedence over interruption")
        .expect_err("the unacknowledged plagiarism response remains ambiguous");
    assert_eq!(error.error().code(), ErrorCode::SubmissionOutcomeUnknown);
    let payload = serde_json::to_string(error.error()).unwrap();
    assert!(payload.contains("request_sha256"), "{payload}");
    assert!(payload.contains("last_status"), "{payload}");
    assert_eq!(fixture.post_count(), 2);
    fixture.shutdown().await;
}

#[tokio::test]
async fn combined_analysis_preserves_plagiarism_success_when_ai_submission_fails() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Status(401, None, None));
    fixture.on_plagiarism(Step::Json(plagiarism_success()));

    let analysis = Analyzer::from_client(fixture.client())
        .analyze_combined(
            request(SYNTHETIC_TEXT),
            WaitOptions::UNBOUNDED,
            |_| {},
            StopObserving::new(),
        )
        .await
        .expect("observation is not interrupted")
        .expect("combined analysis retains the successful plagiarism check");

    assert_eq!(analysis.status(), AnalysisStatus::Partial);
    let Check::AiDetection(CheckState::Failed { error, .. }) = &analysis.checks()[0] else {
        panic!("AI-detection failure expected");
    };
    assert_eq!(error.code(), ErrorCode::InvalidApiKey);
    assert!(matches!(
        &analysis.checks()[1],
        Check::Plagiarism(CheckState::Succeeded { .. })
    ));
    assert_eq!(fixture.post_count(), 2);
    fixture.shutdown().await;
}
