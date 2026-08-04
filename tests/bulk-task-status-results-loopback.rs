//! Compiled-binary bulk status/wait/results and task status/wait integration
//! against the real loopback Pangram 4 fixture. No mocks, no live Pangram, no
//! real credentials.
//!
//! Each test boots the Axum fixture on an ephemeral loopback port, points the
//! `dev-tools`-gated `PANGRAM_DETECT_ENDPOINT` override at it (one loopback
//! set derives both the task and bulk routes), runs the compiled `pangram`
//! binary in an isolated config/data environment, and asserts the exact
//! stdout envelope, stderr separation, exit code, and the recorded upstream
//! request grammar. The synthetic key and content are fixture constants;
//! assertion helpers never echo header or key values.
//!
//! Contract coverage (contracts.md 9.1, 12, 14.3-14.4): a status read is one
//! safe snapshot; a terminal failed collection or failed task exits per the
//! canonical upstream category (6); partial exits 3; an explicit results
//! offset/limit reads one documented page while the offset-0 no-limit default
//! fetch-alls; and the recorded POST/GET routes prove exactly one billable
//! send with no replay and no credential or content leak.

#![cfg(feature = "dev-tools")]

#[path = "support/bulk_cli_env.rs"]
mod env;

use std::process::Stdio;

use serde_json::{Value, json};

use env::fixture::{self, BulkRequestView, ProtocolFixture, Step, TASK_ID, pangram4_success};
use env::{
    BULK_ID, Isolated, accepted_202, assert_no_leak, interrupt, jsonl, results_page,
    spawn_with_stdin, status_body, stdout_envelope,
};

// `bulk submit --wait` observes through to a terminal success, exits 0, and
// preserves the accepted counters and the one-POST-plus-polls grammar.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_wait_reaches_succeeded_exit_0() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_202(2))));
    fixture.on_bulk_status(Step::Json(status_body("succeeded", 2, 2, 2, 0)));
    let isolated = Isolated::new();
    let input = jsonl(&[("row-000", "first words"), ("row-001", "second words")]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--wait",
        ],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    // The wait projection of the submitted job emits the bulk_wait envelope
    // (both data roots are the canonical BulkCollection).
    assert_eq!(envelope["command"], "bulk_wait");
    assert_eq!(envelope["data"]["status"], "succeeded");
    assert_eq!(envelope["data"]["succeeded"], 2);
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(fixture.get_count(), 1);
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A partial terminal collection (mixed succeeded/failed) exits 3.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_wait_partial_exits_3() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_status(Step::Json(status_body("partial", 2, 2, 1, 1)));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "wait", BULK_ID])
        .output()
        .expect("run bulk wait");
    // The collection is observed and partial -> exit 3.
    assert_eq!(output.status.code(), Some(3));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["status"], "partial");
    fixture.shutdown().await;
}

// A terminal failed bulk collection failed every item through an upstream
// terminal analysis failure and exits 6 (upstream category), not 1.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_wait_failed_exits_6() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_status(Step::Json(status_body("failed", 2, 2, 0, 2)));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "wait", BULK_ID])
        .output()
        .expect("run bulk wait");
    assert_eq!(
        output.status.code(),
        Some(6),
        "a terminal failed collection is an upstream failure"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["status"], "failed");
    fixture.shutdown().await;
}

// A succeeded bulk wait exits 0.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_wait_succeeded_exits_0() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_status(Step::Json(status_body("succeeded", 1, 1, 1, 0)));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "wait", BULK_ID])
        .output()
        .expect("run bulk wait");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["status"], "succeeded");
    fixture.shutdown().await;
}

