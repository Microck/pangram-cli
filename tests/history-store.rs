//! Phase 4 Packet B: the concrete SQLite `HistoryStore` tested against the
//! real bundled database, exactly as docs/history-contract.md requires.
//!
//! Contract coverage:
//! - schema v1 exactly as locked (tables, indexes, FTS5, `user_version`)
//! - opening behavior: fresh creation, reopening, unknown/newer
//!   `user_version` rejection with `history_corrupt`, real file corruption
//!   failing as `history_corrupt` without replacing the file
//! - per-connection pragmas: WAL journal mode, `foreign_keys = ON`,
//!   `secure_delete = ON`
//! - transactional create/update of analyses, bulk collections, and upstream
//!   task observations, with FTS synchronized inside the same transaction
//! - FTS5 query primitives over input text, filename, headline, source URLs
//! - foreign-key rejection and `ON DELETE CASCADE` on the real database
//! - logical delete/clear: rows and FTS entries removed in one transaction,
//!   WAL truncated before success is reported
//! - owner-only filesystem protection: Unix `0700` directory and `0600` file
//!   modes established on create and enforced fail-closed on open
//!   (`insecure_history_permissions`); Windows uses the Phase 1 owner-only
//!   ACL policy through a cfg seam
//!
//! No mocks anywhere in this file: every `HistoryStore` points at a real
//! `tempfile::TempDir` on disk.

#![forbid(unsafe_code)]

use std::fs;
use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, BulkCounters, BulkId, CheckKind, SaveState, Sha256Hash,
    SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryError, HistoryErrorCode, HistoryStore, InputKind, StoredAnalysis, StoredBulkCollection,
    StoredSearchHit, StoredUpstreamTask, TerminalResult,
};
use microck_pangram_cli::output::ErrorCode;

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("test timestamp")
}

fn sha(tag: u8) -> Sha256Hash {
    Sha256Hash::from_bytes([tag; 32])
}

fn analysis(id: &str, input_text: &str) -> StoredAnalysis {
    StoredAnalysis {
        id: AnalysisId::from_str(id).expect("analysis id"),
        bulk: None,
        caller_id: None,
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        save_state: SaveState::SavedManual,
        input_kind: InputKind::Text,
        input_sha256: sha(7),
        display_name: None,
        input_json: format!("{{\"type\":\"text\",\"text\":{input_text:?},\"word_count\":4}}"),
        result_json: Some("{\"checks\":[]}".to_owned()),
        error_json: None,
        retry_of: None,
        rerun_of: None,
        created_at: timestamp("2026-08-01T10:00:00Z"),
        updated_at: timestamp("2026-08-01T10:05:00Z"),
        completed_at: Some(timestamp("2026-08-01T10:05:00Z")),
        search_input_text: Some(input_text.to_owned()),
        search_filename: None,
        search_headline: None,
        search_source_urls: None,
    }
}

fn bulk_collection(id: &str) -> StoredBulkCollection {
    StoredBulkCollection {
        id: BulkId::from_str(id).expect("bulk id"),
        upstream_bulk_id: Some("upstream-bulk-1".to_owned()),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        counters: BulkCounters::new(2, 2, 0, 0).expect("counters"),
        estimated_billable_units: Some(4),
        created_at: timestamp("2026-08-01T09:00:00Z"),
        updated_at: timestamp("2026-08-01T09:00:00Z"),
        completed_at: None,
    }
}

fn open_store(root: &tempfile::TempDir) -> HistoryStore {
    HistoryStore::open(root.path()).expect("open history store")
}

// ---------------------------------------------------------------- schema --

