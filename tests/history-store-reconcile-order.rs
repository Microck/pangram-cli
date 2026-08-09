//! Phase 4 Packet C remediation: order-independent task/bulk reconciliation
//! against the real SQLite store (docs/history-contract.md task-first/bulk-second rule).
//!
//! A standalone `task status` observation of one task identity may already own the stored row a
//! later bulk read attests for one of its children (and vice versa). The store resolves each candidate child by BOTH its
//! `(bulk_id, bulk_index)` membership AND its attested `(check_kind, upstream_task_id)` keys inside the one immediate
//! transaction, reusing the one existing durable row when they agree and
//! failing closed when they conflict. No duplicate analysis or observation
//! row ever persists, and an adopted row keeps its first-recorded identity,
//! authorship, save state, local input/FTS payload, and creation time.
//!
//! No mocks anywhere: every `HistoryStore` points at a real
//! `tempfile::TempDir` database and every assertion reads committed catalog
//! state back through a plain rusqlite handle.

#![forbid(unsafe_code)]

use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, BulkCounters, BulkId, CheckKind, SaveState, Sha256Hash,
    SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryError, HistoryErrorCode, HistoryStore, InputKind, ObservationSnapshot, StoredAnalysis,
    StoredBulkCollection, StoredUpstreamTask,
};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("test timestamp")
}

/// One standalone (local-top) stored analysis projection, as a task read
/// would build it: no membership link, its own fresh identity, and the
/// locally held input/search payload an automatic save records.
fn standalone(id: &str, input: &str) -> StoredAnalysis {
    let input_sha256 = Sha256Hash::digest(input);
    StoredAnalysis {
        id: AnalysisId::from_str(id).expect("analysis id"),
        bulk: None,
        caller_id: None,
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        save_state: SaveState::SavedHistory,
        input_kind: InputKind::Text,
        input_sha256,
        display_name: None,
        input_json: serde_json::json!({
            "type": "text",
            "origin": "literal",
            "sha256": input_sha256,
            "byte_count": input.len(),
            "word_count": input.split_whitespace().count(),
            "text": input
        })
        .to_string(),
        result_json: Some(
            serde_json::json!({
                "classification": "human",
                "headline": "Human-written",
                "prediction": "Human",
                "fraction_ai": 0.0,
                "fraction_ai_assisted": 0.0,
                "fraction_human": 1.0,
                "num_ai_segments": 0,
                "num_ai_assisted_segments": 0,
                "num_human_segments": 1,
                "segments": []
            })
            .to_string(),
        ),
        error_json: None,
        upstream_version: None,
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(timestamp("2026-08-01T09:59:00Z")),
        created_at: timestamp("2026-08-01T10:00:00Z"),
        updated_at: timestamp("2026-08-01T10:05:00Z"),
        completed_at: Some(timestamp("2026-08-01T10:05:00Z")),
        search_input_text: Some(input.to_owned()),
        search_filename: None,
        search_headline: Some("Human-written".to_owned()),
        search_source_urls: None,
    }
}

fn observation(analysis_id: &str, task: &str) -> StoredUpstreamTask {
    StoredUpstreamTask {
        analysis_id: AnalysisId::from_str(analysis_id).expect("analysis id"),
        check_kind: CheckKind::AiDetection,
        upstream_task_id: task.to_owned(),
        last_stage: Some("STAGE_SUCCESS".to_owned()),
        observed_at: timestamp("2026-08-01T10:05:00Z"),
    }
}

fn collection(id: &str, upstream: &str) -> StoredBulkCollection {
    StoredBulkCollection {
        id: BulkId::from_str(id).expect("bulk id"),
        upstream_bulk_id: Some(upstream.to_owned()),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        counters: BulkCounters::new(1, 1, 0, 0).expect("counters"),
        estimated_billable_units: Some(1),
        created_at: timestamp("2026-08-01T09:00:00Z"),
        updated_at: timestamp("2026-08-01T09:00:00Z"),
        completed_at: None,
    }
}

/// One bulk-child projection as the bulk read builds it: a fresh identity,
/// a provisional membership link (unoccupied input, no local search text of
/// its own beyond its item), and the accepted-queued observation state.
fn child(id: &str, bulk_id: &str, index: i64) -> StoredAnalysis {
    let mut record = standalone(id, &format!("bulk item {index}"));
    record.bulk = Some((BulkId::from_str(bulk_id).expect("bulk id"), index));
    record.caller_id = Some(format!("row-{index:03}"));
    record.submission_outcome = SubmissionOutcome::Accepted;
    record.status = AnalysisStatus::Running;
    record.result_json = None;
    record.search_headline = None;
    record.completed_at = None;
    record.updated_at = timestamp("2026-08-01T10:06:00Z");
    record
}

