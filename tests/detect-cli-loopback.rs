//! Compiled-binary detect integration against the real loopback Pangram 4
//! fixture. No mocks, no live Pangram, no real credentials.
//!
//! Each test boots the Axum fixture on an ephemeral loopback port, points the
//! `dev-tools`-gated `PANGRAM_DETECT_ENDPOINT` override at it, runs the
//! compiled `pangram` binary in an isolated config/data environment, and
//! asserts the exact stdout envelope, stderr separation, exit code, and the
//! recorded upstream request grammar. The synthetic key and content are
//! fixture constants; assertion helpers never echo header or key values.

#![cfg(feature = "dev-tools")]

#[path = "support/protocol_loopback/mod.rs"]
mod fixture;

use std::io::Write as _;
use std::process::{Command, Stdio};

use serde_json::Value;
use tempfile::TempDir;

use fixture::{ProtocolFixture, SYNTHETIC_KEY, Step, TASK_ID, pangram4_success};

/// Various lengths of the synthetic key that would prove a leak if printed.
const KEY_FRAGMENT: &str = "synthetic_key_0000";

fn pangram() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pangram"))
}

/// An isolated invocation: credential, config, and data state rooted in one
/// temporary directory, with `CI` set (never interactive) and a synthetic key.
struct Isolated {
    _root: TempDir,
    env: Vec<(String, String)>,
}

impl Isolated {
    fn new() -> Self {
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

    fn command(&self, endpoint: &str) -> Command {
        let mut command = pangram();
        command
            .env("PANGRAM_API_KEY", SYNTHETIC_KEY)
            .env("PANGRAM_DETECT_ENDPOINT", endpoint);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
}

fn stdout_envelope(output: &std::process::Output) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    serde_json::from_str(stdout.trim_end())
        .unwrap_or_else(|error| panic!("stdout is one JSON envelope: {error}\nstdout: {stdout}"))
}

fn assert_no_leak(output: &std::process::Output) {
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

/// A full wait for a terminal success: POST acceptance then a successful
/// GET, rendered as the default canonical JSON with progress-free stderr.
#[tokio::test(flavor = "multi_thread")]
async fn detect_literal_text_waits_and_prints_canonical_success() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    let text = "The quick brown fox jumps over the lazy dog repeatedly today";
    fixture.on_poll(Step::Json(pangram4_success(text)));

    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["detect", text])
        .output()
        .expect("run pangram detect");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, b"", "no progress on a non-TTY pipe");
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["schema_version"], "1");
    assert_eq!(envelope["command"], "detect");
    assert!(envelope.get("error").is_none());
    let data = &envelope["data"];
    assert_eq!(data["status"], "succeeded");
    assert_eq!(data["submission_outcome"], "terminal");
    assert_eq!(data["input"]["origin"], "literal");
    assert_eq!(data["input"]["type"], "text");
    assert!(data["input"]["text"].is_null() || data["input"].get("text").is_none());
    assert_eq!(data["checks"][0]["kind"], "ai_detection");
    assert_eq!(data["checks"][0]["status"], "succeeded");
    assert_eq!(data["checks"][0]["result"]["classification"], "human");
    assert_eq!(data["provenance"]["provider"], "pangram");
    assert_eq!(data["provenance"]["upstream_version"], "4.0");
    assert!(envelope["meta"]["started_at"].is_string());
    assert!(envelope["meta"]["duration_ms"].is_number());
    assert_no_leak(&output);

    // Exactly one billable POST and one safe GET, both to the task route.
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(fixture.get_count(), 1);
    let requests = fixture.requests();
    let submit = &requests[0];
    assert_eq!(submit.method, "POST");
    assert_eq!(submit.path, "/task");
    assert!(submit.header_equals("x-api-key", SYNTHETIC_KEY));
    let body = submit.body_json();
    assert_eq!(body["text"], text);
    assert_eq!(body["model"], "pangram-4");
    assert_eq!(body["public_dashboard_link"], false);

    fixture.shutdown().await;
}

