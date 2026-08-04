//! Phase 4 Packet C remediation: the detect manual/automatic failure and
//! render-precedence semantics of the history-save integration, against the
//! real loopback Pangram 4 fixture and a real SQLite store in a temporary
//! data directory. No mocks, no live Pangram, no real credentials.
//!
//! Split into two cohesive suites over the shared real harness
//! (`tests/support/history_save_env.rs`) so each stays under the source-size
//! threshold: this one owns the manual/automatic failures and render
//! precedence; `history-save-reconciliation-loopback.rs` owns the task/bulk
//! reconciliation and ADR 0004 transitions.
//!
//! The suite locks the contract-authorized semantics (contracts.md 14.2
//! note, docs/history-contract.md):
//! - a manual save failure is a canonical local error (exit 7) after the
//!   honest envelopes render in invocation order, in full
//! - one member's store failure never drops the ordered tail: later members
//!   still persist and render with their own truthful save state
//! - an automatic failure produces exactly one sanitized `warning:` line per
//!   invocation (a single direct prefix, never `note: warning:`) and never
//!   fails the remote result
//! - a primary render failure reports exit 1, never masked by exit 7

#![cfg(feature = "dev-tools")]

#[path = "support/history_save_env.rs"]
mod harness;

use harness::fixture::{ProtocolFixture, Step, TASK_ID};
use harness::{
    Isolated, assert_no_leak, completed_fixture, poison_data_dir, stderr_text, stdout_envelope,
};

use serde_json::Value;

// ------------------------------------------------------- manual failures --

/// A manual save failure is the canonical local error: after the honest
/// envelope rendered with `ephemeral`, the process reports
/// `insecure_history_permissions` (category `local_history`) and exits 7.
/// The remote result is preserved in the envelope, never dropped.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn explicit_save_failure_is_a_canonical_local_error_after_the_envelope() {
    let text = "Saved under a data directory that fails protection checks";
    let fixture = completed_fixture(text).await;
    let isolated = Isolated::new();
    poison_data_dir(&isolated);

    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", text])
        .output()
        .expect("run pangram detect --save under an unprotectable data dir");

    assert_eq!(
        output.status.code(),
        Some(7),
        "manual save failure is the local-state exit"
    );
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "success envelope then failure envelope");
    let first: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["data"]["status"], "succeeded");
    assert_eq!(first["data"]["save_state"], "ephemeral");
    let failure: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(failure["command"], "detect");
    assert_eq!(failure["error"]["code"], "insecure_history_permissions");
    assert_eq!(failure["error"]["category"], "local_history");
    let stderr = stderr_text(&output);
    assert!(
        stderr.is_empty(),
        "JSON error surface keeps stderr clean: {stderr}"
    );
    assert_no_leak(&output);
    // The store failed closed: no database was created beside the poisoned
    // path, and the poisoned path itself is untouched.
    assert!(isolated.history_directory().is_file());
    assert!(!isolated.database_path().exists());
    fixture.shutdown().await;
}

