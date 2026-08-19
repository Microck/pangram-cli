//! Compiled MCP history tools against the real SQLite history store.

// This contract reuses only the SQLite seeding subset of the broader CLI
// fixture. Other integration targets exercise its process helpers.
#[allow(dead_code)]
#[path = "../support/history_cli_env.rs"]
mod history_env;

use serde_json::{Value, json};

use crate::mcp_stdio::{McpProcess, result};
use history_env::Env;

#[cfg(feature = "dev-tools")]
use crate::fixture::{ProtocolFixture, Step, TASK_ID, pangram4_success, plagiarism_success};

fn server(env: &Env, mutations: bool) -> McpProcess {
    let arguments = if mutations {
        &["--history", "--allow-history-mutations"][..]
    } else {
        &["--history"][..]
    };
    McpProcess::spawn_with_env(
        arguments,
        &[("PANGRAM_DATA_DIR", env.data_dir().as_os_str())],
    )
}

fn call(server: &mut McpProcess, name: &str, arguments: Value) -> Value {
    result(&server.request(
        "tools/call",
        json!({"name": name, "arguments": arguments}),
        true,
    ))
    .clone()
}

fn structured(call: &Value) -> &Value {
    call.get("structuredContent")
        .unwrap_or_else(|| panic!("missing structured content: {call}"))
}

#[test]
fn list_search_and_get_use_canonical_history_projection_with_content_redacted_by_default() {
    let env = Env::new();
    let older = "anl_01983c20-0180-7a80-a001-000000000001";
    let newer = "anl_01983c20-0180-7a80-a001-000000000002";
    env.seed(older, "2026-08-05T10:00:00Z", "older private words", None);
    env.seed(newer, "2026-08-05T11:00:00Z", "newer private words", None);

    let mut mcp = server(&env, false);
    result(&mcp.discover());

    let listed = call(&mut mcp, "history_list", json!({"limit": 1}));
    assert_eq!(listed["resultType"], "complete");
    assert_ne!(listed["isError"], true);
    assert_eq!(structured(&listed)["command"], "history_list");
    assert_eq!(structured(&listed)["data"]["items"][0]["id"], newer);

    let searched = call(
        &mut mcp,
        "history_search",
        json!({"query": "older private"}),
    );
    assert_eq!(structured(&searched)["command"], "history_search");
    assert_eq!(structured(&searched)["data"]["items"][0]["id"], older);

    let redacted = call(&mut mcp, "history_get", json!({"analysis_id": older}));
    assert_eq!(structured(&redacted)["command"], "history_show");
    assert!(
        !redacted.to_string().contains("older private words"),
        "history_get must redact retained content by default"
    );

    let included = call(
        &mut mcp,
        "history_get",
        json!({"analysis_id": older, "include_content": true}),
    );
    assert_eq!(
        structured(&included)["data"]["input"]["text"],
        "older private words"
    );
    assert_eq!(mcp.shutdown(), "");
}

#[test]
fn delete_and_clear_commit_exact_history_mutations() {
    let env = Env::new();
    let first = "anl_01983c20-0180-7a80-a001-000000000011";
    let second = "anl_01983c20-0180-7a80-a001-000000000012";
    env.seed(first, "2026-08-05T10:00:00Z", "first durable words", None);
    env.seed(second, "2026-08-05T11:00:00Z", "second durable words", None);

    let mut mcp = server(&env, true);
    result(&mcp.discover());
    let deleted = call(&mut mcp, "history_delete", json!({"analysis_id": first}));
    assert_eq!(structured(&deleted)["command"], "history_delete");
    assert_eq!(structured(&deleted)["data"]["ok"], true);

    let missing = call(&mut mcp, "history_get", json!({"analysis_id": first}));
    assert_eq!(missing["isError"], true);
    assert_eq!(structured(&missing)["command"], "history_show");
    assert_eq!(structured(&missing)["error"]["code"], "history_unavailable");

    let cleared = call(&mut mcp, "history_clear", json!({}));
    assert_eq!(structured(&cleared)["command"], "history_clear");
    assert_eq!(structured(&cleared)["data"]["ok"], true);
    let listed = call(&mut mcp, "history_list", json!({}));
    assert_eq!(structured(&listed)["data"]["items"], json!([]));
    assert_eq!(mcp.shutdown(), "");
}

