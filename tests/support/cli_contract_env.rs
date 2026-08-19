//! Shared hermetic process and source-inspection helpers for CLI contracts.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use microck_pangram_cli::cli::{ArgumentSpec, CommandSpec, FULL_GRAMMAR};

struct HermeticRoot {
    root: tempfile::TempDir,
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
    ROOT.get_or_init(|| HermeticRoot {
        root: tempfile::tempdir().unwrap(),
    })
}

pub(crate) fn pangram() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pangram"));
    command.env_remove("PANGRAM_API_KEY");
    for (key, value) in hermetic_env(&hermetic_root().root) {
        command.env(key, value);
    }
    command
}

pub(crate) const HELP: &str = include_str!("../../generated/cli-help.txt");

pub(crate) const PLANNED_TOP_LEVEL_COMMANDS: &[&str] = &[];

pub(crate) const RUNTIME_DEPENDENCIES: &[&str] = &[
    "base64",
    "cap-fs-ext",
    "cap-std",
    "clap",
    "clap_complete",
    "crossterm",
    "directories",
    "ed25519-dalek",
    "fs4",
    "jiff",
    "ratatui",
    "reqwest",
    "rmcp",
    "rpassword",
    "rusqlite",
    "schemars",
    "secrecy",
    "semver",
    "serde",
    "serde_json",
    "sha2",
    "signal-hook",
    "tar",
    "thiserror",
    "toon-format",
    "tokio",
    "tokio-util",
    "toml",
    "url",
    "uuid",
    "windows-sys",
    "xz2",
    "zeroize",
    "zip",
];

pub(crate) const FORBIDDEN_NETWORK_APIS: &[&str] = &[
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

pub(crate) const FORBIDDEN_NETWORK_ENDPOINTS: &[&str] = &[
    "text.external-api.pangram.com",
    "file-external.api.pangram.com",
    "plagiarism.api.pangram.com",
    "github.com/Microck/pangram-cli/releases/latest/download",
];

pub(crate) fn command(path: &[&str]) -> &'static CommandSpec {
    FULL_GRAMMAR
        .commands
        .iter()
        .find(|command| command.path == path)
        .unwrap_or_else(|| panic!("missing command path: {path:?}"))
}

pub(crate) fn argument(command: &'static CommandSpec, name: &str) -> &'static ArgumentSpec {
    command
        .arguments
        .iter()
        .find(|argument| argument.name == name)
        .unwrap_or_else(|| panic!("missing argument {name} on {:?}", command.path))
}

pub(crate) fn rust_source_paths() -> Vec<PathBuf> {
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

pub(crate) fn code_before_line_comment(line: &str) -> &str {
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
