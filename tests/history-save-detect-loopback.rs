//! Phase 4 Packet C: explicit `--save` and automatic history-save
//! persistence semantics for completed detection work, against the real
//! loopback Pangram 4 fixture and a real SQLite store in a temporary data
//! directory. No mocks, no live Pangram, no real credentials.
//!
//! The suite locks the contract-authorized persistence grammar
//! (contracts.md 14.1/14.2 note, docs/history-contract.md):
//! - `--save` persists one completed analysis even with history disabled
//! - `history.enabled = true` auto-saves every completed detection
//! - disabled history without `--save` never creates the history directory
//!
//! The failure and reconciliation semantics (manual tails, one-warning
//! automatic failures, render precedence, task/bulk reconciliation, and the
//! ADR 0004 first-enable warning) live in the sibling suite
//! `history-save-failures-loopback.rs`.

#![cfg(feature = "dev-tools")]

#[path = "support/history_save_env.rs"]
mod harness;

use harness::fixture::{ProtocolFixture, Step, TASK_ID, pangram4_success};
use harness::{
    Isolated, analyses_rows, assert_no_leak, completed_fixture, search_payload, stderr_text,
    stdout_envelope, task_rows,
};
#[cfg(unix)]
use harness::{database_mode, parent_dir_mode};

use serde_json::Value;

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

/// Explicit `--save` on a completed detection persists the analysis, reports
/// `saved_manual` in the canonical JSON, and leaves the process at exit 0
/// with no WAL/SHM sidecar and owner-only artifacts.
#[tokio::test(flavor = "multi_thread")]
async fn save_persists_completed_detection_and_reports_saved_manual() {
    let text = "The quick brown fox saves its analysis to local history";
    let fixture = completed_fixture(text).await;
    let isolated = Isolated::new();

    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", text])
        .output()
        .expect("run pangram detect --save");

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "detect");
    assert_eq!(envelope["data"]["status"], "succeeded");
    assert_eq!(envelope["data"]["save_state"], "saved_manual");
    assert_no_leak(&output);

    // Exactly one analysis row: the terminal snapshot with its search
    // payload, saved under the manual state.
    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 1, "one analysis row persisted");
    let (
        id,
        status,
        outcome,
        save_state,
        input_kind,
        input_json,
        result_json,
        error_json,
        completed_at,
    ) = &rows[0];
    assert_eq!(status, "succeeded");
    assert_eq!(outcome, "terminal");
    assert_eq!(save_state, "saved_manual");
    assert_eq!(input_kind, "text");
    let mut retained_input = envelope["data"]["input"].clone();
    retained_input["text"] = Value::String(text.to_owned());
    assert_eq!(
        serde_json::from_str::<Value>(input_json).unwrap(),
        retained_input,
        "history retains plaintext after the explicit save even when primary output redacts it"
    );
    assert_eq!(
        serde_json::from_str::<Value>(result_json.as_ref().unwrap()).unwrap(),
        envelope["data"]["checks"][0]["result"],
        "the stored result is exactly the canonical terminal result"
    );
    assert!(error_json.is_none());
    assert!(completed_at.is_some());

    // Observation identity was recorded once for the ai_detection check.
    let tasks = task_rows(&connection);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].0, *id);
    assert_eq!(tasks[0].1, "ai_detection");
    assert_eq!(tasks[0].2, TASK_ID);

    // Explicit retention indexes the caller-owned plaintext even though the
    // primary output omitted it; the result headline is indexed separately.
    let rows = search_payload(&connection);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, *id);
    assert_eq!(rows[0].1.as_deref(), Some(text));
    assert_eq!(rows[0].2.as_deref(), Some("Human-written"));
    drop(connection);

    // Owner-only database and directory modes, and no WAL/SHM sidecar
    // survives the closed store.
    #[cfg(unix)]
    {
        assert_eq!(database_mode(&isolated.database_path()), 0o600);
        assert_eq!(parent_dir_mode(&isolated.database_path()), 0o700);
    }
    for sidecar in ["pangram-history.db-wal", "pangram-history.db-shm"] {
        assert!(
            !isolated.history_directory().join(sidecar).exists(),
            "no {sidecar} survives a closed store"
        );
    }
    fixture.shutdown().await;
}

