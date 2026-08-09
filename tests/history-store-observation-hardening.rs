//! Real-SQLite proof of the reconciliation-refresh invariants (contracts.md
//! 14.2 durable-authorship invariance, docs/history-contract.md ownership):
//!
//! - a non-terminal observation refresh can never regress an attested
//!   terminal snapshot (status, outcome, checks, body, completion, FTS, and
//!   provenance remain terminal)
//! - `find_analysis_by_task` resolves deterministically when one upstream
//!   task ID exists across kinds (the schema's
//!   `UNIQUE (check_kind, upstream_task_id)` now rejects a same-kind
//!   duplicate at commit, so the anomalous same-kind duplicate state the
//!   lookup once tolerated can no longer be persisted)
//!
//! No mocks: every `HistoryStore` points at a real `tempfile::TempDir`.

#![forbid(unsafe_code)]

#[path = "support/history_store.rs"]
mod history_store_support;

use std::str::FromStr;

use history_store_support::{ai_result, canonical_error, prepared_child};
use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, CheckKind, CheckStatus, SaveState, Sha256Hash, SubmissionOutcome,
    UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryErrorCode, HistoryStore, InputKind, ObservationSnapshot, StoredAnalysis, StoredCheck,
    StoredUpstreamTask,
};
use microck_pangram_cli::output::ErrorCode;

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("test timestamp")
}

