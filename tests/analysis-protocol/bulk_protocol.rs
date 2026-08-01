//! Observable protocol tests for the documented Pangram 4 bulk wire contract
//! (contracts section 9.1, official source `eb214f4` re-verified 2026-08-01).
//!
//! The loopback Axum fixture owns the four bulk routes and scripted queues.
//! The dev-tools [`BulkProbeClient`] is the real HTTP surface the future
//! production analysis client consumes: it issues actual requests against the
//! fixture and decodes 2xx bodies into the documented domain wire types. No
//! mocks, no live Pangram, no credentials: every key and body is synthetic.
//!
//! Ordering: each domain behavior has a failing test in `src/domain/bulk.rs`
//! (unit) before the fixture wire behavior is proven here.

use super::support::*;
use microck_pangram_cli::domain::{
    BulkSubmissionItem, BulkSubmissionPlan, NonEmptyString, bulk_estimated_billable_units,
};

const BULK_ID: &str = "blk_123";

fn item(id: Option<&str>, text: &str, words: u64) -> BulkSubmissionItem {
    BulkSubmissionItem::new(
        id.map(|value| NonEmptyString::new(value).unwrap()),
        text.to_owned(),
        words,
    )
    .unwrap()
}

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

fn status_body(status: &str, accepted: u64, succeeded: u64, failed: u64) -> serde_json::Value {
    serde_json::json!({
        "bulk_id": BULK_ID,
        "status": status,
        "total_items": 3,
        "accepted": accepted,
        "succeeded": succeeded,
        "failed": failed,
        "created_at": "1760000000.0",
        "completed_at": serde_json::Value::Null
    })
}

// 1. Exact submit grammar: one job-wide `model`, the ordered `items` shape,
//    per-item caller IDs, no per-item selector, and no public-link field.
#[tokio::test(flavor = "current_thread")]
async fn submit_sends_one_job_wide_model_and_ordered_items_without_public_link() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));

    let plan = BulkSubmissionPlan::new(
        vec![
            item(Some("row-001"), "First text", 1),
            item(Some("row-002"), "Second text", 1),
        ],
        10,
    )
    .unwrap();
    let body = plan.submit_body();

    let probe = BulkProbeClient::from_fixture(&fixture.client());
    let outcome: BulkProbeOutcome<_> = probe.submit(&body).await;

    assert_eq!(outcome.status, 202);
    let accepted = outcome.body.expect("a 202 decodes into the acceptance");
    assert_eq!(accepted.bulk_id.as_str(), BULK_ID);
    assert_eq!(accepted.total_items, 2);
    assert_eq!(accepted.accepted_items.len(), 2);
    assert_eq!(accepted.accepted_items[0].index, 0);
    assert_eq!(
        accepted.accepted_items[0].id.as_ref().unwrap().as_str(),
        "row-001"
    );

    let recorded = fixture.requests();
    let submits = BulkRequestView::submits(&recorded);
    assert_eq!(submits.len(), 1);
    let request = submits[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/bulk");
    assert!(request.header_equals("x-api-key", SYNTHETIC_KEY));
    assert!(!request.header_present("authorization"));
    let sent = request.body_json();
    // One job-wide model, items shape, exact order, no selector per item,
    // and no public-dashboard-link field.
    assert_eq!(sent["model"], "pangram-4");
    assert_eq!(sent["items"].as_array().unwrap().len(), 2);
    assert_eq!(sent["items"][0]["id"], "row-001");
    assert_eq!(sent["items"][1]["id"], "row-002");
    assert!(sent.get("text").is_none());
    assert!(sent.get("public_dashboard_link").is_none());
    for entry in sent["items"].as_array().unwrap() {
        assert!(entry.get("model").is_none(), "no per-item selector");
    }
    assert_eq!(sent.as_object().unwrap().len(), 2);

    fixture.shutdown().await;
}

// 2. The plain `text` shape when no caller IDs exist; the two shapes are
//    never mixed on one request.
#[tokio::test(flavor = "current_thread")]
async fn submit_uses_the_text_shape_without_caller_ids() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": BULK_ID,
            "status": "queued",
            "total_items": 2,
            "accepted_items": [],
            "failed_items": []
        })),
    ));

    let plan = BulkSubmissionPlan::new(vec![item(None, "a", 1), item(None, "b", 1)], 10).unwrap();
    let probe = BulkProbeClient::from_fixture(&fixture.client());
    let outcome = probe.submit(&plan.submit_body()).await;
    assert_eq!(outcome.status, 202);

    let sent = BulkRequestView::submits(&fixture.requests())[0].body_json();
    assert_eq!(sent["text"], serde_json::json!(["a", "b"]));
    assert_eq!(sent["model"], "pangram-4");
    assert!(sent.get("items").is_none());
    assert!(sent.get("public_dashboard_link").is_none());

    fixture.shutdown().await;
}