/// The automatic gate persists every completed detection as `saved_history`
/// without any `--save` flag, and the canonical JSON reports it.
#[tokio::test(flavor = "multi_thread")]
async fn automatic_history_persists_completed_detection_as_saved_history() {
    let text = "Automatic history records this analysis without a flag";
    let fixture = completed_fixture(text).await;
    let isolated = Isolated::new();
    isolated.enable_history();

    let output = isolated
        .command(fixture.base_url())
        .args(["detect", text])
        .output()
        .expect("run pangram detect with history enabled");

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["save_state"], "saved_history");
    assert_no_leak(&output);

    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].3, "saved_history");
    assert_eq!(rows[0].1, "succeeded");
    assert_eq!(
        serde_json::from_str::<Value>(&rows[0].5).unwrap()["text"],
        text,
        "automatic retention keeps plaintext after the first-enable warning"
    );
    assert_eq!(search_payload(&connection)[0].1.as_deref(), Some(text));
    drop(connection);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn history_rerun_submits_once_with_fresh_identity_and_lineage() {
    let text = "Retained text reruns exactly once";
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": "task-rerun"})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    let isolated = Isolated::new();
    isolated.enable_history();

    let saved = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", text])
        .output()
        .expect("save original analysis");
    assert_eq!(saved.status.code(), Some(0));
    let original = stdout_envelope(&saved)["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let rerun = isolated
        .command(fixture.base_url())
        .args(["history", "rerun", &original, "--progress", "never"])
        .output()
        .expect("rerun retained analysis");
    assert_eq!(rerun.status.code(), Some(0));
    let envelope = stdout_envelope(&rerun);
    assert_eq!(envelope["command"], "history_rerun");
    assert_eq!(envelope["data"]["rerun_of"], original);
    assert_ne!(envelope["data"]["id"], envelope["data"]["rerun_of"]);
    assert!(envelope["data"].get("retry_of").is_none());
    assert_eq!(envelope["data"]["save_state"], "saved_history");
    assert_eq!(
        fixture.post_count(),
        2,
        "one POST per invocation, no replay"
    );
    let posts = fixture
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST")
        .collect::<Vec<_>>();
    assert_eq!(posts[1].body_json()["text"], text);
    assert_eq!(posts[1].body_json()["public_dashboard_link"], false);
    let connection = isolated.open_database();
    let (saved_state, retry_of, rerun_of): (String, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT save_state, retry_of, rerun_of
             FROM analyses
             WHERE rerun_of = ?1",
            [&original],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("saved rerun row");
    assert_eq!(saved_state, "saved_history");
    assert_eq!(retry_of, None);
    assert_eq!(rerun_of.as_deref(), Some(original.as_str()));
    assert_eq!(
        analyses_rows(&connection).len(),
        2,
        "automatic history stores the original and its fresh rerun"
    );
    drop(connection);
    fixture.shutdown().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn history_rerun_sigint_during_issued_post_is_ambiguous_and_never_replayed() {
    use std::process::Stdio;

    let text = "Retained text for an interrupted rerun submission";
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    fixture.on_submit(Step::Hang);
    let isolated = Isolated::new();
    let saved = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", text])
        .output()
        .expect("save original analysis");
    let original = stdout_envelope(&saved)["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let mut child = isolated
        .command(fixture.base_url())
        .args(["history", "rerun", &original, "--progress", "never"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn history rerun");
    fixture.wait_for_posts(2).await;
    interrupt(&mut child);
    let output = child
        .wait_with_output()
        .expect("wait for interrupted rerun");

    assert_eq!(output.status.code(), Some(130));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "history_rerun");
    assert_eq!(envelope["error"]["code"], "submission_outcome_unknown");
    assert!(envelope["error"]["details"]["analysis_id"].is_string());
    assert!(envelope["error"]["details"]["request_sha256"].is_string());
    assert_eq!(
        fixture.post_count(),
        2,
        "one original POST plus exactly one rerun POST"
    );
    assert_no_leak(&output);
    fixture.shutdown().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn history_rerun_sigint_during_wait_reports_reconciliation_identity() {
    use std::process::Stdio;

    let text = "Retained text for an interrupted rerun observation";
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    fixture.on_submit(Step::Json(
        serde_json::json!({"task_id": "task-rerun-stop"}),
    ));
    fixture.on_poll(Step::Json(serde_json::json!({"stage": "STAGE_INFERENCE"})));
    fixture.on_poll(Step::Hang);
    let isolated = Isolated::new();
    let saved = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", text])
        .output()
        .expect("save original analysis");
    let original = stdout_envelope(&saved)["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let mut child = isolated
        .command(fixture.base_url())
        .args(["history", "rerun", &original, "--progress", "never"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn history rerun");
    fixture.wait_for_posts(2).await;
    fixture.wait_for_gets(2).await;
    interrupt(&mut child);
    let output = child
        .wait_with_output()
        .expect("wait for interrupted rerun");

    assert_eq!(output.status.code(), Some(130));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "history_rerun");
    assert_eq!(envelope["error"]["code"], "network_unavailable");
    let stderr = stderr_text(&output);
    assert!(stderr.contains("task-rerun-stop") || stderr.contains("local analysis id"));
    assert_eq!(fixture.post_count(), 2, "rerun POST is never replayed");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// The `--include-input` precedent extends to the stored record: an explicit
/// manual save of an include-input run persists the submitted text in the
/// canonical input record and its search payload.
#[tokio::test(flavor = "multi_thread")]
async fn save_with_include_input_persists_the_submitted_text() {
    let text = "This exact sentence must round trip into the saved record";
    let fixture = completed_fixture(text).await;
    let isolated = Isolated::new();

    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", "--include-input", text])
        .output()
        .expect("run pangram detect --save --include-input");

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["input"]["text"], text);

    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(&rows[0].5).unwrap()["text"],
        text,
        "the stored input record carries the submitted text"
    );
    let payload = search_payload(&connection);
    assert_eq!(payload[0].1.as_deref(), Some(text));
    drop(connection);
    fixture.shutdown().await;
}

/// Persistence covers the honest failed state: an upstream terminal
/// STAGE_FAILED is saved with its canonical check error, and the exit stays
/// the upstream category (6). The failure is not hidden from history.
#[tokio::test(flavor = "multi_thread")]
async fn save_persists_upstream_terminal_failure_without_changing_the_exit() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(serde_json::json!({
        "stage": "STAGE_FAILED",
        "error_message": "the submitted text was too short to analyze reliably"
    })));
    let isolated = Isolated::new();

    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", "tiny"])
        .output()
        .expect("run pangram detect --save");

    assert_eq!(
        output.status.code(),
        Some(6),
        "the upstream category exit is unchanged"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["status"], "failed");
    assert_eq!(envelope["data"]["save_state"], "saved_manual");
    assert_no_leak(&output);

    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 1, "the honest failed snapshot is persisted");
    assert_eq!(rows[0].1, "failed");
    assert_eq!(rows[0].3, "saved_manual");
    let error_json: Value = serde_json::from_str(rows[0].7.as_ref().unwrap()).unwrap();
    assert_eq!(error_json["code"], "upstream_analysis_failed");
    assert!(rows[0].6.is_none(), "no result body on a failed record");
    drop(connection);
    fixture.shutdown().await;
}

/// A repeated-file run persists each newly analyzed file exactly once, in
/// invocation order, with one observation row per file.
#[tokio::test(flavor = "multi_thread")]
async fn save_repeated_files_persist_one_row_each_in_order() {
    let first_text = "First saved file content with several words to analyze";
    let second_text = "Second saved file content also analyzed in turn here";
    let fixture = ProtocolFixture::start().await;
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
            "--save",
            "--file",
            first.to_str().unwrap(),
            "--file",
            second.to_str().unwrap(),
        ])
        .output()
        .expect("run pangram detect --save with two files");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "JSONL streams one envelope per saved file");
    let first_env: Value = serde_json::from_str(lines[0]).unwrap();
    let second_env: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(first_env["data"]["save_state"], "saved_manual");
    assert_eq!(second_env["data"]["save_state"], "saved_manual");
    assert_no_leak(&output);

    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 2, "each file persisted exactly once");
    assert_eq!(
        serde_json::from_str::<Value>(&rows[0].5).unwrap()["name"],
        "first.txt"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&rows[1].5).unwrap()["name"],
        "second.txt"
    );
    assert_eq!(rows[0].4, "text");
    let tasks = task_rows(&connection);
    assert_eq!(tasks.len(), 2, "one observation row per saved file");
    drop(connection);
    fixture.shutdown().await;
}