/// The adapter's content-pure merge closure: a non-terminal refresh carries
/// no body and keeps the stored row's authorship.
fn running_merge(prior: &StoredAnalysis) -> Result<ObservationSnapshot, HistoryError> {
    Ok(ObservationSnapshot {
        status: AnalysisStatus::Running,
        submission_outcome: prior.submission_outcome,
        result_json: None,
        error_json: None,
        upstream_version: None,
        completed_at: None,
        search_input_text: None,
        search_filename: None,
        search_headline: None,
        search_source_urls: None,
    })
}

fn database(root: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(root.path().join("history").join("pangram-history.db"))
        .expect("open saved database")
}

fn count(connection: &rusqlite::Connection, sql: &str) -> i64 {
    connection
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .expect("count rows")
}

/// Task-first, bulk-second: a standalone task observation already saved the
/// row; the later bulk read attests the same `(check_kind, upstream_task_id)`
/// key for the child at membership `(bulk, 0)`. The bulk reconcile must
/// adopt the one existing durable row in place: no second analysis inserts,
/// no UNIQUE `(check_kind, upstream_task_id)` collision rolls the batch
/// back, the adoption adds the membership link, and the adopted row keeps
/// its original identity, authorship, save state, local input/FTS payload,
/// and creation time exactly.
#[test]
fn task_first_then_bulk_adopts_the_one_existing_row() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open store");
    // 1. Standalone task observation persists (the task-first path).
    let record = standalone(
        "anl_01983c20-0180-7a80-a001-00000000d001",
        "adopted task text",
    );
    let observations = vec![observation(
        "anl_01983c20-0180-7a80-a001-00000000d001",
        "task-adopted",
    )];
    let outcome = store
        .reconcile_observed_analysis_atomic(
            &record,
            &observations,
            timestamp("2026-08-01T10:05:00Z"),
            running_merge,
        )
        .expect("standalone task save commits");
    assert!(outcome.inserted, "the first observation inserted");

    // 2. A later bulk read attests the same task identity for the child at
    // membership 0. Before this remediation the child insert collided with
    // UNIQUE (check_kind, upstream_task_id) and rolled the batch back.
    let bulk = collection(
        "bulk_01983c20-0180-7a80-a001-00000000d011",
        "upstream-bulk-adopt",
    );
    let child_record = child(
        "anl_01983c20-0180-7a80-a001-00000000d0c1",
        "bulk_01983c20-0180-7a80-a001-00000000d011",
        0,
    );
    let child_observations = vec![observation(
        "anl_01983c20-0180-7a80-a001-00000000d0c1",
        "task-adopted",
    )];
    let bulk_outcome = store
        .reconcile_bulk_collection_atomic(&bulk, &[(child_record, child_observations)])
        .expect("the bulk reconcile adopts instead of rolling back");

    let connection = database(&root);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM analyses"),
        1,
        "no duplicate analysis row: the bulk child adopted the existing one"
    );
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM upstream_tasks"), 1);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM analysis_search"),
        1,
        "exactly one FTS payload survives"
    );

    // The adopted row keeps its first-recorded identity and authorship:
    // the original `anl_` id, its locally held input text and headline,
    // its `saved_history` state, and its original creation time. The
    // membership link landed on it.
    let (id, bulk_id, bulk_index, save_state, created_at): (
        String,
        Option<String>,
        Option<i64>,
        String,
        String,
    ) = connection
        .query_row(
            "SELECT id, bulk_id, bulk_index, save_state, created_at FROM analyses",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("the one stored row");
    assert_eq!(id, outcome.stored_id.to_string());
    assert_eq!(id, "anl_01983c20-0180-7a80-a001-00000000d001");
    let expected_bulk = bulk_outcome.stored_id.to_string();
    assert_eq!(bulk_id.as_deref(), Some(expected_bulk.as_str()));
    assert_eq!(bulk_index, Some(0));
    assert_eq!(save_state, "saved_history");
    assert_eq!(created_at, "2026-08-01T10:00:00Z");
    let (input_text, headline): (Option<String>, Option<String>) = connection
        .query_row(
            "SELECT input_text, headline FROM analysis_search",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("search payload");
    assert_eq!(
        input_text.as_deref(),
        Some("adopted task text"),
        "the locally held input text survived adoption"
    );
    assert_eq!(
        headline.as_deref(),
        Some("Human-written"),
        "the first-recorded headline survived adoption"
    );
    // The observation row attests the real upstream identity against the
    // one adopted analysis.
    let (task_analysis, task_id): (String, String) = connection
        .query_row(
            "SELECT analysis_id, upstream_task_id FROM upstream_tasks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("task row");
    assert_eq!(task_analysis, outcome.stored_id.to_string());
    assert_eq!(task_id, "task-adopted");
}

/// Bulk-first, task-second (reverse order preserved): the bulk acceptance
/// attested no task identity for its child, so the bulk save inserted the
/// child at its membership without an observation row; a later bulk status
/// read attested the task identity and attached it to that same row; and a
/// still-later standalone task read of the attested identity reuses the
/// same membership row: no duplicate analysis inserts, the task
/// observation rebinds onto the membership row, and the row keeps its
/// authorship, save state, local input, and creation time.
#[test]
fn bulk_first_then_task_leads_the_one_membership_row() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open store");

    // 1. Bulk acceptance persists a child at membership 0 with NO attested
    // task identity (immediate acceptance carried no task id).
    let bulk = collection(
        "bulk_01983c20-0180-7a80-a001-00000000d021",
        "upstream-bulk-lead",
    );
    let mut child_record = child(
        "anl_01983c20-0180-7a80-a001-00000000d0d1",
        "bulk_01983c20-0180-7a80-a001-00000000d021",
        0,
    );
    child_record.search_input_text = Some("bulk item 0".to_owned());
    store
        .reconcile_bulk_collection_atomic(&bulk, &[(child_record, Vec::new())])
        .expect("the bulk acceptance commits");
    let connection = database(&root);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM analyses"), 1);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM upstream_tasks"), 0);
    drop(connection);

    // 2. A later bulk status read attests the task identity for the same
    // membership: the child row is reused in place and gains its
    // observation row (no duplicate).
    let mut attested = child(
        "anl_01983c20-0180-7a80-a001-00000000d0d2",
        "bulk_01983c20-0180-7a80-a001-00000000d021",
        0,
    );
    attested.search_input_text = None; // a body-less observation refresh
    attested.search_headline = None;
    let attested_observations = vec![observation(
        "anl_01983c20-0180-7a80-a001-00000000d0d2",
        "task-lead",
    )];
    store
        .reconcile_bulk_collection_atomic(&bulk, &[(attested, attested_observations)])
        .expect("the bulk status read commits");
    let connection = database(&root);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM analyses"), 1);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM upstream_tasks"), 1);
    drop(connection);

    // 3. The standalone task read of that attested identity reconciles:
    // its fresh record carries no membership (the adapter minted a fresh
    // local identity for the read); the stored row it resolves is the
    // membership child, so no duplicate inserts.
    let read = standalone(
        "anl_01983c20-0180-7a80-a001-00000000d002",
        "read payload the store must not persist",
    );
    // The read carries no locally held search content of its own that
    // should displace the child's: a body-less observation.
    let read = StoredAnalysis {
        search_input_text: None,
        search_headline: None,
        search_filename: None,
        search_source_urls: None,
        ..read
    };
    let read_observations = vec![observation(
        "anl_01983c20-0180-7a80-a001-00000000d002",
        "task-lead",
    )];
    let outcome = store
        .reconcile_observed_analysis_atomic(
            &read,
            &read_observations,
            timestamp("2026-08-01T10:07:00Z"),
            running_merge,
        )
        .expect("the task read reconciles");

    let connection = database(&root);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM analyses"),
        1,
        "the task read led the existing membership row instead of inserting"
    );
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM upstream_tasks"), 1);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM analysis_search"),
        1
    );
    // The led row IS the membership child: its identity, authorship, save
    // state, membership, local input text, and creation time are intact.
    let (id, bulk_id, bulk_index, save_state, created_at): (
        String,
        Option<String>,
        Option<i64>,
        String,
        String,
    ) = connection
        .query_row(
            "SELECT id, bulk_id, bulk_index, save_state, created_at FROM analyses",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("the one stored row");
    assert_eq!(id, outcome.stored_id.to_string());
    // The led row is the original membership child, not a new row minted
    // for the read.
    assert_eq!(id, "anl_01983c20-0180-7a80-a001-00000000d0d1");
    assert_eq!(
        bulk_id.as_deref(),
        Some("bulk_01983c20-0180-7a80-a001-00000000d021")
    );
    assert_eq!(bulk_index, Some(0));
    assert_eq!(save_state, "saved_history");
    assert_eq!(created_at, "2026-08-01T10:00:00Z");
    let input_text: Option<String> = connection
        .query_row("SELECT input_text FROM analysis_search", [], |row| {
            row.get(0)
        })
        .expect("search payload");
    assert_eq!(input_text.as_deref(), Some("bulk item 0"));
    // Its observation attests the task identity against the led row.
    let (task_analysis, task_id): (String, String) = connection
        .query_row(
            "SELECT analysis_id, upstream_task_id FROM upstream_tasks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("task row");
    assert_eq!(task_analysis, outcome.stored_id.to_string());
    assert_eq!(task_id, "task-lead");
}

/// A standalone task read attesting two task identities that resolve two
/// DIFFERENT stored rows fails closed (order-independence conflict rule):
/// the whole reconcile rolls back, the error is `history_write_failed`,
/// and neither pre-existing row is touched.
#[test]
fn a_task_read_with_conflicting_task_keys_rolls_back() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open store");
    for (id, task) in [
        ("anl_01983c20-0180-7a80-a001-00000000d008", "task-one"),
        ("anl_01983c20-0180-7a80-a001-00000000d009", "task-two"),
    ] {
        let record = standalone(id, "conflict read text");
        store
            .reconcile_observed_analysis_atomic(
                &record,
                &[observation(id, task)],
                timestamp("2026-08-01T10:05:00Z"),
                running_merge,
            )
            .expect("standalone saves commit");
    }

    // One read attesting BOTH identities must not pick one silently.
    let read = standalone(
        "anl_01983c20-0180-7a80-a001-00000000d00a",
        "conflicting read text",
    );
    let conflict = vec![
        observation("anl_01983c20-0180-7a80-a001-00000000d00a", "task-one"),
        observation("anl_01983c20-0180-7a80-a001-00000000d00a", "task-two"),
    ];
    let error = store
        .reconcile_observed_analysis_atomic(
            &read,
            &conflict,
            timestamp("2026-08-01T10:07:00Z"),
            running_merge,
        )
        .expect_err("the conflicting read must fail closed");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    let connection = database(&root);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM analyses"), 2);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM upstream_tasks"), 2);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM analysis_search"),
        2
    );
}

