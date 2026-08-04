//! Real-SQLite proof of the reconciliation-refresh invariants (contracts.md
//! 14.2 durable-authorship invariance, docs/history-contract.md ownership):
//!
//! - a non-terminal observation refresh can never erase an attested terminal
//!   result or error body (the update coalesces `result_json`, `error_json`,
//!   and `completed_at`)
//! - `find_analysis_by_task` resolves deterministically when one upstream
//!   task ID exists across kinds (the schema's
//!   `UNIQUE (check_kind, upstream_task_id)` now rejects a same-kind
//!   duplicate at commit, so the anomalous same-kind duplicate state the
//!   lookup once tolerated can no longer be persisted)
//!
//! No mocks: every `HistoryStore` points at a real `tempfile::TempDir`.

#![forbid(unsafe_code)]

use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, CheckKind, SaveState, Sha256Hash, SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryErrorCode, HistoryStore, InputKind, ObservationSnapshot, StoredAnalysis,
    StoredUpstreamTask,
};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("test timestamp")
}

fn terminal_analysis(id: &str) -> StoredAnalysis {
    StoredAnalysis {
        id: AnalysisId::from_str(id).expect("analysis id"),
        bulk: None,
        caller_id: None,
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        save_state: SaveState::SavedManual,
        input_kind: InputKind::Text,
        input_sha256: Sha256Hash::from_bytes([7; 32]),
        display_name: Some("authored.txt".to_owned()),
        input_json: "{\"type\":\"text\",\"text\":\"authored\"}".to_owned(),
        result_json: Some("{\"headline\":\"Human-written\"}".to_owned()),
        error_json: None,
        upstream_version: Some("4.0".to_owned()),
        retry_of: None,
        rerun_of: None,
        created_at: timestamp("2026-08-01T10:00:00Z"),
        updated_at: timestamp("2026-08-01T10:05:00Z"),
        completed_at: Some(timestamp("2026-08-01T10:05:00Z")),
        search_input_text: Some("authored".to_owned()),
        search_filename: Some("authored.txt".to_owned()),
        search_headline: Some("Human-written".to_owned()),
        search_source_urls: None,
    }
}

fn observation(
    analysis_id: &str,
    kind: CheckKind,
    task: &str,
    stage: Option<&str>,
) -> StoredUpstreamTask {
    StoredUpstreamTask {
        analysis_id: AnalysisId::from_str(analysis_id).expect("analysis id"),
        check_kind: kind,
        upstream_task_id: task.to_owned(),
        last_stage: stage.map(str::to_owned),
        observed_at: timestamp("2026-08-01T10:05:00Z"),
    }
}

/// A non-terminal observation refresh after a terminal result was recorded
/// must never blank the stored terminal body. This locks the durable
/// authorship invariant at the SQL seam: the `UPDATE` coalesces
/// `result_json`, `error_json`, and `completed_at`, so a `running` read that
/// arrives after the terminal snapshot cannot regress it.
#[test]
fn nonterminal_refresh_never_erases_the_attested_terminal_body() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let record = terminal_analysis("anl_01983c20-0180-7a80-a001-000000000001");
    store
        .save_analysis_atomic(&record, &[])
        .expect("terminal save commits");

    // A later non-terminal observation carries no terminal fields: status
    // moves, but the recorded body must survive. Search columns are the
    // merge-resolved values (the adapter merge keeps prior content when the
    // observation carries none).
    let snapshot = ObservationSnapshot {
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Terminal,
        result_json: None,
        error_json: None,
        upstream_version: None,
        completed_at: None,
        search_input_text: Some("authored".to_owned()),
        search_filename: Some("authored.txt".to_owned()),
        search_headline: Some("Human-written".to_owned()),
        search_source_urls: None,
    };
    store
        .update_observation_snapshot(
            &AnalysisId::from_str("anl_01983c20-0180-7a80-a001-000000000001").unwrap(),
            timestamp("2026-08-02T09:00:00Z"),
            &snapshot,
        )
        .expect("refresh commits");

    let stored = store
        .get_analysis(&AnalysisId::from_str("anl_01983c20-0180-7a80-a001-000000000001").unwrap())
        .expect("read back");
    assert_eq!(stored.status, AnalysisStatus::Running);
    assert_eq!(
        stored.result_json.as_deref(),
        Some("{\"headline\":\"Human-written\"}"),
        "a non-terminal refresh never blanks the recorded result"
    );
    assert_eq!(
        stored.completed_at,
        Some(timestamp("2026-08-01T10:05:00Z")),
        "a non-terminal refresh never blanks the recorded completed_at"
    );
    // Search payload columns the refresh leaves empty keep the stored values.
    assert_eq!(
        stored.search_input_text.as_deref(),
        Some("authored"),
        "a body-empty refresh never wipes locally held input text"
    );
    assert_eq!(stored.search_headline.as_deref(), Some("Human-written"));
    assert_eq!(
        stored.upstream_version.as_deref(),
        Some("4.0"),
        "an absent incoming version preserves validated provenance"
    );
}

