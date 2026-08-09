//! Final Packet C reconciliation regressions against real SQLite.
//!
//! These tests lock fail-closed membership adoption, evidence-only
//! task/bulk correlation, and terminal-body merge behavior at each store
//! reconciliation path. No mock store or protocol seam is used.

#![forbid(unsafe_code)]

#[path = "support/history_store.rs"]
mod history_store_support;

use std::str::FromStr;

use history_store_support::{ai_result, canonical_error};
use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, BulkCounters, BulkId, CheckKind, SaveState, Sha256Hash,
    SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryError, HistoryErrorCode, HistoryStore, InputKind, ObservationSnapshot, StoredAnalysis,
    StoredBulkCollection, StoredUpstreamTask,
};
use microck_pangram_cli::output::ErrorCode;

fn stamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("test timestamp")
}

fn analysis(id: &str, result: Option<&str>, error: Option<&str>) -> StoredAnalysis {
    let input_sha256 = Sha256Hash::digest("durable");
    let search_headline = result
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .and_then(|value| value["headline"].as_str().map(str::to_owned));
    StoredAnalysis {
        id: AnalysisId::from_str(id).expect("analysis id"),
        bulk: None,
        caller_id: None,
        status: if error.is_some() {
            AnalysisStatus::Failed
        } else {
            AnalysisStatus::Succeeded
        },
        submission_outcome: SubmissionOutcome::Terminal,
        save_state: SaveState::SavedHistory,
        input_kind: InputKind::Text,
        input_sha256,
        display_name: None,
        input_json: serde_json::json!({
            "type": "text",
            "origin": "literal",
            "sha256": input_sha256,
            "byte_count": 7,
            "word_count": 1,
            "text": "durable"
        })
        .to_string(),
        result_json: result.map(str::to_owned),
        error_json: error.map(str::to_owned),
        upstream_version: None,
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(stamp("2026-08-01T09:59:00Z")),
        created_at: stamp("2026-08-01T10:00:00Z"),
        updated_at: stamp("2026-08-01T10:05:00Z"),
        completed_at: Some(stamp("2026-08-01T10:05:00Z")),
        search_input_text: Some("durable".to_owned()),
        search_filename: None,
        search_headline,
        search_source_urls: None,
    }
}

fn child(id: &str, bulk: &str, index: i64) -> StoredAnalysis {
    let mut row = analysis(id, None, None);
    row.bulk = Some((BulkId::from_str(bulk).expect("bulk id"), index));
    row.status = AnalysisStatus::Running;
    row.submission_outcome = SubmissionOutcome::Accepted;
    row.completed_at = None;
    row
}

fn bulk(id: &str, upstream: &str) -> StoredBulkCollection {
    StoredBulkCollection {
        id: BulkId::from_str(id).expect("bulk id"),
        upstream_bulk_id: Some(upstream.to_owned()),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        counters: BulkCounters::new(2, 2, 0, 0).expect("counters"),
        estimated_billable_units: Some(2),
        created_at: stamp("2026-08-01T09:00:00Z"),
        updated_at: stamp("2026-08-01T09:00:00Z"),
        completed_at: None,
    }
}

fn task(analysis_id: &str, task_id: &str) -> StoredUpstreamTask {
    StoredUpstreamTask {
        analysis_id: AnalysisId::from_str(analysis_id).expect("analysis id"),
        check_kind: CheckKind::AiDetection,
        upstream_task_id: task_id.to_owned(),
        last_stage: Some("STAGE_SUCCESS".to_owned()),
        observed_at: stamp("2026-08-01T10:05:00Z"),
    }
}

fn bodyless_terminal(_: &StoredAnalysis) -> Result<ObservationSnapshot, HistoryError> {
    Ok(ObservationSnapshot {
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        result_json: None,
        error_json: None,
        upstream_version: None,
        completed_at: Some(stamp("2026-08-02T10:00:00Z")),
        search_input_text: None,
        search_filename: None,
        search_headline: None,
        search_source_urls: None,
    })
}

fn database(root: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(root.path().join("history/pangram-history.db"))
        .expect("open database")
}

