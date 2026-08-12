//! Compiled-binary detect integration against the real loopback Pangram 4
//! fixture. No mocks, no live Pangram, no real credentials.
//!
//! Each test boots the Axum fixture on an ephemeral loopback port, passes it
//! to the development-only compiled test driver, enters the real CLI adapter,
//! and
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

/// Sends SIGINT to a running child by PID. Signal-driven interruption is a
/// POSIX-only behavior; the compiled-binary SIGINT coverage is cfg-gated to
/// unix and skipped where raising a signal is not available.
#[cfg(unix)]
fn interrupt(child: &mut std::process::Child) {
    let pid = nix_pid(child);
    // SAFETY: `pid` is a live child of this process and `SIGINT` is a valid
    // signal number; the call has no memory-safety surface.
    let result = unsafe { libc_kill(pid, 2) };
    assert_eq!(result, 0, "raise(SIGINT) on the child must succeed");
}

#[cfg(unix)]
fn nix_pid(child: &std::process::Child) -> i32 {
    i32::try_from(child.id()).expect("child PID fits i32")
}

#[cfg(unix)]
unsafe fn libc_kill(pid: i32, signal: i32) -> i32 {
    // Resolved through the C veneer so no extra crate enters the dev graph.
    unsafe { kill(pid, signal) }
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

/// Various lengths of the synthetic key that would prove a leak if printed.
const KEY_FRAGMENT: &str = "synthetic_key_0000";

fn pangram() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pangram"))
}

fn test_driver() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pangram-test-driver"))
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
        let mut command = test_driver();
        command.env("PANGRAM_API_KEY", SYNTHETIC_KEY).arg(endpoint);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }

    fn command_without_key(&self) -> Command {
        let mut command = pangram();
        command.env_remove("PANGRAM_API_KEY");
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
}

/// Spawns `pangram` with piped stdin/stdout/stderr and writes `input` to
/// stdin, returning the finished output. Used by bare-dispatch and
/// interruption tests that drive a real pipe.
fn spawn_with_stdin(mut command: Command, input: &[u8]) -> std::process::Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pangram");
    if let Err(error) = child.stdin.as_mut().unwrap().write_all(input) {
        // A child that exits before reading closes its end of the pipe.
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "stdin: {error}"
        );
    }
    child.wait_with_output().expect("await pangram")
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
    if let Err(error) = child.stdin.as_mut().unwrap().write_all(text.as_bytes()) {
        // A child that exits before draining stdin closes the pipe; tolerate
        // that here as `spawn_with_stdin` already does, while still failing
        // loudly on any other write error.
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "stdin: {error}"
        );
    }
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

/// An upstream terminal failure yields a failed analysis (exit 6) with a
/// canonical upstream_analysis_failed check error and sanitized message.
/// The exit derives from the check error's upstream category, locked in
/// contracts.md section 12, not the general-failure default.
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

    assert_eq!(
        output.status.code(),
        Some(6),
        "upstream analysis failure exits 6"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["status"], "failed");
    assert_eq!(
        envelope["data"]["checks"][0]["error"]["code"],
        "upstream_analysis_failed"
    );
    assert_eq!(
        envelope["data"]["checks"][0]["error"]["category"],
        "upstream"
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

/// F1: a bare launch with piped stdin detects; it must not print help.
#[tokio::test(flavor = "multi_thread")]
async fn bare_piped_stdin_detects_instead_of_printing_help() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    let text = "Bare piped content analyzes through the implicit detect path";
    fixture.on_poll(Step::Json(pangram4_success(text)));

    let isolated = Isolated::new();
    let output = spawn_with_stdin(isolated.command(fixture.base_url()), text.as_bytes());

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "detect");
    assert_eq!(envelope["data"]["input"]["origin"], "stdin");
    assert_eq!(envelope["data"]["status"], "succeeded");
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert!(
        !stdout.contains("Usage:"),
        "bare piped stdin must not print help: {stdout}"
    );
    assert_no_leak(&output);
    assert_eq!(fixture.post_count(), 1);
    fixture.shutdown().await;
}

/// F1: a bare launch over an empty pipe is the canonical input_required
/// usage error (exit 2), not help and not a billable call.
#[tokio::test(flavor = "multi_thread")]
async fn bare_empty_pipe_is_input_required_without_billing() {
    let fixture = ProtocolFixture::start().await;

    let isolated = Isolated::new();
    let output = spawn_with_stdin(isolated.command(fixture.base_url()), b"");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stderr, b"");
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert!(
        !stdout.contains("Usage:"),
        "empty pipe must not print help: {stdout}"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "input_required");
    assert_eq!(fixture.post_count(), 0, "no billable POST for empty input");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// F1: a bare launch over a whitespace-only pipe is input_required (exit 2).
