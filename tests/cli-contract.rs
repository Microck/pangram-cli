use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use microck_pangram_cli::cli::{Availability, CommandKind, FULL_GRAMMAR};
use serde_json::Value;

#[path = "support/cli_contract_env.rs"]
mod harness;

use harness::{
    FORBIDDEN_NETWORK_APIS, FORBIDDEN_NETWORK_ENDPOINTS, HELP, PHASE_4_RUNTIME_DEPENDENCIES,
    PLANNED_TOP_LEVEL_COMMANDS, argument, code_before_line_comment, command, pangram,
    rust_source_paths,
};

#[test]
fn help_lists_only_implemented_command_entries() {
    let output = pangram().arg("--help").output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), HELP);
    assert!(output.stderr.is_empty());
}

/// F5: every compiled-binary spawn in this file is credential/environment
/// hermetic. The shared `pangram()` builder removes any inherited
/// `PANGRAM_API_KEY` and roots config, data, and home state in a private
/// temporary directory, so no stored credential or host environment can
/// reach a production billing path. A bare `auth status` must report no
/// configured credential rather than any stored key.
#[test]
fn spawns_are_credential_and_environment_hermetic() {
    let output = pangram().arg("auth").arg("status").output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["data"]["configured"], false);
    assert_eq!(body["data"]["source"], "none");
}

#[test]
fn short_help_matches_the_exact_help_contract() {
    let output = pangram().arg("-h").output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), HELP);
    assert!(output.stderr.is_empty());
}

#[test]
fn bare_invocation_with_an_empty_stdin_is_input_required_not_help() {
    // A compiled-binary spawn gets a non-TTY (null) stdin: bare dispatch
    // evaluates the source before any help surface, so an empty stdin is the
    // canonical input_required usage error (exit 2), never help text. The
    // all-TTY help fallback cannot be exercised without a real terminal; the
    // PTY harness owns that path.
    let output = pangram().output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("Usage:"), "not help:\n{stdout}");
    let body: Value = serde_json::from_str(stdout.trim_end()).unwrap();
    assert_eq!(body["command"], "detect");
    assert_eq!(body["error"]["code"], "input_required");
}

#[test]
fn version_reports_the_package_version() {
    let output = pangram().arg("--version").output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "pangram 0.1.0\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn short_version_reports_the_package_version() {
    let output = pangram().arg("-V").output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "pangram 0.1.0\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn planned_top_level_names_are_literal_text_and_not_advertised_as_commands() {
    let planned_commands: BTreeSet<_> = FULL_GRAMMAR
        .commands
        .iter()
        .filter(|command| command.availability == Availability::Planned)
        .filter(|command| command.path.len() == 1)
        .filter_map(|command| command.path.first().copied())
        .collect();
    let expected: BTreeSet<_> = PLANNED_TOP_LEVEL_COMMANDS.iter().copied().collect();

    assert_eq!(planned_commands, expected);
    for command in planned_commands {
        // A bare token spelling a planned command name is literal text for
        // detection, not a rejected subcommand (contracts.md 14.1). Without a
        // configured key it reaches the canonical missing-credential failure.
        let output = pangram().arg(command).output().unwrap();

        assert_eq!(output.status.code(), Some(4), "{command}");
        assert!(output.stderr.is_empty(), "{command}");
        let body: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["command"], "detect", "{command}");
        assert_eq!(body["error"]["code"], "missing_api_key", "{command}");

        // The planned command must not be advertised as an available command
        // entry. (An arbitrary substring match cannot be used: the `--data-dir`
        // help legitimately contains the word "history".)
        let listed_commands = HELP
            .lines()
            .skip_while(|line| *line != "Commands:")
            .skip(1)
            .take_while(|line| line.starts_with("  "))
            .map(|line| line.split_whitespace().next().unwrap());
        assert!(
            !listed_commands.clone().any(|name| name == command),
            "{command} must not appear in the help Commands listing"
        );
    }
}