/// Result-owned search metadata is observation state, not durable local
/// authorship. A newer terminal observation replaces a stale headline and
/// source URL payload when present, while input text and filename remain
/// first-recorded local evidence.
#[test]
fn terminal_refresh_replaces_result_search_metadata_but_preserves_input_authorship() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let mut record = terminal_analysis("anl_01983c20-0180-7a80-a001-000000000002");
    record.search_source_urls = Some("https://old.example/source".to_owned());
    store
        .save_analysis_atomic(&record, &[])
        .expect("terminal save commits");

    let snapshot = ObservationSnapshot {
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        result_json: Some("{\"headline\":\"Refreshed result\"}".to_owned()),
        error_json: None,
        upstream_version: Some("4.1".to_owned()),
        completed_at: Some(timestamp("2026-08-02T09:00:00Z")),
        // A remote observation cannot replace locally held input authorship.
        search_input_text: Some("remote replacement must not win".to_owned()),
        search_filename: Some("remote.txt".to_owned()),
        search_headline: Some("Refreshed result".to_owned()),
        search_source_urls: Some("https://new.example/source".to_owned()),
    };
    store
        .update_observation_snapshot(
            &AnalysisId::from_str("anl_01983c20-0180-7a80-a001-000000000002").unwrap(),
            timestamp("2026-08-02T09:00:00Z"),
            &snapshot,
        )
        .expect("refresh commits");

    let stored = store
        .get_analysis(&AnalysisId::from_str("anl_01983c20-0180-7a80-a001-000000000002").unwrap())
        .expect("read refreshed row");
    assert_eq!(stored.search_input_text.as_deref(), Some("authored"));
    assert_eq!(stored.search_filename.as_deref(), Some("authored.txt"));
    assert_eq!(stored.search_headline.as_deref(), Some("Refreshed result"));
    assert_eq!(
        stored.search_source_urls.as_deref(),
        Some("https://new.example/source")
    );
    assert_eq!(store.search("Refreshed", 10).unwrap().len(), 1);
    assert_eq!(store.search("old", 10).unwrap().len(), 0);
    assert_eq!(store.search("new", 10).unwrap().len(), 1);
    assert_eq!(stored.upstream_version.as_deref(), Some("4.1"));

    let absent_metadata = ObservationSnapshot {
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        result_json: Some("{\"headline\":\"No replacement metadata\"}".to_owned()),
        error_json: None,
        upstream_version: None,
        completed_at: Some(timestamp("2026-08-02T10:00:00Z")),
        search_input_text: None,
        search_filename: None,
        search_headline: None,
        search_source_urls: None,
    };
    store
        .update_observation_snapshot(
            &AnalysisId::from_str("anl_01983c20-0180-7a80-a001-000000000002").unwrap(),
            timestamp("2026-08-02T10:00:00Z"),
            &absent_metadata,
        )
        .expect("metadata-absent refresh commits");
    let preserved = store
        .get_analysis(&AnalysisId::from_str("anl_01983c20-0180-7a80-a001-000000000002").unwrap())
        .expect("read metadata-preserving refresh");
    assert_eq!(preserved.search_input_text.as_deref(), Some("authored"));
    assert_eq!(preserved.search_filename.as_deref(), Some("authored.txt"));
    assert_eq!(
        preserved.search_headline.as_deref(),
        Some("Refreshed result")
    );
    assert_eq!(
        preserved.search_source_urls.as_deref(),
        Some("https://new.example/source")
    );
    assert_eq!(
        preserved.upstream_version.as_deref(),
        Some("4.1"),
        "a later versionless observation preserves the refreshed version"
    );
}