#[tokio::test(flavor = "multi_thread")]
async fn bare_whitespace_pipe_is_input_required() {
    let fixture = ProtocolFixture::start().await;

    let isolated = Isolated::new();
    let output = spawn_with_stdin(isolated.command(fixture.base_url()), b"   \n\t  ");

    assert_eq!(output.status.code(), Some(2));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "input_required");
    assert_eq!(fixture.post_count(), 0);
    fixture.shutdown().await;
}

/// F2: when no --timeout is supplied there is no hidden local wait ceiling.
/// The observation waits for the terminal poll with `WaitOptions::UNBOUNDED`;
/// the loopback proves the long-running task still reaches success.
#[tokio::test(flavor = "multi_thread")]
async fn detect_without_timeout_waits_unbounded_for_terminal() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    let text = "A task that stays in progress across several observation cycles";
    fixture.on_poll(Step::Json(
        serde_json::json!({"stage": "STAGE_PREPROCESSING"}),
    ));
    fixture.on_poll(Step::Json(serde_json::json!({"stage": "STAGE_INFERENCE"})));
    fixture.on_poll(Step::Json(pangram4_success(text)));

    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["detect", text])
        .output()
        .expect("run pangram detect");

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["status"], "succeeded");
    // Multiple in-progress polls were consumed, proving the wait was not cut
    // by a local ceiling before the terminal response.
    assert!(
        fixture.get_count() >= 3,
        "expected >=3 polls, got {}",
        fixture.get_count()
    );
    fixture.shutdown().await;
}

