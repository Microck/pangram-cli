use std::fs;
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;

fn pangram() -> Command {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    let root = ROOT.get_or_init(|| tempfile::tempdir().expect("private MCP CLI root"));
    let home = root.path().join("home");
    let config_home = root.path().join("config-home");
    let data_home = root.path().join("data-home");
    let data_dir = root.path().join("pangram-data");
    for directory in [&home, &config_home, &data_home, &data_dir] {
        fs::create_dir_all(directory).expect("create MCP CLI test directory");
    }

    let mut command = Command::new(env!("CARGO_BIN_EXE_pangram"));
    command
        .env_remove("PANGRAM_API_KEY")
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_DATA_HOME", data_home)
        .env("PANGRAM_CONFIG", root.path().join("config.toml"))
        .env("PANGRAM_DATA_DIR", data_dir)
        .env("CI", "true")
        .env("TERM", "dumb");
    command
}

#[test]
fn agent_prints_the_exact_compact_embedded_markdown() {
    let output = pangram().arg("agent").output().unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        include_bytes!("../generated/agent-reference.md")
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn skills_get_selects_compact_or_full_embedded_markdown() {
    let compact = pangram()
        .args(["skills", "get", "pangram"])
        .output()
        .unwrap();
    assert_eq!(compact.status.code(), Some(0));
    assert_eq!(
        compact.stdout,
        include_bytes!("../generated/agent-reference.md")
    );
    assert!(compact.stderr.is_empty());

    let full = pangram()
        .args(["skills", "get", "pangram", "--full"])
        .output()
        .unwrap();
    assert_eq!(full.status.code(), Some(0));
    assert_eq!(full.stdout, include_bytes!("../skills/pangram/SKILL.md"));
    assert!(full.stderr.is_empty());
}

#[test]
fn skills_list_and_paths_are_exact_raw_stdout() {
    for (arguments, expected) in [
        (
            &["skills", "list"][..],
            "# Embedded skills\n\n- `pangram`\n",
        ),
        (&["skills", "path"][..], "embedded://skills\n"),
        (
            &["skills", "path", "pangram"][..],
            "embedded://skills/pangram/SKILL.md\n",
        ),
    ] {
        let output = pangram().args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{arguments:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{arguments:?}");
        assert!(output.stderr.is_empty(), "{arguments:?}");
    }
}

#[test]
fn invalid_mcp_startup_configuration_never_writes_protocol_stdout() {
    let output = pangram()
        .args(["mcp", "--allow-history-mutations"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "MCP history mutations require --history\n"
    );
}

#[test]
fn mcp_file_roots_are_validated_before_stdin_is_read() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    let output = pangram()
        .arg("mcp")
        .arg("--allow-file-root")
        .arg(missing)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        stderr.lines().count(),
        1,
        "one startup diagnostic: {stderr:?}"
    );
    assert!(stderr.ends_with('\n'));
}

#[test]
fn installer_dry_run_returns_the_closed_plan_without_writing() {
    let output = pangram()
        .args(["mcp", "install", "--target", "codex", "--dry-run"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["command"], "mcp_install");
    assert_eq!(body["data"]["dry_run"], true);
    assert_eq!(body["data"]["targets"][0]["client"], "codex");
    assert_eq!(body["data"]["targets"][0]["action"], "create");
    assert!(body["data"]["targets"][0]["path"].is_string());

    let status = pangram().args(["mcp", "status"]).output().unwrap();
    assert_eq!(status.status.code(), Some(0));
    let body: Value = serde_json::from_slice(&status.stdout).unwrap();
    let codex = body["data"]["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|client| client["client"] == "codex")
        .unwrap();
    assert_eq!(codex["installed"], false, "dry run wrote no entry");
}

#[test]
fn all_target_install_preflights_unavailable_clients_before_any_write() {
    let output = pangram()
        .args(["mcp", "install", "--all"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(7));
    assert!(output.stderr.is_empty());
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["command"], "mcp_install");
    assert_eq!(body["error"]["code"], "invalid_config");

    let status = pangram().args(["mcp", "status"]).output().unwrap();
    assert_eq!(status.status.code(), Some(0));
    let body: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(
        body["data"]["clients"]
            .as_array()
            .unwrap()
            .iter()
            .all(|client| client["installed"] == false),
        "preflight failure wrote no client"
    );
}