/// Conflicting task identities fail closed atomically: the standalone row
/// already attests `task-a`, and the bulk child at an unoccupied membership
/// attests BOTH `task-a` and `task-b`, where `task-b` already belongs to a
/// DIFFERENT stored row. The candidates disagree (two task keys resolve two
/// different rows), so the whole bulk batch (collection, child, and
/// observation rows) rolls back: the error is `history_write_failed` and
/// neither stored row is deleted, merged, or rekeyed.
#[test]
fn conflicting_task_identities_roll_the_whole_bulk_batch_back() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open store");

    // Two independent standalone rows, each owning one task identity.
    for (id, task) in [
        ("anl_01983c20-0180-7a80-a001-00000000d003", "task-a"),
        ("anl_01983c20-0180-7a80-a001-00000000d004", "task-b"),
    ] {
        let record = standalone(id, "conflict text");
        let observations = vec![observation(id, task)];
        store
            .reconcile_observed_analysis_atomic(
                &record,
                &observations,
                timestamp("2026-08-01T10:05:00Z"),
                running_merge,
            )
            .expect("standalone saves commit");
    }

    // A bulk child attesting BOTH identities at once is contradictory.
    let bulk = collection(
        "bulk_01983c20-0180-7a80-a001-00000000d031",
        "upstream-bulk-conflict",
    );
    let child_id = "anl_01983c20-0180-7a80-a001-00000000d0e1";
    let child_record = child(child_id, "bulk_01983c20-0180-7a80-a001-00000000d031", 0);
    let child_observations = vec![
        observation(child_id, "task-a"),
        observation(child_id, "task-b"),
    ];
    let error = store
        .reconcile_bulk_collection_atomic(&bulk, &[(child_record, child_observations)])
        .expect_err("the conflicting batch must fail closed");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    // Atomic rollback: no collection row, no child row, no observation row
    // for the bulk read; the two pre-existing standalone rows are intact.
    let connection = database(&root);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM bulk_collections"),
        0,
        "the collection row rolled back with its batch"
    );
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM analyses"), 2);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM upstream_tasks"), 2);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM analysis_search"),
        2
    );
    let mut tasks: Vec<String> = connection
        .prepare("SELECT upstream_task_id FROM upstream_tasks ORDER BY upstream_task_id")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    tasks.sort();
    assert_eq!(tasks, vec!["task-a".to_owned(), "task-b".to_owned()]);
}

