//! Real-SQLite proof that a fresh analysis save (the typed row, its FTS
//! payload, and its current observation rows) commits atomically: a
//! mid-write failure rolls the whole batch back, so a half-committed
//! analysis can never persist (docs/history-contract.md transaction rule).
//!
//! No mocks: every `HistoryStore` points at a real `tempfile::TempDir`.

#![forbid(unsafe_code)]

use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, CheckKind, CheckStatus, SaveState, Sha256Hash, SubmissionOutcome,
    UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryErrorCode, HistoryStore, InputKind, StoredAnalysis, StoredCheck, StoredUpstreamTask,
};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("test timestamp")
}

fn analysis(id: &str) -> StoredAnalysis {
    let input = "atomic text";
    let input_sha256 = Sha256Hash::digest(input);
    StoredAnalysis {
        id: AnalysisId::from_str(id).expect("analysis id"),
        bulk: None,
        caller_id: None,
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        save_state: SaveState::SavedManual,
        input_kind: InputKind::Text,
        input_sha256,
        display_name: None,
        input_json: serde_json::json!({
            "type": "text",
            "origin": "literal",
            "sha256": input_sha256,
            "byte_count": input.len(),
            "word_count": 2,
            "text": input
        })
        .to_string(),
        result_json: None,
        error_json: None,
        upstream_version: None,
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(timestamp("2026-08-01T09:59:00Z")),
        created_at: timestamp("2026-08-01T10:00:00Z"),
        updated_at: timestamp("2026-08-01T10:05:00Z"),
        completed_at: None,
        search_input_text: Some("atomic text".to_owned()),
        search_filename: None,
        search_headline: None,
        search_source_urls: None,
    }
}

fn observation(analysis_id: &str, kind: CheckKind, task: &str) -> StoredUpstreamTask {
    StoredUpstreamTask {
        analysis_id: AnalysisId::from_str(analysis_id).expect("analysis id"),
        check_kind: kind,
        upstream_task_id: task.to_owned(),
        last_stage: None,
        observed_at: timestamp("2026-08-01T10:05:00Z"),
    }
}

fn row_counts(root: &tempfile::TempDir) -> (i64, i64, i64, i64) {
    let connection =
        rusqlite::Connection::open(root.path().join("history").join("pangram-history.db"))
            .expect("open saved database");
    let count = |table: &str| {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count rows")
    };
    (
        count("analyses"),
        count("analysis_search"),
        count("analysis_checks"),
        count("upstream_tasks"),
    )
}

/// A committed atomic save lands the analysis row, its FTS payload, and its
/// observation rows together.
#[test]
fn atomic_save_commits_row_fts_and_observations_together() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let record = analysis("anl_01983c20-0180-7a80-a001-000000000001");
    let observations = vec![
        observation(
            "anl_01983c20-0180-7a80-a001-000000000001",
            CheckKind::AiDetection,
            "task-a",
        ),
        observation(
            "anl_01983c20-0180-7a80-a001-000000000001",
            CheckKind::Plagiarism,
            "task-b",
        ),
    ];
    store
        .save_analysis_atomic(&record, &observations)
        .expect("atomic save commits");
    assert_eq!(row_counts(&root), (1, 1, 2, 2));

    let canonical = store
        .canonical_analysis(&record.id, true)
        .expect("the committed record is canonically readable");
    assert_eq!(canonical.status(), AnalysisStatus::Running);
    assert_eq!(canonical.checks().len(), 2);
    assert_eq!(canonical.checks()[0].status(), CheckStatus::Running);
    assert_eq!(canonical.checks()[1].status(), CheckStatus::Running);
    let value = serde_json::to_value(canonical).expect("canonical JSON");
    assert_eq!(value["checks"][0]["kind"], "ai_detection");
    assert_eq!(value["checks"][0]["upstream"]["task_id"], "task-a");
    assert_eq!(value["checks"][1]["kind"], "plagiarism");
    assert_eq!(value["checks"][1]["upstream"]["task_id"], "task-b");
}

