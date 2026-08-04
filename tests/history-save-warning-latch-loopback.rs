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
use harness::{Isolated, analyses_rows, assert_no_leak, stderr_text, stdout_envelope};

// ------------------------------------------------------ one-warning latch --

/// A failed children reconstruction read after a terminal `bulk wait`
/// (observed children could not be fetched: the results window request
/// fails) surfaces exactly one sanitized warning and never degrades the
/// observed collection: the wait's own truthful counters and exit code
/// stay, and the stored children are reconciled from the acceptance the
/// 202 already attested (never a dropped member).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn bulk_wait_with_armed_history_warns_once_when_the_children_read_fails() {
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
    // The store reconciled the acceptance-attested children (never a
    // dropped member) despite the failed children-read refresh.
    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(
        rows.len(),
        2,
        "the two acceptance-attested children persisted (admits the read fell back, never a dropped member)"
    );
    for row in &rows {
        assert_eq!(row.3, "saved_history");
    }
    drop(connection);
    fixture.shutdown().await;
}

/// The exactly-one-warning latch binds across the two automatic-history
/// phases of one bulk invocation (contracts.md 14.2 note): when BOTH the
/// observed-children read fails (an undecodable results window) AND the
/// subsequent store open/write fails (an unprotectable data directory),
/// the whole `bulk submit --wait` run still emits exactly one direct
/// `warning:` line, never one per phase, and the remote outcome stays
/// honest: the observed collection renders with its truthful succeeded
/// counters and the upstream-mapped exit code, and no store is created.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_wait_with_both_children_and_store_failures_warns_exactly_once() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": "blk_both_fail",
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
        "bulk_id": "blk_both_fail",
        "status": "succeeded",
        "total_items": 2,
        "accepted": 2,
        "succeeded": 2,
        "failed": 0,
        "created_at": "1760000000.0",
        "completed_at": "1760000001.0"
    })));
    // Phase one failure: the observed-children results window is
    // undecodable contract drift (never retried).
    fixture.on_bulk_results(Step::Json(serde_json::json!({"unexpected": true})));
    let isolated = Isolated::new();
    isolated.enable_history();
    // Phase two failure: the store's owner-only protection cannot be
    // established on the existing investigation path (`data/history` is
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
        .expect("run bulk submit --wait with both automatic phases failing");

    // The remote outcome stays honest: succeeded collection, exit 0.
    assert_eq!(
        output.status.code(),
        Some(0),
        "the upstream wait result still exits 0: {}",
        stderr_text(&output)
    );
    let envelope = stdout_envelope(&output);
    let data = &envelope["data"];
    assert_eq!(data["status"], "succeeded");
    assert_eq!(data["succeeded"], 2);
    // Exactly one direct `warning:` line for the whole invocation, and it
    // is a direct warning (never a doubled `note: warning:` prefix).
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