/// The same invariant binds the bulk-child reconciliation upsert: a child
/// whose stored row carries an attested error keeps it even when the refresh
/// projects no body for that slot.
#[test]
fn bulk_child_refresh_never_erases_the_attested_error_body() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");

    let collection_id = "bulk_01983c20-0180-7a80-a001-0000000000a1";
    let collection = microck_pangram_cli::history::StoredBulkCollection {
        id: microck_pangram_cli::domain::BulkId::from_str(collection_id).expect("bulk id"),
        upstream_bulk_id: Some("upstream-bulk-coalesce".to_owned()),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        counters: microck_pangram_cli::domain::BulkCounters::new(1, 1, 0, 0).expect("counters"),
        estimated_billable_units: Some(1),
        created_at: timestamp("2026-08-01T09:00:00Z"),
        updated_at: timestamp("2026-08-01T09:00:00Z"),
        completed_at: None,
    };
    let mut failed_child = terminal_analysis("anl_01983c20-0180-7a80-a001-000000000041");
    failed_child.bulk = Some((
        microck_pangram_cli::domain::BulkId::from_str(collection_id).unwrap(),
        0,
    ));
    failed_child.status = AnalysisStatus::Failed;
    failed_child.submission_outcome = SubmissionOutcome::Accepted;
    failed_child.save_state = SaveState::SavedHistory;
    failed_child.result_json = None;
    failed_child.error_json = Some("{\"code\":\"upstream_analysis_failed\"}".to_owned());
    store
        .upsert_bulk_collection_atomic(&collection, &[(failed_child, Vec::new())])
        .expect("first save commits");

    // A later refresh of the same membership projects a non-terminal child
    // with no error body (for example an intermediate status read): the
    // stored terminal error stays attested.
    let mut running_child = terminal_analysis("anl_01983c20-0180-7a80-a001-0000000000ee");
    running_child.bulk = Some((
        microck_pangram_cli::domain::BulkId::from_str(collection_id).unwrap(),
        0,
    ));
    running_child.status = AnalysisStatus::Running;
    running_child.submission_outcome = SubmissionOutcome::Accepted;
    running_child.save_state = SaveState::Ephemeral;
    running_child.result_json = None;
    running_child.error_json = None;
    running_child.completed_at = None;
    running_child.input_json = "{}".to_owned();
    running_child.display_name = None;
    running_child.search_input_text = None;
    running_child.search_filename = None;
    running_child.search_headline = None;
    store
        .upsert_bulk_collection_atomic(&collection, &[(running_child, Vec::new())])
        .expect("refresh commits");

    let members = store
        .list_bulk_analyses(&microck_pangram_cli::domain::BulkId::from_str(collection_id).unwrap())
        .expect("list members");
    assert_eq!(members.len(), 1, "one reconciled child");
    assert_eq!(members[0].status, AnalysisStatus::Running);
    assert_eq!(
        members[0].error_json.as_deref(),
        Some("{\"code\":\"upstream_analysis_failed\"}"),
        "a non-terminal refresh never blanks the recorded error"
    );
    assert_eq!(
        members[0].completed_at,
        Some(timestamp("2026-08-01T10:05:00Z")),
        "the recorded terminal stamp survives"
    );
    assert_eq!(members[0].upstream_version.as_deref(), Some("4.0"));
}