/// Occupied membership fails closed: the membership `(bulk, 0)` already
/// belongs to one stored child (attesting `task-held`), and a later bulk
/// refresh of the same membership attests a DIFFERENT, overlapping task
/// identity `task-held` plus `task-new` whose key already belongs to
/// another row. The membership holder's attested set overlaps but differs,
/// so the batch rolls back: no rekey, no annex, no duplicate.
#[test]
fn an_occupied_membership_with_a_conflicting_task_set_rolls_back() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open store");

    // Membership 0 is already held by a child attesting task-held.
    let bulk = collection(
        "bulk_01983c20-0180-7a80-a001-00000000d041",
        "upstream-bulk-occupied",
    );
    let holder = child(
        "anl_01983c20-0180-7a80-a001-00000000d051",
        "bulk_01983c20-0180-7a80-a001-00000000d041",
        0,
    );
    let holder_observations = vec![observation(
        "anl_01983c20-0180-7a80-a001-00000000d051",
        "task-held",
    )];
    store
        .reconcile_bulk_collection_atomic(&bulk, &[(holder, holder_observations)])
        .expect("the first bulk save commits");

    // A standalone row owns task-new.
    let standalone_id = "anl_01983c20-0180-7a80-a001-00000000d005";
    let record = standalone(standalone_id, "occupied text");
    store
        .reconcile_observed_analysis_atomic(
            &record,
            &[observation(standalone_id, "task-new")],
            timestamp("2026-08-01T10:05:00Z"),
            running_merge,
        )
        .expect("the standalone save commits");

    // A refresh of the same membership now attests task-held AND task-new:
    // the overlapping-but-different set fails closed.
    let refreshed = child(
        "anl_01983c20-0180-7a80-a001-00000000d0f1",
        "bulk_01983c20-0180-7a80-a001-00000000d041",
        0,
    );
    let refreshed_observations = vec![
        observation("anl_01983c20-0180-7a80-a001-00000000d0f1", "task-held"),
        observation("anl_01983c20-0180-7a80-a001-00000000d0f1", "task-new"),
    ];
    let error = store
        .reconcile_bulk_collection_atomic(&bulk, &[(refreshed, refreshed_observations)])
        .expect_err("the occupied-membership conflict must fail closed");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    let connection = database(&root);
    // The holder and the standalone row are intact; nothing duplicated or
    // rekeyed.
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM analyses"), 2);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM upstream_tasks"), 2);
    let membership: (String, i64) = connection
        .query_row(
            "SELECT id, bulk_index FROM analyses WHERE bulk_index IS NOT NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("membership row");
    assert_eq!(
        membership.0, "anl_01983c20-0180-7a80-a001-00000000d051",
        "the membership holder kept its identity"
    );
    assert_eq!(membership.1, 0);
}