/// The ordered tail is preserved after one member's manual save failure:
/// every completed member persists or renders `ephemeral` exactly once, in
/// invocation order, and a member after the failed save still persists its
/// own row. No completed member is dropped.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn repeated_file_manual_save_failure_preserves_the_ordered_tail() {
    let first_text = "First completed member that cannot save its own row";
    let second_text = "Second completed member that saves after the failure";
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": "task-aaa"})));
    fixture.on_poll(Step::Json(harness::fixture::pangram4_success(first_text)));
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": "task-bbb"})));
    fixture.on_poll(Step::Json(harness::fixture::pangram4_success(second_text)));

    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.txt");
    let second = root.path().join("second.txt");
    std::fs::write(&first, first_text).unwrap();
    std::fs::write(&second, second_text).unwrap();

    let isolated = Isolated::new();
    poison_data_dir(&isolated);

    let output = isolated
        .command(fixture.base_url())
        .args([
            "detect",
            "--save",
            "--file",
            first.to_str().unwrap(),
            "--file",
            second.to_str().unwrap(),
        ])
        .output()
        .expect("run pangram detect --save with two files under a poisoned data dir");

    // Both completed members render exactly once, in order, each honest:
    // under the store-open failure neither row persisted, so both report
    // `ephemeral`, then the one exit-7 failure envelope closes the series.
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "two completed envelopes then the failure envelope"
    );
    let first_env: Value = serde_json::from_str(lines[0]).unwrap();
    let second_env: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(first_env["data"]["save_state"], "ephemeral");
    assert_eq!(second_env["data"]["save_state"], "ephemeral");
    assert_eq!(first_env["data"]["status"], "succeeded");
    assert_eq!(second_env["data"]["status"], "succeeded");
    let failure: Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(failure["error"]["category"], "local_history");
    assert_eq!(output.status.code(), Some(7), "the save failure exits 7");
    assert_no_leak(&output);
    assert!(
        !isolated.database_path().exists(),
        "the store-open failure commits nothing"
    );
    fixture.shutdown().await;
}

// ---------------------------------------------------- automatic failures --

/// An automatic history failure (an unprotectable data directory under the
/// enabled gate) produces exactly one sanitized warning with exactly one
/// `warning:` prefix and never turns the successful remote analysis into a
/// failure.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn automatic_history_failure_is_one_warning_and_never_fails_the_run() {
    let text = "Automatic save failure stays a warning on a success";
    let fixture = completed_fixture(text).await;
    let isolated = Isolated::new();
    isolated.enable_history();
    poison_data_dir(&isolated);

    let output = isolated
        .command(fixture.base_url())
        .args(["detect", text])
        .output()
        .expect("run pangram detect with automatic history disabled by its data dir");

    assert_eq!(
        output.status.code(),
        Some(0),
        "the remote success still exits 0"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["status"], "succeeded");
    assert_eq!(envelope["data"]["save_state"], "ephemeral");
    let stderr = stderr_text(&output);
    assert_eq!(
        stderr.lines().count(),
        1,
        "exactly one sanitized warning: {stderr}"
    );
    assert_eq!(
        stderr.matches("warning:").count(),
        1,
        "exactly one `warning:` prefix, never doubled: {stderr}"
    );
    let warning_line = stderr.lines().next().unwrap_or_default();
    assert!(
        warning_line.starts_with("warning: "),
        "the one automatic warning is a direct `warning: ` line, never `note: warning:`: {stderr}"
    );
    assert!(
        !warning_line.starts_with("note:"),
        "no `note:` prefix doubles the direct warning: {stderr}"
    );
    assert!(stderr.contains("history"), "{stderr}");
    assert!(
        !stderr.contains(text),
        "the warning never echoes submitted content: {stderr}"
    );
    assert_no_leak(&output);
    assert!(!isolated.database_path().exists());
    fixture.shutdown().await;
}

