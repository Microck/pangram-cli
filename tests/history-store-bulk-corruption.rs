//! Fail-closed bulk reconciliation against complete canonical stored
//! aggregates. Every case uses real SQLite corruption and proves the
//! `IMMEDIATE` transaction leaves all logical rows byte-for-byte unchanged.

#![forbid(unsafe_code)]

use std::fs;
use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, BulkCounters, BulkId, CheckKind, SaveState, Sha256Hash,
    SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryErrorCode, HistoryStore, InputKind, StoredAnalysis, StoredBulkCollection,
    StoredUpstreamTask,
};

#[derive(Clone, Copy, Debug)]
enum Corruption {
    ParentStatus,
    TaskCorrespondence,
    Search,
    Input,
    Provenance,
    Membership,
    Lineage,
}

const CORRUPTIONS: [Corruption; 7] = [
    Corruption::ParentStatus,
    Corruption::TaskCorrespondence,
    Corruption::Search,
    Corruption::Input,
    Corruption::Provenance,
    Corruption::Membership,
    Corruption::Lineage,
];

fn stamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("timestamp")
}

fn collection(id: &str, upstream: &str) -> StoredBulkCollection {
    StoredBulkCollection {
        id: BulkId::from_str(id).expect("bulk id"),
        upstream_bulk_id: Some(upstream.to_owned()),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        counters: BulkCounters::new(1, 1, 0, 0).expect("counters"),
        estimated_billable_units: Some(1),
        created_at: stamp("2026-08-01T09:00:00Z"),
        updated_at: stamp("2026-08-01T09:00:00Z"),
        completed_at: None,
    }
}

fn analysis(id: &str, bulk: Option<(&str, i64)>) -> StoredAnalysis {
    let input = format!("canonical input {id}");
    let input_sha256 = Sha256Hash::digest(&input);
    StoredAnalysis {
        id: AnalysisId::from_str(id).expect("analysis id"),
        bulk: bulk.map(|(id, index)| (BulkId::from_str(id).expect("bulk id"), index)),
        caller_id: bulk.map(|_| "row-000".to_owned()),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        save_state: SaveState::SavedHistory,
        input_kind: InputKind::Text,
        input_sha256,
        display_name: None,
        input_json: serde_json::json!({
            "type": "text",
            "origin": "literal",
            "sha256": input_sha256,
            "byte_count": input.len(),
            "word_count": 3,
            "text": input
        })
        .to_string(),
        result_json: None,
        error_json: None,
        upstream_version: Some("4.0".to_owned()),
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(stamp("2026-08-01T09:59:00Z")),
        created_at: stamp("2026-08-01T10:00:00Z"),
        updated_at: stamp("2026-08-01T10:05:00Z"),
        completed_at: None,
        search_input_text: Some(input),
        search_filename: None,
        search_headline: None,
        search_source_urls: None,
    }
}

fn observation(id: &str, task: &str) -> StoredUpstreamTask {
    StoredUpstreamTask {
        analysis_id: AnalysisId::from_str(id).expect("analysis id"),
        check_kind: CheckKind::AiDetection,
        upstream_task_id: task.to_owned(),
        last_stage: Some("RUNNING".to_owned()),
        observed_at: stamp("2026-08-01T10:05:00Z"),
    }
}

fn database(root: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(root.path().join("history/pangram-history.db"))
        .expect("open database")
}