#[test]
fn absent_history_returns_empty_pages_without_creating_a_database() {
    let env = Env::new();
    let database = env.data_dir().join("history").join("pangram-history.db");
    let mut mcp = server(&env, true);
    result(&mcp.discover());

    for (name, arguments) in [
        ("history_list", json!({})),
        ("history_search", json!({"query": "anything"})),
        ("history_clear", json!({})),
    ] {
        let response = call(&mut mcp, name, arguments);
        assert_ne!(response["isError"], true, "{name}: {response}");
    }
    assert!(
        !database.exists(),
        "read and empty clear must not create SQLite"
    );
    assert_eq!(mcp.shutdown(), "");
}

#[cfg(feature = "dev-tools")]
#[tokio::test(flavor = "multi_thread")]
async fn rerun_submits_once_with_fresh_lineage_and_saves_the_terminal_result() {
    use std::str::FromStr as _;

    use microck_pangram_cli::domain::{AnalysisId, AnalysisInput};
    use microck_pangram_cli::history::HistoryStore;

    let env = Env::new();
    let original = "anl_01983c20-0180-7a80-a001-000000000021";
    let text = "one retained synthetic billing block";
    env.seed(original, "2026-08-05T10:00:00Z", text, None);

    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    let mut mcp = McpProcess::spawn_loopback_with_env(
        fixture.base_url(),
        &["--history", "--allow-history-mutations"],
        &[("PANGRAM_DATA_DIR", env.data_dir().as_os_str())],
    );
    result(&mcp.discover());

    let rerun = call(
        &mut mcp,
        "history_rerun",
        json!({"analysis_id": original, "max_billable_units": 1}),
    );
    assert_ne!(rerun["isError"], true, "{rerun}");
    assert_eq!(structured(&rerun)["command"], "history_rerun");
    assert_eq!(structured(&rerun)["data"]["rerun_of"], original);
    assert_eq!(structured(&rerun)["data"]["save_state"], "saved_manual");
    let fresh = structured(&rerun)["data"]["id"].as_str().unwrap();
    assert_ne!(fresh, original);
    assert_eq!(fixture.post_count(), 1, "rerun must issue exactly one POST");
    assert_eq!(mcp.shutdown(), "");

    let store = HistoryStore::open(env.data_dir()).unwrap();
    let saved = store
        .canonical_analysis(&AnalysisId::from_str(fresh).unwrap(), true)
        .unwrap();
    assert_eq!(saved.rerun_of().unwrap().to_string(), original);
    let AnalysisInput::Text(input) = saved.input().unwrap() else {
        panic!("a text rerun must persist text input");
    };
    assert_eq!(input.text.as_deref(), Some(text));
    fixture.shutdown().await;
}

#[cfg(feature = "dev-tools")]
#[tokio::test(flavor = "multi_thread")]
async fn rerun_repeats_the_saved_combined_check_set_and_its_full_ceiling() {
    let env = Env::new();
    let text = "one retained synthetic combined billing block";
    let fixture = ProtocolFixture::start().await;
    for task_id in ["task-combined-original", "task-combined-rerun"] {
        fixture.on_submit(Step::Json(json!({"task_id": task_id})));
        fixture.on_poll(Step::Json(pangram4_success(text)));
        fixture.on_plagiarism(Step::Json(plagiarism_success()));
    }
    let mut mcp = McpProcess::spawn_loopback_with_env(
        fixture.base_url(),
        &["--history", "--allow-history-mutations"],
        &[("PANGRAM_DATA_DIR", env.data_dir().as_os_str())],
    );
    result(&mcp.discover());

    let original = call(
        &mut mcp,
        "analyze_text",
        json!({"text": text, "max_billable_units": 6, "save": true}),
    );
    assert_ne!(original["isError"], true, "{original}");
    let original_id = structured(&original)["data"]["id"].as_str().unwrap();

    let too_low = call(
        &mut mcp,
        "history_rerun",
        json!({"analysis_id": original_id, "max_billable_units": 5}),
    );
    assert_eq!(too_low["isError"], true);
    assert_eq!(structured(&too_low)["error"]["code"], "unsupported_input");
    assert_eq!(fixture.post_count(), 2, "a rejected rerun sends nothing");

    let rerun = call(
        &mut mcp,
        "history_rerun",
        json!({"analysis_id": original_id, "max_billable_units": 6}),
    );
    assert_ne!(rerun["isError"], true, "{rerun}");
    assert_eq!(structured(&rerun)["data"]["rerun_of"], original_id);
    assert_eq!(
        structured(&rerun)["data"]["checks"][0]["kind"],
        "ai_detection"
    );
    assert_eq!(
        structured(&rerun)["data"]["checks"][1]["kind"],
        "plagiarism"
    );
    assert_eq!(fixture.post_count(), 4, "each combined run sends two POSTs");
    assert_eq!(fixture.get_count(), 2);
    assert_eq!(mcp.shutdown(), "");
    fixture.shutdown().await;
}
