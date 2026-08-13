use std::process::{Command, Stdio};

use serde_json::json;

#[cfg(feature = "dev-tools")]
use crate::fixture::{ProtocolFixture, Step, TASK_ID};
use crate::mcp_stdio::{McpProcess, result};

#[test]
fn discovery_selects_only_the_2026_protocol_and_static_capabilities() {
    let mut server = McpProcess::spawn(&[]);
    let response = server.discover();
    let discovery = result(&response);

    assert_eq!(discovery["resultType"], "complete");
    assert_eq!(discovery["supportedVersions"], json!(["2026-07-28"]));
    assert_eq!(discovery["ttlMs"], 0);
    assert_eq!(discovery["cacheScope"], "private");
    assert!(discovery["capabilities"]["tools"].is_object());
    assert!(discovery["capabilities"]["resources"].is_object());
    assert!(discovery["capabilities"].get("prompts").is_none());
    assert!(discovery["capabilities"].get("extensions").is_none());
    assert_eq!(server.shutdown(), "");
}

#[test]
fn every_request_after_discovery_requires_inline_client_context() {
    let mut server = McpProcess::spawn(&[]);
    result(&server.discover());

    let response = server.request("tools/list", json!({}), false);
    assert_eq!(response["error"]["code"], -32602);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("required fields")
    );
    assert_eq!(server.shutdown(), "");
}

#[test]
fn requests_cannot_select_an_older_protocol() {
    let mut server = McpProcess::spawn(&[]);
    result(&server.discover());
    let response = server.request(
        "tools/list",
        json!({
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2025-11-25",
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }),
        false,
    );

    assert_eq!(response["error"]["code"], -32022);
    assert_eq!(server.shutdown(), "");
}

#[test]
fn removed_initialize_lifecycle_is_rejected() {
    let mut server = McpProcess::spawn(&[]);
    let response = server.request(
        "initialize",
        json!({
            "protocolVersion": "2026-07-28",
            "capabilities": {},
            "clientInfo": {"name": "contract-test", "version": "1"}
        }),
        false,
    );

    assert_eq!(response["error"]["code"], -32601);
}

#[test]
fn invalid_capability_configuration_fails_before_reading_stdin() {
    let root = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_pangram"));
    command
        .args(["mcp", "--allow-history-mutations"])
        .env_remove("PANGRAM_API_KEY")
        .env("HOME", root.path())
        .env("XDG_CONFIG_HOME", root.path())
        .env("XDG_DATA_HOME", root.path())
        .env("PANGRAM_CONFIG", root.path().join("config.toml"))
        .env("PANGRAM_DATA_DIR", root.path().join("data"))
        .stdin(Stdio::null());
    let output = command.output().expect("run invalid MCP startup");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "MCP history mutations require --history\n"
    );
}

#[cfg(feature = "dev-tools")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_stops_local_observation_without_a_json_rpc_response() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Hang);
    let mut server = McpProcess::spawn_loopback(fixture.base_url(), &[]);
    result(&server.discover());

    let cancelled_id = server.start_request(
        "tools/call",
        json!({
            "name": "wait_task",
            "arguments": {"upstream_task_id": TASK_ID}
        }),
        true,
    );
    fixture.wait_for_gets(1).await;
    server.send(json!({
        "jsonrpc": "2.0",
        "method": "notifications/cancelled",
        "params": {
            "requestId": cancelled_id,
            "reason": "contract test cancellation"
        }
    }));

    // A later request must complete on the same process. If cancellation
    // produced a response, it races into this read and fails the ID check.
    let live_response = server.request("tools/list", json!({}), true);
    assert_eq!(result(&live_response)["resultType"], "complete");

    // Also catch a cancelled response that lost the race to tools/list.
    assert!(
        server
            .response_within(std::time::Duration::from_secs(1))
            .is_none(),
        "the cancelled request must never produce a JSON-RPC response"
    );
    assert_eq!(fixture.get_count(), 1);
    assert_eq!(fixture.post_count(), 0);
    assert_eq!(
        server.shutdown(),
        format!("pangram: cancelled local observation for upstream task {TASK_ID}\n")
    );
    fixture.shutdown().await;
}