/// Explicit save is completed-envelope-only, so Clap rejects the detached
/// combination before credentials, network, or history work.
#[tokio::test(flavor = "multi_thread")]
async fn save_and_detach_are_a_preflight_usage_conflict() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", "--detach", "detach and save this task"])
        .output()
        .expect("run pangram detect --save --detach");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "usage errors print no stdout");
    let stderr = stderr_text(&output);
    assert!(stderr.contains("--save"), "{stderr}");
    assert!(stderr.contains("--detach"), "{stderr}");
    assert!(stderr.contains("Usage:"), "{stderr}");
    assert_no_leak(&output);
    assert!(
        fixture.requests().is_empty(),
        "the usage conflict is rejected before any network request"
    );
    assert!(
        !isolated.history_directory().exists(),
        "the usage conflict is rejected before history opens"
    );
    fixture.shutdown().await;
}

/// Automatic history does not persist an unfinished detached snapshot. The
/// accepted task remains ephemeral without a history warning; a later
/// terminal task read persists the completed evidence in real SQLite.
#[tokio::test(flavor = "multi_thread")]
async fn automatic_detach_is_ephemeral_until_terminal_task_read_saves_it() {
    let text = "Terminal task evidence is saved only after a later read";
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(text)));

    let isolated = Isolated::new();
    isolated.enable_history();

    let detached = isolated
        .command(fixture.base_url())
        .args(["detect", "--detach", text])
        .output()
        .expect("run automatic-history detached detect");
    assert_eq!(detached.status.code(), Some(0));
    let detached_envelope = stdout_envelope(&detached);
    assert_eq!(detached_envelope["data"]["status"], "queued");
    assert_eq!(detached_envelope["data"]["submission_outcome"], "accepted");
    assert_eq!(detached_envelope["data"]["save_state"], "ephemeral");
    let detached_stderr = stderr_text(&detached);
    assert!(detached_stderr.contains("detached"), "{detached_stderr}");
    assert!(
        !detached_stderr.contains("warning:"),
        "skipping unfinished persistence is not a history failure: {detached_stderr}"
    );
    assert_eq!(
        fixture.requests().len(),
        1,
        "detach submits but does not poll"
    );
    assert!(
        !isolated.history_directory().exists(),
        "unfinished automatic detach does not open SQLite"
    );

    let terminal = isolated
        .command(fixture.base_url())
        .args(["task", "status", TASK_ID])
        .output()
        .expect("read terminal task status");
    assert_eq!(terminal.status.code(), Some(0));
    let terminal_envelope = stdout_envelope(&terminal);
    assert_eq!(terminal_envelope["data"]["status"], "succeeded");
    assert_eq!(terminal_envelope["data"]["save_state"], "saved_history");
    assert_eq!(
        fixture.requests().len(),
        2,
        "later task status performs the one terminal poll"
    );

    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 1, "only terminal evidence is persisted");
    assert_eq!(rows[0].1, "succeeded");
    assert_eq!(rows[0].2, "accepted");
    assert_eq!(rows[0].3, "saved_history");
    assert!(rows[0].6.is_some(), "terminal result is durable");
    assert!(rows[0].8.is_some(), "terminal completion time is durable");
    let tasks = task_rows(&connection);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].2, TASK_ID);
    drop(connection);
    fixture.shutdown().await;
}

