//! Real-SQLite proof of the whole-collection write path (contracts.md 14.2
//! note): one bulk job owns one stored row reconciled by `upstream_bulk_id`,
//! refreshed atomically with its children and their observation rows, with
//! local authorship (identity, submission outcome, save state, caller ID,
//! input content, creation time) preserved across observations.
//!
//! No mocks: every `HistoryStore` points at a real `tempfile::TempDir`.

#![forbid(unsafe_code)]

#[path = "support/history_store.rs"]
mod history_store_support;

use std::str::FromStr;

use history_store_support::{ai_result, prepared_child};
use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, BulkCounters, BulkId, CheckKind, SaveState, Sha256Hash,
    SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryErrorCode, HistoryStore, InputKind, StoredAnalysis, StoredBulkCollection,
    StoredUpstreamTask,
};

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
    let input = format!("item {index} text");
    let name = format!("item-{index}.jsonl");
    let input_sha256 = Sha256Hash::digest(&input);
    StoredAnalysis {
        id: AnalysisId::from_str(id).expect("analysis id"),
        bulk: Some((BulkId::from_str(bulk_id).expect("bulk id"), index)),
        caller_id: caller.map(str::to_owned),
        status: AnalysisStatus::Queued,
        submission_outcome: SubmissionOutcome::Accepted,
        save_state: SaveState::SavedHistory,
        input_kind: InputKind::Text,
        input_sha256,
        display_name: Some(name.clone()),
        input_json: serde_json::json!({
            "type": "text",
            "origin": "file",
            "name": name,
            "sha256": input_sha256,
            "byte_count": input.len(),
            "word_count": 3,
            "text": input
        })
        .to_string(),
        result_json: None,
        error_json: None,
        upstream_version: None,
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(timestamp("2026-08-01T08:59:00Z")),
        created_at: timestamp("2026-08-01T09:00:00Z"),
        updated_at: timestamp("2026-08-01T09:00:00Z"),
        completed_at: None,
        search_input_text: Some(input),
        search_filename: Some(format!("item-{index}.jsonl")),
        search_headline: None,
        search_source_urls: None,
    }
}

fn observation(analysis_id: AnalysisId, task_id: &str) -> StoredUpstreamTask {
    StoredUpstreamTask {
        analysis_id,
        check_kind: CheckKind::AiDetection,
        upstream_task_id: task_id.to_owned(),
        last_stage: Some("STAGE_RUNNING".to_owned()),
        observed_at: timestamp("2026-08-01T09:00:00Z"),
    }
}

