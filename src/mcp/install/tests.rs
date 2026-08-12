use std::collections::BTreeMap;
use std::fs;

use serde_json::Value;
use tempfile::TempDir;

use super::*;

fn installer(root: &TempDir) -> Installer {
    Installer::with_context(
        InstallContext::for_test(InstallPlatform::Linux, root.path().join("home"))
            .with_executable(root.path().join("bin/pangram")),
    )
}

fn request(target: ClientTarget, action: InstallAction, dry_run: bool) -> InstallRequest {
    InstallRequest::new(action, vec![target], "pangram", dry_run).unwrap()
}

#[test]
fn target_names_and_first_occurrence_order_are_stable() {
    assert_eq!(
        ClientTarget::ALL
            .iter()
            .map(|target| target.as_str())
            .collect::<Vec<_>>(),
        [
            "claude-code",
            "claude-desktop",
            "codex",
            "cursor",
            "vscode",
            "windsurf",
            "gemini",
            "opencode",
            "cline",
            "roo-code",
            "droid",
            "antigravity",
        ]
    );
    let request = InstallRequest::new(
        InstallAction::Install,
        vec![
            ClientTarget::Vscode,
            ClientTarget::Cursor,
            ClientTarget::Vscode,
        ],
        "pangram",
        true,
    )
    .unwrap();
    assert_eq!(
        request.targets(),
        &[ClientTarget::Vscode, ClientTarget::Cursor]
    );
}

#[test]
fn dry_run_returns_exact_plan_without_writing() {
    let root = tempfile::tempdir().unwrap();
    let report = installer(&root)
        .apply(request(ClientTarget::Cursor, InstallAction::Install, true))
        .unwrap();
    assert!(report.dry_run());
    assert_eq!(report.targets()[0].change(), InstallChange::Create);
    assert!(!report.targets()[0].path().exists());
}

#[test]
fn json_install_is_surgical_idempotent_and_reversible() {
    let root = tempfile::tempdir().unwrap();
    let installer = installer(&root);
    let path = root.path().join("home/.cursor/mcp.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let before = "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"other\": {\"command\": \"other\", \"args\": []}\n  }\n}\n";
    fs::write(&path, before).unwrap();
    let installed = installer
        .apply(request(ClientTarget::Cursor, InstallAction::Install, false))
        .unwrap();
    assert_eq!(installed.targets()[0].change(), InstallChange::Update);
    let installed_bytes = fs::read(&path).unwrap();
    assert!(String::from_utf8_lossy(&installed_bytes).contains("\"type\":\"stdio\""));
    assert_eq!(
        installer
            .apply(request(ClientTarget::Cursor, InstallAction::Install, false,))
            .unwrap()
            .changed(),
        0
    );
    assert_eq!(fs::read(&path).unwrap(), installed_bytes);
    installer
        .apply(request(
            ClientTarget::Cursor,
            InstallAction::Uninstall,
            false,
        ))
        .unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), before);
}

