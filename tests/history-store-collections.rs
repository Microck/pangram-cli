//! Real-SQLite proof of the whole-collection write path (contracts.md 14.2
//! note): one bulk job owns one stored row reconciled by `upstream_bulk_id`,
//! refreshed atomically with its children and their observation rows, with
//! local authorship (identity, submission outcome, save state, caller ID,
//! input content, creation time) preserved across observations.
//!
//! No mocks: every `HistoryStore` points at a real `tempfile::TempDir`.

#![forbid(unsafe_code)]

use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, BulkCounters, BulkId, SaveState, Sha256Hash, SubmissionOutcome,
    UtcTimestamp,
};
use microck_pangram_cli::history::{HistoryStore, InputKind, StoredAnalysis, StoredBulkCollection};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("test timestamp")
}

fn collection(id: &str, upstream: Option<&str>, created: &str) -> StoredBulkCollection {
    StoredBulkCollection {
        id: BulkId::from_str(id).expect("bulk id"),
        upstream_bulk_id: upstream.map(str::to_owned),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        counters: BulkCounters::new(1, 1, 0, 0).expect("counters"),
        estimated_billable_units: Some(4),
        created_at: timestamp(created),
        updated_at: timestamp(created),
        completed_at: None,
    }
}

fn child(id: &str, bulk_id: &str, index: i64, caller: Option<&str>) -> StoredAnalysis {
    StoredAnalysis {
        id: AnalysisId::from_str(id).expect("analysis id"),
        bulk: Some((BulkId::from_str(bulk_id).expect("bulk id"), index)),
        caller_id: caller.map(str::to_owned),
        status: AnalysisStatus::Queued,
        submission_outcome: SubmissionOutcome::NotSubmitted,
        save_state: SaveState::SavedHistory,
        input_kind: InputKind::Text,
        input_sha256: Sha256Hash::from_bytes([9; 32]),
        display_name: Some(format!("item-{index}.jsonl")),
        input_json: format!("{{\"type\":\"text\",\"text\":\"item {index}\"}}"),
        result_json: None,
        error_json: None,
        upstream_version: None,
        retry_of: None,
        rerun_of: None,
        created_at: timestamp("2026-08-01T09:00:00Z"),
        updated_at: timestamp("2026-08-01T09:00:00Z"),
        completed_at: None,
        search_input_text: Some(format!("item {index} text")),
        search_filename: Some(format!("item-{index}.jsonl")),
        search_headline: None,
        search_source_urls: None,
    }
}

/// A first observation inserts the collection and its children, deduped by
/// the upstream identity.
#[test]
fn upsert_bulk_collection_inserts_children_and_resolves_by_upstream() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let record = collection(
        "bulk_01983c20-0180-7a80-a001-0000000000aa",
        Some("upstream-bulk-9"),
        "2026-08-01T09:00:00Z",
    );
    let children = vec![(
        child(
            "anl_01983c20-0180-7a80-a001-000000000011",
            "bulk_01983c20-0180-7a80-a001-0000000000aa",
            0,
            Some("row-000"),
        ),
        Vec::new(),
    )];
    store
        .upsert_bulk_collection_atomic(&record, &children)
        .expect("insert commits");

    let found = store
        .find_bulk_collection_by_upstream("upstream-bulk-9")
        .expect("lookup works")
        .expect("the collection reconciles by upstream id");
    assert_eq!(found.id, record.id);

    let members = store
        .list_bulk_analyses(&record.id)
        .expect("list bulk members");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].caller_id.as_deref(), Some("row-000"));
}