// A bounded `bulk wait --timeout` on a job that never progresses reaches the
// local wait timeout and exits 6 (`wait_timeout`, network category) with the
// identity details, polling at least once and never replaying a POST.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_wait_timeout_exits_6_with_wait_timeout() {
    let fixture = ProtocolFixture::start().await;
    // A non-terminal job stays running forever; the first poll consumes a
    // running snapshot and the second scripted response holds until the local
    // deadline so a finite queue can never run dry before the wait budget.
    fixture.on_bulk_status(Step::Json(status_body("running", 1, 1, 0, 0)));
    fixture.on_bulk_status(Step::Hang);
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "wait", BULK_ID, "--timeout", "1s"])
        .output()
        .expect("run bulk wait --timeout");
    assert_eq!(
        output.status.code(),
        Some(6),
        "the local wait timeout is a network-category exit"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "wait_timeout");
    // The wait observation synthesizes one local bulk identity for the
    // resumed remote handle, so the timeout reports a fresh local bulk id.
    assert!(envelope["error"]["details"]["bulk_id"].is_string());
    assert_eq!(fixture.post_count(), 0, "a wait never replays a send");
    assert!(fixture.get_count() >= 1, "at least one poll fired");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// `bulk submit --wait` observes a submitted job through to a partial terminal
// collection and exits 3 (some items succeeded, some failed), sending exactly
// one billable POST and polling with no replay.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_wait_reaches_partial_exit_3() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_202(2))));
    fixture.on_bulk_status(Step::Json(status_body("partial", 2, 2, 1, 1)));
    let isolated = Isolated::new();
    let input = jsonl(&[("row-000", "first words"), ("row-001", "second words")]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--wait",
        ],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(3));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_wait");
    assert_eq!(envelope["data"]["status"], "partial");
    assert_eq!(envelope["data"]["succeeded"], 1);
    assert_eq!(envelope["data"]["failed"], 1);
    assert_eq!(fixture.post_count(), 1, "exactly one billable send");
    assert!(fixture.get_count() >= 1, "the wait polls at least once");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// `bulk submit --wait` observes a submitted job through to a terminal failed
// collection and exits 6 (the upstream category), sending exactly one
// billable POST and polling with no replay.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_wait_reaches_failed_exit_6() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(202, None, Some(accepted_202(1))));
    fixture.on_bulk_status(Step::Json(status_body("failed", 1, 1, 0, 1)));
    let isolated = Isolated::new();
    let input = jsonl(&[("row-000", "first words")]);
    let output = spawn_with_stdin(
        isolated.command(fixture.base_url()),
        &[
            "bulk",
            "submit",
            "-",
            "--max-billable-units",
            "10",
            "--wait",
        ],
        input.as_bytes(),
    );
    assert_eq!(output.status.code(), Some(6));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_wait");
    assert_eq!(envelope["data"]["status"], "failed");
    assert_eq!(fixture.post_count(), 1);
    assert!(fixture.get_count() >= 1);
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// `--progress jsonl` on an unbounded bulk wait emits content-free JSONL
// progress events on stderr while the canonical envelope stays on stdout.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_wait_progress_jsonl_emits_content_free_events_on_stderr() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_status(Step::Json(status_body("running", 1, 1, 0, 0)));
    fixture.on_bulk_status(Step::Json(status_body("succeeded", 1, 1, 1, 0)));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "wait", BULK_ID, "--progress", "jsonl"])
        .output()
        .expect("run bulk wait --progress jsonl");
    assert_eq!(output.status.code(), Some(0));
    // The canonical envelope stays on stdout.
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_wait");
    assert_eq!(envelope["data"]["status"], "succeeded");
    // Progress events are JSONL on stderr, one event per line, content-free
    // (IDs and counters only, never item text).
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    let mut saw_progress = false;
    for line in stderr.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("stderr progress is JSONL: {error}\nline: {line}"));
        assert_eq!(event["type"], "progress");
        assert!(
            event.get("text").is_none(),
            "no submitted content in progress: {event}"
        );
        saw_progress = true;
    }
    assert!(saw_progress, "at least one progress event is emitted");
    assert!(fixture.get_count() >= 2);
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// `bulk status` takes one snapshot (one GET) and projects the collection.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_status_is_a_single_snapshot() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_status(Step::Json(status_body("running", 3, 3, 1, 0)));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "status", BULK_ID])
        .output()
        .expect("run bulk status");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_status");
    assert_eq!(envelope["data"]["status"], "running");
    assert_eq!(envelope["data"]["submission_outcome"], "accepted");
    assert_eq!(fixture.get_count(), 1, "one snapshot GET");
    assert_eq!(fixture.post_count(), 0);
    fixture.shutdown().await;
}