/// F3: Ctrl+C during an in-flight billable POST reports the ambiguous
/// acceptance (submission_outcome_unknown) and exits 130, never the false
/// definite "no remote action" claim, and never replays the POST.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn sigint_during_in_flight_post_reports_ambiguous_acceptance() {
    let fixture = ProtocolFixture::start().await;
    // The submit POST hangs: it reaches the fixture and is recorded, but no
    // acceptance response is ever produced.
    fixture.on_submit(Step::Hang);

    let isolated = Isolated::new();
    let mut child = isolated
        .command(fixture.base_url())
        .args(["detect", "text whose submission is interrupted mid-flight"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pangram detect");

    // Wait until the POST actually reaches the fixture so the send is
    // unambiguously issued, then interrupt.
    fixture.wait_for_posts(1).await;
    interrupt(&mut child);
    let output = child.wait_with_output().expect("await pangram");

    assert_eq!(output.status.code(), Some(130), "interrupted by the user");
    // Exactly one POST was issued and never replayed.
    assert_eq!(
        fixture.post_count(),
        1,
        "the ambiguous send is never replayed"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "submission_outcome_unknown");
    assert_eq!(envelope["error"]["retryable"], false);
    assert_eq!(
        envelope["error"]["recovery"]["message"],
        "A manual retry may create a second billable operation."
    );
    // The reported identity carries the local analysis id and request hash.
    assert!(envelope["error"]["details"]["analysis_id"].is_string());
    assert!(envelope["error"]["details"]["request_sha256"].is_string());
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// F3: SIGINT after acceptance (during wait) still exits 130 with identity.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn sigint_during_wait_exits_130_with_identity() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(serde_json::json!({"stage": "STAGE_INFERENCE"})));
    fixture.on_poll(Step::Hang);

    let isolated = Isolated::new();
    let mut child = isolated
        .command(fixture.base_url())
        .args(["detect", "text that stays running until interrupted"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pangram detect");

    fixture.wait_for_posts(1).await;
    fixture.wait_for_gets(1).await;
    interrupt(&mut child);
    let output = child.wait_with_output().expect("await pangram");

    assert_eq!(output.status.code(), Some(130));
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    assert!(
        stderr.contains("interrupted") || stderr.contains("local analysis id"),
        "interruption reports identity: {stderr}"
    );
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// F4: explicit --format json with repeated files wraps the ordered series in
/// one envelope with an ordered analysis array, and never bills then fails
/// rendering.
#[tokio::test(flavor = "multi_thread")]
async fn repeated_files_explicit_json_wraps_ordered_array() {
    let fixture = ProtocolFixture::start().await;
    let first_text = "First ordered file content for the array envelope";
    let second_text = "Second ordered file content for the array envelope";
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
            "--format",
            "json",
            "--file",
            first.to_str().unwrap(),
            "--file",
            second.to_str().unwrap(),
        ])
        .output()
        .expect("run detect --format json with two files");

    assert_eq!(
        output.status.code(),
        Some(0),
        "no post-billing render failure"
    );
    let envelope = stdout_envelope(&output);
    assert!(envelope.get("error").is_none());
    let data = &envelope["data"];
    assert!(data.is_array(), "json data is the ordered analysis array");
    let analyses = data.as_array().unwrap();
    assert_eq!(analyses.len(), 2);
    assert_eq!(analyses[0]["input"]["name"], "first.txt");
    assert_eq!(analyses[1]["input"]["name"], "second.txt");
    assert_eq!(analyses[0]["status"], "succeeded");
    assert_eq!(analyses[1]["status"], "succeeded");
    assert_eq!(fixture.post_count(), 2, "both files were analyzed");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// F4: explicit --format toon with repeated files renders the ordered series
/// through the single-document TOON projection without a billing/render split.
#[tokio::test(flavor = "multi_thread")]
async fn repeated_files_explicit_toon_wraps_ordered_series() {
    let fixture = ProtocolFixture::start().await;
    let first_text = "First toon projected file content here";
    let second_text = "Second toon projected file content here";
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
            "--format",
            "toon",
            "--file",
            first.to_str().unwrap(),
            "--file",
            second.to_str().unwrap(),
        ])
        .output()
        .expect("run detect --format toon with two files");

    assert_eq!(
        output.status.code(),
        Some(0),
        "toon must render the ordered series"
    );
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert!(!stdout.trim().is_empty(), "toon output is non-empty");
    // TOON is a projection of one envelope; the series travels as data.
    assert!(stdout.contains("data"), "{stdout}");
    assert_eq!(fixture.post_count(), 2);
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// F4: explicit --format pretty with repeated files renders the human
/// projection of the whole ordered series without a billing/render split.
#[tokio::test(flavor = "multi_thread")]
async fn repeated_files_explicit_pretty_renders_series() {
    let fixture = ProtocolFixture::start().await;
    let first_text = "First pretty projected file content here";
    let second_text = "Second pretty projected file content here";
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
            "--format",
            "pretty",
            "--no-color",
            "--file",
            first.to_str().unwrap(),
            "--file",
            second.to_str().unwrap(),
        ])
        .output()
        .expect("run detect --format pretty with two files");

    assert_eq!(
        output.status.code(),
        Some(0),
        "pretty must render the ordered series"
    );
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert!(
        stdout.contains("Analyses"),
        "pretty lists the ordered series: {stdout}"
    );
    assert_eq!(fixture.post_count(), 2);
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// F6: explicit --format pretty with no configured key emits a sanitized text
/// message on stderr, empty stdout, exit 4.
#[tokio::test(flavor = "multi_thread")]
async fn pretty_format_missing_key_emits_text_error_exit_4() {
    let isolated = Isolated::new();
    let output = isolated
        .command_without_key()
        .args(["detect", "--format", "pretty", "--no-color", "some text"])
        .output()
        .expect("run detect --format pretty without a key");

    assert_eq!(output.status.code(), Some(4));
    assert!(
        output.stdout.is_empty(),
        "stdout stays empty for a text error"
    );
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    assert!(
        stderr.contains("error:"),
        "stderr carries the text error: {stderr}"
    );
    assert_no_leak(&output);
}

/// F6: --error-format json overrides the pretty default back to a stdout JSON
/// envelope even when --format pretty is selected.
#[tokio::test(flavor = "multi_thread")]
async fn pretty_format_with_json_error_override_emits_json_envelope() {
    let isolated = Isolated::new();
    let output = isolated
        .command_without_key()
        .args([
            "detect",
            "--format",
            "pretty",
            "--no-color",
            "--error-format",
            "json",
            "some text",
        ])
        .output()
        .expect("run detect --format pretty --error-format json without a key");

    assert_eq!(output.status.code(), Some(4));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "missing_api_key");
    assert_no_leak(&output);
}

