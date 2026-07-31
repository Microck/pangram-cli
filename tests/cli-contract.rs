use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use microck_pangram_cli::cli::{ArgumentSpec, Availability, CommandSpec, FULL_GRAMMAR};
use serde_json::Value;

/// One hermetic filesystem root shared by every compiled-binary spawn in
/// this file. Credential, config, data, and home state live under it so no
/// subprocess can see a stored key, a host credential helper, or a real
/// config. It is built once and kept alive for the whole test process.
struct HermeticRoot {
    _root: tempfile::TempDir,
}

fn hermetic_env(root: &tempfile::TempDir) -> Vec<(String, String)> {
    let home = root.path().join("home");
    let xdg_config = root.path().join("xdg-config");
    let xdg_data = root.path().join("xdg-data");
    let config = root.path().join("pangram.toml");
    let data = root.path().join("data");
    for directory in [&home, &xdg_config, &xdg_data, &data] {
        fs::create_dir_all(directory).unwrap();
    }
    [
        ("HOME", home.to_str().unwrap()),
        ("XDG_CONFIG_HOME", xdg_config.to_str().unwrap()),
        ("XDG_DATA_HOME", xdg_data.to_str().unwrap()),
        ("PANGRAM_CONFIG", config.to_str().unwrap()),
        ("PANGRAM_DATA_DIR", data.to_str().unwrap()),
        ("CI", "true"),
        ("TERM", "dumb"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect()
}

fn hermetic_root() -> &'static HermeticRoot {
    static ROOT: OnceLock<HermeticRoot> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = tempfile::tempdir().unwrap();
        HermeticRoot { _root: root }
    })
}

fn pangram() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pangram"));
    // Hermetic by default: remove any inherited credential override and root
    // all config/data/home state in a private temporary directory so no
    // stored credential or production billing path is reachable. A test that
    // needs a key sets it deliberately on the child it spawns.
    command.env_remove("PANGRAM_API_KEY");
    let root = hermetic_root();
    for (key, value) in hermetic_env(&root._root) {
        command.env(key, value);
    }
    command
}

const HELP: &str = "\
Unofficial Pangram terminal client

Usage: pangram [OPTIONS] [TEXT] [COMMAND]

Commands:
  auth    Manage the locally stored Pangram API key
  config  Inspect and edit the local Pangram configuration
  doctor  Run local diagnostics without network access or credential validation
  detect  Detect AI-generated text through Pangram 4
  help    Print this message or the help of the given subcommand(s)

Arguments:
  [TEXT]  Bare text analyzes it through AI detection; the literal `-` reads stdin

Options:
      --config <PATH>          Explicit configuration file path for this invocation
      --data-dir <PATH>        Explicit history and state directory for this invocation
      --error-format <FORMAT>  Surface failures as a JSON envelope or a text message [possible values: json, text]
      --no-color               Disable terminal color in pretty output
  -h, --help                   Print help
  -V, --version                Print version
";

const PLANNED_TOP_LEVEL_COMMANDS: &[&str] = &[
    "agent",
    "analyze",
    "bulk",
    "completions",
    "history",
    "mcp",
    "plagiarism",
    "skills",
    "task",
    "update",
];

// Phase 2 runtime dependencies: the Phase 1 set plus the async analysis core
// (Tokio runtime utilities, the rustls-only Reqwest client, and
// CancellationToken support). The analysis module owns every network path.
// `toon-format` joins for the canonical TOON projection of the JSON envelope.
const PHASE_2_RUNTIME_DEPENDENCIES: &[&str] = &[
    "clap",
    "directories",
    "jiff",
    "reqwest",
    "rpassword",
    "schemars",
    "secrecy",
    "serde",
    "serde_json",
    "sha2",
    "signal-hook",
    "thiserror",
    "toon-format",
    "tokio",
    "tokio-util",
    "toml",
    "url",
    "uuid",
    "windows-sys",
    "zeroize",
];

const FORBIDDEN_NETWORK_APIS: &[&str] = &[
    "hyper::",
    "ureq::",
    "curl::",
    "std::net::",
    "std::os::unix::net::",
    "tokio::net::",
    "mio::net::",
    "socket2::",
    "TcpStream::",
    "TcpListener::",
    "UdpSocket::",
    "UnixStream::",
    "UnixListener::",
    "Command::new(\"curl\")",
    "Command::new(\"wget\")",
];

const FORBIDDEN_NETWORK_ENDPOINTS: &[&str] = &[
    "text.external-api.pangram.com",
    "file-external.api.pangram.com",
    "plagiarism.api.pangram.com",
    "github.com/Microck/pangram-cli/releases/latest/download",
];

fn command(path: &[&str]) -> &'static CommandSpec {
    FULL_GRAMMAR
        .commands
        .iter()
        .find(|command| command.path == path)
        .unwrap_or_else(|| panic!("missing command path: {path:?}"))
}

fn argument(command: &'static CommandSpec, name: &str) -> &'static ArgumentSpec {
    command
        .arguments
        .iter()
        .find(|argument| argument.name == name)
        .unwrap_or_else(|| panic!("missing argument {name} on {:?}", command.path))
}

fn rust_source_paths() -> Vec<PathBuf> {
    fn collect(directory: &std::path::Path, paths: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(&path, paths);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                paths.push(path);
            }
        }
    }

    let mut paths = Vec::new();
    collect(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut paths,
    );
    paths.sort();
    paths
}

fn code_before_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut index = 0;

    while index + 1 < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b'/' && bytes[index + 1] == b'/' {
            return &line[..index];
        }
        index += 1;
    }

    line
}

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

/// N4: `--save` stays in the normative grammar (Phase 4 history) but is not
/// an available Phase 2 capability: the generated reference marks it planned,
/// and the compiled runtime rejects it before any billable work.
#[test]
fn save_is_planned_in_the_reference_and_rejected_at_runtime() {
    let save = argument(command(&["detect"]), "--save");
    assert_eq!(save.availability, Availability::Planned);

    let output = pangram()
        .args(["detect", "--save", "some text"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
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
fn cargo_metadata_reports_the_exact_phase_one_runtime_dependencies() {
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
    let expected: BTreeSet<_> = PHASE_2_RUNTIME_DEPENDENCIES.iter().copied().collect();
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

    for entry in fs::read_dir(&analysis_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
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