/// A resumed `task status` poll that is still running is just as ephemeral as
/// the queued detach snapshot. It must not open SQLite or emit an automatic
/// history warning. A subsequent `task wait` persists only its terminal
/// observation.
#[tokio::test(flavor = "multi_thread")]
async fn running_task_status_is_ephemeral_and_terminal_wait_persists() {
    let text = "A terminal wait follows one ephemeral running status";
    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Json(serde_json::json!({"stage": "STAGE_INFERENCE"})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    let isolated = Isolated::new();
    isolated.enable_history();

    let running = isolated
        .command(fixture.base_url())
        .args(["task", "status", TASK_ID])
        .output()
        .expect("read running task status");
    assert_eq!(running.status.code(), Some(0));
    let running_envelope = stdout_envelope(&running);
    assert_eq!(running_envelope["data"]["status"], "running");
    assert_eq!(running_envelope["data"]["save_state"], "ephemeral");
    assert!(
        !stderr_text(&running).contains("warning:"),
        "skipping nonterminal persistence is not a history failure"
    );
    assert!(
        !isolated.history_directory().exists(),
        "running task status must not open history"
    );

    let terminal = isolated
        .command(fixture.base_url())
        .args(["task", "wait", TASK_ID])
        .output()
        .expect("wait for terminal task");
    assert_eq!(terminal.status.code(), Some(0));
    let terminal_envelope = stdout_envelope(&terminal);
    assert_eq!(terminal_envelope["data"]["status"], "succeeded");
    assert_eq!(terminal_envelope["data"]["save_state"], "saved_history");
    assert!(
        !stderr_text(&terminal).contains("warning:"),
        "successful terminal persistence does not warn"
    );

    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 1, "only the terminal wait is durable");
    assert_eq!(rows[0].1, "succeeded");
    drop(connection);
    fixture.shutdown().await;
}