// A bulk status for an unknown job maps to upstream_not_found.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_status_unknown_job_is_upstream_not_found() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_status(Step::Status(404, None, None));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "status", BULK_ID])
        .output()
        .expect("run bulk status for an unknown job");
    assert_eq!(output.status.code(), Some(6));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "upstream_not_found");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// SIGINT during a bulk wait exits 130 with the identity note on stderr.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn bulk_wait_sigint_exits_130_with_identity() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_status(Step::Json(status_body("running", 1, 1, 0, 0)));
    fixture.on_bulk_status(Step::Hang);
    let isolated = Isolated::new();
    let mut child = isolated
        .command(fixture.base_url())
        .args(["bulk", "wait", BULK_ID])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn bulk wait");
    fixture.wait_for_gets(1).await;
    interrupt(&mut child);
    let output = child.wait_with_output().expect("await bulk wait");
    assert_eq!(output.status.code(), Some(130));
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    assert!(stderr.contains("interrupted"), "{stderr}");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// `bulk results` with no --limit and offset 0 fetches all pages and projects
// one canonical ordered page.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_default_fetch_all_projects_one_page() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(results_page(0, 100, 2, 2)));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID])
        .output()
        .expect("run bulk results (fetch-all)");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_results");
    let data = &envelope["data"];
    assert!(data["items"].is_array());
    assert_eq!(data["items"].as_array().unwrap().len(), 2);
    let recorded = fixture.requests();
    let requests = BulkRequestView::for_path(&recorded, BULK_ID, "/results");
    assert!(!requests.is_empty());
    assert!(requests[0].query.contains("offset=0"));
    fixture.shutdown().await;
}

// An explicit --limit reads exactly one page at the requested window.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_explicit_limit_reads_one_page() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(results_page(0, 5, 2, 2)));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID, "--limit", "5"])
        .output()
        .expect("run bulk results --limit 5");
    assert_eq!(output.status.code(), Some(0));
    let recorded = fixture.requests();
    let requests = BulkRequestView::for_path(&recorded, BULK_ID, "/results");
    assert_eq!(requests.len(), 1, "one explicit page read");
    assert!(requests[0].query.contains("limit=5"));
    fixture.shutdown().await;
}

// An explicit --offset reads one page at that offset, not a fetch-all.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_explicit_offset_reads_one_page() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(results_page(1, 100, 1, 2)));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID, "--offset", "1"])
        .output()
        .expect("run bulk results --offset 1");
    assert_eq!(output.status.code(), Some(0));
    let recorded = fixture.requests();
    let requests = BulkRequestView::for_path(&recorded, BULK_ID, "/results");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].query.contains("offset=1"));
    fixture.shutdown().await;
}

