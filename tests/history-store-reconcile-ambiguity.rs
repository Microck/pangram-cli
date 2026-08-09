//! Ambiguous taskless-membership reconciliation against the real SQLite store.

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

fn child(id: &str, bulk_id: &str) -> StoredAnalysis {
    let mut record = standalone(id, "bulk item 0");
    record.bulk = Some((BulkId::from_str(bulk_id).expect("bulk id"), 0));
    record.caller_id = Some("row-000".to_owned());
    record.submission_outcome = SubmissionOutcome::Accepted;
    record.status = AnalysisStatus::Running;
    record.result_json = None;
    record.completed_at = None;
    record.search_headline = None;
    record
}

fn running_merge(_: &StoredAnalysis) -> Result<ObservationSnapshot, HistoryError> {
    Ok(ObservationSnapshot {
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Terminal,
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

fn count(connection: &rusqlite::Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count rows")
}

/// A taskless membership and a distinct standalone task row are ambiguous
/// regardless of creation order. A later bulk attestation must report
/// corruption and preserve both rows and the standalone evidence owner.
#[test]
fn distinct_task_and_taskless_membership_conflict_in_both_orders() {
    for task_first in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut store = HistoryStore::open(root.path()).expect("open store");
        let bulk_id = if task_first {
            "bulk_01983c20-0180-7a80-a001-00000000d091"
        } else {
            "bulk_01983c20-0180-7a80-a001-00000000d092"
        };
        let member_id = if task_first {
            "anl_01983c20-0180-7a80-a001-00000000d093"
        } else {
            "anl_01983c20-0180-7a80-a001-00000000d094"
        };
        let direct_id = if task_first {
            "anl_01983c20-0180-7a80-a001-00000000d095"
        } else {
            "anl_01983c20-0180-7a80-a001-00000000d096"
        };
        let bulk = collection(bulk_id, if task_first { "order-a" } else { "order-b" });
        let save_member = |store: &mut HistoryStore| {
            store
                .reconcile_bulk_collection_atomic(&bulk, &[(child(member_id, bulk_id), Vec::new())])
        };
        let save_task = |store: &mut HistoryStore| {
            let record = standalone(direct_id, "independent direct row");
            store
                .reconcile_observed_analysis_atomic(
                    &record,
                    &[observation(direct_id, "task-ambiguous")],
                    timestamp("2026-08-01T10:05:00Z"),
                    running_merge,
                )
                .map(|_| ())
        };
        if task_first {
            save_task(&mut store).expect("task row");
            save_member(&mut store).expect("taskless member");
        } else {
            save_member(&mut store).expect("taskless member");
            save_task(&mut store).expect("task row");
        }

        let candidate_id = "anl_01983c20-0180-7a80-a001-00000000d097";
        let error = store
            .reconcile_bulk_collection_atomic(
                &bulk,
                &[(
                    child(candidate_id, bulk_id),
                    vec![observation(candidate_id, "task-ambiguous")],
                )],
            )
            .expect_err("ambiguous provenance must fail");
        assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);

        let connection = database(&root);
        assert_eq!(count(&connection, "analyses"), 2);
        assert_eq!(count(&connection, "upstream_tasks"), 1);
        let owner: String = connection
            .query_row("SELECT analysis_id FROM upstream_tasks", [], |row| {
                row.get(0)
            })
            .expect("task owner");
        assert_eq!(owner, direct_id);
    }
}

/// Independent writers racing the same ambiguous refresh both serialize onto
/// the fail-closed decision; neither may mutate or transfer evidence.
#[test]
fn concurrent_ambiguous_bulk_refreshes_both_roll_back() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().to_path_buf();
    let bulk_id = "bulk_01983c20-0180-7a80-a001-00000000d0a1";
    let bulk = collection(bulk_id, "upstream-concurrent-ambiguous");
    let mut setup = HistoryStore::open(&path).expect("setup store");
    setup
        .reconcile_bulk_collection_atomic(
            &bulk,
            &[(
                child("anl_01983c20-0180-7a80-a001-00000000d0a2", bulk_id),
                Vec::new(),
            )],
        )
        .expect("taskless member");
    let direct_id = "anl_01983c20-0180-7a80-a001-00000000d0a3";
    setup
        .save_analysis_atomic(
            &standalone(direct_id, "direct evidence"),
            &[observation(direct_id, "task-concurrent-ambiguous")],
        )
        .expect("standalone task row");
    drop(setup);

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let run = |suffix: &'static str| {
        let barrier = barrier.clone();
        let path = path.clone();
        let bulk = bulk.clone();
        std::thread::spawn(move || {
            let mut store = HistoryStore::open(&path).expect("thread store");
            let child_id = format!("anl_01983c20-0180-7a80-a001-00000000{suffix}");
            barrier.wait();
            store
                .reconcile_bulk_collection_atomic(
                    &bulk,
                    &[(
                        child(&child_id, bulk_id),
                        vec![observation(&child_id, "task-concurrent-ambiguous")],
                    )],
                )
                .expect_err("ambiguous refresh fails")
                .code()
        })
    };
    let first = run("d0a4");
    let second = run("d0a5");
    assert_eq!(first.join().unwrap(), HistoryErrorCode::HistoryCorrupt);
    assert_eq!(second.join().unwrap(), HistoryErrorCode::HistoryCorrupt);

    let connection = database(&root);
    assert_eq!(count(&connection, "analyses"), 2);
    assert_eq!(count(&connection, "upstream_tasks"), 1);
    let owner: String = connection
        .query_row("SELECT analysis_id FROM upstream_tasks", [], |row| {
            row.get(0)
        })
        .expect("task owner");
    assert_eq!(owner, direct_id);
}