/// With history disabled and no `--save`, the run leaves no history
/// directory, database, or sidecar behind (the privacy default).
#[tokio::test(flavor = "multi_thread")]
async fn without_save_history_disabled_creates_nothing() {
    let text = "Ephemeral detection leaves no local trace in history";
    let fixture = completed_fixture(text).await;
    let isolated = Isolated::new();

    let output = isolated
        .command(fixture.base_url())
        .args(["detect", text])
        .output()
        .expect("run pangram detect");

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["save_state"], "ephemeral");
    assert!(
        !isolated.history_directory().exists(),
        "no history directory is created when history is disabled"
    );
    fixture.shutdown().await;
}

/// Even with `history.enabled = true`, a request rejected before any
/// billable work (a deterministic pre-billing failure) persists nothing:
/// there is no completed analysis to record.
#[tokio::test(flavor = "multi_thread")]
async fn automatic_history_persists_nothing_on_a_pre_billing_rejection() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Status(401, None, None));
    let isolated = Isolated::new();
    isolated.enable_history();

    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "some text that would be analyzed"])
        .output()
        .expect("run pangram detect with a rejected key");

    assert_eq!(output.status.code(), Some(4), "authentication exit");
    assert!(
        !isolated.history_directory().exists(),
        "a top-level pre-billing failure creates no history row"
    );
    fixture.shutdown().await;
}

/// There is no `history save` command: the closed section 14.5 grammar
/// rejects the spelling as an unknown argument before any billable work,
/// exactly like any other unknown flag (exit 2, Clap usage error).
#[test]
fn history_save_is_not_a_command_and_is_rejected() {
    for arguments in [
        &["history", "save"][..],
        &[
            "bulk",
            "submit",
            "--save",
            "--max-billable-units",
            "5",
            "items.jsonl",
        ][..],
        &["task", "status", "--save", TASK_ID][..],
    ] {
        let output = harness::pangram()
            .args(arguments)
            .output()
            .expect("run pangram");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arguments:?} is rejected: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty(), "a usage error prints no stdout");
    }
}
