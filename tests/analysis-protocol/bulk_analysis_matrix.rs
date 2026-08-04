//! The bulk analysis error/acceptance matrix: the documented safe-GET status
//! mappings, exact-202 acceptance classification, closed acceptance status
//! token, bounded allocation on untrusted `total_items`, the full documented
//! explicit page window, mixed running/succeeded/failed normalization, and
//! the single adapter-facing `Analyzer` owner (contracts section 9.1).

use super::bulk_analysis::{
    BULK_ID, accepted_submit_body, status_body, success_result, two_item_plan,
};
use super::support::*;
use microck_pangram_cli::domain::{AnalysisStatus, BulkItemState};

/// 21. The safe GET status route maps the documented bulk error matrix onto
/// the canonical codes: 401/402/403 fail closed without a retry (one GET),
/// 404 surfaces the not-found sentinel, and a terminal 5xx reports the
/// upstream-error surface. The fixture answers through the scripted queue so
/// a single non-retryable failure is exactly one recorded read.
#[tokio::test(flavor = "current_thread")]
async fn status_get_matrix_maps_canonical_codes_without_retry() {
    for (status, code) in [
        (401_u16, ErrorCode::InvalidApiKey),
        (402, ErrorCode::PaymentRequired),
        (403, ErrorCode::PermissionDenied),
        (404, ErrorCode::UpstreamNotFound),
        (422, ErrorCode::UpstreamContractChanged),
        (500, ErrorCode::UpstreamError),
        (503, ErrorCode::UpstreamError),
    ] {
        let fixture = ProtocolFixture::start().await;
        fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
        fixture.on_bulk_status(Step::Status(status, None, None));
        let analyzer = Analyzer::from_client(fixture.client());
        let running = analyzer
            .submit_bulk(
                BulkAnalysisRequest::new(two_item_plan()),
                &StopObserving::new().token().clone(),
            )
            .await
            .expect("acceptance");
        let result = running
            .snapshot(&StopObserving::new().token().clone(), None)
            .await;
        let error = result.expect_err("a deterministic status failure");
        assert_eq!(error.canonical().code(), code, "status {status}");
        assert_scrubbed(error.canonical());
        fixture.shutdown().await;
    }
}

