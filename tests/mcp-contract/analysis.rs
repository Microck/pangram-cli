//! Compiled stdio MCP analysis tools against the real Pangram loopback.

#![cfg(feature = "dev-tools")]

use serde_json::{Value, json};

use crate::fixture::{ProtocolFixture, Step, TASK_ID, pangram4_success};
use crate::mcp_stdio::{McpProcess, result};

fn call(server: &mut McpProcess, name: &str, arguments: Value) -> Value {
    server.request(
        "tools/call",
        json!({"name": name, "arguments": arguments}),
        true,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn detect_text_preflights_the_billable_ceiling_before_posting() {
    let fixture = ProtocolFixture::start().await;
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &[]);
    result(&server.discover());

    let response = call(
        &mut server,
        "detect_text",
        json!({
            "text": (0..101).map(|_| "word").collect::<Vec<_>>().join(" "),
            "max_billable_units": 1
        }),
    );

    let tool = result(&response);
    assert_eq!(tool["resultType"], "complete");
    assert_eq!(tool["isError"], true);
    assert_eq!(
        tool["structuredContent"]["error"]["code"],
        "unsupported_input"
    );
    assert_eq!(fixture.post_count(), 0, "preflight must prevent billing");
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn detect_text_submits_once_waits_and_returns_the_canonical_envelope() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success("synthetic MCP words")));
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &[]);
    result(&server.discover());

    let response = call(
        &mut server,
        "detect_text",
        json!({
            "text": "synthetic MCP words",
            "max_billable_units": 1,
            "include_input": false
        }),
    );

    let tool = result(&response);
    assert_eq!(tool["resultType"], "complete");
    assert_eq!(tool["isError"], false);
    assert_eq!(tool["structuredContent"]["command"], "detect");
    assert_eq!(tool["structuredContent"]["data"]["status"], "succeeded");
    assert!(
        tool["structuredContent"]["data"]["input"]
            .get("text")
            .is_none()
    );
    assert_eq!(fixture.post_count(), 1, "one tool call issues one POST");
    assert_eq!(fixture.get_count(), 1);
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn upstream_task_id_reads_without_history_and_never_posts() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Json(pangram4_success("remote task words")));
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &[]);
    result(&server.discover());

    let response = call(
        &mut server,
        "get_task",
        json!({"upstream_task_id": TASK_ID}),
    );

    let tool = result(&response);
    assert_eq!(tool["isError"], false);
    assert_eq!(tool["structuredContent"]["command"], "task_status");
    assert_eq!(tool["structuredContent"]["data"]["status"], "succeeded");
    assert_eq!(fixture.post_count(), 0);
    assert_eq!(fixture.get_count(), 1);
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sequential_calls_share_fresh_server_lifetime_pacing() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Json(pangram4_success("first paced task")));
    fixture.on_poll(Step::Json(pangram4_success("second paced task")));
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &["--allow-config-mutations"]);
    result(&server.discover());

    let updated = call(
        &mut server,
        "update_config",
        json!({
            "key": "network.max_requests_per_second",
            "value": "1"
        }),
    );
    assert_eq!(result(&updated)["isError"], false);

    let first = call(
        &mut server,
        "get_task",
        json!({"upstream_task_id": TASK_ID}),
    );
    assert_eq!(result(&first)["isError"], false);

    let second = call(
        &mut server,
        "get_task",
        json!({"upstream_task_id": TASK_ID}),
    );

    assert_eq!(result(&second)["isError"], false);
    let requests = fixture.requests();
    let received = requests
        .iter()
        .filter(|request| request.method == "GET")
        .map(|request| request.received_at)
        .collect::<Vec<_>>();
    assert_eq!(received.len(), 2);
    let receipt_gap = received[1].duration_since(received[0]);
    assert!(
        receipt_gap >= std::time::Duration::from_millis(900),
        "fresh 1 req/s pacing must survive analyzer reconstruction; receipt gap {receipt_gap:?}"
    );
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_analysis_id_resolves_through_history() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success("saved MCP words")));
    fixture.on_poll(Step::Json(pangram4_success("saved MCP words")));
    let mut server = McpProcess::spawn_loopback(
        fixture.base_url(),
        &["--history", "--allow-history-mutations"],
    );
    result(&server.discover());

    let submitted = call(
        &mut server,
        "detect_text",
        json!({
            "text": "saved MCP words",
            "max_billable_units": 1,
            "save": true
        }),
    );
    let submitted = result(&submitted);
    assert_eq!(submitted["isError"], false);
    let local_id = submitted["structuredContent"]["data"]["id"]
        .as_str()
        .expect("saved analysis ID")
        .to_owned();

    let observed = call(&mut server, "get_task", json!({"analysis_id": local_id}));
    let observed = result(&observed);
    assert_eq!(observed["isError"], false);
    assert_eq!(observed["structuredContent"]["command"], "task_status");
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(fixture.get_count(), 2);
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn public_links_require_the_explicit_server_capability() {
    let fixture = ProtocolFixture::start().await;
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &[]);
    result(&server.discover());

    let response = call(
        &mut server,
        "detect_text",
        json!({
            "text": "synthetic words",
            "max_billable_units": 1,
            "public_link": true
        }),
    );

    let tool = result(&response);
    assert_eq!(tool["isError"], true);
    assert_eq!(
        tool["structuredContent"]["error"]["code"],
        "mcp_capability_required"
    );
    assert_eq!(fixture.post_count(), 0);
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}
