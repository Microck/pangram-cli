//! Phase 4 Packet C remediation: the exactly-one-warning invocation latch of
//! the automatic history gate (contracts.md 14.2 note), against the real
//! loopback Pangram 4 fixture and a real SQLite store. No mocks, no live
//! Pangram, no real credentials.
//!
//! Split out of `history-save-reconciliation-loopback.rs` so each suite stays
//! under the source-size threshold while both exercise the exact same real
//! store and fixture. This suite locks the latch semantics:
//! - one failed observed-children read in one bulk invocation emits exactly
//!   one direct sanitized `warning:` line, never a doubled `note: warning:`,
//!   and the remote outcome stays truthful (counters and exit preserved)
//! - one invocation in which BOTH automatic-history phases fail (the
//!   observed-children read AND the store open/write) still emits exactly
//!   one `warning:` line: the latch binds across the phases

#![cfg(feature = "dev-tools")]

#[path = "support/history_save_env.rs"]
mod harness;

use harness::fixture::{ProtocolFixture, Step};
use harness::{
    Isolated, analyses_rows, assert_no_leak, bulk_collection_rows, stderr_text, stdout_envelope,
    task_rows,
};

// ------------------------------------------------------ one-warning latch --

/// A failed children reconstruction read after a terminal `bulk wait`
/// (observed children could not be fetched: the results window request
/// fails) surfaces exactly one sanitized warning and never degrades the
/// observed collection: the wait's own truthful counters and exit code
/// stay. The incomplete terminal collection-plus-children unit is not
/// persisted, so history cannot combine terminal counters with queued
/// acceptance children from an earlier point in time.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_wait_with_armed_history_skips_save_when_the_children_read_fails() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": "blk_wait_children_fail",
            "status": "queued",
            "total_items": 2,
            "accepted_items": [
                {"index": 0, "id": "row-000", "task_id": "task-000"},
                {"index": 1, "id": "row-001", "task_id": "task-001"}
            ],
            "failed_items": []
        })),
    ));
    fixture.on_bulk_status(Step::Json(serde_json::json!({
        "bulk_id": "blk_wait_children_fail",
        "status": "succeeded",
        "total_items": 2,
        "accepted": 2,
        "succeeded": 2,
        "failed": 0,
        "created_at": "1760000000.0",
        "completed_at": "1760000001.0"
    })));
    // The observed-children read deliberately fails: an undecodable body is
    // contract drift (never retried), and the read path maps it to one
    // sanitized warning without degrading the wait outcome.
    fixture.on_bulk_results(Step::Json(serde_json::json!({"unexpected": true})));
    let isolated = Isolated::new();
    isolated.enable_history();

    let root = tempfile::tempdir().unwrap();
    let items = root.path().join("wait-items.jsonl");
    std::fs::write(
        &items,
        [
            serde_json::json!({"id": "row-000", "text": "words the fixture accepts"}).to_string(),
            serde_json::json!({"id": "row-001", "text": "more words here"}).to_string(),
        ]
        .join("\n"),
    )
    .unwrap();

    let output = isolated
        .command(fixture.base_url())
        .args([
            "bulk",
            "submit",
            items.to_str().unwrap(),
            "--max-billable-units",
            "5",
            "--wait",
        ])
        .output()
        .expect("run bulk submit --wait with a failing children read");

    assert_eq!(
        output.status.code(),
        Some(0),
        "the upstream wait result still exits 0 (a collection success)"
    );
    let envelope = stdout_envelope(&output);
    let data = &envelope["data"];
    assert_eq!(data["status"], "succeeded");
    assert_eq!(data["succeeded"], 2);
    let stderr = stderr_text(&output);
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.contains("warning:"))
            .count(),
        1,
        "exactly one automatic save warning: {stderr}"
    );
    assert_no_leak(&output);
    assert!(
        !isolated.database_path().exists(),
        "a failed terminal child refresh must not persist a mixed-time snapshot"
    );
    fixture.shutdown().await;
}