fn catalog_counts(store: &HistoryStore) -> (i64, i64, i64, i64, i64) {
    store
        .with_connection(|connection| {
            let count = |table: &str| {
                connection
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .expect("count table")
            };
            (
                count("bulk_collections"),
                count("analyses"),
                count("analysis_checks"),
                count("upstream_tasks"),
                count("analysis_search"),
            )
        })
        .expect("read SQLite catalog")
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
    let children = vec![prepared_child(child(
        "anl_01983c20-0180-7a80-a001-000000000011",
        "bulk_01983c20-0180-7a80-a001-0000000000aa",
        0,
        Some("row-000"),
    ))];
    store
        .reconcile_bulk_collection_complete(&record, &children)
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

#[test]
fn list_bulk_analyses_returns_certified_stored_rows_in_membership_order() {
    const MEMBER_COUNT: i64 = 32;

    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let bulk_id = "bulk_01983c20-0180-7a80-a001-0000000000ab";
    let mut record = collection(
        bulk_id,
        Some("upstream-bulk-set-read"),
        "2026-08-01T09:00:00Z",
    );
    record.counters =
        BulkCounters::new(MEMBER_COUNT as u64, MEMBER_COUNT as u64, 0, 0).expect("counters");

    // Reverse analysis identity order relative to membership order. The
    // certification pass groups child rows by analysis identity, while this
    // public surface must still return the exact stored rows by bulk_index.
    let mut expected = (0..MEMBER_COUNT)
        .map(|index| {
            let identity_suffix = MEMBER_COUNT - index;
            child(
                &format!("anl_01983c20-0180-7a80-a001-{identity_suffix:012x}"),
                bulk_id,
                index,
                Some(&format!("row-{index:03}")),
            )
        })
        .collect::<Vec<_>>();
    let mut insertion_order = expected.clone();
    insertion_order.rotate_left(11);
    let children = insertion_order
        .into_iter()
        .map(prepared_child)
        .collect::<Vec<_>>();
    store
        .reconcile_bulk_collection_complete(&record, &children)
        .expect("save bulk fixture");

    expected.sort_by_key(|member| member.bulk.map(|(_, index)| index));
    assert_eq!(
        store
            .list_bulk_analyses(&record.id)
            .expect("list bulk members"),
        expected
    );
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
        .reconcile_bulk_collection_complete(&first, &[prepared_child(first_child)])
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
    observed_child.completed_at = Some(timestamp("2026-08-02T12:00:00Z"));
    observed_child.result_json = Some(ai_result("Observed complete"));
    observed_child.input_json = "{}".to_owned();
    observed_child.display_name = None;
    observed_child.search_input_text = None;
    observed_child.search_filename = None;
    observed_child.search_headline = Some("Observed complete".to_owned());

    store
        .reconcile_bulk_collection_complete(&refreshed, &[prepared_child(observed_child)])
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

#[test]
fn reconcile_rejects_bulk_upstream_rekey_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let original = collection(
        "bulk_01983c20-0180-7a80-a001-0000000000e1",
        Some("upstream-bulk-original"),
        "2026-08-01T09:00:00Z",
    );
    store
        .reconcile_bulk_collection_complete(
            &original,
            &[prepared_child(child(
                "anl_01983c20-0180-7a80-a001-0000000000e2",
                "bulk_01983c20-0180-7a80-a001-0000000000e1",
                0,
                Some("original-row"),
            ))],
        )
        .expect("original collection commits");
    let before_collection = store
        .get_bulk_collection(&original.id)
        .expect("read original collection");
    let before_members = store
        .list_bulk_analyses(&original.id)
        .expect("read original members");

    let mut conflicting = original.clone();
    conflicting.upstream_bulk_id = Some("upstream-bulk-replacement".to_owned());
    conflicting.status = AnalysisStatus::Succeeded;
    conflicting.updated_at = timestamp("2026-08-02T12:00:00Z");
    conflicting.completed_at = Some(timestamp("2026-08-02T12:00:00Z"));
    let error = store
        .reconcile_bulk_collection_complete(&conflicting, &[])
        .expect_err("a durable upstream identity cannot be replaced");

    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);
    assert_eq!(
        store
            .get_bulk_collection(&original.id)
            .expect("read collection after rejection"),
        before_collection,
        "the rejected reconcile must not mutate the collection"
    );
    assert_eq!(
        store
            .list_bulk_analyses(&original.id)
            .expect("read members after rejection"),
        before_members,
        "the rejected reconcile must not mutate member provenance"
    );
    assert!(
        store
            .find_bulk_collection_by_upstream("upstream-bulk-replacement")
            .expect("look up rejected identity")
            .is_none()
    );
}

#[test]
fn atomic_upsert_rejects_bulk_upstream_rekey_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let original = collection(
        "bulk_01983c20-0180-7a80-a001-0000000000e3",
        Some("upstream-bulk-atomic-original"),
        "2026-08-01T09:00:00Z",
    );
    store
        .reconcile_bulk_collection_complete(
            &original,
            &[prepared_child(child(
                "anl_01983c20-0180-7a80-a001-0000000000e4",
                "bulk_01983c20-0180-7a80-a001-0000000000e3",
                0,
                Some("original-row"),
            ))],
        )
        .expect("original collection commits");
    let before_collection = store
        .get_bulk_collection(&original.id)
        .expect("read original collection");
    let before_members = store
        .list_bulk_analyses(&original.id)
        .expect("read original members");

    let mut conflicting = original.clone();
    conflicting.upstream_bulk_id = Some("upstream-bulk-atomic-replacement".to_owned());
    conflicting.status = AnalysisStatus::Succeeded;
    conflicting.updated_at = timestamp("2026-08-02T12:00:00Z");
    conflicting.completed_at = Some(timestamp("2026-08-02T12:00:00Z"));
    let error = store
        .upsert_bulk_collection_atomic(&conflicting, &[])
        .expect_err("a durable upstream identity cannot be replaced");

    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);
    assert_eq!(
        store
            .get_bulk_collection(&original.id)
            .expect("read collection after rejection"),
        before_collection,
        "the rejected atomic upsert must not mutate the collection"
    );
    assert_eq!(
        store
            .list_bulk_analyses(&original.id)
            .expect("read members after rejection"),
        before_members,
        "the rejected atomic upsert must not mutate member provenance"
    );
    assert!(
        store
            .find_bulk_collection_by_upstream("upstream-bulk-atomic-replacement")
            .expect("look up rejected identity")
            .is_none()
    );
}

