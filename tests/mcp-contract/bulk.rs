//! Compiled stdio MCP bulk tools against the real Pangram loopback.

#![cfg(feature = "dev-tools")]

use serde_json::{Value, json};

use crate::fixture::{ProtocolFixture, Step};
use crate::mcp_stdio::{McpProcess, result};

const UPSTREAM_BULK_ID: &str = "blk_fixture_001";

fn call(server: &mut McpProcess, name: &str, arguments: Value) -> Value {
    server.request(
        "tools/call",
        json!({"name": name, "arguments": arguments}),
        true,
    )
}

fn acceptance() -> Value {
    json!({
        "bulk_id": UPSTREAM_BULK_ID,
        "status": "queued",
        "total_items": 1,
        "accepted_items": [{"index": 0, "id": "row-000", "task_id": "task-000"}],
        "failed_items": []
    })
}

fn results_page() -> Value {
    json!({
        "bulk_id": UPSTREAM_BULK_ID,
        "offset": 0,
        "limit": 1,
        "total_items": 1,
        "items": [{
            "index": 0,
            "id": "row-000",
            "task_id": "task-000",
            "stage": "STAGE_SUCCESS",
            "error": null,
            "result": crate::fixture::pangram4_success("synthetic bulk words")
        }],
        "failed_items": []
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn submit_bulk_preflights_the_caller_ceiling_before_posting() {
    let fixture = ProtocolFixture::start().await;
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &[]);
    result(&server.discover());

    let response = call(
        &mut server,
        "submit_bulk",
        json!({
            "items": [{
                "id": "row-000",
                "text": (0..101).map(|_| "word").collect::<Vec<_>>().join(" ")
            }],
            "max_billable_units": 1
        }),
    );

    let tool = result(&response);
    assert_eq!(tool["isError"], true);
    assert_eq!(
        tool["structuredContent"]["error"]["code"],
        "bulk_limit_exceeded"
    );
    assert_eq!(fixture.post_count(), 0);
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn jsonl_path_without_an_approved_root_fails_before_submission() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("items.jsonl");
    std::fs::write(&path, "{\"id\":\"row-000\",\"text\":\"safe words\"}\n").unwrap();
    let fixture = ProtocolFixture::start().await;
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &[]);
    result(&server.discover());

    let response = call(
        &mut server,
        "submit_bulk",
        json!({
            "jsonl_path": path,
            "max_billable_units": 1
        }),
    );

    let tool = result(&response);
    assert_eq!(tool["isError"], true);
    assert_eq!(
        tool["structuredContent"]["error"]["code"],
        "mcp_root_required"
    );
    assert_eq!(fixture.post_count(), 0);
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn approved_jsonl_path_submits_through_the_preopened_root() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("items.jsonl");
    std::fs::write(&path, "{\"id\":\"row-000\",\"text\":\"safe words\"}\n").unwrap();
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(acceptance())));
    let approved_root = workspace.path().to_str().expect("UTF-8 test root");
    let mut server =
        McpProcess::spawn_loopback(fixture.base_url(), &["--allow-file-root", approved_root]);
    result(&server.discover());

    let response = call(
        &mut server,
        "submit_bulk",
        json!({
            "jsonl_path": path,
            "max_billable_units": 1
        }),
    );

    let tool = result(&response);
    assert_eq!(tool["isError"], false, "{tool:#}");
    assert_eq!(tool["structuredContent"]["command"], "bulk_submit");
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_submission_resolves_its_local_bulk_id_for_an_explicit_results_page() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(acceptance())));
    fixture.on_bulk_results(Step::Json(results_page()));
    let mut server = McpProcess::spawn_loopback(
        fixture.base_url(),
        &["--history", "--allow-history-mutations"],
    );
    result(&server.discover());

    let submitted = call(
        &mut server,
        "submit_bulk",
        json!({
            "items": [{"id": "row-000", "text": "synthetic bulk words"}],
            "max_billable_units": 1,
            "save": true
        }),
    );
    let submitted = result(&submitted);
    assert_eq!(submitted["isError"], false, "{submitted:#}");
    assert_eq!(submitted["structuredContent"]["command"], "bulk_submit");
    let local_id = submitted["structuredContent"]["data"]["id"]
        .as_str()
        .expect("local bulk ID")
        .to_owned();

    let page = call(
        &mut server,
        "get_bulk_results",
        json!({"bulk_id": local_id, "offset": 0, "limit": 1}),
    );
    let page = result(&page);
    assert_eq!(page["isError"], false);
    assert_eq!(page["structuredContent"]["command"], "bulk_results");
    assert_eq!(page["structuredContent"]["data"]["offset"], 0);
    assert_eq!(page["structuredContent"]["data"]["limit"], 1);
    assert_eq!(
        page["structuredContent"]["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(fixture.get_count(), 1);
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn upstream_bulk_results_work_without_history_and_use_explicit_pagination() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(results_page()));
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &[]);
    result(&server.discover());

    let response = call(
        &mut server,
        "get_bulk_results",
        json!({"upstream_bulk_id": UPSTREAM_BULK_ID, "offset": 0, "limit": 1}),
    );

    let tool = result(&response);
    assert_eq!(tool["isError"], false);
    assert_eq!(tool["structuredContent"]["command"], "bulk_results");
    assert_eq!(fixture.post_count(), 0);
    let request = fixture.requests().into_iter().next().expect("one GET");
    assert_eq!(request.query, "offset=0&limit=1");
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}
