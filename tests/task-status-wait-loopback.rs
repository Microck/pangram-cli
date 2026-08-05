//! Compiled-binary task status/wait integration against the real loopback
//! Pangram 4 fixture. Split from the bulk status/results suite so each test
//! target remains below the repository source-size threshold.

#![cfg(feature = "dev-tools")]

#[path = "support/bulk_cli_env.rs"]
mod env;

use std::process::Stdio;

use serde_json::{Value, json};

use env::fixture::{self, ProtocolFixture, Step, TASK_ID, pangram4_success};
use env::{Isolated, assert_no_leak, interrupt, stdout_envelope};

// `task status` observes one task by upstream ID, exits 0 on success, and
// sends exactly one GET with the task-route grammar.
#[tokio::test(flavor = "multi_thread")]
async fn task_status_observes_a_succeeded_task() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Json(pangram4_success("observed synthetic text")));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["task", "status", TASK_ID])
        .output()
        .expect("run task status");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "task_status");
    assert_eq!(envelope["data"]["status"], "succeeded");
    assert_eq!(envelope["data"]["submission_outcome"], "accepted");
    let requests = fixture.requests();
    let polls: Vec<_> = requests
        .iter()
        .filter(|request| request.method == "GET")
        .collect();
    assert_eq!(polls.len(), 1);
    assert!(polls[0].path.starts_with("/task/"));
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A terminal failed task (STAGE_FAILED) carries upstream_analysis_failed
// (category upstream) and exits 6, matching detection.
#[tokio::test(flavor = "multi_thread")]
async fn task_status_failed_exits_6() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Json(fixture::pangram4_failure(
        "the text was too short",
    )));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["task", "status", TASK_ID])
        .output()
        .expect("run task status that failed upstream");
    assert_eq!(
        output.status.code(),
        Some(6),
        "an upstream terminal task failure exits 6"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["status"], "failed");
    assert_eq!(
        envelope["data"]["checks"][0]["error"]["code"],
        "upstream_analysis_failed"
    );
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// `task wait` observes through to a terminal success and exits 0.
#[tokio::test(flavor = "multi_thread")]
async fn task_wait_reaches_succeeded() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Json(json!({"stage": "STAGE_INFERENCE"})));
    fixture.on_poll(Step::Json(pangram4_success("waited synthetic text")));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["task", "wait", TASK_ID])
        .output()
        .expect("run task wait");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "task_wait");
    assert_eq!(envelope["data"]["status"], "succeeded");
    assert!(fixture.get_count() >= 2);
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A bounded `task wait --timeout` on a task that never reaches a terminal
// stage hits the local wait timeout and exits 6 (`wait_timeout`) with the
// upstream task identity, polling at least once and never sending a POST.
#[tokio::test(flavor = "multi_thread")]
async fn task_wait_timeout_exits_6_with_wait_timeout() {
    let fixture = ProtocolFixture::start().await;
    // The first poll observes one stage-bearing snapshot; the second holds
    // until the local deadline so the finite queue can never run dry.
    fixture.on_poll(Step::Json(json!({"stage": "STAGE_INFERENCE"})));
    fixture.on_poll(Step::Hang);
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["task", "wait", TASK_ID, "--timeout", "1s"])
        .output()
        .expect("run task wait --timeout");
    assert_eq!(
        output.status.code(),
        Some(6),
        "the local wait timeout is a network-category exit"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "wait_timeout");
    assert!(envelope["error"]["details"]["upstream_task_id"].is_string());
    assert_eq!(
        envelope["error"]["details"]["last_stage"], "STAGE_INFERENCE",
        "a timed-out observed wait retains the last upstream stage"
    );
    assert_eq!(fixture.post_count(), 0, "a wait never replays a send");
    assert!(fixture.get_count() >= 1, "at least one poll fired");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// SIGINT during a task wait exits 130 with the identity note.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn task_wait_sigint_exits_130() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Json(json!({"stage": "STAGE_INFERENCE"})));
    fixture.on_poll(Step::Hang);
    let isolated = Isolated::new();
    let mut child = isolated
        .command(fixture.base_url())
        .args(["task", "wait", TASK_ID])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn task wait");
    fixture.wait_for_gets(1).await;
    interrupt(&mut child);
    let output = child.wait_with_output().expect("await task wait");
    assert_eq!(output.status.code(), Some(130));
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// An empty task ID is a usage error before any request.
#[tokio::test(flavor = "multi_thread")]
async fn task_status_empty_id_is_a_usage_error() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["task", "status", ""])
        .output()
        .expect("run task status with an empty id");
    assert_eq!(output.status.code(), Some(2));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "input_required");
    assert_eq!(fixture.get_count(), 0);
    fixture.shutdown().await;
}

// `--progress jsonl` on a task wait emits content-free JSONL events on
// stderr while the canonical envelope stays on stdout.
#[tokio::test(flavor = "multi_thread")]
async fn task_wait_jsonl_progress_is_content_free_and_on_stderr() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Json(json!({"stage": "STAGE_INFERENCE"})));
    fixture.on_poll(Step::Json(pangram4_success("progress synthetic text")));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["task", "wait", TASK_ID, "--progress", "jsonl"])
        .output()
        .expect("run task wait --progress jsonl");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "task_wait");
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    for line in stderr.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("stderr progress is JSONL: {error}\nline: {line}"));
        assert_eq!(event["type"], "progress");
        assert!(
            event.get("text").is_none(),
            "no content in progress: {event}"
        );
    }
    assert_no_leak(&output);
    fixture.shutdown().await;
}