fn count(connection: &rusqlite::Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count")
}

fn logical_state(connection: &rusqlite::Connection) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "SELECT 'bulk|' || quote(id) || '|' || quote(upstream_bulk_id) || '|' ||
                    quote(status) || '|' || quote(submission_outcome) || '|' ||
                    quote(total_items) || '|' || quote(accepted) || '|' ||
                    quote(succeeded) || '|' || quote(failed) || '|' ||
                    quote(estimated_billable_units) || '|' || quote(created_at) || '|' ||
                    quote(updated_at) || '|' || quote(completed_at)
             FROM bulk_collections
             UNION ALL
             SELECT 'analysis|' || quote(id) || '|' || quote(bulk_id) || '|' ||
                    quote(bulk_index) || '|' || quote(caller_id) || '|' ||
                    quote(status) || '|' || quote(submission_outcome) || '|' ||
                    quote(save_state) || '|' || quote(input_type) || '|' ||
                    quote(input_sha256) || '|' || quote(display_name) || '|' ||
                    quote(input_json) || '|' || quote(result_json) || '|' ||
                    quote(error_json) || '|' || quote(upstream_version) || '|' ||
                    quote(retry_of) || '|' ||
                    quote(rerun_of) || '|' || quote(created_at) || '|' ||
                    quote(updated_at) || '|' || quote(completed_at)
             FROM analyses
             UNION ALL
             SELECT 'task|' || quote(analysis_id) || '|' || quote(check_kind) || '|' ||
                    quote(upstream_task_id) || '|' || quote(last_stage) || '|' ||
                    quote(observed_at)
             FROM upstream_tasks
             UNION ALL
             SELECT 'search|' || quote(analysis_id) || '|' || quote(input_text) || '|' ||
                    quote(filename) || '|' || quote(headline) || '|' || quote(source_urls)
             FROM analysis_search
             ORDER BY 1",
        )
        .expect("prepare logical snapshot");
    statement
        .query_map([], |row| row.get(0))
        .expect("query logical snapshot")
        .collect::<Result<_, _>>()
        .expect("read logical snapshot")
}

#[test]
fn bulk_membership_refresh_rejects_a_missing_search_row_without_mutation() {
    for (suffix, with_task) in [("091", false), ("092", true)] {
        let root = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(root.path()).expect("store");
        let bulk_id = format!("bulk_01983c20-0180-7a80-a001-00000000e{suffix}");
        let child_id = format!("anl_01983c20-0180-7a80-a001-00000000e{suffix}");
        let mut collection = bulk(&bulk_id, &format!("upstream-fts-{suffix}"));
        let observations = if with_task {
            vec![task(&child_id, &format!("task-fts-{suffix}"))]
        } else {
            Vec::new()
        };
        store
            .reconcile_bulk_collection_atomic(
                &collection,
                &[(child(&child_id, &bulk_id, 0), observations.clone())],
            )
            .expect("initial member");
        store
            .with_connection(|connection| {
                connection.execute(
                    "DELETE FROM analysis_search WHERE analysis_id = ?1",
                    [&child_id],
                )
            })
            .expect("raw connection")
            .expect("remove synchronized search row");

        let database_path = root.path().join("history/pangram-history.db");
        let before_bytes = std::fs::read(&database_path).expect("read database bytes");
        let before_state = logical_state(&database(&root));
        collection.status = AnalysisStatus::Succeeded;
        collection.updated_at = stamp("2026-08-02T12:00:00Z");
        let mut refreshed = child("anl_01983c20-0180-7a80-a001-00000000e093", &bulk_id, 0);
        refreshed.status = AnalysisStatus::Succeeded;
        refreshed.updated_at = stamp("2026-08-02T12:00:00Z");
        let refreshed_observations = if with_task {
            vec![task(
                "anl_01983c20-0180-7a80-a001-00000000e093",
                &format!("task-fts-{suffix}"),
            )]
        } else {
            Vec::new()
        };

        let error = store
            .reconcile_bulk_collection_atomic(&collection, &[(refreshed, refreshed_observations)])
            .expect_err("missing synchronized search row must fail closed");
        assert_eq!(
            error.code(),
            HistoryErrorCode::HistoryCorrupt,
            "{suffix} task={with_task}: {error:?}"
        );
        assert_eq!(
            logical_state(&database(&root)),
            before_state,
            "{suffix} task={with_task}: failed reconciliation mutated logical state"
        );
        assert_eq!(
            std::fs::read(&database_path).expect("reread database bytes"),
            before_bytes,
            "{suffix} task={with_task}: failed reconciliation mutated database bytes"
        );
    }
}