#[test]
fn atomic_upsert_allows_bulk_upstream_identity_enrichment_and_replay() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let mut original = collection(
        "bulk_01983c20-0180-7a80-a001-0000000000e5",
        None,
        "2026-08-01T09:00:00Z",
    );
    original.submission_outcome = SubmissionOutcome::AcceptanceUnknown;
    let mut original_child = child(
        "anl_01983c20-0180-7a80-a001-0000000000e6",
        "bulk_01983c20-0180-7a80-a001-0000000000e5",
        0,
        Some("original-row"),
    );
    original_child.submission_outcome = SubmissionOutcome::AcceptanceUnknown;
    store
        .reconcile_bulk_collection_complete(&original, &[prepared_child(original_child)])
        .expect("local collection commits");

    let mut enriched = original.clone();
    enriched.upstream_bulk_id = Some("upstream-bulk-enriched".to_owned());
    enriched.submission_outcome = SubmissionOutcome::Accepted;
    enriched.updated_at = timestamp("2026-08-02T12:00:00Z");
    store
        .upsert_bulk_collection_atomic(&enriched, &[])
        .expect("missing upstream identity is enriched");
    let after_enrichment = store
        .get_bulk_collection(&original.id)
        .expect("read enriched collection");
    assert_eq!(
        after_enrichment.upstream_bulk_id.as_deref(),
        Some("upstream-bulk-enriched")
    );

    store
        .upsert_bulk_collection_atomic(&enriched, &[])
        .expect("same upstream identity is idempotent");
    assert_eq!(
        store
            .get_bulk_collection(&original.id)
            .expect("read replayed collection"),
        after_enrichment
    );
}

#[test]
fn complete_reconcile_rejects_swapped_child_observations_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let mut record = collection(
        "bulk_01983c20-0180-7a80-a001-0000000000f0",
        Some("upstream-bulk-swapped-observations"),
        "2026-08-01T09:00:00Z",
    );
    record.counters = BulkCounters::new(2, 2, 0, 0).expect("counters");
    let first = child(
        "anl_01983c20-0180-7a80-a001-0000000000f1",
        "bulk_01983c20-0180-7a80-a001-0000000000f0",
        0,
        Some("row-000"),
    );
    let second = child(
        "anl_01983c20-0180-7a80-a001-0000000000f2",
        "bulk_01983c20-0180-7a80-a001-0000000000f0",
        1,
        Some("row-001"),
    );
    let first_observations = vec![observation(first.id, "task-first")];
    let second_observations = vec![observation(second.id, "task-second")];
    let (first, first_checks, _) = prepared_child(first);
    let (second, second_checks, _) = prepared_child(second);

    assert_eq!(catalog_counts(&store), (0, 0, 0, 0, 0));
    let error = store
        .reconcile_bulk_collection_complete(
            &record,
            &[
                (first, first_checks, second_observations),
                (second, second_checks, first_observations),
            ],
        )
        .expect_err("swapped observation owners must fail closed");

    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);
    assert_eq!(
        catalog_counts(&store),
        (0, 0, 0, 0, 0),
        "the invalid complete reconciliation must not write any SQLite row"
    );
}

#[test]
fn complete_bulk_reconcile_rejects_duplicate_observation_kinds_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let record = collection(
        "bulk_01983c20-0180-7a80-a001-0000000000f6",
        Some("upstream-bulk-duplicate-observations"),
        "2026-08-01T09:00:00Z",
    );
    let member = child(
        "anl_01983c20-0180-7a80-a001-0000000000f7",
        "bulk_01983c20-0180-7a80-a001-0000000000f6",
        0,
        Some("row-000"),
    );
    let mut first = observation(member.id, "task-first");
    first.last_stage = Some("STAGE_FIRST".to_owned());
    let mut second = observation(member.id, "task-second");
    second.last_stage = Some("STAGE_SECOND".to_owned());
    let (member, checks, _) = prepared_child(member);

    assert_eq!(catalog_counts(&store), (0, 0, 0, 0, 0));
    let error = store
        .reconcile_bulk_collection_complete(&record, &[(member, checks, vec![first, second])])
        .expect_err("same-kind observations must fail before either upsert");

    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);
    assert_eq!(
        catalog_counts(&store),
        (0, 0, 0, 0, 0),
        "duplicate identities and stages must not hybridize a bulk child observation"
    );
}

#[test]
fn atomic_bulk_upsert_rejects_foreign_observation_owner_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let record = collection(
        "bulk_01983c20-0180-7a80-a001-0000000000f3",
        Some("upstream-bulk-foreign-observation"),
        "2026-08-01T09:00:00Z",
    );
    let member = child(
        "anl_01983c20-0180-7a80-a001-0000000000f4",
        "bulk_01983c20-0180-7a80-a001-0000000000f3",
        0,
        Some("row-000"),
    );
    let foreign_owner =
        AnalysisId::from_str("anl_01983c20-0180-7a80-a001-0000000000f5").expect("analysis id");

    assert_eq!(catalog_counts(&store), (0, 0, 0, 0, 0));
    let error = store
        .upsert_bulk_collection_atomic(
            &record,
            &[(
                member,
                vec![observation(foreign_owner, "task-foreign-owner")],
            )],
        )
        .expect_err("a foreign observation owner must fail closed");

    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);
    assert_eq!(
        catalog_counts(&store),
        (0, 0, 0, 0, 0),
        "the invalid public atomic upsert must not write any SQLite row"
    );
}