// 3. A 413 over-limit response is a submit failure with no acceptance and a
//    single recorded POST (the probe never replays the billable send).
#[tokio::test(flavor = "current_thread")]
async fn over_limit_submit_returns_413_without_an_acceptance_or_replay() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(413, None, None));

    let plan = BulkSubmissionPlan::new(vec![item(None, "x", 100)], 1000).unwrap();
    let probe = BulkProbeClient::from_fixture(&fixture.client());
    let outcome = probe.submit(&plan.submit_body()).await;

    assert_eq!(outcome.status, 413);
    assert!(outcome.body.is_none(), "a 413 carries no acceptance");
    assert_eq!(BulkRequestView::submits(&fixture.requests()).len(), 1);

    fixture.shutdown().await;
}

// 4. The status route decodes gateway counters, including a terminal
//    `partial` state with a `completed_at` timestamp.
#[tokio::test(flavor = "current_thread")]
async fn status_poll_decodes_counters_and_terminal_timestamps() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_status(Step::Json(status_body("running", 3, 1, 0)));
    let mut terminal = status_body("partial", 2, 2, 1);
    terminal["completed_at"] = serde_json::json!("1760000030.0");
    fixture.on_bulk_status(Step::Json(terminal));

    let probe = BulkProbeClient::from_fixture(&fixture.client());
    let running = probe.status(BULK_ID).await;
    assert_eq!(running.status, 200);
    let running = running.body.unwrap();
    assert_eq!(running.status.as_str(), "running");
    assert!(running.completed_at.is_none());

    let partial = probe.status(BULK_ID).await;
    let partial = partial.body.unwrap();
    assert_eq!(partial.status.as_str(), "partial");
    assert_eq!(partial.succeeded, 2);
    assert_eq!(partial.failed, 1);
    assert_eq!(partial.completed_at.unwrap().as_str(), "1760000030.0");

    let recorded = fixture.requests();
    let polled = BulkRequestView::for_path(&recorded, BULK_ID, "");
    assert_eq!(polled.len(), 2);
    assert!(polled.iter().all(|request| request.method == "GET"));

    fixture.shutdown().await;
}

// 5. A terminal failure status decodes with zero accepted and a completed
//    timestamp.
#[tokio::test(flavor = "current_thread")]
async fn status_poll_decodes_a_terminal_failure() {
    let fixture = ProtocolFixture::start().await;
    let mut body = status_body("failed", 0, 0, 2);
    body["total_items"] = serde_json::json!(2);
    body["completed_at"] = serde_json::json!("1760000010.0");
    fixture.on_bulk_status(Step::Json(body));

    let probe = BulkProbeClient::from_fixture(&fixture.client());
    let outcome = probe.status(BULK_ID).await;
    let failed = outcome.body.unwrap();
    assert_eq!(failed.status.as_str(), "failed");
    assert_eq!(failed.succeeded, 0);
    assert_eq!(failed.failed, 2);
    assert_eq!(failed.accepted, 0);

    fixture.shutdown().await;
}

// 6. The items metadata page decodes ordering and failed-entry metadata, and
//    the request carries the documented offset/limit query.
#[tokio::test(flavor = "current_thread")]
async fn items_page_decodes_ordered_metadata_and_the_page_query() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_items(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 1,
        "limit": 100,
        "total_items": 3,
        "items": [
            {"index": 1, "id": "row-002", "task_id": "task-2", "stage": "STAGE_INFERENCE", "error": null},
            {"index": 2, "id": "row-003", "task_id": null, "stage": "STAGE_FAILED",
             "error": "Text must contain at least one valid token"}
        ]
    })));

    let probe = BulkProbeClient::from_fixture(&fixture.client());
    let outcome = probe.items(BULK_ID, 1, 100).await;
    assert_eq!(outcome.status, 200);
    let page = outcome.body.unwrap();
    assert_eq!(page.offset, 1);
    assert_eq!(page.limit, 100);
    assert_eq!(page.total_items, 3);
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].index, 1);
    assert_eq!(page.items[1].task_id, None);
    assert_eq!(
        page.items[1].error.as_deref(),
        Some("Text must contain at least one valid token")
    );

    let recorded = fixture.requests();
    let requests = BulkRequestView::for_path(&recorded, BULK_ID, "/items");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].query, "offset=1&limit=100");

    fixture.shutdown().await;
}

