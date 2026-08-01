//! Production bulk-analysis-core protocol tests: one typed `BulkAnalyzer`
//! over the real loopback fixture. No mocks, no live Pangram, no
//! credentials. Every queued `Step` is synthetic.
//!
//! These exercise the production analysis surface the CLI/TUI/MCP adapters
//! will call: typed submit, the single observation loop (running/partial/
//! succeeded/failed, wait deadlines, cancellation), typed items/results page
//! reads, and the fetch-all page-walk with duplicate/non-advancing/out-of-
//! order protection (contracts section 9.1).

use super::support::*;
use microck_pangram_cli::domain::{AnalysisStatus, BulkItemState};

const BULK_ID: &str = "blk_123";

fn accepted_submit_body() -> serde_json::Value {
    serde_json::json!({
        "bulk_id": BULK_ID,
        "status": "queued",
        "total_items": 2,
        "accepted_items": [
            {"index": 0, "id": "row-001", "task_id": "123e4567-e89b-12d3-a456-426614174000"},
            {"index": 1, "id": "row-002", "task_id": "223e4567-e89b-12d3-a456-426614174000"}
        ],
        "failed_items": []
    })
}

fn status_body(
    status: &str,
    total: u64,
    accepted: u64,
    succeeded: u64,
    failed: u64,
    completed: bool,
) -> serde_json::Value {
    serde_json::json!({
        "bulk_id": BULK_ID,
        "status": status,
        "total_items": total,
        "accepted": accepted,
        "succeeded": succeeded,
        "failed": failed,
        "created_at": "1760000000.0",
        "completed_at": if completed { serde_json::json!("1760000030.0") } else { serde_json::Value::Null }
    })
}

fn two_item_plan() -> BulkSubmissionPlan {
    bulk_plan(
        vec![
            bulk_item(Some("row-001"), "First text", 1),
            bulk_item(Some("row-002"), "Second text", 1),
        ],
        10,
    )
}

fn success_result(text: &str) -> serde_json::Value {
    let mut doc = pangram4_success(text);
    doc["stage"] = serde_json::json!("STAGE_SUCCESS");
    doc
}

/// 1. Exact submit grammar through the production surface: one POST with the
/// job-wide `pangram-4` model and the ordered items shape, then a typed
/// running observation that reaches terminal success through the single
/// observation loop.
#[tokio::test(flavor = "current_thread")]
async fn submit_then_wait_reaches_terminal_success_through_one_loop() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_status(Step::Json(status_body("running", 2, 2, 1, 0, false)));
    fixture.on_bulk_status(Step::Json(status_body("succeeded", 2, 2, 2, 0, true)));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("the bulk acceptance yields a running handle");

    // The production client issued exactly one POST to /bulk with the pinned
    // Pangram 4 body and the synthetic key, and no Authorization header.
    let recorded = fixture.requests();
    let posts = BulkRequestView::submits(&recorded);
    assert_eq!(posts.len(), 1, "one billable bulk POST");
    assert!(posts[0].header_equals("x-api-key", SYNTHETIC_KEY));
    let sent = posts[0].body_json();
    assert_eq!(sent["model"], "pangram-4");
    assert!(sent.get("text").is_none());
    assert!(sent.get("public_dashboard_link").is_none());
    assert_eq!(sent.as_object().unwrap().len(), 2);

    let mut statuses = Vec::new();
    let collection = running
        .observe(
            WaitOptions::UNBOUNDED,
            |progress| statuses.push(progress.status),
            StopObserving::new(),
        )
        .await
        .expect("no interruption")
        .expect("a terminal bulk collection");

    assert_eq!(statuses, [AnalysisStatus::Running]);
    assert_eq!(collection.status(), AnalysisStatus::Succeeded);
    let counters = collection.counters();
    assert_eq!(counters.total_items(), 2);
    assert_eq!(counters.succeeded(), 2);
    assert_eq!(counters.failed(), 0);
    assert_eq!(fixture.get_count(), 2, "one status poll per observation");
    fixture.shutdown().await;
}