#[derive(Clone, Copy, Debug)]
enum BulkMutation {
    AtomicUpsert,
    AtomicReconcile,
    CompleteReconcile,
}

fn invoke_bulk_mutation(
    store: &mut HistoryStore,
    mutation: BulkMutation,
    collection: &StoredBulkCollection,
    member: StoredAnalysis,
) -> Result<(), microck_pangram_cli::history::HistoryError> {
    match mutation {
        BulkMutation::AtomicUpsert => {
            store.upsert_bulk_collection_atomic(collection, &[(member, Vec::new())])
        }
        BulkMutation::AtomicReconcile => store
            .reconcile_bulk_collection_atomic(collection, &[(member, Vec::new())])
            .map(|_| ()),
        BulkMutation::CompleteReconcile => store
            .reconcile_bulk_collection_complete(collection, &[prepared_child(member)])
            .map(|_| ()),
    }
}

fn two_collection_state(
    store: &HistoryStore,
    first: BulkId,
    second: BulkId,
) -> (
    StoredBulkCollection,
    Vec<StoredAnalysis>,
    StoredBulkCollection,
    Vec<StoredAnalysis>,
) {
    (
        store
            .get_bulk_collection(&first)
            .expect("read first collection"),
        store
            .list_bulk_analyses(&first)
            .expect("read first collection members"),
        store
            .get_bulk_collection(&second)
            .expect("read second collection"),
        store
            .list_bulk_analyses(&second)
            .expect("read second collection members"),
    )
}

#[test]
fn public_bulk_mutations_reject_contradictory_memberships_without_mutation() {
    let mutations = [
        BulkMutation::AtomicUpsert,
        BulkMutation::AtomicReconcile,
        BulkMutation::CompleteReconcile,
    ];
    for mutation in mutations {
        for invalid_membership in ["foreign collection", "out of range"] {
            let root = tempfile::tempdir().unwrap();
            let mut store = HistoryStore::open(root.path()).expect("open history store");
            let first = collection(
                "bulk_01983c20-0180-7a80-a001-000000000101",
                Some("upstream-membership-first"),
                "2026-08-01T09:00:00Z",
            );
            let second = collection(
                "bulk_01983c20-0180-7a80-a001-000000000102",
                Some("upstream-membership-second"),
                "2026-08-01T09:00:00Z",
            );
            store
                .reconcile_bulk_collection_complete(
                    &first,
                    &[prepared_child(child(
                        "anl_01983c20-0180-7a80-a001-000000000103",
                        "bulk_01983c20-0180-7a80-a001-000000000101",
                        0,
                        Some("first-row"),
                    ))],
                )
                .expect("seed first collection");
            store
                .reconcile_bulk_collection_complete(
                    &second,
                    &[prepared_child(child(
                        "anl_01983c20-0180-7a80-a001-000000000104",
                        "bulk_01983c20-0180-7a80-a001-000000000102",
                        0,
                        Some("second-row"),
                    ))],
                )
                .expect("seed second collection");
            let before = two_collection_state(&store, first.id, second.id);

            let (membership, index) = if invalid_membership == "foreign collection" {
                (second.id.to_string(), 0)
            } else {
                (first.id.to_string(), 1)
            };
            let mut incoming = child(
                "anl_01983c20-0180-7a80-a001-000000000105",
                &membership,
                index,
                Some("contradictory-row"),
            );
            incoming.status = AnalysisStatus::Running;
            incoming.updated_at = timestamp("2026-08-02T12:00:00Z");
            let mut refreshed = first.clone();
            refreshed.status = AnalysisStatus::Succeeded;
            refreshed.counters = BulkCounters::new(1, 1, 1, 0).expect("counters");
            refreshed.updated_at = timestamp("2026-08-02T12:00:00Z");
            refreshed.completed_at = Some(timestamp("2026-08-02T12:00:00Z"));

            let error = invoke_bulk_mutation(&mut store, mutation, &refreshed, incoming)
                .expect_err("contradictory child membership must fail closed");

            assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);
            assert_eq!(
                two_collection_state(&store, first.id, second.id),
                before,
                "{mutation:?} must not mutate either collection for {invalid_membership}"
            );
        }
    }
}