/// A membership holder cannot silently replace one task identity with
/// another identity of the same check kind. The exact keys are disjoint,
/// but their `ai_detection` slot overlaps semantically; accepting the
/// refresh would overwrite the holder's primary-keyed observation row.
#[test]
fn an_occupied_membership_cannot_replace_a_task_of_the_same_kind() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open store");
    let bulk = collection(
        "bulk_01983c20-0180-7a80-a001-00000000d081",
        "upstream-bulk-replace",
    );
    let holder = child(
        "anl_01983c20-0180-7a80-a001-00000000d082",
        "bulk_01983c20-0180-7a80-a001-00000000d081",
        0,
    );
    store
        .reconcile_bulk_collection_atomic(
            &bulk,
            &[(
                holder,
                vec![observation(
                    "anl_01983c20-0180-7a80-a001-00000000d082",
                    "task-original",
                )],
            )],
        )
        .expect("first membership save commits");

    let replacement = child(
        "anl_01983c20-0180-7a80-a001-00000000d083",
        "bulk_01983c20-0180-7a80-a001-00000000d081",
        0,
    );
    let error = store
        .reconcile_bulk_collection_atomic(
            &bulk,
            &[(
                replacement,
                vec![observation(
                    "anl_01983c20-0180-7a80-a001-00000000d083",
                    "task-replacement",
                )],
            )],
        )
        .expect_err("same-kind replacement must fail closed");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    let connection = database(&root);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM analyses"), 1);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM upstream_tasks"), 1);
    let task: String = connection
        .query_row("SELECT upstream_task_id FROM upstream_tasks", [], |row| {
            row.get(0)
        })
        .expect("original task survives");
    assert_eq!(task, "task-original");
}