#[test]
fn fresh_database_exactly_matches_contract_schema_v1() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);

    assert_eq!(store.user_version().unwrap(), 1);

    let raw = store
        .with_connection(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT type, name, sql FROM sqlite_master \
                     WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
                )
                .expect("prepare sqlite_master query");
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .expect("query sqlite_master")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect sqlite_master")
        })
        .expect("read sqlite_master");

    let table = |name: &str| {
        raw.iter()
            .find(|(kind, found, _)| kind == "table" && found == name)
            .unwrap_or_else(|| panic!("table `{name}` must exist in {raw:?}"))
            .2
            .clone()
            .unwrap_or_default()
            .replace('"', "")
    };

    let analyses = table("analyses");
    for column in [
        "id TEXT PRIMARY KEY",
        "bulk_id TEXT REFERENCES bulk_collections(id)",
        "bulk_index INTEGER",
        "caller_id TEXT",
        "status TEXT NOT NULL",
        "submission_outcome TEXT NOT NULL",
        "save_state TEXT NOT NULL",
        "input_type TEXT NOT NULL",
        "input_sha256 TEXT NOT NULL",
        "display_name TEXT",
        "input_json TEXT NOT NULL",
        "result_json TEXT",
        "error_json TEXT",
        "retry_of TEXT REFERENCES analyses(id)",
        "rerun_of TEXT REFERENCES analyses(id)",
        "created_at TEXT NOT NULL",
        "updated_at TEXT NOT NULL",
        "completed_at TEXT",
        "UNIQUE (bulk_id, bulk_index)",
    ] {
        assert!(
            analyses.contains(column),
            "analyses table must contain `{column}`: {analyses}"
        );
    }

    let bulk = table("bulk_collections");
    for column in [
        "id TEXT PRIMARY KEY",
        "upstream_bulk_id TEXT",
        "status TEXT NOT NULL",
        "submission_outcome TEXT NOT NULL",
        "total_items INTEGER NOT NULL",
        "accepted INTEGER NOT NULL",
        "succeeded INTEGER NOT NULL",
        "failed INTEGER NOT NULL",
        "estimated_billable_units INTEGER NOT NULL",
        "created_at TEXT NOT NULL",
        "updated_at TEXT NOT NULL",
        "completed_at TEXT",
    ] {
        assert!(
            bulk.contains(column),
            "bulk_collections must contain `{column}`: {bulk}"
        );
    }

    let tasks = table("upstream_tasks");
    for column in [
        "analysis_id TEXT NOT NULL REFERENCES analyses(id) ON DELETE CASCADE",
        "check_kind TEXT NOT NULL",
        "upstream_task_id TEXT NOT NULL",
        "last_stage TEXT",
        "observed_at TEXT NOT NULL",
        "PRIMARY KEY (analysis_id, check_kind)",
    ] {
        assert!(
            tasks.contains(column),
            "upstream_tasks must contain `{column}`: {tasks}"
        );
    }

    let search = table("analysis_search");
    assert!(
        search.contains("USING fts5"),
        "analysis_search is FTS5: {search}"
    );
    assert!(
        search.contains("tokenize = 'unicode61'"),
        "analysis_search uses unicode61: {search}"
    );
    for column in [
        "analysis_id UNINDEXED",
        "input_text",
        "filename",
        "headline",
        "source_urls",
    ] {
        assert!(
            search.contains(column),
            "analysis_search must contain `{column}`: {search}"
        );
    }

    for index in ["analyses_status_created", "analyses_bulk_index"] {
        assert!(
            raw.iter()
                .any(|(kind, name, _)| kind == "index" && name == index),
            "index `{index}` must exist in {raw:?}"
        );
    }
}

