//! Compiled CLI acceptance for Phase 7 against the real loopback fixture.
//! No live Pangram endpoint or credential is used.

#![cfg(feature = "dev-tools")]

#[path = "support/protocol_loopback/mod.rs"]
mod fixture;

use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

use fixture::{
    ProtocolFixture, SYNTHETIC_KEY, Step, TASK_ID, file_success, pangram4_success,
    plagiarism_success,
};

#[cfg(unix)]
fn interrupt(child: &mut std::process::Child) {
    let pid = i32::try_from(child.id()).expect("child PID fits i32");
    // SAFETY: `pid` is a live child process and SIGINT is a valid signal.
    assert_eq!(unsafe { kill(pid, 2) }, 0);
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

struct Isolated {
    _root: TempDir,
    env: Vec<(String, String)>,
}

impl Isolated {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let data = root.path().join("data");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let env = [
            ("HOME", home.to_str().unwrap()),
            ("XDG_CONFIG_HOME", home.to_str().unwrap()),
            ("XDG_DATA_HOME", home.to_str().unwrap()),
            ("PANGRAM_DATA_DIR", data.to_str().unwrap()),
            ("CI", "true"),
            ("TERM", "dumb"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        Self { _root: root, env }
    }

    fn command(&self, endpoint: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_pangram-test-driver"));
        command.env("PANGRAM_API_KEY", SYNTHETIC_KEY).arg(endpoint);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
}

fn envelope(output: &std::process::Output) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    serde_json::from_str(stdout.trim_end())
        .unwrap_or_else(|error| panic!("one JSON envelope expected: {error}\n{stdout}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn plagiarism_uses_the_sync_route_and_renders_its_check() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_plagiarism(Step::Json(plagiarism_success()));
    let text = "Synthetic plagiarism input with enough words for the check";

    let output = Isolated::new()
        .command(fixture.base_url())
        .args(["plagiarism", text, "--max-billable-units", "5"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, b"");
    let value = envelope(&output);
    assert_eq!(value["command"], "plagiarism");
    assert_eq!(value["data"]["status"], "succeeded");
    assert_eq!(value["data"]["checks"][0]["kind"], "plagiarism");
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(fixture.requests()[0].path, "/plagiarism");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn combined_analysis_keeps_ai_success_and_exits_partial_on_plagiarism_failure() {
    let fixture = ProtocolFixture::start().await;
    let text = "Synthetic combined input with enough words for both checks";
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    fixture.on_plagiarism(Step::Status(402, None, None));

    let output = Isolated::new()
        .command(fixture.base_url())
        .args(["analyze", text, "--max-billable-units", "6"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(3));
    let value = envelope(&output);
    assert_eq!(value["command"], "analyze");
    assert_eq!(value["data"]["status"], "partial");
    assert_eq!(value["data"]["checks"][0]["kind"], "ai_detection");
    assert_eq!(value["data"]["checks"][0]["status"], "succeeded");
    assert_eq!(value["data"]["checks"][1]["kind"], "plagiarism");
    assert_eq!(value["data"]["checks"][1]["status"], "failed");
    assert_eq!(
        value["data"]["checks"][1]["error"]["code"],
        "payment_required"
    );
    fixture.shutdown().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn combined_analysis_interruption_keeps_ambiguity_and_exits_130() {
    use std::process::Stdio;

    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Hang);
    fixture.on_plagiarism(Step::Hang);
    let text = "Synthetic combined input interrupted after both submissions";
    let isolated = Isolated::new();

    let mut child = isolated
        .command(fixture.base_url())
        .args(["analyze", text, "--max-billable-units", "6"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn combined analysis");
    fixture.wait_for_posts(2).await;
    interrupt(&mut child);
    let output = child
        .wait_with_output()
        .expect("wait for interrupted analysis");

    assert_eq!(output.status.code(), Some(130));
    let value = envelope(&output);
    assert_eq!(value["command"], "analyze");
    assert_eq!(value["error"]["code"], "submission_outcome_unknown");
    assert!(value["error"]["details"]["request_sha256"].is_string());
    assert_eq!(fixture.post_count(), 2, "neither billable POST is replayed");
    fixture.shutdown().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn combined_history_rerun_interruption_keeps_ambiguity_and_exits_130() {
    use std::process::Stdio;

    let fixture = ProtocolFixture::start().await;
    let text = "Synthetic retained combined input for an interrupted rerun";
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    fixture.on_plagiarism(Step::Json(plagiarism_success()));
    fixture.on_submit(Step::Hang);
    fixture.on_plagiarism(Step::Hang);
    let isolated = Isolated::new();

    let saved = isolated
        .command(fixture.base_url())
        .args(["analyze", "--save", text, "--max-billable-units", "6"])
        .output()
        .expect("save original combined analysis");
    assert_eq!(saved.status.code(), Some(0));
    let original = envelope(&saved)["data"]["id"]
        .as_str()
        .expect("saved analysis ID")
        .to_owned();

    let mut child = isolated
        .command(fixture.base_url())
        .args(["history", "rerun", &original, "--progress", "never"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn combined history rerun");
    fixture.wait_for_posts(4).await;
    interrupt(&mut child);
    let output = child
        .wait_with_output()
        .expect("wait for interrupted rerun");

    assert_eq!(output.status.code(), Some(130));
    let value = envelope(&output);
    assert_eq!(value["command"], "history_rerun");
    assert_eq!(value["error"]["code"], "submission_outcome_unknown");
    assert!(value["error"]["details"]["request_sha256"].is_string());
    assert_eq!(fixture.post_count(), 4, "neither billable POST is replayed");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn binary_detection_uses_file_route_and_respects_include_input() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_file(Step::Json(serde_json::json!([file_success(
        "sample.RTF",
        "synthetic extracted file text"
    )])));
    let files = tempfile::tempdir().unwrap();
    let path = files.path().join("sample.RTF");
    std::fs::write(&path, b"{\\rtf1 synthetic}").unwrap();

    let output = Isolated::new()
        .command(fixture.base_url())
        .args([
            "detect",
            "--file",
            path.to_str().unwrap(),
            "--include-input",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let value = envelope(&output);
    assert_eq!(value["command"], "detect");
    assert_eq!(value["data"]["input"]["type"], "file");
    assert_eq!(value["data"]["input"]["filename"], "sample.RTF");
    assert_eq!(value["data"]["input"]["media_type"], "application/rtf");
    assert_eq!(
        value["data"]["input"]["extracted_text"],
        "synthetic extracted file text"
    );
    assert_eq!(value["data"]["input"]["path"], path.to_str().unwrap());
    assert_eq!(fixture.requests()[0].path, "/file");
    assert_eq!(fixture.get_count(), 0);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_binary_detection_retains_private_file_fields_outside_primary_output() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_file(Step::Json(serde_json::json!([file_success(
        "saved.pdf",
        "retained extracted words"
    )])));
    let files = tempfile::tempdir().unwrap();
    let path = files.path().join("saved.pdf");
    std::fs::write(&path, b"%PDF-1.4\nsynthetic\n%%EOF").unwrap();
    let isolated = Isolated::new();

    let detected = isolated
        .command(fixture.base_url())
        .args(["detect", "--file", path.to_str().unwrap(), "--save"])
        .output()
        .unwrap();

    assert_eq!(detected.status.code(), Some(0));
    let detected = envelope(&detected);
    assert!(detected["data"]["input"].get("path").is_none());
    assert!(detected["data"]["input"].get("extracted_text").is_none());
    let analysis_id = detected["data"]["id"].as_str().unwrap();

    let shown = isolated
        .command(fixture.base_url())
        .args(["history", "show", analysis_id, "--include-input"])
        .output()
        .unwrap();

    assert_eq!(shown.status.code(), Some(0));
    let shown = envelope(&shown);
    assert_eq!(shown["data"]["input"]["path"], path.to_str().unwrap());
    assert_eq!(
        shown["data"]["input"]["extracted_text"],
        "retained extracted words"
    );
    assert_eq!(fixture.post_count(), 1);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unsupported_binary_combinations_stop_before_credentials_or_network() {
    let fixture = ProtocolFixture::start().await;
    let files = tempfile::tempdir().unwrap();
    let path = files.path().join("sample.pdf");
    std::fs::write(&path, b"%PDF-1.4\n%%EOF").unwrap();
    let path = path.to_str().unwrap();
    let cases: &[&[&str]] = &[
        &["plagiarism", "--file", path],
        &["analyze", "--file", path],
        &["detect", "--file", path, "--max-billable-units", "100"],
        &["detect", "--file", path, "--public-link"],
        &["detect", "--file", path, "--detach"],
    ];

    for arguments in cases {
        let output = Isolated::new()
            .command(fixture.base_url())
            .args(*arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "arguments: {arguments:?}");
        let value = envelope(&output);
        assert_eq!(value["error"]["code"], "unsupported_input");
    }
    assert_eq!(fixture.post_count(), 0);
    assert_eq!(fixture.get_count(), 0);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn text_billing_ceiling_includes_the_fixed_plagiarism_charge() {
    let fixture = ProtocolFixture::start().await;
    for arguments in [
        ["plagiarism", "one word", "--max-billable-units", "4"],
        ["analyze", "one word", "--max-billable-units", "5"],
    ] {
        let output = Isolated::new()
            .command(fixture.base_url())
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(envelope(&output)["error"]["code"], "unsupported_input");
    }
    assert_eq!(fixture.post_count(), 0);
    fixture.shutdown().await;
}