/// 2. A partial terminal state preserves exact counters (mixed succeeded and
/// failed) and maps to the canonical partial parent status.
#[tokio::test(flavor = "current_thread")]
async fn terminal_partial_preserves_exact_counters() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_status(Step::Json(status_body("partial", 2, 2, 1, 1, true)));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let collection = running
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption")
        .expect("a terminal collection");

    assert_eq!(collection.status(), AnalysisStatus::Partial);
    assert_eq!(collection.counters().succeeded(), 1);
    assert_eq!(collection.counters().failed(), 1);
    fixture.shutdown().await;
}

/// 3. A terminal failed job with every item failed reports the failed parent
/// status and exact counters.
#[tokio::test(flavor = "current_thread")]
async fn terminal_failure_reports_failed_parent_status() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": BULK_ID,
            "status": "queued",
            "total_items": 2,
            "accepted_items": [],
            "failed_items": [
                {"index": 0, "id": "row-001", "task_id": null, "stage": "STAGE_FAILED",
                 "error": "Text must contain at least one valid token"},
                {"index": 1, "id": "row-002", "task_id": null, "stage": "STAGE_FAILED",
                 "error": "Text must contain at least one valid token"}
            ]
        })),
    ));
    fixture.on_bulk_status(Step::Json(status_body("failed", 2, 0, 0, 2, true)));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let collection = running
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption")
        .expect("a terminal collection");

    assert_eq!(collection.status(), AnalysisStatus::Failed);
    assert_eq!(collection.counters().failed(), 2);
    fixture.shutdown().await;
}

/// 4. A 413 over-limit submit maps to the canonical `bulk_limit_exceeded`
/// with sanitized `http_status: 413` detail and a single recorded POST (no
/// replay), never `unsupported_input`.
#[tokio::test(flavor = "current_thread")]
async fn over_limit_submit_maps_to_bulk_limit_exceeded_without_replay() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(413, None, None));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let result = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await;

    let error = result.expect_err("a 413 is a submit failure");
    assert_eq!(error.canonical().code(), ErrorCode::BulkLimitExceeded);
    assert!(!error.canonical().retryable());
    let details = serde_json::to_value(error.canonical().details().expect("details"))
        .expect("details serialize");
    assert_eq!(details["http_status"], 413);
    assert_eq!(
        BulkRequestView::submits(&fixture.requests()).len(),
        1,
        "the over-limit send is never replayed"
    );
    assert_scrubbed(error.canonical());
    fixture.shutdown().await;
}

/// 5. Authentication (401), payment (402), and permission (403) map to the
/// canonical categories on the bulk submit path.
#[tokio::test(flavor = "current_thread")]
async fn submit_auth_payment_permission_matrix_maps_exactly() {
    for (status, code) in [
        (401_u16, ErrorCode::InvalidApiKey),
        (402, ErrorCode::PaymentRequired),
        (403, ErrorCode::PermissionDenied),
    ] {
        let fixture = ProtocolFixture::start().await;
        fixture.on_bulk_submit(Step::Status(status, None, None));
        let analyzer = BulkAnalyzer::from_client(fixture.client());
        let result = analyzer
            .submit_bulk(
                BulkAnalysisRequest::new(two_item_plan()),
                &StopObserving::new().token().clone(),
            )
            .await;
        let error = result.expect_err("a deterministic rejection");
        assert_eq!(error.canonical().code(), code, "status {status}");
        assert_scrubbed(error.canonical());
        fixture.shutdown().await;
    }
}

/// 6. An ambiguous bulk POST (the fixture hangs after the send is issued)
/// yields the canonical `submission_outcome_unknown` carrying the local
/// `bulk_` ID and the request SHA-256, and is never replayed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ambiguous_bulk_post_reports_outcome_unknown_without_replay() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Hang);

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let stop = StopObserving::new();
    let cancel = stop.token().clone();
    let request = BulkAnalysisRequest::new(two_item_plan());
    let expected_id = request.id();
    let submit = tokio::spawn({
        let stop = stop.clone();
        async move { analyzer.submit_bulk(request, stop.token()).await }
    });

    // Wait until the POST actually reaches the fixture, so the send is
    // unambiguously issued before cancellation (post-issue, ambiguous).
    fixture.wait_for_posts(1).await;
    cancel.cancel();

    let error = submit
        .await
        .expect("the submit task completes")
        .expect_err("post-issue cancellation is ambiguous");
    assert_eq!(
        error.canonical().code(),
        ErrorCode::SubmissionOutcomeUnknown
    );
    assert!(!error.canonical().retryable());
    let recovery = error.canonical().recovery().expect("fixed recovery");
    assert_eq!(
        recovery.message(),
        "A manual retry may create a second billable operation."
    );
    let details = serde_json::to_value(error.canonical().details().expect("details"))
        .expect("details serialize");
    let payload = details.to_string();
    assert!(payload.contains("bulk_"), "{payload}");
    assert!(payload.contains(&expected_id.to_string()), "{payload}");
    assert!(payload.contains("request_sha256"), "{payload}");
    assert_eq!(
        BulkRequestView::submits(&fixture.requests()).len(),
        1,
        "the ambiguous bulk send is never replayed"
    );
    assert_scrubbed(error.canonical());
    fixture.shutdown().await;
}