#[test]
fn conflict_malformed_and_duplicate_json_never_write() {
    let root = tempfile::tempdir().unwrap();
    let installer = installer(&root);
    let path = root.path().join("home/.cursor/mcp.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    for source in [
        r#"{"mcpServers":{"pangram":{"command":"other","args":["mcp"]}}}"#,
        r#"{"mcpServers":{"pangram":,}}"#,
        r#"{"mcpServers":{},"mcpServers":{}}"#,
    ] {
        fs::write(&path, source).unwrap();
        assert!(
            installer
                .apply(request(ClientTarget::Cursor, InstallAction::Install, false,))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn jsonc_preserves_comments_crlf_and_permissions() {
    let root = tempfile::tempdir().unwrap();
    let installer = installer(&root);
    let path = root.path().join("home/.config/Code/User/mcp.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let source = "{\r\n  // user comment\r\n  \"servers\": {\r\n    \"other\": {\"command\":\"x\",},\r\n  },\r\n}\r\n";
    fs::write(&path, source).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    }
    installer
        .apply(request(ClientTarget::Vscode, InstallAction::Install, false))
        .unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains("// user comment\r\n"));
    assert!(!after.replace("\r\n", "").contains('\n'));
    installer
        .apply(request(
            ClientTarget::Vscode,
            InstallAction::Uninstall,
            false,
        ))
        .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}

#[test]
fn jsonc_uninstall_removes_the_sole_members_trailing_comma() {
    let root = tempfile::tempdir().unwrap();
    let installer = installer(&root);
    let path = root.path().join("home/.config/Code/User/mcp.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let executable =
        serde_json::to_string(root.path().join("bin/pangram").to_str().unwrap()).unwrap();
    let source = format!(
        r#"{{"servers":{{"pangram":{{"type":"stdio","command":{executable},"args":["mcp"]}},}}}}"#
    );
    fs::write(&path, source).unwrap();

    installer
        .apply(request(
            ClientTarget::Vscode,
            InstallAction::Uninstall,
            false,
        ))
        .unwrap();

    let after = fs::read_to_string(path).unwrap();
    assert_eq!(after, r#"{"servers":{}}"#);
    assert!(serde_json::from_str::<Value>(&after).is_ok());
}

#[test]
fn codex_toml_preserves_and_restores_unrelated_bytes() {
    let root = tempfile::tempdir().unwrap();
    let installer = installer(&root);
    let path = root.path().join("home/.codex/config.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let source =
        "# settings\nmodel = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"other\"\nargs = []\n";
    fs::write(&path, source).unwrap();
    installer
        .apply(request(ClientTarget::Codex, InstallAction::Install, false))
        .unwrap();
    assert!(fs::read_to_string(&path).unwrap().starts_with(source));
    installer
        .apply(request(
            ClientTarget::Codex,
            InstallAction::Uninstall,
            false,
        ))
        .unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn codex_uninstall_accepts_an_equivalent_quoted_owned_table_header() {
    let root = tempfile::tempdir().unwrap();
    let installer = installer(&root);
    let path = root.path().join("home/.codex/config.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let executable = root.path().join("bin/pangram");
    let source = format!(
        "model = \"gpt-5\"\n\n[mcp_servers.\"pangram\"] # installed entry\ncommand = {}\nargs = [\"mcp\"]\n",
        serde_json::to_string(executable.to_str().unwrap()).unwrap()
    );
    fs::write(&path, source).unwrap();
    installer
        .apply(request(
            ClientTarget::Codex,
            InstallAction::Uninstall,
            false,
        ))
        .unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), "model = \"gpt-5\"\n");
}

#[test]
fn concurrent_change_and_symlink_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let installer = installer(&root);
    let path = root.path().join("home/.cursor/mcp.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{}\n").unwrap();
    let plan = installer
        .plan(&request(
            ClientTarget::Cursor,
            InstallAction::Install,
            false,
        ))
        .unwrap();
    fs::write(&path, "{\"raced\":true}\n").unwrap();
    assert!(matches!(
        installer.apply_plan(plan),
        Err(InstallError::ConcurrentChange { .. })
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let victim = root.path().join("victim.json");
        fs::remove_file(&path).unwrap();
        fs::write(&victim, "{}\n").unwrap();
        symlink(&victim, &path).unwrap();
        assert!(matches!(
            installer.apply(request(ClientTarget::Cursor, InstallAction::Install, false,)),
            Err(InstallError::Symlink { .. })
        ));
        assert_eq!(fs::read_to_string(victim).unwrap(), "{}\n");
    }
}

#[test]
fn paths_honor_overrides_and_roo_status_is_reportable() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let overrides = BTreeMap::from([
        ("CLAUDE_CONFIG_DIR", root.path().join("claude-code")),
        ("CODEX_HOME", root.path().join("codex")),
        ("XDG_CONFIG_HOME", root.path().join("xdg")),
    ]);
    let mut context = InstallContext::for_test(InstallPlatform::Linux, home.clone())
        .with_executable(root.path().join("pangram"));
    for (name, path) in overrides {
        context = context.with_env_path(name, path);
    }
    let installer = Installer::with_context(context);
    assert_eq!(
        installer.path_for(ClientTarget::ClaudeCode).unwrap(),
        root.path().join("claude-code/.claude.json")
    );
    assert_eq!(
        installer.path_for(ClientTarget::OpenCode).unwrap(),
        root.path().join("xdg/opencode/opencode.json")
    );
    assert_eq!(
        installer.path_for(ClientTarget::Windsurf).unwrap(),
        home.join(".config/devin/mcp_config.json")
    );
    assert!(installer.path_for(ClientTarget::RooCode).is_err());
    let status = installer
        .status(&[ClientTarget::RooCode], "pangram")
        .unwrap();
    assert!(!status[0].installed());
    assert_eq!(status[0].path(), None);
}

#[test]
fn status_reports_a_same_named_unowned_entry_as_not_installed() {
    let root = tempfile::tempdir().unwrap();
    let installer = installer(&root);
    let path = root.path().join("home/.cursor/mcp.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        r#"{"mcpServers":{"pangram":{"command":"someone-else","args":[]}}}"#,
    )
    .unwrap();
    let status = installer
        .status(&[ClientTarget::Cursor], "pangram")
        .unwrap();
    assert!(!status[0].installed());
}

#[test]
fn macos_and_windows_paths_use_only_the_pinned_global_locations() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let macos = Installer::with_context(
        InstallContext::for_test(InstallPlatform::Macos, home.clone())
            .with_executable(root.path().join("pangram")),
    );
    assert_eq!(
        macos.path_for(ClientTarget::ClaudeDesktop).unwrap(),
        home.join("Library/Application Support/Claude/claude_desktop_config.json")
    );
    assert_eq!(
        macos.path_for(ClientTarget::Vscode).unwrap(),
        home.join("Library/Application Support/Code/User/mcp.json")
    );

    let appdata = root.path().join("AppData/Roaming");
    let local = root.path().join("AppData/Local");
    let windows = Installer::with_context(
        InstallContext::for_test(InstallPlatform::Windows, home)
            .with_executable(root.path().join("pangram.exe"))
            .with_env_path("APPDATA", appdata.clone())
            .with_env_path("LOCALAPPDATA", local),
    );
    assert_eq!(
        windows.path_for(ClientTarget::ClaudeDesktop).unwrap(),
        appdata.join("Claude/claude_desktop_config.json")
    );
    assert_eq!(
        windows.path_for(ClientTarget::Vscode).unwrap(),
        appdata.join("Code/User/mcp.json")
    );
    assert_eq!(
        windows.path_for(ClientTarget::Windsurf).unwrap(),
        appdata.join("devin/mcp_config.json")
    );
}

#[test]
fn all_enabled_json_targets_emit_current_owned_shapes() {
    let root = tempfile::tempdir().unwrap();
    let installer = installer(&root);
    for target in ClientTarget::ALL {
        if matches!(target, ClientTarget::Codex | ClientTarget::RooCode) {
            continue;
        }
        installer
            .apply(request(*target, InstallAction::Install, false))
            .unwrap();
        let value: Value = serde_json::from_str(
            &fs::read_to_string(installer.path_for(*target).unwrap()).unwrap(),
        )
        .unwrap();
        let entry = match target {
            ClientTarget::Vscode => &value["servers"]["pangram"],
            ClientTarget::OpenCode => &value["mcp"]["pangram"],
            _ => &value["mcpServers"]["pangram"],
        };
        if *target == ClientTarget::OpenCode {
            assert_eq!(entry["type"], "local");
            assert_eq!(entry["command"][1], "mcp");
        } else if *target == ClientTarget::Cline {
            assert_eq!(entry["transport"]["type"], "stdio");
        } else {
            assert_eq!(entry["args"][0], "mcp");
        }
    }
}
