//! Real-SQLite proof that a fresh analysis save (the typed row, its FTS
//! payload, and its current observation rows) commits atomically: a
//! mid-write failure rolls the whole batch back, so a half-committed
//! analysis can never persist (docs/history-contract.md transaction rule).
//!
//! No mocks: every `HistoryStore` points at a real `tempfile::TempDir`.

#![forbid(unsafe_code)]

use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, CheckKind, SaveState, Sha256Hash, SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryErrorCode, HistoryStore, InputKind, StoredAnalysis, StoredUpstreamTask,
};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("test timestamp")
}

fn analysis(id: &str) -> StoredAnalysis {
    StoredAnalysis {
        id: AnalysisId::from_str(id).expect("analysis id"),
        bulk: None,
        caller_id: None,
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        save_state: SaveState::SavedManual,
        input_kind: InputKind::Text,
        input_sha256: Sha256Hash::from_bytes([1; 32]),
        display_name: None,
        input_json: "{\"type\":\"text\"}".to_owned(),
        result_json: Some("{}".to_owned()),
        error_json: None,
        upstream_version: None,
        retry_of: None,
        rerun_of: None,
        created_at: timestamp("2026-08-01T10:00:00Z"),
        updated_at: timestamp("2026-08-01T10:05:00Z"),
        completed_at: Some(timestamp("2026-08-01T10:05:00Z")),
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

fn row_counts(root: &tempfile::TempDir) -> (i64, i64, i64) {
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
    assert_eq!(row_counts(&root), (1, 1, 2));
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
        (0, 0, 0),
        "a rolled-back batch leaves no row, FTS payload, or observation behind"
    );
}