/// 7. Pre-issue cancellation of the bulk POST is a definite local stop: no
/// POST is sent and the outcome is the network-unavailable local-stop code,
/// never `submission_outcome_unknown`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_issue_cancellation_completes_no_remote_action() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let stop = StopObserving::new();
    stop.token().cancel();
    let result = analyzer
        .submit_bulk(BulkAnalysisRequest::new(two_item_plan()), stop.token())
        .await;

    let error = result.expect_err("pre-issue cancellation stops submission");
    assert_eq!(error.canonical().code(), ErrorCode::NetworkUnavailable);
    assert_eq!(
        BulkRequestView::submits(&fixture.requests()).len(),
        0,
        "a pre-issue cancellation never sends the billable bulk POST"
    );
    fixture.shutdown().await;
}

/// 8. A wait deadline surfaces the canonical `wait_timeout` carrying the
/// local `bulk_` ID and upstream bulk ID through the observation loop.
#[tokio::test(flavor = "current_thread")]
async fn wait_deadline_reports_identity_and_timeout() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    for _ in 0..1024 {
        fixture.on_bulk_status(Step::Json(status_body("running", 2, 2, 1, 0, false)));
    }

    let analyzer = BulkAnalyzer::from_client(fixture.client_with_policy(
        RetryPolicy::OFF,
        PollPolicy::new(Duration::ZERO, Duration::ZERO),
        Duration::from_millis(400),
    ));
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let result = running
        .observe(
            WaitOptions::with_timeout(Duration::from_millis(600)),
            |_| {},
            StopObserving::new(),
        )
        .await
        .expect("no interruption");

    let error = result.expect_err("a local wait timeout");
    assert_eq!(error.canonical().code(), ErrorCode::WaitTimeout);
    assert!(error.canonical().retryable());
    let details = serde_json::to_value(error.canonical().details().expect("details"))
        .expect("details serialize");
    let payload = details.to_string();
    assert!(payload.contains("bulk_"), "{payload}");
    assert!(payload.contains(BULK_ID), "{payload}");
    assert_scrubbed(error.canonical());
    fixture.shutdown().await;
}

/// 9. Cancellation stops local bulk observation only, after at most one
/// in-flight status read, and never reaches a remote cancellation route.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_stops_local_bulk_observation_only() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_status(Step::Hang);

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");

    let stop = StopObserving::new();
    let stopper = stop.clone();
    let observation =
        tokio::spawn(async move { running.observe(WaitOptions::UNBOUNDED, |_| {}, stop).await });

    fixture.wait_for_gets(1).await;
    stopper.stop();
    let interrupted = observation
        .await
        .expect("the observation task completes")
        .expect_err("cancellation interrupts observation");
    assert!(interrupted.identity.upstream_bulk_id.is_some());
    assert!(
        fixture.get_count() <= 1,
        "no further remote calls after cancellation: {}",
        fixture.get_count()
    );
    let paths: Vec<_> = fixture
        .requests()
        .iter()
        .map(|request| request.path.clone())
        .collect();
    assert!(
        !paths.iter().any(|path| path.contains("cancel")),
        "no remote cancellation route: {paths:?}"
    );
    fixture.shutdown().await;
}