fn terminal_analysis(id: &str) -> StoredAnalysis {
    let input = "authored";
    let input_sha256 = Sha256Hash::digest(input);
    StoredAnalysis {
        id: AnalysisId::from_str(id).expect("analysis id"),
        bulk: None,
        caller_id: None,
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        save_state: SaveState::SavedManual,
        input_kind: InputKind::Text,
        input_sha256,
        display_name: None,
        input_json: serde_json::json!({
            "type": "text",
            "origin": "literal",
            "sha256": input_sha256,
            "byte_count": input.len(),
            "word_count": 1,
            "text": input
        })
        .to_string(),
        result_json: Some(ai_result("Human-written")),
        error_json: None,
        upstream_version: Some("4.0".to_owned()),
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(timestamp("2026-08-01T09:59:00Z")),
        created_at: timestamp("2026-08-01T10:00:00Z"),
        updated_at: timestamp("2026-08-01T10:05:00Z"),
        completed_at: Some(timestamp("2026-08-01T10:05:00Z")),
        search_input_text: Some("authored".to_owned()),
        search_filename: None,
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

fn raw_state(store: &HistoryStore, id: AnalysisId) -> Vec<String> {
    store
        .with_connection(|connection| {
            let mut rows = Vec::new();
            for sql in [
                "SELECT status || '|' || updated_at || '|' ||
                        COALESCE(result_json, '') || '|' || COALESCE(error_json, '')
                 FROM analyses WHERE id = ?1",
                "SELECT check_index || '|' || check_kind || '|' || status || '|' ||
                        COALESCE(result_json, '') || '|' || COALESCE(error_json, '')
                 FROM analysis_checks WHERE analysis_id = ?1 ORDER BY check_index",
                "SELECT check_kind || '|' || upstream_task_id || '|' ||
                        COALESCE(last_stage, '') || '|' || observed_at
                 FROM upstream_tasks WHERE analysis_id = ?1 ORDER BY check_kind",
                "SELECT COALESCE(input_text, '') || '|' || COALESCE(filename, '') || '|' ||
                        COALESCE(headline, '') || '|' || COALESCE(source_urls, '')
                 FROM analysis_search WHERE analysis_id = ?1",
            ] {
                let mut statement = connection.prepare(sql)?;
                rows.extend(
                    statement
                        .query_map([id.to_string()], |row| row.get::<_, String>(0))?
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            Ok::<_, rusqlite::Error>(rows)
        })
        .expect("borrow database")
        .expect("read raw state")
}

/// A non-terminal observation refresh after a terminal result was recorded
/// must never regress any terminal parent state.
#[test]
fn nonterminal_refresh_never_erases_the_attested_terminal_body() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let record = terminal_analysis("anl_01983c20-0180-7a80-a001-000000000001");
    store
        .save_analysis_atomic(&record, &[])
        .expect("terminal save commits");

    // Repeat a later non-terminal observation to prove idempotent terminal
    // dominance regardless of arrival order.
    let snapshot = ObservationSnapshot {
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Terminal,
        result_json: None,
        error_json: None,
        upstream_version: None,
        completed_at: None,
        search_input_text: Some("authored".to_owned()),
        search_filename: None,
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
    store
        .update_observation_snapshot(
            &AnalysisId::from_str("anl_01983c20-0180-7a80-a001-000000000001").unwrap(),
            timestamp("2026-08-02T09:01:00Z"),
            &snapshot,
        )
        .expect("repeated refresh commits");

    let stored = store
        .get_analysis(&AnalysisId::from_str("anl_01983c20-0180-7a80-a001-000000000001").unwrap())
        .expect("read back");
    assert_eq!(stored.status, AnalysisStatus::Succeeded);
    assert_eq!(stored.submission_outcome, SubmissionOutcome::Terminal);
    assert_eq!(
        stored.result_json.as_deref(),
        Some(ai_result("Human-written").as_str()),
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

#[test]
fn observation_mutations_reject_corrupt_aggregates_without_repair() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let id_text = "anl_01983c20-0180-7a80-a001-000000000003";
    let id = AnalysisId::from_str(id_text).expect("analysis id");
    let record = terminal_analysis(id_text);
    store
        .save_analysis_atomic(&record, &[])
        .expect("terminal save commits");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analysis_checks SET result_json = '{' WHERE analysis_id = ?1",
                [id.to_string()],
            )
        })
        .expect("raw connection")
        .expect("install malformed check");
    let before = raw_state(&store, id);
    let snapshot = ObservationSnapshot {
        status: AnalysisStatus::Failed,
        submission_outcome: SubmissionOutcome::Terminal,
        result_json: None,
        error_json: Some(canonical_error(
            ErrorCode::UpstreamAnalysisFailed,
            "Replacement must not commit.",
        )),
        upstream_version: Some("4.1".to_owned()),
        completed_at: Some(timestamp("2026-08-02T09:00:00Z")),
        search_input_text: None,
        search_filename: None,
        search_headline: None,
        search_source_urls: None,
    };

    let error = store
        .update_observation_snapshot(&id, timestamp("2026-08-02T09:00:00Z"), &snapshot)
        .expect_err("snapshot must not repair malformed checks");
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    assert_eq!(raw_state(&store, id), before);

    let error = store
        .record_observation(&observation(
            id_text,
            CheckKind::AiDetection,
            "task-must-not-commit",
            Some("STAGE_COMPLETE"),
        ))
        .expect_err("task upsert must not bypass aggregate corruption");
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    assert_eq!(raw_state(&store, id), before);
}

#[test]
fn record_observation_preserves_owned_task_identities_before_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let id_text = "anl_01983c20-0180-7a80-a001-000000000004";
    let id = AnalysisId::from_str(id_text).expect("analysis id");
    let mut record = terminal_analysis(id_text);
    record.search_source_urls = Some("https://example.invalid/source".to_owned());
    let checks = [
        StoredCheck {
            analysis_id: id,
            check_index: 0,
            check_kind: CheckKind::AiDetection,
            status: CheckStatus::Succeeded,
            result_json: record.result_json.clone(),
            error_json: None,
        },
        StoredCheck {
            analysis_id: id,
            check_index: 1,
            check_kind: CheckKind::Plagiarism,
            status: CheckStatus::Succeeded,
            result_json: Some(
                serde_json::json!({
                    "plagiarism_detected": true,
                    "total_sentences": 1,
                    "plagiarized_sentence_count": 1,
                    "percent_plagiarized": 100.0,
                    "matches": [{
                        "source_url": "https://example.invalid/source",
                        "matched_text": "Synthetic match",
                        "similarity_score": 1.0
                    }]
                })
                .to_string(),
            ),
            error_json: None,
        },
    ];
    store
        .save_analysis_complete(
            &record,
            &checks,
            &[observation(
                id_text,
                CheckKind::AiDetection,
                "task-owned-ai",
                Some("STAGE_COMPLETE"),
            )],
        )
        .expect("seed aggregate");

    let before = raw_state(&store, id);
    let error = store
        .record_observation(&StoredUpstreamTask {
            analysis_id: id,
            check_kind: CheckKind::AiDetection,
            upstream_task_id: "task-replacement-ai".to_owned(),
            last_stage: Some("STAGE_REPLACEMENT".to_owned()),
            observed_at: timestamp("2026-08-02T10:00:00Z"),
        })
        .expect_err("a different task ID cannot replace owned evidence");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);
    assert_eq!(
        raw_state(&store, id),
        before,
        "the identity conflict fails before any SQLite mutation"
    );

    store
        .record_observation(&StoredUpstreamTask {
            analysis_id: id,
            check_kind: CheckKind::AiDetection,
            upstream_task_id: "task-owned-ai".to_owned(),
            last_stage: Some("STAGE_REFRESHED".to_owned()),
            observed_at: timestamp("2026-08-02T10:01:00Z"),
        })
        .expect("the same task identity remains refreshable");
    store
        .record_observation(&StoredUpstreamTask {
            analysis_id: id,
            check_kind: CheckKind::Plagiarism,
            upstream_task_id: "task-new-plagiarism".to_owned(),
            last_stage: Some("STAGE_COMPLETE".to_owned()),
            observed_at: timestamp("2026-08-02T10:02:00Z"),
        })
        .expect("an unowned check kind may add its first task identity");

    let tasks = store
        .with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT check_kind, upstream_task_id, last_stage
                 FROM upstream_tasks WHERE analysis_id = ?1 ORDER BY check_kind",
            )?;
            let rows = statement
                .query_map([id.to_string()], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, rusqlite::Error>(rows)
        })
        .expect("borrow database")
        .expect("read task evidence");
    assert_eq!(
        tasks,
        vec![
            (
                "ai_detection".to_owned(),
                "task-owned-ai".to_owned(),
                Some("STAGE_REFRESHED".to_owned()),
            ),
            (
                "plagiarism".to_owned(),
                "task-new-plagiarism".to_owned(),
                Some("STAGE_COMPLETE".to_owned()),
            ),
        ]
    );
}