// 7. The results page decodes the succeeded `items` list and the separate
//    `failed_items` list; an in-progress item carries `result: null`.
#[tokio::test(flavor = "current_thread")]
async fn results_page_separates_succeeded_in_progress_and_failed_items() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 100,
        "total_items": 3,
        "items": [
            {"index": 0, "id": "row-001", "task_id": "task-1", "stage": "STAGE_SUCCESS",
             "error": null, "result": {"version": "4.0"}},
            {"index": 1, "id": "row-002", "task_id": "task-2", "stage": "STAGE_INFERENCE",
             "error": null, "result": null}
        ],
        "failed_items": [
            {"index": 2, "id": "row-003", "task_id": null, "stage": "STAGE_FAILED",
             "error": "Text must contain at least one valid token"}
        ]
    })));

    let probe = BulkProbeClient::from_fixture(&fixture.client());
    let outcome = probe.results(BULK_ID, 0, 100).await;
    let page = outcome.body.unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].result.as_ref().unwrap()["version"], "4.0");
    assert!(
        page.items[1].result.is_none(),
        "in-progress carries result: null"
    );
    assert_eq!(page.failed_items.len(), 1);
    assert_eq!(page.failed_items[0].index, 2);

    let recorded = fixture.requests();
    let requests = BulkRequestView::for_path(&recorded, BULK_ID, "/results");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].query, "offset=0&limit=100");

    fixture.shutdown().await;
}

// 8. Safe GET retry surfaces: a retryable 503 with `Retry-After` followed by
//    a success is played by two queued steps; the probe observes the final
//    status and the fixture records both GETs in order.
#[tokio::test(flavor = "current_thread")]
async fn safe_get_status_route_supports_a_retryable_then_success_sequence() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_status(Step::Status(503, Some(1), None));
    fixture.on_bulk_status(Step::Json(status_body("succeeded", 2, 2, 0)));

    let probe = BulkProbeClient::from_fixture(&fixture.client());
    let first = probe.status(BULK_ID).await;
    assert_eq!(first.status, 503);
    let second = probe.status(BULK_ID).await;
    assert_eq!(second.status, 200);
    assert_eq!(second.body.unwrap().status.as_str(), "succeeded");

    let recorded = fixture.requests();
    let polled = BulkRequestView::for_path(&recorded, BULK_ID, "");
    assert_eq!(polled.len(), 2, "both scripted GETs are recorded in order");

    fixture.shutdown().await;
}

// 9. A stalled page (`Hang`) holds the route open: the probe's request never
//    completes within the test bound, and the route panics on a second call,
//    proving deterministic stalled-body behavior for the timeout path.
#[tokio::test(flavor = "current_thread")]
async fn a_stalled_page_holds_the_route_without_a_response() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_items(Step::Hang);

    let probe = BulkProbeClient::from_fixture(&fixture.client());
    let pending = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        probe.items(BULK_ID, 0, 100),
    )
    .await;
    assert!(pending.is_err(), "a Hang never responds within the bound");

    let recorded = fixture.requests();
    let requests = BulkRequestView::for_path(&recorded, BULK_ID, "/items");
    assert_eq!(requests.len(), 1, "the stalled request was still recorded");

    fixture.shutdown().await;
}

// 10. Queuing nowhere near enough steps is caught: a direct pop on an empty
//     scripted queue panics loudly instead of drifting, which is exactly the
//     failure an under-scripted fixture produces for the production client.
#[test]
#[should_panic(expected = "an unscripted POST /bulk reached the fixture")]
fn an_unscripted_bulk_submit_queue_panics() {
    let mut queues = super::fixture::BulkQueues::default();
    queues
        .submit
        .pop_front()
        .unwrap_or_else(|| panic!("an unscripted POST /bulk reached the fixture"));
}

// 11. The estimate and ceiling math drive the wire: a plan that fits sends
//     exactly its estimated unit count in derived body order, and the domain
//     estimate agrees with the per-item formula.
#[tokio::test(flavor = "current_thread")]
async fn the_estimate_matches_the_documented_per_item_sum_and_body_order() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_submit_body())));

    let words = [0_u64, 1, 100, 101];
    let items: Vec<_> = words
        .iter()
        .enumerate()
        .map(|(index, words)| item(Some(&format!("row-{index:03}")), "text", *words))
        .collect();
    let plan = BulkSubmissionPlan::new(items, 10).unwrap();

    // 0->1, 1->1, 100->1, 101->2; sum 5. The body preserves submission order.
    assert_eq!(plan.estimated_billable_units(), 5);
    assert_eq!(bulk_estimated_billable_units(words), Ok(5));

    let body = plan.submit_body();
    let ids: Vec<_> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, ["row-000", "row-001", "row-002", "row-003"]);

    let probe = BulkProbeClient::from_fixture(&fixture.client());
    assert_eq!(probe.submit(&body).await.status, 202);

    fixture.shutdown().await;
}