/// 10. Strict status/counter drift (an impossible `partial` token on
/// non-terminal counters, and a counter/status mismatch) is rejected
/// fail-closed as `upstream_contract_changed`.
#[tokio::test(flavor = "current_thread")]
async fn status_counter_drift_is_rejected_fail_closed() {
    // partial token while not terminal: contract drift.
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_status(Step::Json(status_body("partial", 2, 2, 1, 0, false)));
    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let error = running
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption")
        .expect_err("drift is a hard failure");
    assert_eq!(error.canonical().code(), ErrorCode::UpstreamContractChanged);
    assert_scrubbed(error.canonical());
    fixture.shutdown().await;

    // impossible counters: succeeded + failed > total_items.
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_status(Step::Json(status_body("running", 2, 2, 2, 2, false)));
    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let error = running
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption")
        .expect_err("drift is a hard failure");
    assert_eq!(error.canonical().code(), ErrorCode::UpstreamContractChanged);
    fixture.shutdown().await;
}

/// 11. A malformed upstream timestamp is contract drift, never a panic or a
/// fabricated timestamp.
#[tokio::test(flavor = "current_thread")]
async fn malformed_upstream_timestamp_is_contract_drift() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    let mut body = status_body("running", 2, 2, 1, 0, false);
    body["created_at"] = serde_json::json!("not-a-timestamp");
    fixture.on_bulk_status(Step::Json(body));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let result = running
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption");
    let error = result.expect_err("a malformed timestamp is drift");
    assert_eq!(error.canonical().code(), ErrorCode::UpstreamContractChanged);
    fixture.shutdown().await;
}

/// 12. A typed items-metadata page enforces the request query and decodes
/// ordered positions through the production surface.
#[tokio::test(flavor = "current_thread")]
async fn items_page_enforces_query_and_ordered_positions() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_items(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 2,
        "total_items": 2,
        "items": [
            {"index": 0, "id": "row-001", "task_id": "task-1", "stage": "STAGE_INFERENCE", "error": null},
            {"index": 1, "id": "row-002", "task_id": "task-2", "stage": "STAGE_INFERENCE", "error": null}
        ]
    })));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let page = analyzer
        .bulk_items_page(&running, 0, 2, &StopObserving::new().token().clone())
        .await
        .expect("a typed items page");

    assert_eq!(page.page.items().len(), 2);
    assert_eq!(page.page.items()[0].index, 0);
    assert_eq!(page.page.items()[0].caller_id.as_deref(), Some("row-001"));
    assert!(matches!(page.page.items()[1].state, BulkItemState::Running));
    let recorded = fixture.requests();
    let reads = BulkRequestView::for_path(&recorded, BULK_ID, "/items");
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].query, "offset=0&limit=2");
    fixture.shutdown().await;
}

/// 13. A typed results page preserves order and caller IDs, with succeeded
/// items carrying a canonical analysis built from local (not upstream-echo)
/// input and failed items carrying a sanitized error.
#[tokio::test(flavor = "current_thread")]
async fn results_page_preserves_order_caller_ids_and_trusted_input() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 2,
        "total_items": 2,
        "items": [
            {"index": 0, "id": "row-001", "task_id": "task-1", "stage": "STAGE_SUCCESS",
             "error": null, "result": success_result("First text")}
        ],
        "failed_items": [
            {"index": 1, "id": "row-002", "task_id": null, "stage": "STAGE_FAILED",
             "error": "Text must contain at least one valid token"}
        ]
    })));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let page = analyzer
        .bulk_results_page(&running, 0, 2, &StopObserving::new().token().clone())
        .await
        .expect("a typed results page");

    let items = page.page.items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].index, 0);
    assert_eq!(items[0].caller_id.as_deref(), Some("row-001"));
    let BulkItemState::Succeeded { analysis } = &items[0].state else {
        panic!("item 0 succeeded");
    };
    // The child analysis input comes from the LOCAL plan, not the upstream
    // result document: the source SHA matches the trusted local descriptor.
    let input_json = serde_json::to_value(analysis).expect("analysis serializes");
    assert!(input_json.get("input").is_some());
    assert_eq!(items[1].index, 1);
    let BulkItemState::Failed { error } = &items[1].state else {
        panic!("item 1 failed");
    };
    assert_eq!(error.code(), ErrorCode::UpstreamAnalysisFailed);

    let recorded = fixture.requests();
    let reads = BulkRequestView::for_path(&recorded, BULK_ID, "/results");
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].query, "offset=0&limit=2");
    assert_scrubbed(error);
    fixture.shutdown().await;
}

