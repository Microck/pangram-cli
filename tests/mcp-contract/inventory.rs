use serde_json::{Value, json};

use crate::mcp_stdio::{McpProcess, result};

const BASE_TOOLS: &[&str] = &[
    "detect_text",
    "get_task",
    "wait_task",
    "submit_bulk",
    "get_bulk",
    "wait_bulk",
    "get_bulk_results",
];

#[test]
fn default_inventory_is_ordered_closed_and_contains_no_future_or_fixture_tools() {
    let mut server = McpProcess::spawn(&[]);
    result(&server.discover());
    let response = server.request("tools/list", json!({}), true);
    let listed = result(&response);

    assert_eq!(listed["resultType"], "complete");
    assert_eq!(listed["ttlMs"], 0);
    assert_eq!(listed["cacheScope"], "private");
    let tools = listed["tools"].as_array().unwrap();
    assert_eq!(names(tools), BASE_TOOLS);
    for tool in tools {
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert!(tool["outputSchema"].is_object());
        assert!(tool["annotations"].is_object());
        assert!(!tool["name"].as_str().unwrap().starts_with("test_"));
    }
    assert_eq!(server.shutdown(), "");
}

#[test]
fn history_and_mutation_gates_add_only_their_owned_tools() {
    let mut history = McpProcess::spawn(&["--history"]);
    result(&history.discover());
    let listed = history.request("tools/list", json!({}), true);
    let mut expected = BASE_TOOLS.to_vec();
    expected.extend(["history_list", "history_search", "history_get"]);
    assert_eq!(
        names(result(&listed)["tools"].as_array().unwrap()),
        expected
    );
    assert_eq!(history.shutdown(), "");

    let mut mutations = McpProcess::spawn(&[
        "--history",
        "--allow-history-mutations",
        "--allow-config-mutations",
    ]);
    result(&mutations.discover());
    let listed = mutations.request("tools/list", json!({}), true);
    expected.extend([
        "history_rerun",
        "history_delete",
        "history_clear",
        "update_config",
    ]);
    assert_eq!(
        names(result(&listed)["tools"].as_array().unwrap()),
        expected
    );
    assert_eq!(mutations.shutdown(), "");
}

fn names(tools: &[Value]) -> Vec<&str> {
    tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect()
}