/// The README "Available today" table lists exactly the compiled binary's
/// available top-level command names. This pins the advertised surface to the
/// Rust-owned grammar so a README claim can never drift from the compiled
/// reality in either direction: a newly available command must be listed, and
/// a listed command must really be available. (The bare `pangram` row spells
/// the available root entrypoint, not a subcommand name.)
#[test]
fn readme_available_table_matches_the_available_top_level_grammar() {
    let available_names: BTreeSet<&str> = FULL_GRAMMAR
        .commands
        .iter()
        .filter(|command| command.availability == Availability::Available)
        .filter(|command| command.path.len() == 1)
        .filter(|command| matches!(command.kind, CommandKind::Command | CommandKind::Namespace))
        .filter_map(|command| command.path.first().copied())
        .collect();

    let readme = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("read README.md");

    // Extract the "Available today" table only, stopping at the next
    // section heading, and pull the command token from each `pangram X` row.
    let available_section = readme
        .split("Available today:")
        .nth(1)
        .expect("the README keeps an \"Available today:\" section");
    let available_section = available_section
        .split("\n## ")
        .next()
        .unwrap_or(available_section);
    let available_section = available_section
        .split("\nPlanned:")
        .next()
        .unwrap_or(available_section);

    let mut listed_names = BTreeSet::new();
    for line in available_section.lines() {
        let line = line.trim();
        if !line.starts_with("| `pangram ") {
            continue;
        }
        // `| `pangram detect` | ...` -> "detect".
        let cell = line
            .split('`')
            .nth(1)
            .expect("a backtick-quoted command cell");
        if let Some(name) = cell.strip_prefix("pangram ") {
            // Take the leading command token only (a subcommand path lists
            // its first segment; the bare `pangram` row never matches here).
            let first = name.split_whitespace().next().unwrap_or(name);
            listed_names.insert(first.to_owned());
        }
    }
    let listed_names: BTreeSet<&str> = listed_names.iter().map(String::as_str).collect();

    assert_eq!(
        listed_names, available_names,
        "the README \"Available today\" table must list exactly the available \
         top-level command names from the Rust-owned grammar (no more, no fewer)"
    );
}