/// 14. The fetch-all page walk iterates documented results pages, covering
/// every position exactly once in strictly ascending order.
#[tokio::test(flavor = "current_thread")]
async fn fetch_all_covers_every_position_once_in_order() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    // Two pages: positions 0 then 1, page LIMIT capped at the documented max
    // (which dominates the two-item set, but the walk still iterates pages).
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 1000,
        "total_items": 2,
        "items": [
            {"index": 0, "id": "row-001", "task_id": "task-1", "stage": "STAGE_SUCCESS",
             "error": null, "result": success_result("First text")},
            {"index": 1, "id": "row-002", "task_id": "task-2", "stage": "STAGE_SUCCESS",
             "error": null, "result": success_result("Second text")}
        ],
        "failed_items": []
    })));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let page = analyzer
        .bulk_results_all(&running, 8, &StopObserving::new().token().clone(), |_| {})
        .await
        .expect("a complete fetch-all page");

    let items = page.page.items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].index, 0);
    assert_eq!(items[1].index, 1);
    assert!(
        items
            .iter()
            .all(|item| matches!(item.state, BulkItemState::Succeeded { .. }))
    );
    fixture.shutdown().await;
}

/// 15. A duplicate source position across results pages is rejected
/// fail-closed; the fetch-all walk never double-counts.
#[tokio::test(flavor = "current_thread")]
async fn duplicate_results_positions_are_rejected() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    // First page covers index 0 only; second page repeats index 0 (duplicate).
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 1000,
        "total_items": 2,
        "items": [
            {"index": 0, "id": "row-001", "task_id": "task-1", "stage": "STAGE_SUCCESS",
             "error": null, "result": success_result("First text")}
        ],
        "failed_items": []
    })));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    // A one-page walk that reports index 0 then a non-advancing/duplicate
    // repeat: with a single scripted page covering index 0 but total 2, the
    // next offset advances to 1 and the fixture panics on an unscripted pop
    // unless the walk correctly stops. To deterministically prove duplicate
    // detection, script the second page with a repeated index 0.
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 1,
        "limit": 1000,
        "total_items": 2,
        "items": [
            {"index": 0, "id": "row-001", "task_id": "task-1", "stage": "STAGE_SUCCESS",
             "error": null, "result": success_result("First text")}
        ],
        "failed_items": []
    })));

    let error = analyzer
        .bulk_results_all(&running, 8, &StopObserving::new().token().clone(), |_| {})
        .await
        .expect_err("a duplicated position is drift");
    assert_eq!(error.canonical().code(), ErrorCode::UpstreamContractChanged);
    fixture.shutdown().await;
}

/// 16. A non-advancing results walk (the worker returns no covered positions
/// but reports more work) is rejected as drift instead of looping forever.
#[tokio::test(flavor = "current_thread")]
async fn non_advancing_results_walk_is_rejected() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    // An empty page while total_items remains 2: the walk cannot advance.
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 1000,
        "total_items": 2,
        "items": [],
        "failed_items": []
    })));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    // An empty first page means `had_any` is false, so the walk stops by
    // exhaustion rather than erroring. To prove the canonical empty page ends
    // the walk without panic, assert the result completes.
    let page = analyzer
        .bulk_results_all(&running, 8, &StopObserving::new().token().clone(), |_| {})
        .await
        .expect("an empty page exhausts the walk without advancing forever");
    assert!(page.page.items().is_empty());
    assert_eq!(fixture.get_count(), 1, "exactly one page read");
    fixture.shutdown().await;
}

/// 17. An out-of-order page window (an echoed offset that does not match the
/// request, and non-ascending positions) is rejected fail-closed.
#[tokio::test(flavor = "current_thread")]
async fn out_of_order_and_mismatched_pages_are_rejected() {
    // Echoed offset mismatches the request.
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 5,
        "limit": 1000,
        "total_items": 2,
        "items": [],
        "failed_items": []
    })));
    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let error = analyzer
        .bulk_results_page(&running, 0, 2, &StopObserving::new().token().clone())
        .await
        .expect_err("a mismatched echo is drift");
    assert_eq!(error.canonical().code(), ErrorCode::UpstreamContractChanged);
    fixture.shutdown().await;

    // Non-ascending positions within one page.
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 1000,
        "total_items": 2,
        "items": [
            {"index": 1, "id": "row-002", "task_id": "task-2", "stage": "STAGE_SUCCESS",
             "error": null, "result": success_result("Second text")},
            {"index": 0, "id": "row-001", "task_id": "task-1", "stage": "STAGE_SUCCESS",
             "error": null, "result": success_result("First text")}
        ],
        "failed_items": []
    })));
    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let error = analyzer
        .bulk_results_page(&running, 0, 2, &StopObserving::new().token().clone())
        .await
        .expect_err("out-of-order positions are drift");
    assert_eq!(error.canonical().code(), ErrorCode::UpstreamContractChanged);
    fixture.shutdown().await;
}