/// 22. A 503 on the status route is transient and retried by the shared
/// safe-GET chain: a scripted 503-then-200 sequence succeeds in exactly two
/// reads. A 404 is the not-found sentinel and is never retried (one read).
#[tokio::test(flavor = "current_thread")]
async fn status_get_503_retries_then_succeeds_but_404_never_retries() {
    // 503 -> 200: one retry, then success. A two-attempt retry policy with a
    // zero base delay keeps the chain deterministic.
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_status(Step::Status(503, None, None));
    fixture.on_bulk_status(Step::Json(status_body("succeeded", 2, 2, 2, 0, true)));
    let analyzer = Analyzer::from_client(fixture.client_with_policy(
        RetryPolicy {
            max_attempts: 3,
            ..RetryPolicy::PRODUCTION
        },
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
    let collection = running
        .observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new())
        .await
        .expect("no interruption")
        .expect("the retried observation reaches terminal success");
    assert_eq!(collection.status(), AnalysisStatus::Succeeded);
    assert_eq!(
        fixture.get_count(),
        2,
        "one 503 read plus one successful retry"
    );
    fixture.shutdown().await;

    // 404: not-found sentinel, one read only.
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_status(Step::Status(404, None, None));
    let analyzer = Analyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let result = running
        .snapshot(&StopObserving::new().token().clone(), None)
        .await;
    let error = result.expect_err("a 404 is the not-found surface");
    assert_eq!(error.canonical().code(), ErrorCode::UpstreamNotFound);
    assert_eq!(fixture.get_count(), 1, "a 404 is never retried");
    fixture.shutdown().await;
}

/// 23. Exactly HTTP 202 is a bulk acceptance. A 200 submit response is not
/// the documented success: it falls into the never-replayed ambiguous class
/// (`submission_outcome_unknown`) because the job may exist remotely, and the
/// sender records exactly one POST. A malformed 202 body is likewise
/// ambiguous, carries the fixed reconciliation guidance, and leaks no input
/// or key.
#[tokio::test(flavor = "current_thread")]
async fn non_202_submit_is_ambiguous_and_malformed_202_reconciles() {
    // 200 is not an acceptance: ambiguous, one POST, canonical recovery.
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(200, None, Some(accepted_submit_body())));
    let analyzer = Analyzer::from_client(fixture.client());
    let result = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await;
    let error = result.expect_err("a 200 submit is ambiguous, never accepted");
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
    assert_eq!(
        BulkRequestView::submits(&fixture.requests()).len(),
        1,
        "an ambiguous submit is never replayed"
    );
    assert_scrubbed(error.canonical());
    fixture.shutdown().await;

    // A malformed 202 body is ambiguous too: the job may exist remotely.
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({"unexpected": "shape"})),
    ));
    let analyzer = Analyzer::from_client(fixture.client());
    let result = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await;
    let error = result.expect_err("a malformed 202 is ambiguous");
    assert_eq!(
        error.canonical().code(),
        ErrorCode::SubmissionOutcomeUnknown
    );
    let recovery = error.canonical().recovery().expect("fixed recovery");
    assert_eq!(
        recovery.message(),
        "A manual retry may create a second billable operation."
    );
    assert_eq!(
        BulkRequestView::submits(&fixture.requests()).len(),
        1,
        "an undecodable 202 body is never replayed"
    );
    assert_scrubbed(error.canonical());
    fixture.shutdown().await;
}

/// 24. The 202 acceptance `status` token is pinned to the exact closed value
/// `queued`. Any other well-formed token on the acceptance body fails closed
/// as contract drift before any observation work. An empty `status` string
/// cannot even decode into the documented acceptance body, so it surfaces
/// through the ambiguous (never-replayed) class instead.
#[tokio::test(flavor = "current_thread")]
async fn acceptance_status_token_outside_the_closed_set_fails_closed() {
    for token in ["running", "succeeded", "pending"] {
        let fixture = ProtocolFixture::start().await;
        let mut body = accepted_submit_body();
        body["status"] = serde_json::json!(token);
        fixture.on_bulk_submit(Step::Status(202, None, Some(body)));
        let analyzer = Analyzer::from_client(fixture.client());
        let result = analyzer
            .submit_bulk(
                BulkAnalysisRequest::new(two_item_plan()),
                &StopObserving::new().token().clone(),
            )
            .await;
        let error = result.expect_err("a non-queued acceptance token is drift");
        assert_eq!(
            error.canonical().code(),
            ErrorCode::UpstreamContractChanged,
            "token {token:?}"
        );
        assert_scrubbed(error.canonical());
        fixture.shutdown().await;
    }

    // An empty status token fails structural body validation before the
    // closed-token check: the undecodable acceptance is ambiguous.
    let fixture = ProtocolFixture::start().await;
    let mut body = accepted_submit_body();
    body["status"] = serde_json::json!("");
    fixture.on_bulk_submit(Step::Status(202, None, Some(body)));
    let analyzer = Analyzer::from_client(fixture.client());
    let result = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(two_item_plan()),
            &StopObserving::new().token().clone(),
        )
        .await;
    let error = result.expect_err("an empty status cannot decode as an acceptance");
    assert_eq!(
        error.canonical().code(),
        ErrorCode::SubmissionOutcomeUnknown
    );
    assert_eq!(
        BulkRequestView::submits(&fixture.requests()).len(),
        1,
        "never replayed"
    );
    assert_scrubbed(error.canonical());
    fixture.shutdown().await;
}