/// One candidate cannot attest two different upstream IDs for the same
/// check kind, even when neither key exists yet. Such a pair cannot coexist
/// under the `(analysis_id, check_kind)` primary key and must fail before
/// inserting any durable row.
#[test]
fn one_read_with_two_unresolved_ids_for_one_kind_rolls_back() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open store");
    let id = "anl_01983c20-0180-7a80-a001-00000000d084";
    let record = standalone(id, "internally conflicting observation");
    let error = store
        .reconcile_observed_analysis_atomic(
            &record,
            &[
                observation(id, "task-first-unresolved"),
                observation(id, "task-second-unresolved"),
            ],
            timestamp("2026-08-01T10:08:00Z"),
            running_merge,
        )
        .expect_err("one check kind cannot attest two task ids");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    let connection = database(&root);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM analyses"), 0);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM upstream_tasks"), 0);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM analysis_search"),
        0
    );
}

/// Same-key concurrent task reconciles from two independent store handles
/// (real OS threads over the one real WAL database) converge on exactly one
/// row even when one contender's record is the one a bulk read would later
/// adopt: the schema-enforced `(check_kind, upstream_task_id)` uniqueness
/// plus the immediate-transaction resolution serialize the pair, and the
/// loser refreshes the winner's row.
#[test]
fn same_key_concurrency_converges_on_one_row_before_a_bulk_read() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().to_path_buf();
    let store_a = HistoryStore::open(&path).expect("open first store");
    let store_b = HistoryStore::open(&path).expect("open second store");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

    let run = |mut store: HistoryStore, id: &'static str| {
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let record = standalone(id, "concurrent adopted text");
            let observations = vec![observation(id, "task-race")];
            barrier.wait();
            store
                .reconcile_observed_analysis_atomic(
                    &record,
                    &observations,
                    timestamp("2026-08-01T10:06:00Z"),
                    running_merge,
                )
                .expect("reconcile commits")
        })
    };
    let first = run(store_a, "anl_01983c20-0180-7a80-a001-00000000d006");
    let second = run(store_b, "anl_01983c20-0180-7a80-a001-00000000d007");
    let outcome_a = first.join().expect("first joins");
    let outcome_b = second.join().expect("second joins");
    assert_eq!(
        [outcome_a.inserted, outcome_b.inserted]
            .into_iter()
            .filter(|flag| *flag)
            .count(),
        1,
        "exactly one inserted; the racing peer refreshed"
    );
    assert_eq!(outcome_a.stored_id, outcome_b.stored_id);

    // The later bulk read attests the raced task identity for its child:
    // it adopts whichever identity the race committed, still exactly one
    // row, still preserving the race winner's authorship.
    let mut store = HistoryStore::open(&path).expect("reopen store");
    let bulk = collection(
        "bulk_01983c20-0180-7a80-a001-00000000d061",
        "upstream-bulk-race",
    );
    let child_record = child(
        "anl_01983c20-0180-7a80-a001-00000000d071",
        "bulk_01983c20-0180-7a80-a001-00000000d061",
        0,
    );
    let child_observations = vec![observation(
        "anl_01983c20-0180-7a80-a001-00000000d071",
        "task-race",
    )];
    store
        .reconcile_bulk_collection_atomic(&bulk, &[(child_record, child_observations)])
        .expect("the bulk read adopts the race winner");

    let connection = database(&root);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM analyses"), 1);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM upstream_tasks"), 1);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM analysis_search"),
        1
    );
    let (task_analysis, task_id): (String, String) = connection
        .query_row(
            "SELECT analysis_id, upstream_task_id FROM upstream_tasks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("task row");
    assert_eq!(task_analysis, outcome_a.stored_id.to_string());
    assert_eq!(task_id, "task-race");
}