#[test]
fn bulk_membership_refresh_rejects_duplicate_or_malformed_search_state() {
    for (suffix, corruption) in [("094", "duplicate"), ("095", "malformed")] {
        let root = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(root.path()).expect("store");
        let bulk_id = format!("bulk_01983c20-0180-7a80-a001-00000000e{suffix}");
        let child_id = format!("anl_01983c20-0180-7a80-a001-00000000e{suffix}");
        let collection = bulk(&bulk_id, &format!("upstream-fts-{suffix}"));
        store
            .reconcile_bulk_collection_atomic(
                &collection,
                &[(child(&child_id, &bulk_id, 0), Vec::new())],
            )
            .expect("initial member");
        store
            .with_connection(|connection| match corruption {
                "duplicate" => connection.execute(
                    "INSERT INTO analysis_search
                        (analysis_id, input_text, filename, headline, source_urls)
                     SELECT analysis_id, input_text, filename, headline, source_urls
                     FROM analysis_search WHERE analysis_id = ?1",
                    [&child_id],
                ),
                "malformed" => connection.execute(
                    "UPDATE analysis_search SET input_text = 42 WHERE analysis_id = ?1",
                    [&child_id],
                ),
                _ => unreachable!(),
            })
            .expect("raw connection")
            .expect("corrupt synchronized search state");
        let before_state = logical_state(&database(&root));

        let error = store
            .reconcile_bulk_collection_atomic(
                &collection,
                &[(
                    child("anl_01983c20-0180-7a80-a001-00000000e096", &bulk_id, 0),
                    Vec::new(),
                )],
            )
            .expect_err("invalid synchronized search state must fail closed");
        assert_eq!(
            error.code(),
            HistoryErrorCode::HistoryCorrupt,
            "{corruption}: {error:?}"
        );
        assert_eq!(
            logical_state(&database(&root)),
            before_state,
            "{corruption}: failed reconciliation mutated logical state"
        );
    }
}

#[test]
fn task_key_adoption_cannot_move_a_row_to_another_position() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("store");
    let bulk_id = "bulk_01983c20-0180-7a80-a001-00000000e001";
    let collection = bulk(bulk_id, "upstream-position");
    let holder_id = "anl_01983c20-0180-7a80-a001-00000000e011";
    store
        .reconcile_bulk_collection_atomic(
            &collection,
            &[(
                child(holder_id, bulk_id, 0),
                vec![task(holder_id, "task-position")],
            )],
        )
        .expect("initial member");

    let incoming_id = "anl_01983c20-0180-7a80-a001-00000000e012";
    let error = store
        .reconcile_bulk_collection_atomic(
            &collection,
            &[(
                child(incoming_id, bulk_id, 1),
                vec![task(incoming_id, "task-position")],
            )],
        )
        .expect_err("a task-key row cannot move positions");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    let connection = database(&root);
    assert_eq!(count(&connection, "analyses"), 1);
    let (id, index): (String, i64) = connection
        .query_row("SELECT id, bulk_index FROM analyses", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("member");
    assert_eq!(id, holder_id);
    assert_eq!(index, 0);
}