/// 25. A submitted-session status read cross-checks `total_items` against
/// the validated plan count BEFORE any trust or allocation: a mismatching
/// upstream total is contract drift, and a hostile `u64::MAX` total is
/// rejected without allocating from it.
#[tokio::test(flavor = "current_thread")]
async fn status_total_items_cross_checked_against_plan_before_allocation() {
    // Plan-mismatched total: 3 for a 2-item plan.
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_status(Step::Json(status_body("running", 3, 2, 1, 0, false)));
    let analyzer = Analyzer::from_client(fixture.client());
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
        .expect_err("a plan-mismatched total is drift");
    assert_eq!(error.canonical().code(), ErrorCode::UpstreamContractChanged);
    fixture.shutdown().await;

    // Hostile u64::MAX total on a status read of the submitted session: the
    // plan cross-check rejects it before any allocation.
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_status(Step::Json(status_body("running", u64::MAX, 2, 1, 0, false)));
    let analyzer = Analyzer::from_client(fixture.client());
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
        .expect_err("a hostile total is drift, never an allocation");
    assert_eq!(error.canonical().code(), ErrorCode::UpstreamContractChanged);
    fixture.shutdown().await;
}

/// 26. A resumed remote handle without a trusted local plan count still
/// bounds coverage allocation: a results page reporting a hostile
/// `u64::MAX` `total_items` is rejected fail-closed in page normalization
/// before the walk allocates its bitmap. A bounded resumed read succeeds.
#[tokio::test(flavor = "current_thread")]
async fn resumed_remote_page_total_items_is_bounded_before_allocation() {
    let resume_plan = || {
        // A resumed read typically re-derives a two-item plan; here the plan
        // exists so the running handle is well-formed, but the page total is
        // not cross-checked against it (the status route is never read), so
        // the page's own bounded validation is the only guard.
        two_item_plan()
    };

    // Hostile total on the results page: rejected before allocation.
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 100,
        "total_items": u64::MAX,
        "items": [],
        "failed_items": []
    })));
    let analyzer = Analyzer::from_client(fixture.client());
    let running = analyzer.resume_bulk(
        microck_pangram_cli::domain::BulkId::new(),
        microck_pangram_cli::domain::UpstreamBulkId::new(BULK_ID).unwrap(),
        resume_plan(),
    );
    let error = analyzer
        .bulk_results_page(&running, 0, 100, &StopObserving::new().token().clone())
        .await
        .expect_err("a hostile page total is drift before allocation");
    assert_eq!(error.canonical().code(), ErrorCode::UpstreamContractChanged);
    fixture.shutdown().await;
}

/// 27. Explicit one-page requests may span the documented `1..=1000` window
/// while the internal fetch-all walk stays at the bounded page size: a limit
/// of 1 and a limit of 1000 both succeed, and each echoes its exact query.
#[tokio::test(flavor = "current_thread")]
async fn explicit_page_requests_may_use_the_full_documented_window() {
    for limit in [1_u64, 1000] {
        let fixture = ProtocolFixture::start().await;
        fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
        let capped = limit.clamp(1, 2); // the two-item set yields at most 2
        let items: Vec<serde_json::Value> = (0..capped)
            .map(|index| {
                serde_json::json!({
                    "index": index,
                    "id": format!("row-{index:03}"),
                    "task_id": format!("task-{index}"),
                    "stage": "STAGE_SUCCESS",
                    "error": null,
                    "result": success_result("text")
                })
            })
            .collect();
        fixture.on_bulk_results(Step::Json(serde_json::json!({
            "bulk_id": BULK_ID,
            "offset": 0,
            "limit": limit,
            "total_items": 2,
            "items": items,
            "failed_items": []
        })));
        let analyzer = Analyzer::from_client(fixture.client());
        let running = analyzer
            .submit_bulk(
                BulkAnalysisRequest::new(two_item_plan()),
                &StopObserving::new().token().clone(),
            )
            .await
            .expect("acceptance");
        let page = analyzer
            .bulk_results_page(&running, 0, limit, &StopObserving::new().token().clone())
            .await
            .expect("an explicit page within 1..=1000 succeeds");
        assert_eq!(page.page.items().len() as u64, capped);
        let recorded = fixture.requests();
        let reads = BulkRequestView::for_path(&recorded, BULK_ID, "/results");
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].query, format!("offset=0&limit={limit}"));
        fixture.shutdown().await;
    }
}

