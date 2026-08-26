//! Compiled stdio MCP analysis tools against the real Pangram loopback.

#![cfg(feature = "dev-tools")]

use serde_json::{Value, json};

use crate::fixture::{ProtocolFixture, Step, TASK_ID, pangram4_success, plagiarism_success};
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

#[tokio::test(flavor = "multi_thread")]
async fn plagiarism_tool_uses_fixed_ceiling_and_returns_canonical_analysis() {
    let fixture = ProtocolFixture::start().await;
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &[]);
    result(&server.discover());

    let rejected = call(
        &mut server,
        "check_plagiarism",
        json!({"text": "synthetic words", "max_billable_units": 4}),
    );
    assert_eq!(
        result(&rejected)["structuredContent"]["error"]["code"],
        "unsupported_input"
    );
    assert_eq!(fixture.post_count(), 0);

    fixture.on_plagiarism(Step::Json(plagiarism_success()));
    let response = call(
        &mut server,
        "check_plagiarism",
        json!({"text": "synthetic words", "max_billable_units": 5}),
    );
    let tool = result(&response);
    assert_eq!(tool["isError"], false);
    assert_eq!(tool["structuredContent"]["command"], "plagiarism");
    assert_eq!(
        tool["structuredContent"]["data"]["checks"][0]["kind"],
        "plagiarism"
    );
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn analyze_text_returns_partial_success_when_plagiarism_fails() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success("synthetic words")));
    fixture.on_plagiarism(Step::Status(402, None, None));
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &[]);
    result(&server.discover());

    let response = call(
        &mut server,
        "analyze_text",
        json!({"text": "synthetic words", "max_billable_units": 6}),
    );

    let tool = result(&response);
    assert_eq!(tool["isError"], false);
    assert_eq!(tool["structuredContent"]["command"], "analyze");
    assert_eq!(tool["structuredContent"]["data"]["status"], "partial");
    assert_eq!(
        tool["structuredContent"]["data"]["checks"][0]["status"],
        "succeeded"
    );
    assert_eq!(
        tool["structuredContent"]["data"]["checks"][1]["error"]["code"],
        "payment_required"
    );
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_combined_analysis_stops_its_shared_observation_token() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Hang);
    fixture.on_plagiarism(Step::Json(plagiarism_success()));
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &[]);
    result(&server.discover());

    let cancelled_id = server.start_request(
        "tools/call",
        json!({
            "name": "analyze_text",
            "arguments": {"text": "synthetic words", "max_billable_units": 6}
        }),
        true,
    );
    fixture.wait_for_posts(2).await;
    fixture.wait_for_gets(1).await;
    server.send(json!({
        "jsonrpc": "2.0",
        "method": "notifications/cancelled",
        "params": {
            "requestId": cancelled_id,
            "reason": "contract test cancellation"
        }
    }));

    let live_response = server.request("tools/list", json!({}), true);
    assert_eq!(result(&live_response)["resultType"], "complete");
    assert!(
        server
            .response_within(std::time::Duration::from_secs(1))
            .is_none(),
        "the cancelled combined request must never produce a JSON-RPC response"
    );
    assert_eq!(fixture.post_count(), 2);
    assert_eq!(fixture.get_count(), 1);
    assert_eq!(server.shutdown(), "");
    fixture.shutdown().await;
}