#[test]
fn task_key_adoption_cannot_move_a_row_to_another_collection() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("store");
    let first_id = "bulk_01983c20-0180-7a80-a001-00000000e021";
    let first = bulk(first_id, "upstream-first");
    let holder_id = "anl_01983c20-0180-7a80-a001-00000000e022";
    store
        .reconcile_bulk_collection_atomic(
            &first,
            &[(
                child(holder_id, first_id, 0),
                vec![task(holder_id, "task-collection")],
            )],
        )
        .expect("initial member");

    let second_id = "bulk_01983c20-0180-7a80-a001-00000000e023";
    let second = bulk(second_id, "upstream-second");
    let incoming_id = "anl_01983c20-0180-7a80-a001-00000000e024";
    let error = store
        .reconcile_bulk_collection_atomic(
            &second,
            &[(
                child(incoming_id, second_id, 0),
                vec![task(incoming_id, "task-collection")],
            )],
        )
        .expect_err("a task-key row cannot move collections");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    let connection = database(&root);
    assert_eq!(count(&connection, "bulk_collections"), 1);
    assert_eq!(count(&connection, "analyses"), 1);
    let stored_bulk: String = connection
        .query_row("SELECT bulk_id FROM analyses", [], |row| row.get(0))
        .expect("membership");
    assert_eq!(stored_bulk, first_id);
}