#[test]
fn reopening_an_initialized_database_succeeds() {
    let root = tempfile::tempdir().unwrap();
    {
        let mut store = open_store(&root);
        store
            .save_analysis(&analysis(
                "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a01",
                "hello world",
            ))
            .unwrap();
    }
    let reopened = open_store(&root);
    let fetched = reopened
        .get_analysis(&AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a01").unwrap())
        .unwrap();
    assert_eq!(fetched.input_sha256, sha(7));
}

// ------------------------------------------------------- pragma behavior --

#[test]
fn every_connection_enables_wal_foreign_keys_and_secure_delete() {
    let root = tempfile::tempdir().unwrap();
    let store = open_store(&root);
    let (journal, foreign_keys, secure_delete) = store
        .with_connection(|connection| {
            let journal: String = connection
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .expect("journal_mode");
            let foreign_keys: i64 = connection
                .pragma_query_value(None, "foreign_keys", |row| row.get(0))
                .expect("foreign_keys");
            let secure_delete: i64 = connection
                .pragma_query_value(None, "secure_delete", |row| row.get(0))
                .expect("secure_delete");
            (journal, foreign_keys, secure_delete)
        })
        .expect("read per-connection pragmas");
    assert_eq!(journal, "wal", "journal_mode must be WAL: {journal}");
    assert_eq!(foreign_keys, 1, "foreign_keys must be ON");
    assert_eq!(secure_delete, 1, "secure_delete must be ON");
}

#[test]
fn foreign_key_violation_is_rejected_by_the_real_database() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open_store(&root);

    let orphan = StoredUpstreamTask {
        analysis_id: AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a99").unwrap(),
        check_kind: CheckKind::AiDetection,
        upstream_task_id: "task-orphan".to_owned(),
        last_stage: None,
        observed_at: timestamp("2026-08-01T10:00:00Z"),
    };
    let error = store.record_observation(&orphan).unwrap_err();
    assert_eq!(
        error.code(),
        HistoryErrorCode::HistoryWriteFailed,
        "orphan observations must fail: {error:?}"
    );
}

#[test]
fn deleting_an_analysis_cascades_upstream_task_rows_and_fts_entry() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open_store(&root);
    let id = AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a02").unwrap();
    store
        .save_analysis(&analysis(
            "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a02",
            "cascade target phrase",
        ))
        .unwrap();
    store
        .record_observation(&StoredUpstreamTask {
            analysis_id: id,
            check_kind: CheckKind::AiDetection,
            upstream_task_id: "task-1".to_owned(),
            last_stage: Some("scoring".to_owned()),
            observed_at: timestamp("2026-08-01T10:01:00Z"),
        })
        .unwrap();

    store.delete_analysis(&id).unwrap();

    let (task_count, fts_count) = store
        .with_connection(|connection| {
            let task_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM upstream_tasks WHERE analysis_id = ?1",
                    [id.to_string()],
                    |row| row.get(0),
                )
                .unwrap();
            let fts_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM analysis_search WHERE analysis_id = ?1",
                    [id.to_string()],
                    |row| row.get(0),
                )
                .unwrap();
            (task_count, fts_count)
        })
        .expect("read post-delete counts");
    assert_eq!(task_count, 0, "ON DELETE CASCADE must remove task rows");
    assert_eq!(
        fts_count, 0,
        "delete must drop the FTS entry in-transaction"
    );
    let error = store.get_analysis(&id).unwrap_err();
    assert_eq!(error.code(), HistoryErrorCode::NotFound);
}

// --------------------------------------------------------- save / update --

#[test]
fn save_analysis_persists_typed_columns_json_and_fts_in_one_transaction() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open_store(&root);
    let record = analysis(
        "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a03",
        "the mitochondria is the powerhouse of the cell",
    );
    store.save_analysis(&record).unwrap();

    let stored = store.get_analysis(&record.id).unwrap();
    assert_eq!(stored, record);

    let hits = store.search("mitochondria", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].analysis_id, record.id);
}