// A mixed results page (one succeeded + one failed child) is a successful
// page retrieval: it exits 0 (a page is not authoritative for whole-job
// terminal state; contracts.md 12/14.3), preserves the failed child as a
// failed child with its sanitized error, and never fails with
// `upstream_contract_changed`. The observed resumed read marks every child
// analysis `accepted` (never `terminal`; contracts.md 4.6).
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_mixed_page_exits_0_and_preserves_failures() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 100,
        "total_items": 2,
        "items": [
            {"index": 0, "id": "row-000", "task_id": "task-000", "stage": "STAGE_SUCCESS",
             "error": null, "result": pangram4_success("synthetic loopback words")}
        ],
        "failed_items": [
            {"index": 1, "id": "row-001", "task_id": null, "stage": "STAGE_FAILED",
             "error": "Text must contain at least one valid token"}
        ]
    })));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID, "--limit", "100"])
        .output()
        .expect("run bulk results with a mixed page");
    assert_eq!(
        output.status.code(),
        Some(0),
        "a successful page read exits 0 regardless of failed children"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_results");
    assert!(
        envelope.get("error").is_none(),
        "no false upstream_contract_changed: {envelope}"
    );
    let items = envelope["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    // Items are strictly ascending by index: 0 succeeded, 1 failed.
    assert_eq!(items[0]["index"], 0);
    assert_eq!(items[0]["status"], "succeeded");
    assert_eq!(items[0]["analysis"]["submission_outcome"], "accepted");
    assert_eq!(items[1]["index"], 1);
    assert_eq!(items[1]["status"], "failed");
    assert_eq!(
        items[1]["error"]["code"], "upstream_analysis_failed",
        "the failed child keeps its sanitized upstream error"
    );
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A results page whose succeeded child's terminal document carries no
// `text` field is still a valid observed read: the command succeeds (exit 0)
// and emits the child analysis `accepted`, never a false
// `upstream_contract_changed`.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_textless_success_exits_0_without_contract_drift() {
    let fixture = ProtocolFixture::start().await;
    let mut no_text = pangram4_success("unused");
    no_text.as_object_mut().unwrap().remove("text");
    fixture.on_bulk_results(Step::Json(json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 100,
        "total_items": 1,
        "items": [
            {"index": 0, "id": "row-000", "task_id": "task-000", "stage": "STAGE_SUCCESS",
             "error": null, "result": no_text}
        ],
        "failed_items": []
    })));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID, "--limit", "100"])
        .output()
        .expect("run bulk results with a text-less success");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert!(
        envelope.get("error").is_none(),
        "a text-less success is not contract drift: {envelope}"
    );
    let items = envelope["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["index"], 0);
    assert_eq!(items[0]["status"], "succeeded");
    assert_eq!(items[0]["analysis"]["submission_outcome"], "accepted");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A remote-only results read keeps every piece of evidence already validated
// on the terminal child analysis. The history child projector consumes this
// same value, so this compiled-flow assertion prevents the returned child
// from losing its upstream-attested input descriptor, version, task ID, or
// last stage before persistence.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_preserves_terminal_child_input_and_upstream_evidence() {
    let text = "attested remote result metadata survives projection";
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 100,
        "total_items": 1,
        "items": [
            {"index": 0, "id": "row-000", "task_id": TASK_ID, "stage": "STAGE_SUCCESS",
             "error": null, "result": pangram4_success(text)}
        ],
        "failed_items": []
    })));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID, "--limit", "100"])
        .output()
        .expect("run bulk results with complete terminal evidence");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    let analysis = &envelope["data"]["items"][0]["analysis"];
    assert_eq!(analysis["input"]["origin"], "unknown");
    assert_eq!(analysis["input"]["byte_count"], text.len());
    assert_eq!(
        analysis["input"]["word_count"],
        text.split_whitespace().count()
    );
    assert_eq!(analysis["provenance"]["upstream_version"], "4.0");
    assert_eq!(analysis["provenance"]["upstream_task_ids"][0], TASK_ID);
    assert_eq!(
        analysis["checks"][0]["upstream"]["last_stage"],
        "STAGE_SUCCESS"
    );
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A fetch-all read over a multi-page job reassembles every walked page into
// one canonical aggregate window: `offset: 0`, `limit: max(1, total_items)`
// bounded by 1,000 (the complete set, not the 100-item walk granularity),
// and no `next_offset`. The walker requests pages of 100; the aggregate
// reports the whole reassembled window (contracts.md 9.1/14.3).
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_fetch_all_reports_one_aggregate_window() {
    let fixture = ProtocolFixture::start().await;
    // total 120 items across two walked pages (100 + 20).
    fixture.on_bulk_results(Step::Json(results_page(0, 100, 100, 120)));
    fixture.on_bulk_results(Step::Json(results_page(100, 100, 20, 120)));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID])
        .output()
        .expect("run bulk results fetch-all over two pages");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_results");
    let data = &envelope["data"];
    assert_eq!(data["offset"], 0);
    assert_eq!(
        data["limit"], 120,
        "the aggregate window limit is max(1, total_items), not the 100-item page size"
    );
    assert!(
        data.get("next_offset").is_none(),
        "no next_offset on the complete aggregate"
    );
    assert_eq!(data["items"].as_array().unwrap().len(), 120);
    // The walk requested pages of 100 at offsets 0 and 100.
    let recorded = fixture.requests();
    let requests = BulkRequestView::for_path(&recorded, BULK_ID, "/results");
    assert_eq!(requests.len(), 2, "two walked pages");
    assert!(requests[0].query.contains("offset=0"));
    assert!(requests[0].query.contains("limit=100"));
    assert!(requests[1].query.contains("offset=100"));
    assert!(requests[1].query.contains("limit=100"));
    fixture.shutdown().await;
}