#[test]
fn task_key_adoption_rejects_a_partially_populated_membership() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("store");
    let bulk_id = "bulk_01983c20-0180-7a80-a001-00000000e031";
    let collection = bulk(bulk_id, "upstream-partial");
    store.save_bulk_collection(&collection).expect("collection");
    let holder_id = "anl_01983c20-0180-7a80-a001-00000000e032";
    store
        .save_analysis_atomic(
            &analysis(holder_id, Some(&ai_result("Stored")), None),
            &[task(holder_id, "task-partial")],
        )
        .expect("standalone");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analyses SET bulk_id = ?1, bulk_index = NULL WHERE id = ?2",
                [bulk_id, holder_id],
            )
        })
        .expect("raw connection")
        .expect("partial fixture");

    let incoming_id = "anl_01983c20-0180-7a80-a001-00000000e033";
    let error = store
        .reconcile_bulk_collection_atomic(
            &collection,
            &[(
                child(incoming_id, bulk_id, 1),
                vec![task(incoming_id, "task-partial")],
            )],
        )
        .expect_err("partial membership must fail closed");
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    let connection = database(&root);
    let membership: (Option<String>, Option<i64>) = connection
        .query_row(
            "SELECT bulk_id, bulk_index FROM analyses WHERE id = ?1",
            [holder_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("partial membership");
    assert_eq!(membership, (Some(bulk_id.to_owned()), None));
}

#[test]
fn direct_task_read_does_not_fabricate_adoption_of_a_taskless_child() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("store");
    let bulk_id = "bulk_01983c20-0180-7a80-a001-00000000e041";
    let collection = bulk(bulk_id, "upstream-taskless");
    let child_id = "anl_01983c20-0180-7a80-a001-00000000e042";
    store
        .reconcile_bulk_collection_atomic(&collection, &[(child(child_id, bulk_id, 0), Vec::new())])
        .expect("taskless child");

    let direct_id = "anl_01983c20-0180-7a80-a001-00000000e043";
    let outcome = store
        .reconcile_observed_analysis_atomic(
            &analysis(direct_id, Some(&ai_result("Direct")), None),
            &[task(direct_id, "task-later")],
            stamp("2026-08-01T11:00:00Z"),
            bodyless_terminal,
        )
        .expect("direct read remains separate");
    assert!(outcome.inserted);
    assert_eq!(outcome.stored_id.to_string(), direct_id);

    let connection = database(&root);
    assert_eq!(count(&connection, "analyses"), 2);
    let direct_membership: (Option<String>, Option<i64>) = connection
        .query_row(
            "SELECT bulk_id, bulk_index FROM analyses WHERE id = ?1",
            [direct_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("direct row");
    assert_eq!(direct_membership, (None, None));
    assert_eq!(
        count(
            &connection,
            "(SELECT 1 FROM upstream_tasks WHERE analysis_id = 'anl_01983c20-0180-7a80-a001-00000000e042')"
        ),
        0
    );
}

#[test]
fn bulk_refresh_rejects_distinct_taskless_membership_and_task_row_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("store");
    let bulk_id = "bulk_01983c20-0180-7a80-a001-00000000e051";
    let collection = bulk(bulk_id, "upstream-attaches");
    let member_id = "anl_01983c20-0180-7a80-a001-00000000e052";
    store
        .reconcile_bulk_collection_atomic(
            &collection,
            &[(child(member_id, bulk_id, 0), Vec::new())],
        )
        .expect("taskless child");

    // A direct read before the bulk has attested any task key remains a
    // separate row; there is no membership evidence to correlate it.
    let first_direct_id = "anl_01983c20-0180-7a80-a001-00000000e053";
    store
        .reconcile_observed_analysis_atomic(
            &analysis(first_direct_id, Some(&ai_result("Direct")), None),
            &[task(first_direct_id, "task-attached")],
            stamp("2026-08-01T11:00:00Z"),
            bodyless_terminal,
        )
        .expect("uncorrelated direct read");
    assert_eq!(count(&database(&root), "analyses"), 2);

    let before = logical_state(&database(&root));

    // A later bulk observation cannot prove that these two independently
    // authored rows are one analysis. It must fail closed without moving,
    // deleting, or rekeying the standalone row's evidence.
    let refresh_id = "anl_01983c20-0180-7a80-a001-00000000e054";
    let error = store
        .reconcile_bulk_collection_atomic(
            &collection,
            &[(
                child(refresh_id, bulk_id, 0),
                vec![task(refresh_id, "task-attached")],
            )],
        )
        .expect_err("ambiguous provenance must fail closed");
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    let connection = database(&root);
    assert_eq!(logical_state(&connection), before);
    assert_eq!(count(&connection, "analyses"), 2);
    let task_owner: String = connection
        .query_row(
            "SELECT analysis_id FROM upstream_tasks WHERE upstream_task_id = 'task-attached'",
            [],
            |row| row.get(0),
        )
        .expect("attached identity");
    assert_eq!(task_owner, first_direct_id);
    assert_eq!(count(&connection, "upstream_tasks"), 1);
}

#[test]
fn taskless_membership_transfer_rejects_invalid_search_rows_with_full_rollback() {
    let corruptions = ["missing", "duplicate", "malformed"];
    for (case, corruption) in corruptions.into_iter().enumerate() {
        for corrupt_source in [false, true] {
            let suffix = 70 + case * 2 + usize::from(corrupt_source);
            let root = tempfile::tempdir().unwrap();
            let mut store = HistoryStore::open(root.path()).expect("store");
            let bulk_id = format!("bulk_01983c20-0180-7a80-a001-00000000e{suffix:03}");
            let mut collection = bulk(&bulk_id, &format!("upstream-transfer-fts-{suffix}"));
            let member_id = format!("anl_01983c20-0180-7a80-a001-00000000e{suffix:03}");
            store
                .reconcile_bulk_collection_atomic(
                    &collection,
                    &[(child(&member_id, &bulk_id, 0), Vec::new())],
                )
                .expect("taskless membership target");

            let source_id = format!("anl_01983c20-0180-7a80-a001-00000000f{suffix:03}");
            let task_id = format!("task-transfer-fts-{suffix}");
            store
                .save_analysis_atomic(
                    &analysis(&source_id, Some(&ai_result("Direct")), None),
                    &[task(&source_id, &task_id)],
                )
                .expect("standalone task source");

            let corrupted_id = if corrupt_source {
                source_id.as_str()
            } else {
                member_id.as_str()
            };
            store
                .with_connection(|connection| match corruption {
                    "missing" => connection.execute(
                        "DELETE FROM analysis_search WHERE analysis_id = ?1",
                        [corrupted_id],
                    ),
                    "duplicate" => connection.execute(
                        "INSERT INTO analysis_search
                            (analysis_id, input_text, filename, headline, source_urls)
                         SELECT analysis_id, input_text, filename, headline, source_urls
                         FROM analysis_search WHERE analysis_id = ?1",
                        [corrupted_id],
                    ),
                    "malformed" => connection.execute(
                        "UPDATE analysis_search SET input_text = 42 WHERE analysis_id = ?1",
                        [corrupted_id],
                    ),
                    _ => unreachable!(),
                })
                .expect("raw connection")
                .expect("corrupt transfer search row");
            let before = logical_state(&database(&root));

            collection.status = AnalysisStatus::Succeeded;
            collection.updated_at = stamp("2026-08-03T12:00:00Z");
            let refresh_id = format!("anl_01983c20-0180-7a80-a001-000000001{suffix:03}");
            let error = store
                .reconcile_bulk_collection_atomic(
                    &collection,
                    &[(
                        child(&refresh_id, &bulk_id, 0),
                        vec![task(&refresh_id, &task_id)],
                    )],
                )
                .expect_err("invalid transfer search state must fail closed");
            assert_eq!(
                error.code(),
                HistoryErrorCode::HistoryCorrupt,
                "{corruption} source={corrupt_source}: {error:?}"
            );

            let connection = database(&root);
            assert_eq!(
                logical_state(&connection),
                before,
                "{corruption} source={corrupt_source}: transfer failure mutated state"
            );
            let task_owner: String = connection
                .query_row(
                    "SELECT analysis_id FROM upstream_tasks WHERE upstream_task_id = ?1",
                    [&task_id],
                    |row| row.get(0),
                )
                .expect("standalone task remains");
            assert_eq!(task_owner, source_id);
            let member_task_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM upstream_tasks WHERE analysis_id = ?1",
                    [&member_id],
                    |row| row.get(0),
                )
                .expect("membership task count");
            assert_eq!(member_task_count, 0);
        }
    }
}