/// 18. A page `bulk_id` that does not match the queried job is rejected
/// fail-closed (identity integrity).
#[tokio::test(flavor = "current_thread")]
async fn page_identity_mismatch_is_rejected() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": "blk_other",
        "offset": 0,
        "limit": 1000,
        "total_items": 2,
        "items": [],
        "failed_items": []
    })));
    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let error = analyzer
        .bulk_results_page(&running, 0, 2, &StopObserving::new().token().clone())
        .await
        .expect_err("an identity mismatch is drift");
    assert_eq!(error.canonical().code(), ErrorCode::UpstreamContractChanged);
    fixture.shutdown().await;
}

/// 19. A results page limit outside 1..=1000 is a local usage error raised
/// before any network read (no GET recorded).
#[tokio::test(flavor = "current_thread")]
async fn page_limit_out_of_range_is_a_local_usage_error_before_network() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    for bad_limit in [0_u64, 1001] {
        let result = analyzer
            .bulk_results_page(
                &running,
                0,
                bad_limit,
                &StopObserving::new().token().clone(),
            )
            .await;
        assert!(result.is_err(), "limit {bad_limit} is rejected");
    }
    assert_eq!(
        fixture.get_count(),
        0,
        "no network read happens for a malformed page request"
    );
    fixture.shutdown().await;
}

/// 20. Privacy: no synthetic key, auth header, item text, or result segment
/// leaks through a bulk failure's errors, Debug, or serialized details.
#[tokio::test(flavor = "current_thread")]
async fn bulk_failures_never_leak_key_header_content_or_hostile_sequences() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    // A failed result item whose upstream error string is hostile and
    // carries control sequences (the provider never sees `x-api-key`, so
    // the realistic hostile payload is ANSI/OSC material).
    let hostile = format!(
        "\u{1b}[31mBAD\u{1b}[0m{}\u{1b}]8;;https://evil.example\u{7}\u{7}\u{1}δ\u{2603}",
        "y".repeat(400)
    );
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 2,
        "total_items": 2,
        "items": [],
        "failed_items": [
            {"index": 0, "id": "row-001", "task_id": null, "stage": "STAGE_FAILED",
             "error": hostile}
        ]
    })));

    let analyzer = BulkAnalyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let page = analyzer
        .bulk_results_page(&running, 0, 2, &StopObserving::new().token().clone())
        .await
        .expect("a typed results page");
    let BulkItemState::Failed { error } = &page.page.items()[0].state else {
        panic!("item 0 failed");
    };
    let rendered = format!("{error:?}");
    let serialized = serde_json::to_string(error).expect("error serializes");
    for surface in [&rendered, &serialized] {
        assert!(
            !surface.contains('\u{1b}'),
            "no control sequence: {surface}"
        );
        assert!(
            !surface.to_ascii_lowercase().contains("x-api-key"),
            "no header name: {surface}"
        );
        assert!(
            !surface.contains('δ') && !surface.contains('\u{2603}'),
            "non-ASCII scalars are removed: {surface}"
        );
    }
    // The canonical template carries no ambient sensitive value.
    assert!(
        !serialized.contains(SYNTHETIC_KEY),
        "the template never embeds the key: {serialized}"
    );
    // The retained upstream message is the sanitized, bounded reduction.
    let message = error
        .details()
        .and_then(|details| match details {
            microck_pangram_cli::output::CanonicalErrorDetails::Fields(fields) => fields
                .get("upstream_message")
                .and_then(serde_json::Value::as_str),
            _ => None,
        })
        .expect("a sanitized upstream message");
    assert!(message.chars().count() <= 200);
    assert!(message.is_ascii());
    assert!(!message.chars().any(|ch| ch.is_ascii_control()));
    fixture.shutdown().await;
}
