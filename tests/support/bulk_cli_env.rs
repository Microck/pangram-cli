//! Shared helpers for the compiled-binary bulk/task loopback suites
//! (dev-tools only). These are test-support code consumed through `#[path]`
//! by more than one test crate, so a few fixtures are unused in each and the
//! whole module allows that rather than peppering `cfg` gates.
//!
//! The helpers own the isolated invocation environment, the stdin driver,
//! the envelope and no-leak assertions, and the scripted bulk response
//! shapes. They never echo header or key values.

#![allow(dead_code)]

#[path = "protocol_loopback/mod.rs"]
pub mod fixture;

use std::io::Write as _;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

use fixture::{SYNTHETIC_KEY, pangram4_success};

pub const BULK_ID: &str = "blk_fixture_001";
pub const KEY_FRAGMENT: &str = "synthetic_key_0000";

/// Sends SIGINT to a running child by PID (POSIX-only; skipped elsewhere).
#[cfg(unix)]
pub fn interrupt(child: &mut std::process::Child) {
    let pid = i32::try_from(child.id()).expect("child PID fits i32");
    // SAFETY: `pid` is a live child of this process and `SIGINT` is valid.
    let result = unsafe { kill(pid, 2) };
    assert_eq!(result, 0, "raise(SIGINT) on the child must succeed");
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

pub fn pangram() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pangram"))
}

/// An isolated invocation: credential, config, and data state rooted in one
/// temporary directory, with `CI` set (never interactive) and a synthetic key.
pub struct Isolated {
    _root: TempDir,
    env: Vec<(String, String)>,
}

impl Isolated {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let config = root.path().join("config.toml");
        let data = root.path().join("data");
        for directory in [&home, &data] {
            std::fs::create_dir_all(directory).unwrap();
        }
        let env = [
            ("HOME", home.to_str().unwrap()),
            ("XDG_CONFIG_HOME", home.to_str().unwrap()),
            ("XDG_DATA_HOME", home.to_str().unwrap()),
            ("PANGRAM_CONFIG", config.to_str().unwrap()),
            ("PANGRAM_DATA_DIR", data.to_str().unwrap()),
            ("CI", "true"),
            ("TERM", "dumb"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
        Self { _root: root, env }
    }

    pub fn command(&self, endpoint: &str) -> Command {
        let mut command = pangram();
        command
            .env("PANGRAM_API_KEY", SYNTHETIC_KEY)
            .env("PANGRAM_DETECT_ENDPOINT", endpoint);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }

    pub fn command_without_key(&self, endpoint: &str) -> Command {
        let mut command = pangram();
        command.env_remove("PANGRAM_API_KEY");
        command.env("PANGRAM_DETECT_ENDPOINT", endpoint);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
}

/// Runs `pangram` with `input` piped on stdin, returning output.
pub fn spawn_with_stdin(mut command: Command, args: &[&str], input: &[u8]) -> std::process::Output {
    let mut child = command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pangram");
    if let Err(error) = child.stdin.as_mut().unwrap().write_all(input) {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "stdin: {error}"
        );
    }
    child.wait_with_output().expect("await pangram")
}

pub fn stdout_envelope(output: &std::process::Output) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    serde_json::from_str(stdout.trim_end())
        .unwrap_or_else(|error| panic!("stdout is one JSON envelope: {error}\nstdout: {stdout}"))
}

pub fn assert_no_leak(output: &std::process::Output) {
    for surface in [
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ] {
        assert!(
            !surface.contains(SYNTHETIC_KEY) && !surface.contains(KEY_FRAGMENT),
            "the credential must never appear in any output: {surface}"
        );
        assert!(
            !surface.to_ascii_lowercase().contains("x-api-key"),
            "auth header names stay out of output"
        );
    }
}

/// One JSONL line: an optional caller id plus the text.
pub fn jsonl(items: &[(&str, &str)]) -> String {
    items
        .iter()
        .map(|(id, text)| json!({"id": id, "text": text}).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The documented 202 acceptance over `total_items`, all accepted.
pub fn accepted_202(total: u64) -> Value {
    let accepted: Vec<Value> = (0..total)
        .map(|index| {
            json!({
                "index": index,
                "id": format!("row-{index:03}"),
                "task_id": format!("task-{index:03}"),
            })
        })
        .collect();
    json!({
        "bulk_id": BULK_ID,
        "status": "queued",
        "total_items": total,
        "accepted_items": accepted,
        "failed_items": []
    })
}

/// The documented status body for a job over `total` with the given counters.
/// A terminal status carries a non-null `completed_at`; a non-terminal status
/// carries null (contracts.md 9.1).
pub fn status_body(status: &str, total: u64, accepted: u64, succeeded: u64, failed: u64) -> Value {
    let terminal = matches!(status, "succeeded" | "failed" | "partial");
    json!({
        "bulk_id": BULK_ID,
        "status": status,
        "total_items": total,
        "accepted": accepted,
        "succeeded": succeeded,
        "failed": failed,
        "created_at": "1760000000.0",
        "completed_at": if terminal { json!("1760000001.0") } else { Value::Null }
    })
}

/// A one-page results document covering `succeeds` terminal two-word items.
pub fn results_page(offset: u64, limit: u64, succeeds: u64, total: u64) -> Value {
    let items: Vec<Value> = (0..succeeds)
        .map(|index| {
            json!({
                "index": offset + index,
                "id": format!("row-{:03}", offset + index),
                "task_id": format!("task-{:03}", offset + index),
                "stage": "STAGE_SUCCESS",
                "error": null,
                "result": pangram4_success("synthetic loopback words"),
            })
        })
        .collect();
    json!({
        "bulk_id": BULK_ID,
        "offset": offset,
        "limit": limit,
        "total_items": total,
        "items": items,
        "failed_items": []
    })
}