fn corrupt(
    store: &mut HistoryStore,
    target: &str,
    corruption: Corruption,
) -> Result<(), rusqlite::Error> {
    if matches!(corruption, Corruption::Membership) {
        store
            .upsert_bulk_collection_atomic(
                &collection(
                    "bulk_01983c20-0180-7a80-a001-00000000f001",
                    "corruption-parent",
                ),
                &[],
            )
            .expect("corruption parent");
    }
    store
        .with_connection(|connection| match corruption {
            Corruption::ParentStatus => connection.execute(
                "UPDATE analyses SET status = 'succeeded' WHERE id = ?1",
                [target],
            ),
            Corruption::TaskCorrespondence => connection.execute(
                "INSERT INTO upstream_tasks
                   (analysis_id, check_kind, upstream_task_id, observed_at)
                 VALUES (?1, 'plagiarism', ?2, '2026-08-01T10:05:00Z')",
                rusqlite::params![target, format!("orphan-{target}")],
            ),
            Corruption::Search => connection.execute(
                "DELETE FROM analysis_search WHERE analysis_id = ?1",
                [target],
            ),
            Corruption::Input => connection.execute(
                "UPDATE analyses SET input_sha256 = ?2 WHERE id = ?1",
                rusqlite::params![target, "0".repeat(64)],
            ),
            Corruption::Provenance => connection.execute(
                "UPDATE analyses SET submitted_at = 'not-a-timestamp' WHERE id = ?1",
                [target],
            ),
            Corruption::Membership => connection.execute(
                "UPDATE analyses SET bulk_id = ?2, bulk_index = 1 WHERE id = ?1",
                rusqlite::params![target, "bulk_01983c20-0180-7a80-a001-00000000f001"],
            ),
            Corruption::Lineage => connection.execute(
                "UPDATE analyses SET retry_of = id, rerun_of = id WHERE id = ?1",
                [target],
            ),
        })
        .expect("raw corruption connection")
        .map(|_| ())
}

fn logical_state(connection: &rusqlite::Connection) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "SELECT 'b|' || quote(id) || '|' || quote(upstream_bulk_id) || '|' ||
                    quote(status) || '|' || quote(total_items) || '|' || quote(updated_at)
             FROM bulk_collections
             UNION ALL
             SELECT 'a|' || quote(id) || '|' || quote(bulk_id) || '|' ||
                    quote(bulk_index) || '|' || quote(status) || '|' ||
                    quote(input_sha256) || '|' || quote(input_json) || '|' ||
                    quote(retry_of) || '|' || quote(rerun_of) || '|' ||
                    quote(submitted_at) || '|' || quote(updated_at)
             FROM analyses
             UNION ALL
             SELECT 'c|' || quote(analysis_id) || '|' || quote(check_index) || '|' ||
                    quote(check_kind) || '|' || quote(status) || '|' ||
                    quote(result_json) || '|' || quote(error_json)
             FROM analysis_checks
             UNION ALL
             SELECT 't|' || quote(analysis_id) || '|' || quote(check_kind) || '|' ||
                    quote(upstream_task_id) || '|' || quote(last_stage) || '|' ||
                    quote(observed_at)
             FROM upstream_tasks
             UNION ALL
             SELECT 's|' || quote(analysis_id) || '|' || quote(input_text) || '|' ||
                    quote(filename) || '|' || quote(headline) || '|' || quote(source_urls)
             FROM analysis_search
             ORDER BY 1",
        )
        .expect("prepare logical state");
    statement
        .query_map([], |row| row.get(0))
        .expect("query logical state")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect logical state")
}

#[test]
fn existing_and_adopted_children_reject_every_canonical_corruption_without_mutation() {
    for adopt in [false, true] {
        for corruption in CORRUPTIONS {
            let root = tempfile::tempdir().expect("temp root");
            let mut store = HistoryStore::open(root.path()).expect("store");
            let bulk_id = "bulk_01983c20-0180-7a80-a001-00000000f010";
            let member_id = "anl_01983c20-0180-7a80-a001-00000000f011";
            let source_id = "anl_01983c20-0180-7a80-a001-00000000f012";
            let bulk = collection(bulk_id, "canonical-corruption");
            let target = if adopt {
                store
                    .save_analysis_atomic(
                        &analysis(source_id, None),
                        &[observation(source_id, "task-adopt-corrupt")],
                    )
                    .expect("standalone source");
                source_id
            } else {
                store
                    .reconcile_bulk_collection_atomic(
                        &bulk,
                        &[(
                            analysis(member_id, Some((bulk_id, 0))),
                            vec![observation(member_id, "task-member-corrupt")],
                        )],
                    )
                    .expect("existing member");
                member_id
            };
            corrupt(&mut store, target, corruption).expect("corrupt target");
            let before = logical_state(&database(&root));
            let (incoming_id, task_id) = if adopt {
                (
                    "anl_01983c20-0180-7a80-a001-00000000f013",
                    "task-adopt-corrupt",
                )
            } else {
                (member_id, "task-member-corrupt")
            };
            let error = store
                .reconcile_bulk_collection_atomic(
                    &bulk,
                    &[(
                        analysis(incoming_id, Some((bulk_id, 0))),
                        vec![observation(incoming_id, task_id)],
                    )],
                )
                .expect_err("corrupt aggregate must fail closed");
            assert_eq!(
                error.code(),
                HistoryErrorCode::HistoryCorrupt,
                "adopt={adopt} corruption={corruption:?}"
            );
            assert_eq!(
                logical_state(&database(&root)),
                before,
                "adopt={adopt} corruption={corruption:?}"
            );
        }
    }
}