/// Greptile P1 (Option A): a repeated `--file` run preserves completed
/// billable analyses when a later file fails. The default JSONL stream keeps
/// the ordered series: the first file's success envelope, then the failed
/// file's synthesized member with status `failed`, honest `submission_outcome`
/// (`acceptance_unknown` because the ambiguous POST may have reached the
/// peer), real input metadata, a canonical `submission_outcome_unknown` check
/// error, and no fabricated upstream identity. The process exits 3 (partial).
#[tokio::test(flavor = "multi_thread")]
async fn repeated_files_preserve_completed_and_report_failed_member_as_partial() {
    let fixture = ProtocolFixture::start().await;
    let first_text = "First file that completes billing successfully here";
    let second_text = "Second file whose submission POST is issued then lost";
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": "task-aaa"})));
    fixture.on_poll(Step::Json(pangram4_success(first_text)));
    // The second file's billable POST is issued, then the connection drops:
    // ambiguous delivery, never replayed, honest submission_outcome_unknown.
    fixture.on_submit(Step::Hang);

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
        .expect("run pangram detect with a completing and a failing file");

    assert_eq!(output.status.code(), Some(3), "partial result exits 3");
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert!(!stdout.starts_with('['), "JSONL never wraps in an array");
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "one ordered envelope per analyzed file");

    let succeeded: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(succeeded["data"]["status"], "succeeded");
    assert_eq!(succeeded["data"]["input"]["name"], "first.txt");
    assert_eq!(succeeded["data"]["input"]["origin"], "file");
    assert_eq!(succeeded["data"]["checks"][0]["status"], "succeeded");

    let failed: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(failed["data"]["status"], "failed");
    assert_eq!(failed["data"]["input"]["name"], "second.txt");
    assert_eq!(failed["data"]["input"]["origin"], "file");
    // The most precise honest outcome for an ambiguous issued POST.
    assert_eq!(failed["data"]["submission_outcome"], "acceptance_unknown");
    assert_eq!(failed["data"]["save_state"], "ephemeral");
    let check = &failed["data"]["checks"][0];
    assert_eq!(check["kind"], "ai_detection");
    assert_eq!(check["status"], "failed");
    assert_eq!(check["error"]["code"], "submission_outcome_unknown");
    // No fabricated upstream identity on a dropped-connection failure.
    assert!(check["upstream"].is_null() || check.get("upstream").is_none());
    assert!(check.get("result").is_none(), "no fabricated result");
    // Local reconciliation identity is present; the ambiguous billable POST
    // was issued exactly once and never replayed.
    assert_eq!(fixture.post_count(), 2, "two issued POSTs, no replay");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// Greptile P1 (Option A) single-document form: repeated `--file` under an
/// explicit `--format json` wraps the preserved order plus the failed member
/// in one ordered partial array envelope, never discarding completed work.
#[tokio::test(flavor = "multi_thread")]
async fn repeated_files_json_wraps_partial_series_with_failed_member() {
    let fixture = ProtocolFixture::start().await;
    let first_text = "Ordered completing file one for the partial array";
    let second_text = "Ordered failing file two for the partial array";
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": "task-aaa"})));
    fixture.on_poll(Step::Json(pangram4_success(first_text)));
    fixture.on_submit(Step::Hang);

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
            "--format",
            "json",
            "--file",
            first.to_str().unwrap(),
            "--file",
            second.to_str().unwrap(),
        ])
        .output()
        .expect("run detect --format json, one completing one failing file");

    assert_eq!(output.status.code(), Some(3));
    let envelope = stdout_envelope(&output);
    assert!(
        envelope.get("error").is_none(),
        "partial is a success envelope"
    );
    let data = &envelope["data"];
    assert!(data.is_array(), "json data is the ordered analysis array");
    let analyses = data.as_array().unwrap();
    assert_eq!(analyses.len(), 2);
    assert_eq!(analyses[0]["input"]["name"], "first.txt");
    assert_eq!(analyses[0]["status"], "succeeded");
    assert_eq!(analyses[1]["input"]["name"], "second.txt");
    assert_eq!(analyses[1]["status"], "failed");
    assert_eq!(analyses[1]["submission_outcome"], "acceptance_unknown");
    assert_eq!(analyses[1]["checks"][0]["status"], "failed");
    assert_eq!(fixture.post_count(), 2, "no billable replay");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// Greptile P1 (Finding 2): when the observation GET never completes, an
/// explicit short `--timeout` stops the body read at its own deadline instead
/// of waiting for the much longer per-request transport timeout, surfacing
/// the canonical wait-timeout outcome and exiting per its category. The body
/// read is selected against the observation stop, so a stalled peer cannot
/// pin the flow.
#[tokio::test(flavor = "multi_thread")]
async fn timeout_during_stalled_poll_body_exits_promptly() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    // The poll GET never produces a complete response: only the caller's
    // wait deadline (or local cancellation) can end the read promptly.
    fixture.on_poll(Step::Hang);

    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args([
            "detect",
            "--timeout",
            "1",
            "text observed under a stalled poll",
        ])
        .output()
        .expect("run pangram detect with a stalled poll and short timeout");

    assert_eq!(
        output.status.code(),
        Some(6),
        "wait timeout is network/upstream"
    );
    let envelope = stdout_envelope(&output);
    // A local observation failure (wait timeout) surfaces as the canonical
    // top-level wait-timeout error envelope, selected against the stop so a
    // stalled peer cannot pin the read past the caller's deadline.
    assert_eq!(envelope["error"]["code"], "wait_timeout");
    assert_eq!(fixture.post_count(), 1, "submission completed once");
    assert_no_leak(&output);
    fixture.shutdown().await;
}