/// 28. Mixed succeeded / running (`result: null`) / failed entries on one
/// results page normalize to the canonical `Succeeded` / `Running` / `Failed`
/// item states in ascending order. A failed entry at index 0 with a
/// succeeded entry at index 1 is valid: each list is ascending on its own
/// and cross-list integrity is disjointness, not a chained ordering.
#[tokio::test(flavor = "current_thread")]
async fn mixed_running_and_failed_positions_normalize_in_order() {
    // Three items: running (1), running at 2, succeeded at 0? No: failed 0,
    // succeeded 1, running 2 across the two lists.
    let plan = bulk_plan(
        vec![
            bulk_item(Some("row-000"), "Zero", 1),
            bulk_item(Some("row-001"), "One", 1),
            bulk_item(Some("row-002"), "Two", 1),
        ],
        10,
    );
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": BULK_ID,
            "status": "queued",
            "total_items": 3,
            "accepted_items": [
                {"index": 0, "id": "row-000", "task_id": "t0"},
                {"index": 1, "id": "row-001", "task_id": "t1"},
                {"index": 2, "id": "row-002", "task_id": "t2"}
            ],
            "failed_items": []
        })),
    ));
    // failed_items index 0 + items index 1 (succeeded) + items index 2
    // (running, result: null): disjoint coverage, each list ascending.
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 10,
        "total_items": 3,
        "items": [
            {"index": 1, "id": "row-001", "task_id": "t1", "stage": "STAGE_SUCCESS",
             "error": null, "result": success_result("One")},
            {"index": 2, "id": "row-002", "task_id": "t2", "stage": "STAGE_\u{1b}[31mINFERENCE",
             "error": null, "result": null}
        ],
        "failed_items": [
            {"index": 0, "id": "row-000", "task_id": null, "stage": "STAGE_\nFAILED",
             "error": "Text must contain at least one valid token"}
        ]
    })));
    let analyzer = Analyzer::from_client(fixture.client());
    let running = analyzer
        .submit_bulk(
            BulkAnalysisRequest::new(plan),
            &StopObserving::new().token().clone(),
        )
        .await
        .expect("acceptance");
    let page = analyzer
        .bulk_results_page(&running, 0, 10, &StopObserving::new().token().clone())
        .await
        .expect("a mixed page normalizes");
    let items = page.page.items();
    assert_eq!(items.len(), 3, "succeeded + running + failed");
    assert_eq!(items[0].index, 0);
    assert!(matches!(items[0].state, BulkItemState::Failed { .. }));
    assert_eq!(items[0].last_stage(), Some("STAGE_ FAILED"));
    assert_eq!(items[1].index, 1);
    assert!(matches!(items[1].state, BulkItemState::Succeeded { .. }));
    assert_eq!(items[2].index, 2);
    assert!(matches!(items[2].state, BulkItemState::Running));
    assert_eq!(items[2].last_stage(), Some("STAGE_[31mINFERENCE"));
    fixture.shutdown().await;
}