#[test]
fn body_empty_terminal_refresh_preserves_standalone_body() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("store");
    let id = "anl_01983c20-0180-7a80-a001-00000000e061";
    let result = serde_json::json!({
        "classification": "human",
        "headline": "Stored result",
        "prediction": "Human",
        "fraction_ai": 0.0,
        "fraction_ai_assisted": 0.0,
        "fraction_human": 1.0,
        "num_ai_segments": 0,
        "num_ai_assisted_segments": 0,
        "num_human_segments": 1,
        "segments": []
    })
    .to_string();
    store
        .save_analysis_atomic(
            &analysis(id, Some(&result), None),
            &[task(id, "task-body-standalone")],
        )
        .expect("stored result");
    store
        .reconcile_observed_analysis_atomic(
            &analysis("anl_01983c20-0180-7a80-a001-00000000e062", None, None),
            &[task(
                "anl_01983c20-0180-7a80-a001-00000000e062",
                "task-body-standalone",
            )],
            stamp("2026-08-02T10:00:00Z"),
            bodyless_terminal,
        )
        .expect("bodyless terminal refresh");
    let stored = store
        .get_analysis(&AnalysisId::from_str(id).unwrap())
        .expect("stored");
    assert_eq!(stored.result_json.as_deref(), Some(result.as_str()));
    assert_eq!(stored.error_json, None);
    assert_eq!(stored.status, AnalysisStatus::Succeeded);
    assert_eq!(stored.completed_at, Some(stamp("2026-08-01T10:05:00Z")));
}