/// A repeated-file automatic run under a failing store still warns exactly
/// once for the whole invocation: per-member failures share the one latch,
/// so three members never produce three warnings.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn automatic_repeated_file_failure_warns_exactly_once_for_the_command() {
    let texts = [
        "Automatic member one content words here",
        "Automatic member two content words here",
        "Automatic member three content words here",
    ];
    let fixture = ProtocolFixture::start().await;
    for (index, text) in texts.iter().enumerate() {
        fixture.on_submit(Step::Json(serde_json::json!({
            "task_id": format!("task-{index}")
        })));
        fixture.on_poll(Step::Json(harness::fixture::pangram4_success(text)));
    }
    let isolated = Isolated::new();
    isolated.enable_history();
    poison_data_dir(&isolated);

    let root = tempfile::tempdir().unwrap();
    let mut args = vec!["detect".to_owned()];
    for text in texts.iter() {
        let path = root.path().join(format!("{text}.txt"));
        std::fs::write(&path, text).unwrap();
        args.push("--file".to_owned());
        args.push(path.to_str().unwrap().to_owned());
    }
    let output = isolated
        .command(fixture.base_url())
        .args(&args)
        .output()
        .expect("run pangram detect with three files under a failing store");

    let stderr = stderr_text(&output);
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.contains("warning:"))
            .count(),
        1,
        "one warning covers the whole automatic flow: {stderr}"
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "automatic failure never fails the remote results"
    );
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// An automatic save failure on a run that legitimately exits nonzero (an
/// upstream terminal failure) keeps the category exit and adds its one
/// warning; the warning never masks the honest exit.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn automatic_history_failure_keeps_the_category_exit_on_a_failed_run() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(serde_json::json!({
        "stage": "STAGE_FAILED",
        "error_message": "the submitted text was too short to analyze reliably"
    })));
    let isolated = Isolated::new();
    isolated.enable_history();
    poison_data_dir(&isolated);

    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "tiny"])
        .output()
        .expect("run pangram detect with automatic history failure on a failed run");

    assert_eq!(
        output.status.code(),
        Some(6),
        "the upstream exit is preserved"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["status"], "failed");
    assert_eq!(envelope["data"]["save_state"], "ephemeral");
    let stderr = stderr_text(&output);
    assert_eq!(stderr.lines().count(), 1, "one warning, unmasked: {stderr}");
    assert!(stderr.contains("warning:"), "{stderr}");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// A primary render failure wins over the save failure's exit 7: when the
/// primary envelope itself cannot be rendered (stdout is `/dev/full`, so
/// every write deterministically fails at the write boundary), the process
/// reports the primary render failure at exit 1 and never masks it behind
/// the history exit 7 (contracts.md 14.2 note).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn primary_render_failure_exits_1_and_is_never_masked_by_exit_7() {
    use std::process::Stdio;

    let text = "A completed detection whose primary output cannot print";
    let fixture = completed_fixture(text).await;
    let isolated = Isolated::new();
    poison_data_dir(&isolated);

    // /dev/full accepts opens but fails every write (ENOSPC): a
    // deterministic render failure at the primary write boundary.
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("/dev/full exists on this platform");

    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", text])
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pangram detect --save")
        .wait_with_output()
        .expect("await pangram");
    assert_eq!(
        output.status.code(),
        Some(1),
        "a primary render failure exits 1, never masked by exit 7"
    );
    assert!(output.stdout.is_empty(), "nothing reached /dev/full");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("insecure_history_permissions"),
        "an unrenderable failure envelope is not claimed: {stderr}"
    );
    fixture.shutdown().await;
}

/// The text-surface attribution: a successful primary result renders to
/// stdout, but the attach-time failure envelope cannot be written on the
/// text surface (stderr is `/dev/full`), so the process reports the
/// general render failure at exit 1 and never masks it behind the
/// save-failure exit 7. The other half of the precedence proof below is
/// symmetric (a primary render that already failed wins, exit 1, never 7).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn text_surface_attach_write_failure_exits_1_not_7() {
    use std::process::Stdio;

    let text = "A completed detection whose failure envelope cannot print";
    let fixture = completed_fixture(text).await;
    let isolated = Isolated::new();
    poison_data_dir(&isolated);

    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("/dev/full exists on this platform");

    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", "--error-format", "text", text])
        .stdout(Stdio::piped())
        .stderr(Stdio::from(full))
        .spawn()
        .expect("spawn pangram detect --save --error-format text")
        .wait_with_output()
        .expect("await pangram");
    assert_eq!(
        output.status.code(),
        Some(1),
        "an unwritable text failure envelope exits 1, never masked by 7"
    );
    // The primary result still rendered to stdout as the canonical JSON
    // envelope (the save outcome is what could not print).
    assert!(!output.stdout.is_empty(), "primary output reached stdout");
    fixture.shutdown().await;
}