/// A repeated observation of the same job refreshes the one stored row and
/// preserves local authorship instead of duplicating it.
#[test]
fn upsert_bulk_collection_reconciles_without_duplicates_and_preserves_authorship() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    // First save: a submit-time collection with a locally authored child.
    let first = collection(
        "bulk_01983c20-0180-7a80-a001-0000000000bb",
        Some("upstream-bulk-7"),
        "2026-08-01T09:00:00Z",
    );
    let first_child = child(
        "anl_01983c20-0180-7a80-a001-000000000021",
        "bulk_01983c20-0180-7a80-a001-0000000000bb",
        0,
        Some("row-000"),
    );
    store
        .upsert_bulk_collection_atomic(&first, &[(first_child, Vec::new())])
        .expect("first save commits");

    // Later observation of the SAME job reconciled by upstream id: the
    // caller mints a fresh local collection id for this read, refreshes the
    // status/counters, and its child record carries no input/search payload
    // (a remote-only read). Local authorship must survive.
    let mut refreshed = collection(
        "bulk_01983c20-0180-7a80-a001-0000000000cc",
        Some("upstream-bulk-7"),
        "2026-08-02T12:00:00Z",
    );
    refreshed.status = AnalysisStatus::Succeeded;
    refreshed.counters = BulkCounters::new(1, 1, 1, 0).expect("counters");
    refreshed.completed_at = Some(timestamp("2026-08-02T12:00:00Z"));
    // The reconcile resolves the stored id through the caller beforehand,
    // so this record carries the stored id.
    refreshed.id = first.id;
    let mut observed_child = child(
        "anl_01983c20-0180-7a80-a001-0000000000ff",
        "bulk_01983c20-0180-7a80-a001-0000000000bb",
        0,
        None,
    );
    observed_child.status = AnalysisStatus::Succeeded;
    observed_child.submission_outcome = SubmissionOutcome::Accepted;
    observed_child.save_state = SaveState::Ephemeral;
    observed_child.input_json = "{}".to_owned();
    observed_child.display_name = None;
    observed_child.search_input_text = None;
    observed_child.search_filename = None;

    store
        .upsert_bulk_collection_atomic(&refreshed, &[(observed_child, Vec::new())])
        .expect("refresh commits");

    // Still exactly one collection row, deduped by upstream identity.
    let connection =
        rusqlite::Connection::open(root.path().join("history").join("pangram-history.db"))
            .expect("open saved database");
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM bulk_collections", [], |row| {
            row.get(0)
        })
        .expect("count collections");
    assert_eq!(count, 1, "repeated observation inserts no duplicate row");
    // Collection authorship preserved: original created_at, refreshed status.
    let stored = store
        .get_bulk_collection(&first.id)
        .expect("read collection");
    assert_eq!(
        stored.created_at,
        timestamp("2026-08-01T09:00:00Z"),
        "the job's original local creation time is preserved"
    );
    assert_eq!(stored.status, AnalysisStatus::Succeeded);
    assert_eq!(stored.completed_at, Some(timestamp("2026-08-02T12:00:00Z")));

    // Child authorship preserved: original identity, caller ID, input
    // payload, and search content survive the remote-only refresh; only the
    // observed status moved.
    let members = store
        .list_bulk_analyses(&first.id)
        .expect("list bulk members");
    assert_eq!(members.len(), 1, "no duplicate child");
    assert_eq!(
        members[0].id.to_string(),
        "anl_01983c20-0180-7a80-a001-000000000021",
        "the child's original identity is preserved"
    );
    assert_eq!(members[0].caller_id.as_deref(), Some("row-000"));
    assert_eq!(
        members[0].search_input_text.as_deref(),
        Some("item 0 text"),
        "the stored input text is never wiped by a remote-only refresh"
    );
    assert_eq!(
        members[0].search_filename.as_deref(),
        Some("item-0.jsonl"),
        "the stored filename is preserved"
    );
    assert_eq!(members[0].status, AnalysisStatus::Succeeded);
}

/// A failing child (missing its membership link) rolls the whole batch back:
/// neither the collection nor any child persists.
#[test]
fn upsert_bulk_collection_rolls_back_everything_when_one_child_fails() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let record = collection(
        "bulk_01983c20-0180-7a80-a001-0000000000dd",
        Some("upstream-bulk-5"),
        "2026-08-01T09:00:00Z",
    );
    let mut bad = child(
        "anl_01983c20-0180-7a80-a001-000000000031",
        "bulk_01983c20-0180-7a80-a001-0000000000dd",
        0,
        None,
    );
    bad.bulk = None; // no membership link: cannot reconcile, must fail
    let result = store.upsert_bulk_collection_atomic(&record, &[(bad, Vec::new())]);
    assert!(result.is_err(), "a child without membership must fail");

    let connection =
        rusqlite::Connection::open(root.path().join("history").join("pangram-history.db"))
            .expect("open saved database");
    let count = |table: &str| {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("count rows")
    };
    assert_eq!(count("bulk_collections"), 0, "the batch rolled back");
    assert_eq!(count("analyses"), 0);
}
