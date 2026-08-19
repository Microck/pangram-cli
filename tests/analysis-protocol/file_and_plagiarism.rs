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
        .detect_file(request, &CancellationToken::new())
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
async fn analyzer_projects_a_terminal_plagiarism_analysis() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_plagiarism(Step::Json(plagiarism_success()));

    let analysis = Analyzer::from_client(fixture.client())
        .plagiarism(request(SYNTHETIC_TEXT), &CancellationToken::new())
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

    assert!(
        outcome.is_err(),
        "local cancellation stops the stalled wait"
    );
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