/// A failing observation row (one that violates the foreign key by pointing
/// at a different, nonexistent analysis) rolls the whole batch back: no
/// typed row, no FTS payload, no observation survives.
#[test]
fn atomic_save_rolls_back_everything_when_one_member_fails() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let record = analysis("anl_01983c20-0180-7a80-a001-000000000002");
    // The second observation points its FK at a nonexistent analysis, which
    // SQLite rejects inside the same transaction; nothing may commit.
    let bad = observation(
        "anl_01983c20-0180-7a80-a001-0000000000ff",
        CheckKind::Plagiarism,
        "task-bad",
    );
    let observations = vec![
        observation(
            "anl_01983c20-0180-7a80-a001-000000000002",
            CheckKind::AiDetection,
            "task-good",
        ),
        bad,
    ];
    let error = store
        .save_analysis_atomic(&record, &observations)
        .expect_err("the batch must fail");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);
    assert_eq!(
        row_counts(&root),
        (0, 0, 0, 0),
        "a rolled-back batch leaves no row, FTS payload, or observation behind"
    );
}

/// The complete-save API validates the aggregate it actually wrote before
/// commit. Individually well-formed rows whose parent/check statuses disagree
/// therefore roll back instead of persisting an unreadable aggregate.
#[test]
fn complete_save_rolls_back_a_noncanonical_written_aggregate() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let record = analysis("anl_01983c20-0180-7a80-a001-000000000003");
    let check = StoredCheck {
        analysis_id: record.id,
        check_index: 0,
        check_kind: CheckKind::AiDetection,
        status: CheckStatus::Succeeded,
        result_json: Some("{}".to_owned()),
        error_json: None,
    };

    let error = store
        .save_analysis_complete(&record, &[check], &[])
        .expect_err("parent/check mismatch must fail certification");
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    assert_eq!(
        row_counts(&root),
        (0, 0, 0, 0),
        "in-transaction certification failure rolls back every aggregate row"
    );
}

/// The legacy atomic API cannot express two distinct terminal result bodies.
/// It therefore rejects malformed, duplicate, or noncanonical observation
/// sets instead of committing orphan task evidence or an unreadable parent.
#[test]
fn atomic_save_rejects_noncanonical_legacy_aggregates_with_full_rollback() {
    let cases = [
        (
            "duplicate",
            vec![
                observation(
                    "anl_01983c20-0180-7a80-a001-000000000010",
                    CheckKind::AiDetection,
                    "task-a",
                ),
                observation(
                    "anl_01983c20-0180-7a80-a001-000000000010",
                    CheckKind::AiDetection,
                    "task-b",
                ),
            ],
        ),
        (
            "out-of-order",
            vec![
                observation(
                    "anl_01983c20-0180-7a80-a001-000000000010",
                    CheckKind::Plagiarism,
                    "task-b",
                ),
                observation(
                    "anl_01983c20-0180-7a80-a001-000000000010",
                    CheckKind::AiDetection,
                    "task-a",
                ),
            ],
        ),
    ];
    for (name, observations) in cases {
        let root = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(root.path()).expect("open history store");
        let record = analysis("anl_01983c20-0180-7a80-a001-000000000010");
        let error = store
            .save_analysis_atomic(&record, &observations)
            .expect_err(name);
        assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed, "{name}");
        assert_eq!(row_counts(&root), (0, 0, 0, 0), "{name}");
    }

    for name in ["both-bodies", "terminal-multicheck", "parent-status"] {
        let root = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(root.path()).expect("open history store");
        let id = "anl_01983c20-0180-7a80-a001-000000000011";
        let mut record = analysis(id);
        let observations = vec![
            observation(id, CheckKind::AiDetection, "task-a"),
            observation(id, CheckKind::Plagiarism, "task-b"),
        ];
        match name {
            "both-bodies" => {
                record.result_json = Some("{}".to_owned());
                record.error_json = Some("{}".to_owned());
            }
            "terminal-multicheck" => {
                record.status = AnalysisStatus::Succeeded;
                record.submission_outcome = SubmissionOutcome::Terminal;
                record.result_json = Some("{}".to_owned());
                record.completed_at = Some(timestamp("2026-08-01T10:05:00Z"));
            }
            "parent-status" => {
                record.status = AnalysisStatus::Partial;
                record.submission_outcome = SubmissionOutcome::Terminal;
                record.completed_at = Some(timestamp("2026-08-01T10:05:00Z"));
            }
            _ => unreachable!(),
        }
        let error = store
            .save_analysis_atomic(&record, &observations)
            .expect_err(name);
        assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed, "{name}");
        assert_eq!(row_counts(&root), (0, 0, 0, 0), "{name}");
    }
}