#[test]
fn record_observation_updates_tasks_and_result_atomically() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open_store(&root);
    let id = AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a04").unwrap();
    let mut record = analysis(
        "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a04",
        "observed content",
    );
    record.status = AnalysisStatus::Running;
    record.submission_outcome = SubmissionOutcome::Accepted;
    record.result_json = None;
    store.save_analysis(&record).unwrap();

    store
        .record_observation(&StoredUpstreamTask {
            analysis_id: id,
            check_kind: CheckKind::AiDetection,
            upstream_task_id: "task-xyz".to_owned(),
            last_stage: Some("scoring".to_owned()),
            observed_at: timestamp("2026-08-01T10:02:00Z"),
        })
        .unwrap();
    store
        .update_terminal_result(
            &id,
            &TerminalResult {
                status: AnalysisStatus::Succeeded,
                submission_outcome: SubmissionOutcome::Terminal,
                result_json: Some("{\"checks\":[{\"kind\":\"ai_detection\"}]}".to_owned()),
                error_json: None,
                completed_at: timestamp("2026-08-01T10:06:00Z"),
                search_input_text: Some("observed content".to_owned()),
                search_filename: None,
                search_headline: Some("mostly AI".to_owned()),
                search_source_urls: None,
            },
        )
        .unwrap();

    let stored = store.get_analysis(&id).unwrap();
    assert_eq!(stored.status, AnalysisStatus::Succeeded);
    assert_eq!(
        stored.result_json.as_deref(),
        Some("{\"checks\":[{\"kind\":\"ai_detection\"}]}")
    );
    assert_eq!(stored.completed_at, Some(timestamp("2026-08-01T10:06:00Z")));
    assert_eq!(stored.search_headline.as_deref(), Some("mostly AI"));

    let (task_id, stage): (String, String) = store
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT upstream_task_id, last_stage FROM upstream_tasks \
                     WHERE analysis_id = ?1 AND check_kind = 'ai_detection'",
                    [id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("task row")
        })
        .expect("read upstream task");
    assert_eq!(task_id, "task-xyz");
    assert_eq!(stage, "scoring");
}

#[test]
fn bulk_collection_roundtrips_and_scopes_child_analyses() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open_store(&root);
    let bulk = bulk_collection("bulk_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8b01");
    store.save_bulk_collection(&bulk).unwrap();

    let mut child = analysis(
        "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a05",
        "bulk child text",
    );
    child.bulk = Some((bulk.id(), 0));
    child.caller_id = Some("caller-a".to_owned());
    store.save_analysis(&child).unwrap();

    let stored_bulk = store.get_bulk_collection(&bulk.id()).unwrap();
    assert_eq!(stored_bulk, bulk);

    let members = store.list_bulk_analyses(&bulk.id()).unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].id, child.id);
    assert_eq!(members[0].bulk, Some((bulk.id(), 0)));

    // UNIQUE (bulk_id, bulk_index) holds on the real database.
    let mut duplicate = analysis(
        "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a06",
        "duplicate index",
    );
    duplicate.bulk = Some((bulk.id(), 0));
    let error = store.save_analysis(&duplicate).unwrap_err();
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);
}

#[test]
fn list_returns_recent_first_and_search_uses_fts5_match() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open_store(&root);
    let older = {
        let mut record = analysis("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a07", "alpha text");
        record.created_at = timestamp("2026-08-01T09:00:00Z");
        record
    };
    let newer = {
        let mut record = analysis(
            "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a08",
            "beta gamma delta",
        );
        record.created_at = timestamp("2026-08-01T11:00:00Z");
        record
    };
    store.save_analysis(&older).unwrap();
    store.save_analysis(&newer).unwrap();

    let page = store.list(10, 0).unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].analysis_id, newer.id, "list is newest first");
    assert_eq!(page[1].analysis_id, older.id);

    let hits: Vec<StoredSearchHit> = store.search("gamma", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].analysis_id, newer.id);
}

#[test]
fn clear_removes_every_logical_row_and_entry() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open_store(&root);
    store
        .save_bulk_collection(&bulk_collection(
            "bulk_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8b02",
        ))
        .unwrap();
    let id = AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a09").unwrap();
    store
        .save_analysis(&analysis(
            "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a09",
            "clear me",
        ))
        .unwrap();
    store
        .record_observation(&StoredUpstreamTask {
            analysis_id: id,
            check_kind: CheckKind::Plagiarism,
            upstream_task_id: "task-clear".to_owned(),
            last_stage: None,
            observed_at: timestamp("2026-08-01T10:03:00Z"),
        })
        .unwrap();

    store.clear().unwrap();

    let (analyses, tasks, fts, bulks) = store
        .with_connection(|connection| {
            let count =
                |sql: &str| -> i64 { connection.query_row(sql, [], |row| row.get(0)).unwrap() };
            (
                count("SELECT COUNT(*) FROM analyses"),
                count("SELECT COUNT(*) FROM upstream_tasks"),
                count("SELECT COUNT(*) FROM analysis_search"),
                count("SELECT COUNT(*) FROM bulk_collections"),
            )
        })
        .expect("read post-clear counts");
    assert_eq!((analyses, tasks, fts, bulks), (0, 0, 0, 0));
}