/// When duplicate upstream task IDs exist across check kinds (a bulk child
/// and a task read can legitimately share one provider task id only across
/// kinds), `find_analysis_by_task` must resolve one row deterministically:
/// the kind filter keeps the lookup exact, never returning a row for the
/// other kind.
#[test]
fn find_analysis_by_task_is_deterministic_across_kinds() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let ai = terminal_analysis("anl_01983c20-0180-7a80-a001-0000000000a1");
    let mut ai2 = terminal_analysis("anl_01983c20-0180-7a80-a001-0000000000a2");
    ai2.save_state = SaveState::SavedHistory;
    store
        .save_analysis_atomic(
            &ai,
            &[observation(
                "anl_01983c20-0180-7a80-a001-0000000000a1",
                CheckKind::AiDetection,
                "task-dup",
                None,
            )],
        )
        .expect("first save commits");
    store
        .save_analysis_atomic(
            &ai2,
            &[observation(
                "anl_01983c20-0180-7a80-a001-0000000000a2",
                CheckKind::Plagiarism,
                "task-dup",
                Some("STAGE_SUCCESS"),
            )],
        )
        .expect("second save commits");

    // Each kind resolves deterministically to exactly its own row.
    let ai_hit = store
        .find_analysis_by_task(CheckKind::AiDetection, "task-dup")
        .expect("lookup works")
        .expect("the ai_detection row resolves");
    assert_eq!(
        ai_hit.id.to_string(),
        "anl_01983c20-0180-7a80-a001-0000000000a1"
    );
    let pl_hit = store
        .find_analysis_by_task(CheckKind::Plagiarism, "task-dup")
        .expect("lookup works")
        .expect("the plagiarism row resolves");
    assert_eq!(
        pl_hit.id.to_string(),
        "anl_01983c20-0180-7a80-a001-0000000000a2"
    );
}

/// The `UNIQUE (check_kind, upstream_task_id)` constraint makes the
/// anomalous same-kind duplicate unrepresentable: persisting a second
/// analysis whose observation references an already-recorded
/// `(check_kind, upstream_task_id)` fails at commit and rolls the whole
/// atomic batch back, so the durable store can never hold two rows for one
/// upstream task identity of the same kind. The first row is untouched.
#[test]
fn a_same_kind_duplicate_task_identity_is_rejected_at_commit() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let one = terminal_analysis("anl_01983c20-0180-7a80-a001-000000000010");
    store
        .save_analysis_atomic(
            &one,
            &[observation(
                "anl_01983c20-0180-7a80-a001-000000000010",
                CheckKind::AiDetection,
                "task-shared",
                None,
            )],
        )
        .expect("first save commits");

    let two = terminal_analysis("anl_01983c20-0180-7a80-a001-000000000011");
    let error = store
        .save_analysis_atomic(
            &two,
            &[observation(
                "anl_01983c20-0180-7a80-a001-000000000011",
                CheckKind::AiDetection,
                "task-shared",
                None,
            )],
        )
        .expect_err("a same-kind duplicate task identity must be rejected");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    // The whole batch rolled back: the second analysis row, its FTS
    // payload, and its observation are absent, and the first row is intact.
    let counts = store
        .with_connection(|connection| {
            let analyses: i64 = connection
                .query_row("SELECT COUNT(*) FROM analyses", [], |row| row.get(0))
                .unwrap();
            let tasks: i64 = connection
                .query_row("SELECT COUNT(*) FROM upstream_tasks", [], |row| row.get(0))
                .unwrap();
            let search: i64 = connection
                .query_row("SELECT COUNT(*) FROM analysis_search", [], |row| row.get(0))
                .unwrap();
            (analyses, tasks, search)
        })
        .expect("read counts");
    assert_eq!(
        counts,
        (1, 1, 1),
        "the duplicate batch never persisted; the first row stands"
    );
    let resolved = store
        .find_analysis_by_task(CheckKind::AiDetection, "task-shared")
        .expect("lookup works")
        .expect("one row resolves");
    assert_eq!(
        resolved.id.to_string(),
        "anl_01983c20-0180-7a80-a001-000000000010",
        "the one authoritative row is the first committed one"
    );
}