#[test]
fn body_empty_terminal_refresh_preserves_membership_and_adoption_bodies() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("store");
    let bulk_id = "bulk_01983c20-0180-7a80-a001-00000000e071";
    let collection = bulk(bulk_id, "upstream-bodies");
    let member_id = "anl_01983c20-0180-7a80-a001-00000000e072";
    let mut member = child(member_id, bulk_id, 0);
    let member_error = canonical_error(ErrorCode::UpstreamError, "stored error");
    member.status = AnalysisStatus::Failed;
    member.error_json = Some(member_error.clone());
    member.completed_at = Some(stamp("2026-08-01T10:05:00Z"));
    store
        .reconcile_bulk_collection_atomic(&collection, &[(member, Vec::new())])
        .expect("terminal member");
    let mut member_refresh = child("anl_01983c20-0180-7a80-a001-00000000e073", bulk_id, 0);
    member_refresh.status = AnalysisStatus::Succeeded;
    member_refresh.completed_at = Some(stamp("2026-08-02T10:00:00Z"));
    store
        .reconcile_bulk_collection_atomic(&collection, &[(member_refresh, Vec::new())])
        .expect("bodyless membership refresh");

    let standalone_id = "anl_01983c20-0180-7a80-a001-00000000e074";
    store
        .save_analysis_atomic(
            &analysis(standalone_id, Some(&ai_result("Adopted")), None),
            &[task(standalone_id, "task-body-adopt")],
        )
        .expect("standalone result");
    let adoption_id = "anl_01983c20-0180-7a80-a001-00000000e075";
    let mut adopted_refresh = child(adoption_id, bulk_id, 1);
    adopted_refresh.completed_at = Some(stamp("2026-08-02T10:00:00Z"));
    store
        .reconcile_bulk_collection_atomic(
            &collection,
            &[(adopted_refresh, vec![task(adoption_id, "task-body-adopt")])],
        )
        .expect("bodyless adoption refresh");

    let member = store
        .get_analysis(&AnalysisId::from_str(member_id).unwrap())
        .expect("member");
    assert_eq!(member.status, AnalysisStatus::Failed);
    assert_eq!(member.result_json, None);
    assert_eq!(member.error_json.as_deref(), Some(member_error.as_str()));
    let adopted = store
        .get_analysis(&AnalysisId::from_str(standalone_id).unwrap())
        .expect("adopted");
    assert_eq!(adopted.status, AnalysisStatus::Succeeded);
    assert_eq!(
        adopted.result_json.as_deref(),
        Some(ai_result("Adopted").as_str())
    );
    assert_eq!(adopted.error_json, None);
}

#[test]
fn terminal_body_branches_replace_opposites_and_both_present_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("store");
    let id = AnalysisId::from_str("anl_01983c20-0180-7a80-a001-00000000e081").unwrap();
    store
        .save_analysis_atomic(
            &analysis(
                id.to_string().as_str(),
                None,
                Some(&canonical_error(ErrorCode::UpstreamError, "old error")),
            ),
            &[],
        )
        .expect("stored error");
    let snapshot = |result: Option<&str>, error: Option<&str>| {
        let search_headline = result
            .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
            .and_then(|value| value["headline"].as_str().map(str::to_owned));
        ObservationSnapshot {
            status: if error.is_some() {
                AnalysisStatus::Failed
            } else {
                AnalysisStatus::Succeeded
            },
            submission_outcome: SubmissionOutcome::Terminal,
            result_json: result.map(str::to_owned),
            error_json: error.map(str::to_owned),
            upstream_version: None,
            completed_at: Some(stamp("2026-08-02T10:00:00Z")),
            search_input_text: None,
            search_filename: None,
            search_headline,
            search_source_urls: None,
        }
    };
    let new_result = ai_result("New result");
    let new_error = canonical_error(ErrorCode::UpstreamError, "new error");

    store
        .update_observation_snapshot(
            &id,
            stamp("2026-08-02T10:00:00Z"),
            &snapshot(Some(&new_result), None),
        )
        .expect("result replaces error");
    let stored = store.get_analysis(&id).expect("result");
    assert_eq!(stored.result_json.as_deref(), Some(new_result.as_str()));
    assert_eq!(stored.error_json, None);

    store
        .update_observation_snapshot(
            &id,
            stamp("2026-08-02T11:00:00Z"),
            &snapshot(None, Some(&new_error)),
        )
        .expect("error replaces result");
    let stored = store.get_analysis(&id).expect("error");
    assert_eq!(stored.result_json, None);
    assert_eq!(stored.error_json.as_deref(), Some(new_error.as_str()));

    let error = store
        .update_observation_snapshot(
            &id,
            stamp("2026-08-02T12:00:00Z"),
            &snapshot(Some("{\"bad\":1}"), Some("{\"bad\":2}")),
        )
        .expect_err("both branches must fail closed");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);
    let stored = store.get_analysis(&id).expect("rollback");
    assert_eq!(stored.result_json, None);
    assert_eq!(stored.error_json.as_deref(), Some(new_error.as_str()));
}