/// N4 (Phase 4 Packet C): `--save` graduated on `detect` only. It is
/// available in the Rust-owned grammar and accepted by the compiled parser
/// (the loopback suite owns its end-to-end persistence semantics); the
/// planned `plagiarism`/`analyze` rows keep it planned, and the bulk/task
/// surfaces still reject the flag as unknown.
#[test]
fn save_is_available_on_detect_and_nowhere_else() {
    let save = argument(command(&["detect"]), "--save");
    assert_eq!(save.availability, Availability::Available);

    // The compiled detect parser accepts the flag (no usage error). The
    // hermetic root has no credentials, so the run reaches the canonical
    // missing-key failure instead, proving the parse succeeded.
    let output = pangram()
        .args(["detect", "--save", "some text"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let envelope: Value = serde_json::from_str(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(envelope["error"]["code"], "missing_api_key");

    for path in [&["plagiarism"][..], &["analyze"][..]] {
        let save = argument(command(path), "--save");
        assert_eq!(save.availability, Availability::Planned);
    }
    // The bulk and task surfaces carry no `--save`: unknown-argument usage
    // errors before any source read or network access.
    for arguments in [
        &[
            "bulk",
            "submit",
            "--save",
            "--max-billable-units",
            "5",
            "items.jsonl",
        ][..],
        &["task", "status", "--save", "task-123"][..],
        &["bulk", "status", "--save", "blk-123"][..],
    ] {
        let output = pangram().args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{arguments:?} rejected");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn every_activated_history_argument_is_available() {
    for path in [
        &["history", "list"][..],
        &["history", "show"][..],
        &["history", "search"][..],
        &["history", "delete"][..],
        &["history", "clear"][..],
        &["history", "export"][..],
        &["history", "rerun"][..],
    ] {
        let spec = command(path);
        assert_eq!(spec.availability, Availability::Available);
        for argument in spec.arguments {
            assert_eq!(
                argument.availability,
                Availability::Available,
                "activated argument {} on {path:?} remains planned",
                argument.name
            );
        }
    }
}

#[test]
fn hyphen_leading_unknown_flags_are_usage_errors() {
    for output in [
        pangram().arg("--frobnicate").output().unwrap(),
        pangram()
            .args(["detect", "--no-such-flag"])
            .output()
            .unwrap(),
    ] {
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("unexpected argument"), "{stderr}");
    }
}

#[test]
fn grammar_records_runtime_short_aliases() {
    let help = FULL_GRAMMAR
        .global_arguments
        .iter()
        .find(|argument| argument.name == "--help")
        .unwrap();
    let version = FULL_GRAMMAR
        .global_arguments
        .iter()
        .find(|argument| argument.name == "--version")
        .unwrap();

    assert_eq!(help.aliases, &["-h"]);
    assert_eq!(version.aliases, &["-V"]);
}

#[test]
fn root_text_conflicts_with_implicit_stdin_as_the_detect_source() {
    let root = command(&[]);
    let detect = command(&["detect"]);
    let source_group = root
        .argument_groups
        .iter()
        .find(|group| group.name == "source_category")
        .unwrap();
    let text = argument(root, "TEXT");

    assert_eq!(root.argument_groups, detect.argument_groups);
    assert!(source_group.required);
    assert!(source_group.exclusive);
    assert_eq!(source_group.implicit_members, &["stdin"]);
    assert_eq!(text.group, Some(source_group.name));
    assert_eq!(text.stdin_marker, Some("-"));
}

#[test]
fn analysis_commands_require_one_explicit_or_implicit_source() {
    for path in [
        ["detect"].as_slice(),
        ["plagiarism"].as_slice(),
        ["analyze"].as_slice(),
    ] {
        let command = command(path);
        let source_group = command
            .argument_groups
            .iter()
            .find(|group| group.name == "source_category")
            .unwrap();
        let text = argument(command, "TEXT");
        let file = argument(command, "--file");

        assert!(source_group.required);
        assert!(source_group.exclusive);
        assert_eq!(source_group.implicit_members, &["stdin"]);
        assert_eq!(text.group, Some("source_category"));
        assert_eq!(text.stdin_marker, Some("-"));
        assert_eq!(file.group, Some("source_category"));
    }
}

#[test]
fn bulk_submit_requires_a_path_or_implicit_stdin() {
    let command = command(&["bulk", "submit"]);
    let source_group = command
        .argument_groups
        .iter()
        .find(|group| group.name == "bulk_source")
        .unwrap();
    let jsonl_path = argument(command, "JSONL_PATH");

    assert!(source_group.required);
    assert!(source_group.exclusive);
    assert_eq!(source_group.implicit_members, &["stdin"]);
    assert_eq!(jsonl_path.group, Some("bulk_source"));
    assert_eq!(jsonl_path.stdin_marker, Some("-"));
}

/// contracts.md 14.3 and docs/mcp-contract.md lock bulk submission with no
/// public-dashboard-link field: Pangram's Bulk API documents no
/// public-dashboard-link request or response field. The Rust-owned grammar
/// must not carry a bulk `--public-link`, while the contracted analysis flags
/// (detect now, analyze when Phase 7 arrives) stay untouched.
#[test]
fn bulk_submit_has_no_public_link_in_the_rust_owned_grammar() {
    let bulk = command(&["bulk", "submit"]);
    assert!(
        !bulk
            .arguments
            .iter()
            .any(|argument| argument.name == "--public-link"),
        "bulk submit must not carry --public-link (contracts.md 14.3)"
    );

    let detect = argument(command(&["detect"]), "--public-link");
    assert_eq!(detect.availability, Availability::Available);
    let analyze = argument(command(&["analyze"]), "--public-link");
    assert_eq!(analyze.availability, Availability::Planned);

    // No bulk or task command accepts the flag.
    for path in [
        ["bulk", "submit"].as_slice(),
        ["bulk", "status"].as_slice(),
        ["bulk", "wait"].as_slice(),
        ["bulk", "results"].as_slice(),
        ["task", "status"].as_slice(),
        ["task", "wait"].as_slice(),
    ] {
        let spec = command(path);
        assert!(
            !spec
                .arguments
                .iter()
                .any(|argument| argument.name == "--public-link"),
            "{path:?} must not carry --public-link"
        );
    }
}

/// Phase 3 packet 4 activated the bulk and task surfaces: the grammar marks
/// every entry available, the compiled help advertises both parents, and a
/// bare invocation of either parent is a Clap usage error (exit 2), never
/// literal detect text.
#[test]
fn bulk_and_task_commands_are_available_at_the_activation_packet() {
    for path in [
        ["bulk"].as_slice(),
        ["bulk", "submit"].as_slice(),
        ["bulk", "status"].as_slice(),
        ["bulk", "wait"].as_slice(),
        ["bulk", "results"].as_slice(),
        ["task"].as_slice(),
        ["task", "status"].as_slice(),
        ["task", "wait"].as_slice(),
    ] {
        let spec = command(path);
        assert_eq!(
            spec.availability,
            Availability::Available,
            "{path:?} must be available at the Phase 3 activation packet"
        );
    }

    let listed_commands: Vec<&str> = HELP
        .lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    for name in ["bulk", "task"] {
        assert!(
            listed_commands.contains(&name),
            "{name} must appear in the compiled help Commands listing"
        );

        // A bare parent is a usage error, never literal detection text.
        let output = pangram().arg(name).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{name}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("Usage:"), "{name}: {stderr}");
    }
}

#[test]
fn mcp_history_mutations_require_history_access() {
    let history_mutations = argument(command(&["mcp"]), "--allow-history-mutations");

    assert_eq!(history_mutations.requires, &["--history"]);
}

#[test]
fn mcp_file_roots_are_explicit_repeatable_startup_options() {
    let file_root = argument(command(&["mcp"]), "--allow-file-root");

    assert_eq!(file_root.value_name, Some("PATH"));
    assert!(file_root.repeatable);
    assert!(!file_root.required);
}

#[test]
fn cargo_metadata_reports_the_exact_runtime_dependencies() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(&manifest_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages = metadata["packages"].as_array().unwrap();
    let members = metadata["workspace_members"].as_array().unwrap();
    let dependencies = packages[0]["dependencies"].as_array().unwrap();

    assert_eq!(packages.len(), 1);
    assert_eq!(members.len(), 1);
    assert_eq!(packages[0]["name"], "microck-pangram-cli");
    assert!(packages[0]["publish"].as_array().unwrap().is_empty());

    let runtime_dependencies: BTreeSet<_> = dependencies
        .iter()
        .filter(|dependency| dependency["kind"].is_null())
        .map(|dependency| dependency["name"].as_str().unwrap())
        .collect();
    let expected: BTreeSet<_> = PHASE_4_RUNTIME_DEPENDENCIES.iter().copied().collect();
    assert_eq!(runtime_dependencies, expected);
}

/// HTTP client construction is allowed only inside the analysis module, the
/// sole owner of Pangram protocol behavior.
#[test]
fn http_client_paths_live_in_the_analysis_module_only() {
    let mut violations = Vec::new();

    for path in rust_source_paths() {
        if path
            .components()
            .any(|component| component.as_os_str() == "analysis")
        {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        for (line_index, line) in source.lines().enumerate() {
            let code = code_before_line_comment(line);
            for forbidden in ["reqwest::"] {
                if code.contains(forbidden) {
                    violations.push(format!(
                        "{}:{} contains {forbidden:?}",
                        path.display(),
                        line_index + 1
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "HTTP client paths outside src/analysis:\n{}",
        violations.join("\n")
    );
}

#[test]
fn source_uses_no_bypassing_network_path() {
    let mut violations = Vec::new();

    for path in rust_source_paths() {
        let source = fs::read_to_string(&path).unwrap();
        for (line_index, line) in source.lines().enumerate() {
            let code = code_before_line_comment(line);
            for forbidden in FORBIDDEN_NETWORK_APIS {
                if code.contains(forbidden) {
                    violations.push(format!(
                        "{}:{} contains {forbidden:?}",
                        path.display(),
                        line_index + 1
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "runtime source contains bypassing network paths:\n{}",
        violations.join("\n")
    );
}

/// Production endpoints are owned by the analysis module as compile-time
/// constants. No environment, flag, or configuration path may select them.
#[test]
fn production_endpoints_are_analysis_owned_constants() {
    let analysis_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("analysis");
    let mut endpoint_sites = Vec::new();
    let mut override_violations = Vec::new();

    // Scan every Rust source under src/analysis, including any future nested
    // submodule, so the no-network-override guard follows the module tree
    // rather than only the top level.
    for path in rust_source_paths() {
        if !path.starts_with(&analysis_dir) {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        for endpoint in FORBIDDEN_NETWORK_ENDPOINTS {
            if source.contains(endpoint) {
                endpoint_sites.push(format!("{}: {endpoint}", path.display()));
            }
        }
        for (line_index, line) in source.lines().enumerate() {
            let code = code_before_line_comment(line);
            for forbidden in ["PANGRAM_ENDPOINT", "PANGRAM_API_URL", "endpoint_override"] {
                if code.contains(forbidden) {
                    override_violations.push(format!(
                        "{}:{} contains {forbidden:?}",
                        path.display(),
                        line_index + 1
                    ));
                }
            }
        }
    }

    assert!(
        !endpoint_sites.is_empty(),
        "the analysis module must own the production endpoint constants"
    );
    assert!(
        override_violations.is_empty(),
        "endpoint override paths are forbidden:\n{}",
        override_violations.join("\n")
    );
}

/// The CLI interruption path must compile on every supported target.
/// `signal_hook::iterator` (the `Signals` blocking driver) is gated
/// `#[cfg(all(not(windows), feature = "iterator"))]`, so referencing it from
/// shared source fails the Windows build with E0433 even though it compiles
/// on Unix. The Phase 2 SIGINT flow must use the cross-platform
/// `signal_hook::low_level::register` handler instead. This guard asserts the
/// Unix-only API is never written in non-comment source so the native Windows
/// CI leg never regresses on it again.
#[test]
fn source_avoids_unix_only_signal_hook_iterator() {
    let mut violations = Vec::new();

    for path in rust_source_paths() {
        let source = fs::read_to_string(&path).unwrap();
        for (line_index, line) in source.lines().enumerate() {
            let code = code_before_line_comment(line);
            for forbidden in ["signal_hook::iterator", "Signals::new", "signals.forever("] {
                if code.contains(forbidden) {
                    violations.push(format!(
                        "{}:{} contains Unix-only signal API {forbidden:?}",
                        path.display(),
                        line_index + 1
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "shared source must use the cross-platform signal_hook::low_level::register, not the Unix-only iterator driver:\n{}",
        violations.join("\n")
    );
}