#[test]
fn external_bulk_upsert_certifies_a_standalone_adoption_before_mutation() {
    for corruption in CORRUPTIONS {
        let root = tempfile::tempdir().expect("temp root");
        let mut store = HistoryStore::open(root.path()).expect("store");
        let bulk_id = "bulk_01983c20-0180-7a80-a001-00000000f040";
        let source_id = "anl_01983c20-0180-7a80-a001-00000000f041";
        let incoming_id = "anl_01983c20-0180-7a80-a001-00000000f042";
        store
            .save_analysis_atomic(
                &analysis(source_id, None),
                &[observation(source_id, "task-external-adopt-corrupt")],
            )
            .expect("standalone source");
        corrupt(&mut store, source_id, corruption).expect("corrupt source");
        let before = logical_state(&database(&root));

        let error = store
            .upsert_bulk_collection_atomic(
                &collection(bulk_id, "external-adoption-corruption"),
                &[(
                    analysis(incoming_id, Some((bulk_id, 0))),
                    vec![observation(incoming_id, "task-external-adopt-corrupt")],
                )],
            )
            .expect_err("corrupt standalone candidate must fail before bulk upsert");
        assert_eq!(
            error.code(),
            HistoryErrorCode::HistoryCorrupt,
            "corruption={corruption:?}"
        );
        assert_eq!(
            logical_state(&database(&root)),
            before,
            "corruption={corruption:?}"
        );
    }
}

#[test]
fn delete_and_clear_reject_corruption_anywhere_without_checkpointing() {
    let root = tempfile::tempdir().expect("temp root");
    let mut store = HistoryStore::open(root.path()).expect("store");
    let target_id = "anl_01983c20-0180-7a80-a001-00000000f050";
    let corrupt_id = "anl_01983c20-0180-7a80-a001-00000000f051";
    store
        .save_analysis_atomic(
            &analysis(target_id, None),
            &[observation(target_id, "task-delete-target")],
        )
        .expect("delete target");
    store
        .save_analysis_atomic(
            &analysis(corrupt_id, None),
            &[observation(corrupt_id, "task-delete-corrupt")],
        )
        .expect("unrelated row");
    corrupt(&mut store, corrupt_id, Corruption::Input).expect("corrupt unrelated row");
    let before = logical_state(&database(&root));
    let wal = store.database_path().with_extension("db-wal");
    let wal_before = fs::read(&wal).expect("read WAL before rejected delete");

    let error = store
        .delete_analysis(&AnalysisId::from_str(target_id).expect("target id"))
        .expect_err("unrelated corruption blocks delete");
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    assert_eq!(logical_state(&database(&root)), before);
    assert_eq!(
        fs::read(&wal).expect("read WAL after rejected delete"),
        wal_before,
        "rejected delete must not run the post-commit checkpoint"
    );

    let clear_root = tempfile::tempdir().expect("clear temp root");
    let mut clear_store = HistoryStore::open(clear_root.path()).expect("clear store");
    let bulk_id = "bulk_01983c20-0180-7a80-a001-00000000f052";
    clear_store
        .upsert_bulk_collection_atomic(
            &collection(bulk_id, "clear-corrupt-bulk"),
            &[(
                analysis(
                    "anl_01983c20-0180-7a80-a001-00000000f053",
                    Some((bulk_id, 0)),
                ),
                Vec::new(),
            )],
        )
        .expect("seed clear store");
    clear_store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE bulk_collections SET status = 'not-a-status' WHERE id = ?1",
                [bulk_id],
            )
        })
        .expect("raw clear corruption connection")
        .expect("corrupt bulk");
    let clear_before = logical_state(&database(&clear_root));
    let clear_wal = clear_store.database_path().with_extension("db-wal");
    let clear_wal_before = fs::read(&clear_wal).expect("read WAL before rejected clear");

    let error = clear_store
        .clear()
        .expect_err("bulk corruption blocks clear");
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    assert_eq!(logical_state(&database(&clear_root)), clear_before);
    assert_eq!(
        fs::read(&clear_wal).expect("read WAL after rejected clear"),
        clear_wal_before,
        "rejected clear must not run the post-commit checkpoint"
    );
}