/// Result-owned search metadata is observation state, not durable local
/// authorship. A newer terminal observation replaces the headline from its
/// authoritative result, while input text and filename remain first-recorded
/// local evidence.
#[test]
fn terminal_refresh_replaces_result_search_metadata_but_preserves_input_authorship() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let record = terminal_analysis("anl_01983c20-0180-7a80-a001-000000000002");
    store
        .save_analysis_atomic(&record, &[])
        .expect("terminal save commits");

    let snapshot = ObservationSnapshot {
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        result_json: Some(ai_result("Refreshed result")),
        error_json: None,
        upstream_version: Some("4.1".to_owned()),
        completed_at: Some(timestamp("2026-08-02T09:00:00Z")),
        // A remote observation cannot replace locally held input authorship.
        search_input_text: Some("remote replacement must not win".to_owned()),
        search_filename: None,
        search_headline: Some("Refreshed result".to_owned()),
        search_source_urls: None,
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
    assert_eq!(stored.search_filename, None);
    assert_eq!(stored.search_headline.as_deref(), Some("Refreshed result"));
    assert_eq!(stored.search_source_urls, None);
    assert_eq!(store.search("Refreshed", 10).unwrap().len(), 1);
    assert_eq!(store.search("old", 10).unwrap().len(), 0);
    assert_eq!(stored.upstream_version.as_deref(), Some("4.1"));

    let absent_metadata = ObservationSnapshot {
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        result_json: Some(ai_result("No replacement metadata")),
        error_json: None,
        upstream_version: None,
        completed_at: Some(timestamp("2026-08-02T10:00:00Z")),
        search_input_text: None,
        search_filename: None,
        search_headline: Some("No replacement metadata".to_owned()),
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
    assert_eq!(preserved.search_filename, None);
    assert_eq!(
        preserved.search_headline.as_deref(),
        Some("No replacement metadata")
    );
    assert_eq!(preserved.search_source_urls, None);
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
    failed_child.search_headline = None;
    let canonical_error = canonical_error(
        ErrorCode::UpstreamAnalysisFailed,
        "Upstream analysis failed.",
    );
    failed_child.error_json = Some(canonical_error.clone());
    store
        .reconcile_bulk_collection_complete(&collection, &[prepared_child(failed_child)])
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
        .reconcile_bulk_collection_complete(&collection, &[prepared_child(running_child)])
        .expect("refresh commits");

    let members = store
        .list_bulk_analyses(&microck_pangram_cli::domain::BulkId::from_str(collection_id).unwrap())
        .expect("list members");
    assert_eq!(members.len(), 1, "one reconciled child");
    assert_eq!(members[0].status, AnalysisStatus::Failed);
    assert_eq!(
        members[0].error_json.as_deref(),
        Some(canonical_error.as_str()),
        "a non-terminal refresh never blanks the recorded error"
    );
    assert_eq!(
        members[0].completed_at,
        Some(timestamp("2026-08-01T10:05:00Z")),
        "the recorded terminal stamp survives"
    );
    assert_eq!(members[0].upstream_version.as_deref(), Some("4.0"));
}

#[test]
fn concurrent_terminal_and_bodyless_refreshes_converge_on_terminal_state() {
    let root = tempfile::tempdir().unwrap();
    let id = AnalysisId::from_str("anl_01983c20-0180-7a80-a001-0000000000c1").unwrap();
    let mut initial = terminal_analysis(id.to_string().as_str());
    initial.status = AnalysisStatus::Running;
    initial.submission_outcome = SubmissionOutcome::Accepted;
    initial.result_json = None;
    initial.upstream_version = None;
    initial.completed_at = None;
    initial.search_headline = None;
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    store
        .save_analysis_atomic(
            &initial,
            &[observation(
                id.to_string().as_str(),
                CheckKind::AiDetection,
                "task-concurrent-terminal",
                Some("STAGE_RUNNING"),
            )],
        )
        .expect("save running row");
    drop(store);

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let run = |snapshot: ObservationSnapshot, barrier: std::sync::Arc<std::sync::Barrier>| {
        let root = root.path().to_owned();
        std::thread::spawn(move || {
            let mut store = HistoryStore::open(&root).expect("open concurrent store");
            barrier.wait();
            store
                .update_observation_snapshot(&id, timestamp("2026-08-02T09:00:00Z"), &snapshot)
                .expect("concurrent refresh");
        })
    };
    let terminal = run(
        ObservationSnapshot {
            status: AnalysisStatus::Succeeded,
            submission_outcome: SubmissionOutcome::Accepted,
            result_json: Some(ai_result("Terminal wins")),
            error_json: None,
            upstream_version: Some("4.1".to_owned()),
            completed_at: Some(timestamp("2026-08-02T09:00:00Z")),
            search_input_text: None,
            search_filename: None,
            search_headline: Some("Terminal wins".to_owned()),
            search_source_urls: None,
        },
        std::sync::Arc::clone(&barrier),
    );
    let bodyless = run(
        ObservationSnapshot {
            status: AnalysisStatus::Running,
            submission_outcome: SubmissionOutcome::Accepted,
            result_json: None,
            error_json: None,
            upstream_version: None,
            completed_at: None,
            search_input_text: None,
            search_filename: None,
            search_headline: None,
            search_source_urls: None,
        },
        std::sync::Arc::clone(&barrier),
    );
    barrier.wait();
    terminal.join().unwrap();
    bodyless.join().unwrap();

    let store = HistoryStore::open(root.path()).expect("reopen history");
    let stored = store.get_analysis(&id).expect("read converged row");
    assert_eq!(stored.status, AnalysisStatus::Succeeded);
    assert_eq!(
        stored.result_json.as_deref(),
        Some(ai_result("Terminal wins").as_str())
    );
    assert_eq!(stored.completed_at, Some(timestamp("2026-08-02T09:00:00Z")));
    assert_eq!(stored.upstream_version.as_deref(), Some("4.1"));
    assert_eq!(stored.search_headline.as_deref(), Some("Terminal wins"));
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
    ai2.result_json = Some(
        serde_json::json!({
            "plagiarism_detected": false,
            "total_sentences": 1,
            "plagiarized_sentence_count": 0,
            "percent_plagiarized": 0.0,
            "matches": []
        })
        .to_string(),
    );
    ai2.search_headline = None;
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