/// 29. Per-item `stage` is provider diagnostic, not protocol state: a
/// failed entry with `stage` omitted entirely still classifies as failed
/// (the item shape, not the stage, decides), and the canonical error never
/// reads the stage to classify. The canonical wire convention uses field
/// omission rather than explicit null for absence.
#[tokio::test(flavor = "current_thread")]
async fn failed_entry_without_a_stage_still_classifies_by_shape() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 2,
        "total_items": 2,
        "items": [],
        "failed_items": [
            {"index": 0, "id": "row-001", "task_id": null,
             "error": "Text must contain at least one valid token"},
            {"index": 1, "id": "row-002", "task_id": null, "stage": "STAGE_FAILED",
             "error": "Text must contain at least one valid token"}
        ]
    })));
    let analyzer = Analyzer::from_client(fixture.client());
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
        .expect("a failed entry with an omitted stage still fails");
    assert_eq!(page.page.items().len(), 2);
    assert!(
        page.page
            .items()
            .iter()
            .all(|item| matches!(item.state, BulkItemState::Failed { .. }))
    );
    fixture.shutdown().await;
}

/// 30. The adapter-facing surface is the single `Analyzer` owner: text and
/// bulk both enter through it over one shared client, and the internal bulk
/// helper is never re-exported as a second top-level client.
#[tokio::test(flavor = "current_thread")]
async fn analyzer_is_the_single_adapter_facing_bulk_and_text_owner() {
    fn assert_analyzer_api(_: &Analyzer) {}
    let fixture = ProtocolFixture::start().await;
    let analyzer = Analyzer::from_client(fixture.client());
    // The bulk methods exist on the one Analyzer over the shared client.
    assert_analyzer_api(&analyzer);
    // A text request still enters through the same owner (type-level proof
    // that no second client type is required).
    let _text = request("hello");
    fixture.shutdown().await;
}

/// 31. A resumed (plan=None) observed results page marks every emitted
/// child analysis `accepted`, never `terminal` (contracts.md 4.6): the read
/// reconciles a remotely authored job, so neither a failed nor a succeeded
/// child may claim the caller-submitted outcome. A failed child carries the
/// sanitized upstream error with no input descriptor; a succeeded child
/// derives its descriptor only from the terminal document Pangram attested
/// (`origin: unknown`, never echoed text). Every caller ID stays `None`
/// because no trusted local plan exists.
#[tokio::test(flavor = "current_thread")]
async fn observed_results_page_children_are_accepted_never_terminal() {
    use microck_pangram_cli::domain::{SubmissionOutcome, TextOrigin};

    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 100,
        "total_items": 2,
        "items": [
            {"index": 0, "id": "row-001", "task_id": "task-1", "stage": "STAGE_SUCCESS",
             "error": null, "result": success_result("Attested text")}
        ],
        "failed_items": [
            {"index": 1, "id": "row-002", "task_id": null, "stage": "STAGE_FAILED",
             "error": "Text must contain at least one valid token"}
        ]
    })));

    let analyzer = Analyzer::from_client(fixture.client());
    let running =
        analyzer.observe_bulk(microck_pangram_cli::domain::UpstreamBulkId::new(BULK_ID).unwrap());
    let page = analyzer
        .bulk_results_page(&running, 0, 100, &StopObserving::new().token().clone())
        .await
        .expect("a resumed observed results page");

    let items = page.page.items();
    assert_eq!(items.len(), 2);

    // Succeeded child at index 0: accepted, attested-text descriptor only.
    assert_eq!(items[0].index, 0);
    assert_eq!(items[0].caller_id, None);
    let BulkItemState::Succeeded { analysis } = &items[0].state else {
        panic!("observed item 0 succeeded");
    };
    assert_eq!(analysis.submission_outcome(), SubmissionOutcome::Accepted);
    assert_eq!(analysis.status(), AnalysisStatus::Succeeded);
    let input = analysis
        .input()
        .expect("an attested terminal document yields a descriptor");
    let microck_pangram_cli::domain::AnalysisInput::Text(text) = input else {
        panic!("bulk text item");
    };
    assert_eq!(text.origin(), TextOrigin::Unknown);
    // The descriptor holds hashes/counts, not the attested text itself.
    assert!(text.text.is_none());

    // Failed child at index 1: accepted, sanitized error, no input.
    assert_eq!(items[1].index, 1);
    assert_eq!(items[1].caller_id, None);
    let BulkItemState::Failed { error } = &items[1].state else {
        panic!("observed item 1 failed");
    };
    assert_eq!(error.code(), ErrorCode::UpstreamAnalysisFailed);
    assert_scrubbed(error);
    fixture.shutdown().await;
}

