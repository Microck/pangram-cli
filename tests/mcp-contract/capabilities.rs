//! Runtime authorization and restricted configuration mutation contracts.

use std::fs;

use serde_json::{Value, json};

use crate::mcp_stdio::{McpProcess, result};

fn call(server: &mut McpProcess, name: &str, arguments: Value) -> Value {
    server.request(
        "tools/call",
        json!({"name": name, "arguments": arguments}),
        true,
    )
}

#[test]
fn disabled_local_tools_cannot_be_reached_by_naming_them_directly() {
    let mut mcp = McpProcess::spawn(&[]);
    result(&mcp.discover());

    for name in ["history_list", "history_delete", "update_config"] {
        let response = call(&mut mcp, name, json!({}));
        assert_eq!(response["error"]["code"], -32601, "{name}: {response}");
    }
    assert_eq!(mcp.shutdown(), "");
}

#[test]
fn update_config_persists_only_the_six_supported_non_secret_keys() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.toml");
    let environment = [("PANGRAM_CONFIG", config.as_os_str())];
    let mut mcp = McpProcess::spawn_with_env(&["--allow-config-mutations"], &environment);
    result(&mcp.discover());

    let supported = [
        ("history.enabled", "true", "enabled = true"),
        ("tui.intro", "off", "intro = \"off\""),
        ("tui.keymap", "vim", "keymap = \"vim\""),
        ("tui.motion", "reduced", "motion = \"reduced\""),
        (
            "updates.check_on_tui_start",
            "false",
            "check_on_tui_start = false",
        ),
        (
            "network.max_requests_per_second",
            "2.5",
            "max_requests_per_second = 2.5",
        ),
    ];
    for (key, value, _) in supported {
        let response = call(
            &mut mcp,
            "update_config",
            json!({"key": key, "value": value}),
        );
        let completed = result(&response);
        assert_eq!(completed["resultType"], "complete");
        assert_ne!(completed["isError"], true);
        assert_eq!(completed["structuredContent"]["command"], "config_set");
        assert_eq!(completed["structuredContent"]["data"]["ok"], true);
    }

    let persisted = fs::read_to_string(&config).unwrap();
    for (_, _, expected) in supported {
        assert!(
            persisted.contains(expected),
            "missing {expected:?}: {persisted}"
        );
    }
    assert_eq!(mcp.shutdown(), "");
}

#[test]
fn update_config_rejects_secret_endpoint_public_link_and_unknown_keys_without_writing() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.toml");
    let environment = [("PANGRAM_CONFIG", config.as_os_str())];
    let mut mcp = McpProcess::spawn_with_env(&["--allow-config-mutations"], &environment);
    result(&mcp.discover());

    for key in [
        "credentials.api_key",
        "endpoint",
        "public_link",
        "unknown.value",
    ] {
        let response = call(
            &mut mcp,
            "update_config",
            json!({"key": key, "value": "private-or-unsafe"}),
        );
        assert_eq!(response["error"]["code"], -32602, "{key}: {response}");
    }
    assert!(
        !config.exists(),
        "invalid updates must not create config.toml"
    );
    assert_eq!(mcp.shutdown(), "");
}