/// If observation fails after HTTP 202 acceptance, the terminal refresh is
/// unavailable but the acceptance unit remains independently certified. The
/// command keeps its exit-1 observation failure while automatic history saves
/// the accepted collection, local inputs, and upstream task identities.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_wait_observation_failure_persists_the_acceptance_unit() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": "blk_wait_observation_fail",
            "status": "queued",
            "total_items": 2,
            "accepted_items": [
                {"index": 0, "id": "row-000", "task_id": "task-000"},
                {"index": 1, "id": "row-001", "task_id": "task-001"}
            ],
            "failed_items": []
        })),
    ));
    fixture.on_bulk_status(Step::Status(404, None, None));
    let isolated = Isolated::new();
    isolated.enable_history();
    let root = tempfile::tempdir().unwrap();
    let items = root.path().join("observation-fail-items.jsonl");
    std::fs::write(
        &items,
        [
            serde_json::json!({"id": "row-000", "text": "first accepted input"}).to_string(),
            serde_json::json!({"id": "row-001", "text": "second accepted input"}).to_string(),
        ]
        .join("\n"),
    )
    .unwrap();

    let output = isolated
        .command(fixture.base_url())
        .args([
            "bulk",
            "submit",
            items.to_str().unwrap(),
            "--max-billable-units",
            "5",
            "--wait",
        ])
        .output()
        .expect("run accepted submission with observation failure");

    assert_eq!(output.status.code(), Some(1));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_wait");
    assert!(envelope["error"].is_object());
    let stderr = stderr_text(&output);
    assert!(stderr.contains("accepted but its local observation failed"));
    assert!(
        !stderr.contains("warning:"),
        "the acceptance save succeeded"
    );
    assert_no_leak(&output);

    let connection = isolated.open_database();
    let collections = bulk_collection_rows(&connection);
    assert_eq!(collections.len(), 1);
    assert_eq!(
        collections[0].1.as_deref(),
        Some("blk_wait_observation_fail")
    );
    assert_eq!(collections[0].2, "queued");
    assert_eq!(collections[0].3, (2, 2, 0, 0));
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.1 == "queued"));
    assert!(rows.iter().all(|row| row.2 == "accepted"));
    assert!(rows.iter().all(|row| row.3 == "saved_history"));
    assert_eq!(task_rows(&connection).len(), 2);
    fixture.shutdown().await;
}

/// An observation failure does not suppress a best-effort acceptance save.
/// If that save also fails, the command keeps its exit-1 observation error,
/// emits exactly one direct automatic-history warning, and creates no store.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_wait_observation_and_acceptance_save_failures_warn_once() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": "blk_observation_and_save_fail",
            "status": "queued",
            "total_items": 2,
            "accepted_items": [
                {"index": 0, "id": "row-000", "task_id": "task-000"},
                {"index": 1, "id": "row-001", "task_id": "task-001"}
            ],
            "failed_items": []
        })),
    ));
    fixture.on_bulk_status(Step::Status(404, None, None));
    let isolated = Isolated::new();
    isolated.enable_history();
    // The acceptance save's owner-only protection cannot be established on
    // the existing investigation path (`data/history` is
    // already present with a non-owner-only mode), so the store fails
    // closed with `insecure_history_permissions` before ever opening
    // SQLite.
    std::fs::create_dir_all(isolated.history_directory()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            isolated.history_directory(),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }

    let root = tempfile::tempdir().unwrap();
    let items = root.path().join("both-items.jsonl");
    std::fs::write(
        &items,
        [
            serde_json::json!({"id": "row-000", "text": "words the fixture accepts"}).to_string(),
            serde_json::json!({"id": "row-001", "text": "more words here"}).to_string(),
        ]
        .join("\n"),
    )
    .unwrap();

    let output = isolated
        .command(fixture.base_url())
        .args([
            "bulk",
            "submit",
            items.to_str().unwrap(),
            "--max-billable-units",
            "5",
            "--wait",
        ])
        .output()
        .expect("run bulk submit --wait with observation and save failures");

    // The accepted submission remains real, but local observation failed.
    assert_eq!(
        output.status.code(),
        Some(1),
        "the observation failure keeps exit 1: {}",
        stderr_text(&output)
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_wait");
    assert!(envelope["error"].is_object());
    // Exactly one direct warning for the failed best-effort acceptance save.
    let stderr = stderr_text(&output);
    let warnings: Vec<&str> = stderr
        .lines()
        .filter(|line| line.contains("warning:"))
        .collect();
    assert_eq!(warnings.len(), 1, "exactly one warning line: {stderr}");
    assert!(
        warnings[0].starts_with("warning: "),
        "the one warning is a direct warning line: {stderr}"
    );
    assert_no_leak(&output);
    assert!(
        !isolated.database_path().exists(),
        "the store failed closed: no database was created"
    );
    fixture.shutdown().await;
}