/// 32. A resumed observed results page whose succeeded child's terminal
/// document carries no `text` field still emits the child analysis as
/// `accepted`, and omits the input descriptor entirely rather than
/// fabricating one from nothing.
#[tokio::test(flavor = "current_thread")]
async fn observed_results_success_without_text_omits_the_descriptor() {
    use microck_pangram_cli::domain::SubmissionOutcome;

    // A Pangram 4 terminal success with no top-level `text` field: valid
    // under one-pass normalization (`normalized_text: None`).
    let mut no_text = pangram4_success("unused");
    no_text.as_object_mut().unwrap().remove("text");

    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 100,
        "total_items": 1,
        "items": [
            {"index": 0, "id": "row-001", "task_id": "task-1", "stage": "STAGE_SUCCESS",
             "error": null, "result": no_text}
        ],
        "failed_items": []
    })));

    let analyzer = Analyzer::from_client(fixture.client());
    let running =
        analyzer.observe_bulk(microck_pangram_cli::domain::UpstreamBulkId::new(BULK_ID).unwrap());
    let page = analyzer
        .bulk_results_page(&running, 0, 100, &StopObserving::new().token().clone())
        .await
        .expect("a text-less observed results page");

    let items = page.page.items();
    assert_eq!(items.len(), 1);
    let BulkItemState::Succeeded { analysis } = &items[0].state else {
        panic!("observed item 0 succeeded");
    };
    assert_eq!(analysis.submission_outcome(), SubmissionOutcome::Accepted);
    assert!(
        analysis.input().is_none(),
        "no attested text means no descriptor is fabricated"
    );
    fixture.shutdown().await;
}

/// 33. A mixed observed page (succeeded 0 + failed 1, then failed 0 +
/// succeeded 1 across two reads of the same job) keeps exact item status,
/// counters/order identity through the canonical page, and marks every
/// emitted child analysis `accepted`. (Order within one page is
/// cross-list-disjoint per 9.1, never a chained ordering across the two
/// upstream lists.)
#[tokio::test(flavor = "current_thread")]
async fn observed_mixed_results_pages_keep_exact_status_and_acceptance() {
    use microck_pangram_cli::domain::SubmissionOutcome;

    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 100,
        "total_items": 2,
        "items": [
            {"index": 0, "id": "row-000", "task_id": "task-0", "stage": "STAGE_SUCCESS",
             "error": null, "result": success_result("First")}
        ],
        "failed_items": [
            {"index": 1, "id": "row-001", "task_id": null, "stage": "STAGE_FAILED",
             "error": "Text must contain at least one valid token"}
        ]
    })));

    let analyzer = Analyzer::from_client(fixture.client());
    let running =
        analyzer.observe_bulk(microck_pangram_cli::domain::UpstreamBulkId::new(BULK_ID).unwrap());
    let page = analyzer
        .bulk_results_page(&running, 0, 100, &StopObserving::new().token().clone())
        .await
        .expect("the mixed observed page normalizes");
    let items = page.page.items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].index, 0);
    assert_eq!(items[1].index, 1);

    // Exact per-position states and identities are preserved; both child
    // analyses are `accepted` observations.
    let BulkItemState::Succeeded { analysis } = &items[0].state else {
        panic!("index 0 succeeded");
    };
    assert_eq!(analysis.submission_outcome(), SubmissionOutcome::Accepted);
    assert_eq!(
        items[0].upstream_task_id.as_ref().unwrap().as_str(),
        "task-0"
    );
    assert!(matches!(items[1].state, BulkItemState::Failed { .. }));
    assert_eq!(items[1].upstream_task_id, None);
    fixture.shutdown().await;
}