/// A piped stdin resolves to the stdin input origin and detects.
#[tokio::test(flavor = "multi_thread")]
async fn detect_from_piped_stdin_uses_stdin_origin() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    let text = "Piped content arrives on standard input for detection";
    fixture.on_poll(Step::Json(pangram4_success(text)));

    let isolated = Isolated::new();
    let mut child = isolated
        .command(fixture.base_url())
        .args(["detect", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pangram detect");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(text.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("await pangram");

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["input"]["origin"], "stdin");
    assert_eq!(envelope["data"]["status"], "succeeded");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// `--detach` reports a running analysis with upstream identity and exits 0
/// without waiting for a terminal poll.
#[tokio::test(flavor = "multi_thread")]
async fn detect_detach_reports_accepted_running_without_terminal_poll() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    // No poll is scripted: detach must not wait.

    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "--detach", "detach me before the result"])
        .output()
        .expect("run pangram detect --detach");

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "detect");
    assert_eq!(envelope["data"]["status"], "queued");
    assert_eq!(envelope["data"]["submission_outcome"], "accepted");
    assert_eq!(envelope["data"]["checks"][0]["status"], "queued");
    assert_eq!(
        envelope["data"]["provenance"]["upstream_task_ids"],
        Value::Array(vec![Value::String(TASK_ID.to_owned())])
    );
    // The diagnostic note carries identity guidance to stderr without content.
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    assert!(stderr.contains("detached"), "{stderr}");
    assert_no_leak(&output);
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(fixture.get_count(), 0, "detach never polls");
    fixture.shutdown().await;
}

/// `--include-input` embeds the submitted text in the canonical input record.
#[tokio::test(flavor = "multi_thread")]
async fn detect_include_input_embeds_the_submitted_text() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    let text = "This exact sentence must round trip into the input record";
    fixture.on_poll(Step::Json(pangram4_success(text)));

    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "--include-input", text])
        .output()
        .expect("run pangram detect --include-input");

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["input"]["text"], text);
    fixture.shutdown().await;
}

/// A repeated `--file` list defaults to JSONL: one canonical envelope per
/// line, inputs analyzed in order.
#[tokio::test(flavor = "multi_thread")]
async fn detect_repeated_files_default_to_jsonl_lines_in_order() {
    let fixture = ProtocolFixture::start().await;
    let first_text = "First file content with several words to analyze";
    let second_text = "Second file content also analyzed in turn here";
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": "task-aaa"})));
    fixture.on_poll(Step::Json(pangram4_success(first_text)));
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": "task-bbb"})));
    fixture.on_poll(Step::Json(pangram4_success(second_text)));

    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.txt");
    let second = root.path().join("second.txt");
    std::fs::write(&first, first_text).unwrap();
    std::fs::write(&second, second_text).unwrap();

    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args([
            "detect",
            "--file",
            first.to_str().unwrap(),
            "--file",
            second.to_str().unwrap(),
        ])
        .output()
        .expect("run pangram detect with two files");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert!(!stdout.starts_with('['), "JSONL never wraps in an array");
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "one canonical envelope per line");
    let first_env: Value = serde_json::from_str(lines[0]).unwrap();
    let second_env: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(first_env["data"]["input"]["origin"], "file");
    assert_eq!(first_env["data"]["input"]["name"], "first.txt");
    assert_eq!(second_env["data"]["input"]["origin"], "file");
    assert_eq!(second_env["data"]["input"]["name"], "second.txt");
    assert_eq!(first_env["data"]["status"], "succeeded");
    assert_eq!(second_env["data"]["status"], "succeeded");
    assert_no_leak(&output);
    assert_eq!(fixture.post_count(), 2);
    fixture.shutdown().await;
}

/// An upstream terminal failure yields a failed analysis (exit 1) with a
/// canonical upstream_analysis_failed check error and sanitized message.
#[tokio::test(flavor = "multi_thread")]
async fn detect_upstream_failure_maps_to_failed_analysis_exit() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(serde_json::json!({
        "stage": "STAGE_FAILED",
        "error_message": "the submitted text was too short to analyze reliably"
    })));

    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "tiny"])
        .output()
        .expect("run pangram detect");

    assert_eq!(output.status.code(), Some(1));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["status"], "failed");
    assert_eq!(
        envelope["data"]["checks"][0]["error"]["code"],
        "upstream_analysis_failed"
    );
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// A 401 rejection maps to the authentication exit with an invalid_api_key
/// envelope and never echoes the key.
#[tokio::test(flavor = "multi_thread")]
async fn detect_rejected_key_maps_to_invalid_api_key_and_scrubs() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Status(401, None, None));

    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "some text that would be analyzed"])
        .output()
        .expect("run pangram detect");

    assert_eq!(output.status.code(), Some(4));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "invalid_api_key");
    assert_no_leak(&output);
    fixture.shutdown().await;
}