#[test]
fn transfer_validates_both_existing_candidates_before_mutation() {
    for corrupt_source in [false, true] {
        for corruption in CORRUPTIONS {
            let root = tempfile::tempdir().expect("temp root");
            let mut store = HistoryStore::open(root.path()).expect("store");
            let bulk_id = "bulk_01983c20-0180-7a80-a001-00000000f020";
            let member_id = "anl_01983c20-0180-7a80-a001-00000000f021";
            let source_id = "anl_01983c20-0180-7a80-a001-00000000f022";
            let bulk = collection(bulk_id, "transfer-corruption");
            store
                .reconcile_bulk_collection_atomic(
                    &bulk,
                    &[(analysis(member_id, Some((bulk_id, 0))), Vec::new())],
                )
                .expect("taskless member");
            store
                .save_analysis_atomic(
                    &analysis(source_id, None),
                    &[observation(source_id, "task-transfer-corrupt")],
                )
                .expect("standalone source");
            corrupt(
                &mut store,
                if corrupt_source { source_id } else { member_id },
                corruption,
            )
            .expect("corrupt candidate");
            let before = logical_state(&database(&root));
            let incoming_id = member_id;
            let error = store
                .reconcile_bulk_collection_atomic(
                    &bulk,
                    &[(
                        analysis(incoming_id, Some((bulk_id, 0))),
                        vec![observation(incoming_id, "task-transfer-corrupt")],
                    )],
                )
                .expect_err("corrupt transfer must fail closed");
            assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
            assert_eq!(
                logical_state(&database(&root)),
                before,
                "source={corrupt_source} corruption={corruption:?}"
            );
        }
    }
}

#[test]
fn concurrent_bulk_refreshes_serialize_on_the_same_fail_closed_state() {
    for corruption in CORRUPTIONS {
        let root = tempfile::tempdir().expect("temp root");
        let path = root.path().to_path_buf();
        let bulk_id = "bulk_01983c20-0180-7a80-a001-00000000f030";
        let member_id = "anl_01983c20-0180-7a80-a001-00000000f031";
        let bulk = collection(bulk_id, "concurrent-corruption");
        let mut setup = HistoryStore::open(&path).expect("store");
        setup
            .reconcile_bulk_collection_atomic(
                &bulk,
                &[(
                    analysis(member_id, Some((bulk_id, 0))),
                    vec![observation(member_id, "task-concurrent-corrupt")],
                )],
            )
            .expect("member");
        corrupt(&mut setup, member_id, corruption).expect("corrupt member");
        let before = logical_state(&database(&root));
        drop(setup);

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let run = || {
            let barrier = barrier.clone();
            let path = path.clone();
            let bulk = bulk.clone();
            std::thread::spawn(move || {
                let mut store = HistoryStore::open(&path).expect("thread store");
                barrier.wait();
                store
                    .reconcile_bulk_collection_atomic(
                        &bulk,
                        &[(
                            analysis(member_id, Some((bulk_id, 0))),
                            vec![observation(member_id, "task-concurrent-corrupt")],
                        )],
                    )
                    .expect_err("corrupt concurrent refresh")
                    .code()
            })
        };
        let first = run();
        let second = run();
        assert_eq!(
            first.join().expect("first"),
            HistoryErrorCode::HistoryCorrupt
        );
        assert_eq!(
            second.join().expect("second"),
            HistoryErrorCode::HistoryCorrupt
        );
        assert_eq!(
            logical_state(&database(&root)),
            before,
            "corruption={corruption:?}"
        );
    }
}