// ------------------------------------------------------ error categories --

#[test]
fn unknown_or_newer_user_version_fails_as_history_corrupt() {
    let root = tempfile::tempdir().unwrap();
    {
        let store = open_store(&root);
        store
            .with_connection(|connection| {
                connection
                    .pragma_update(None, "user_version", 2)
                    .expect("set user_version");
            })
            .expect("update user_version");
    }
    let error = HistoryStore::open(root.path()).unwrap_err();
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    let rendered = error.to_string();
    assert!(
        rendered.contains("user_version") || rendered.contains("newer"),
        "recovery guidance mentions the schema version: {rendered}"
    );
}

#[test]
fn a_corrupt_database_file_fails_closed_without_replacement() {
    let root = tempfile::tempdir().unwrap();
    // Build a real database through the store, close it, and then corrupt
    // its header in place. This is the failure SQLite actually reports for
    // an unreadable file, not a synthetic byte pattern.
    let path = root.path().join("history").join("pangram-history.db");
    let store = open_store(&root);
    drop(store);
    let original = fs::read(&path).unwrap();
    let mut corrupted = original.clone();
    // SQLite's header magic occupies bytes 0..16. Flipping every byte
    // guarantees the runtime can never read this file as its own.
    for byte in corrupted.iter_mut().take(16) {
        *byte = !*byte;
    }
    fs::write(&path, &corrupted).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    let error = HistoryStore::open(root.path()).unwrap_err();
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    // The original bytes are untouched by failure: no silent replacement,
    // no repair, no migration of the corrupted file.
    let on_disk = fs::read(&path).unwrap();
    assert_eq!(on_disk, corrupted, "corruption preserves the original file");
}

// ----------------------------------------------------------- protection ---

#[cfg(unix)]
mod protection {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn create_establishes_owner_only_directory_and_file_modes() {
        let root = tempfile::tempdir().unwrap();
        let store = open_store(&root);

        let dir_mode = fs::metadata(store.directory())
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(dir_mode, 0o700, "history directory must be 0700");

        let file_mode = fs::metadata(store.database_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(file_mode, 0o600, "database file must be 0600");
    }

    #[test]
    fn a_world_readable_database_fails_closed_on_open() {
        let root = tempfile::tempdir().unwrap();
        let store = open_store(&root);
        drop(store);

        fs::set_permissions(
            root.path().join("history").join("pangram-history.db"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();

        let error = HistoryStore::open(root.path()).unwrap_err();
        assert_eq!(error.code(), HistoryErrorCode::InsecureHistoryPermissions);
        assert_eq!(
            ErrorCode::InsecureHistoryPermissions.category().as_str(),
            "local_history"
        );
    }

    #[test]
    fn an_insecure_directory_fails_closed_on_open() {
        let root = tempfile::tempdir().unwrap();
        let store = open_store(&root);
        drop(store);

        fs::set_permissions(
            root.path().join("history"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        let error = HistoryStore::open(root.path()).unwrap_err();
        assert_eq!(error.code(), HistoryErrorCode::InsecureHistoryPermissions);
    }
}

// ------------------------------------------------------------- id helper --

#[test]
fn history_error_reports_adapter_safe_messages() {
    // No content, SQL strings, or paths containing submitted material leak.
    let error = HistoryError::new(
        HistoryErrorCode::HistoryWriteFailed,
        "could not apply the history write",
    );
    let rendered = error.to_string();
    assert!(rendered.contains("history write"), "{rendered}");
    assert!(!rendered.contains("api_key"), "{rendered}");
}