// A --limit outside 1..=1000 is a usage error before any request.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_rejects_an_out_of_range_limit() {
    let fixture = ProtocolFixture::start().await;
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID, "--limit", "1001"])
        .output()
        .expect("run bulk results --limit 1001");
    assert_eq!(output.status.code(), Some(2));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "unsupported_input");
    assert_eq!(fixture.get_count(), 0);
    fixture.shutdown().await;
}

// A bulk results read of an unknown job surfaces the upstream_not_found note
// plus the canonical error.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_unknown_job_notes_the_id() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Status(404, None, None));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID, "--limit", "5"])
        .output()
        .expect("run bulk results for an unknown job");
    assert_eq!(output.status.code(), Some(6));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "upstream_not_found");
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    assert!(stderr.contains("does not recognize"), "{stderr}");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

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
    // The timeout envelope preserves the last observed stage for
    // reconciliation, matching the shared `RunningAnalysis` observation path.
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
    // The canonical envelope stays on stdout.
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "task_wait");
    // Progress events are JSONL on stderr, one event per line, content-free.
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

// A fetch-all results walk over pages that repeat one source position is
// contract drift, never merged silently: the process fails closed as
// `upstream_contract_changed` with the network-or-upstream exit (6).
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_fetch_all_rejects_a_duplicate_position_across_pages() {
    let fixture = ProtocolFixture::start().await;
    // First page covers position 0 of a 2-position job; the same position
    // arrives again on the next page (no union coverage advance).
    fixture.on_bulk_results(Step::Json(json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 100,
        "total_items": 2,
        "items": [
            {"index": 0, "id": "row-000", "task_id": "task-000", "stage": "STAGE_SUCCESS",
             "error": null, "result": pangram4_success("first synthetic words")}
        ],
        "failed_items": []
    })));
    fixture.on_bulk_results(Step::Json(json!({
        "bulk_id": BULK_ID,
        "offset": 1,
        "limit": 100,
        "total_items": 2,
        "items": [
            {"index": 0, "id": "row-000", "task_id": "task-000", "stage": "STAGE_SUCCESS",
             "error": null, "result": pangram4_success("first synthetic words")}
        ],
        "failed_items": []
    })));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID])
        .output()
        .expect("run bulk results with a duplicated position across pages");
    assert_eq!(output.status.code(), Some(6));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "upstream_contract_changed");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// A fetch-all walk whose page carries a source position at or beyond the
// job total is contract drift (no position can be `>= total_items`).
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_fetch_all_rejects_a_position_at_or_above_the_total() {
    let fixture = ProtocolFixture::start().await;
    // total_items=1 but the page claims source position 1: not covered by
    // any valid window, so the read fails contract-clean.
    fixture.on_bulk_results(Step::Json(json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 100,
        "total_items": 1,
        "items": [
            {"index": 1, "id": "row-001", "task_id": "task-001", "stage": "STAGE_SUCCESS",
             "error": null, "result": pangram4_success("spilled words")}
        ],
        "failed_items": []
    })));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID])
        .output()
        .expect("run bulk results with an out-of-total position");
    assert_eq!(output.status.code(), Some(6));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "upstream_contract_changed");
    assert_no_leak(&output);
    fixture.shutdown().await;
}

// The documented page window validation failure (an empty page while
// positions remain uncovered: non-advancing drift) also fails closed with
// the contract category because an empty page can never terminate a
// partially-covered fetch-all walk as a fake success.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_results_fetch_all_rejects_an_empty_page_before_full_coverage() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_results(Step::Json(json!({
        "bulk_id": BULK_ID,
        "offset": 0,
        "limit": 100,
        "total_items": 2,
        "items": [],
        "failed_items": []
    })));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "results", BULK_ID])
        .output()
        .expect("run bulk results with a non-advancing empty page");
    assert_eq!(output.status.code(), Some(6));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["error"]["code"], "upstream_contract_changed");
    assert_no_leak(&output);
    fixture.shutdown().await;
}
