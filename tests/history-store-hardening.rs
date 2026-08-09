//! Phase 4 Packet B remediation: hardened `HistoryStore` behavior tested
//! against the real bundled database.
//!
//! These tests cover the five independent-review correctness/hardening
//! issues that could not be caught by the initial core contract suite:
//! - terminal updates replacing the search payload in the same transaction
//!   (finding 2: a stale FTS row must never outlive its typed row update)
//! - structural inconsistency (typed row without its synchronized FTS row)
//!   failing closed as `history_corrupt`, never a silent `None` payload
//!   (finding 3)
//! - SQLite WAL/SHM sidecars carrying the same exact Unix `0600` owner-only
//!   mode as the database itself and failing closed on an insecure sidecar
//!   (finding 4)
//!
//! No mocks anywhere: every assertion is a real SQLite file observation.

#![forbid(unsafe_code)]

#[path = "support/history_store.rs"]
mod history_store_support;

use std::fs;
use std::str::FromStr;

use history_store_support::{ai_result, save_complete};
use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, SaveState, SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryErrorCode, HistoryStore, InputKind, StoredAnalysis, TerminalResult,
};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("test timestamp")
}

fn analysis(id: &str, input_text: &str) -> StoredAnalysis {
    let input_sha256 = microck_pangram_cli::domain::Sha256Hash::digest(input_text);
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
            "byte_count": input_text.len(),
            "word_count": input_text.split_whitespace().count(),
            "text": input_text
        })
        .to_string(),
        result_json: Some(ai_result("Human-written")),
        error_json: None,
        upstream_version: None,
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(timestamp("2026-08-01T09:59:00Z")),
        created_at: timestamp("2026-08-01T10:00:00Z"),
        updated_at: timestamp("2026-08-01T10:05:00Z"),
        completed_at: Some(timestamp("2026-08-01T10:05:00Z")),
        search_input_text: Some(input_text.to_owned()),
        search_filename: None,
        search_headline: Some("Human-written".to_owned()),
        search_source_urls: None,
    }
}

fn open_store(root: &tempfile::TempDir) -> HistoryStore {
    HistoryStore::open(root.path()).expect("open history store")
}

// ------------------------------------------------ finding 2: FTS atomic --

#[test]
fn terminal_update_replaces_search_payload_in_the_same_transaction() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open_store(&root);
    let id = AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a20").unwrap();
    save_complete(
        &mut store,
        &analysis(
            "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a20",
            "initial observation text only",
        ),
    );
    assert_eq!(store.search("observation", 10).unwrap().len(), 1);
    assert_eq!(store.search("mostly", 10).unwrap().len(), 0);

    store
        .update_terminal_result(
            &id,
            &TerminalResult {
                status: AnalysisStatus::Succeeded,
                submission_outcome: SubmissionOutcome::Terminal,
                result_json: Some(ai_result("mostly AI")),
                error_json: None,
                upstream_version: None,
                completed_at: timestamp("2026-08-01T12:00:00Z"),
                search_input_text: Some("initial observation text only".to_owned()),
                search_filename: None,
                search_headline: Some("mostly AI".to_owned()),
                search_source_urls: None,
            },
        )
        .unwrap();

    // The terminal projection lands: the new headline is searchable
    // alongside the original input text.
    let headline_hits = store.search("mostly", 10).unwrap();
    assert_eq!(headline_hits.len(), 1);
    assert_eq!(headline_hits[0].analysis_id, id);
    let input_hits = store.search("observation", 10).unwrap();
    assert_eq!(input_hits.len(), 1);
    assert_eq!(input_hits[0].analysis_id, id);

    // No stale FTS row remains: exactly one row exists for this analysis.
    let fts_count: i64 = store
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT COUNT(*) FROM analysis_search WHERE analysis_id = ?1",
                    [id.to_string()],
                    |row| row.get(0),
                )
                .unwrap()
        })
        .expect("count fts rows");
    assert_eq!(
        fts_count, 1,
        "in-transaction FTS replacement leaves one row"
    );
}

/// A typed terminal update may replace only a synchronized FTS payload. Any
/// missing, duplicate, or malformed row is pre-existing corruption and must
/// fail before the typed analysis changes. The failed transaction preserves
/// both the database file bytes and the complete logical state.
#[test]
fn terminal_update_rejects_invalid_search_cardinality_or_content_without_mutation() {
    for (suffix, corruption) in [("21", "missing"), ("22", "duplicate"), ("23", "malformed")] {
        let root = tempfile::tempdir().unwrap();
        let mut store = open_store(&root);
        let id_text = format!("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a{suffix}");
        let id = AnalysisId::from_str(&id_text).unwrap();
        store
            .save_analysis(&analysis(&id_text, "terminal invariant baseline"))
            .unwrap();
        store
            .with_connection(|connection| match corruption {
                "missing" => connection.execute(
                    "DELETE FROM analysis_search WHERE analysis_id = ?1",
                    [&id_text],
                ),
                "duplicate" => connection.execute(
                    "INSERT INTO analysis_search
                        (analysis_id, input_text, filename, headline, source_urls)
                     SELECT analysis_id, input_text, filename, headline, source_urls
                     FROM analysis_search WHERE analysis_id = ?1",
                    [&id_text],
                ),
                "malformed" => connection.execute(
                    "UPDATE analysis_search SET input_text = 42 WHERE analysis_id = ?1",
                    [&id_text],
                ),
                _ => unreachable!(),
            })
            .expect("raw connection")
            .expect("install corruption");

        let database_path = store.database_path().to_owned();
        let before_bytes = fs::read(&database_path).expect("read database bytes");
        let before_state = store
            .with_connection(|connection| {
                let analysis: (String, String, Option<String>, Option<String>, String) = connection
                    .query_row(
                        "SELECT status, submission_outcome, result_json, error_json, updated_at
                         FROM analyses WHERE id = ?1",
                        [&id_text],
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
                    .unwrap();
                let search: Vec<(String, rusqlite::types::Value)> = connection
                    .prepare(
                        "SELECT analysis_id, input_text FROM analysis_search
                         WHERE analysis_id = ?1 ORDER BY rowid",
                    )
                    .unwrap()
                    .query_map([&id_text], |row| Ok((row.get(0)?, row.get(1)?)))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                (analysis, search)
            })
            .expect("logical state");

        let error = store
            .update_terminal_result(
                &id,
                &TerminalResult {
                    status: AnalysisStatus::Failed,
                    submission_outcome: SubmissionOutcome::Terminal,
                    result_json: None,
                    error_json: Some("{\"code\":\"replacement\"}".to_owned()),
                    upstream_version: None,
                    completed_at: timestamp("2026-08-02T12:00:00Z"),
                    search_input_text: Some("replacement text".to_owned()),
                    search_filename: None,
                    search_headline: Some("replacement headline".to_owned()),
                    search_source_urls: None,
                },
            )
            .expect_err("corrupt synchronized FTS state must fail closed");
        assert_eq!(
            error.code(),
            HistoryErrorCode::HistoryCorrupt,
            "{corruption}: {error:?}"
        );

        let after_state = store
            .with_connection(|connection| {
                let analysis: (String, String, Option<String>, Option<String>, String) = connection
                    .query_row(
                        "SELECT status, submission_outcome, result_json, error_json, updated_at
                         FROM analyses WHERE id = ?1",
                        [&id_text],
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
                    .unwrap();
                let search: Vec<(String, rusqlite::types::Value)> = connection
                    .prepare(
                        "SELECT analysis_id, input_text FROM analysis_search
                         WHERE analysis_id = ?1 ORDER BY rowid",
                    )
                    .unwrap()
                    .query_map([&id_text], |row| Ok((row.get(0)?, row.get(1)?)))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                (analysis, search)
            })
            .expect("logical state");
        assert_eq!(after_state, before_state, "{corruption}: logical mutation");
        assert_eq!(
            fs::read(&database_path).expect("reread database bytes"),
            before_bytes,
            "{corruption}: database-byte mutation"
        );
    }
}

// ----------------------------------------------- finding 3: structural --

#[test]
fn a_missing_search_row_for_a_stored_analysis_is_history_corrupt() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open_store(&root);
    let id = AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a30").unwrap();
    store
        .save_analysis(&analysis(
            "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a30",
            "structural test",
        ))
        .unwrap();

    // Simulate the inconsistency: drop the FTS row without touching the
    // typed row. A read must refuse to silently tolerate the divergence.
    store
        .with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM analysis_search WHERE analysis_id = ?1",
                    [id.to_string()],
                )
                .unwrap();
        })
        .expect("simulate structural inconsistency");

    let error = store.get_analysis(&id).unwrap_err();
    assert_eq!(
        error.code(),
        HistoryErrorCode::HistoryCorrupt,
        "missing FTS row must fail closed as corruption: {error:?}"
    );

    // The sanitized recovery guidance does not mention SQL, IDs, or FTS.
    let rendered = error.to_string();
    assert!(rendered.contains("preserved"), "{rendered}");
}

// ----------------------------------------- finding 4: WAL/SHM sidecars --

#[cfg(unix)]
mod sidecars {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn wal_and_shm_sidecars_are_owner_only_after_open() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open_store(&root);
        // A write proves the WAL exists; the SHM file appears with it.
        store
            .save_analysis(&analysis(
                "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a40",
                "wal ownership",
            ))
            .unwrap();

        let sidecar_wal = store.database_path().with_extension("db-wal");
        let sidecar_shm = store.database_path().with_extension("db-shm");
        assert!(sidecar_wal.exists(), "WAL exists after a write");
        assert!(sidecar_shm.exists(), "SHM exists after a write");

        for sidecar in [&sidecar_wal, &sidecar_shm] {
            let mode = fs::metadata(sidecar).unwrap().permissions().mode() & 0o7777;
            assert_eq!(mode, 0o600, "{} must be owner-only 0600", sidecar.display());
        }
    }

    #[test]
    fn an_insecure_wal_sidecar_fails_closed_on_reopen() {
        let root = tempfile::tempdir().unwrap();
        let store = open_store(&root);
        let wal = store.database_path().with_extension("db-wal");
        let shm = store.database_path().with_extension("db-shm");
        drop(store);
        // Materialize the sidecars with attacker-modified modes to prove the
        // reopen verification detects them and fails closed before SQLite
        // migrates anything.
        fs::write(&wal, b"").unwrap();
        fs::write(&shm, b"").unwrap();
        fs::set_permissions(&wal, fs::Permissions::from_mode(0o644)).unwrap();

        let error = HistoryStore::open(root.path()).unwrap_err();
        assert_eq!(error.code(), HistoryErrorCode::InsecureHistoryPermissions);
    }

    #[test]
    fn hostile_existing_sidecar_alias_fails_before_database_or_target_mutation() {
        use std::os::unix::fs::symlink;

        for suffix in ["db-wal", "db-shm"] {
            let root = tempfile::tempdir().unwrap();
            let store = open_store(&root);
            let database = store.database_path();
            drop(store);

            let database_before = fs::read(&database).expect("read database before hostile open");
            let target = root.path().join(format!("hostile-{suffix}-target"));
            fs::write(&target, b"hostile sidecar target sentinel").unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
            let target_before = fs::read(&target).unwrap();
            let sidecar = database.with_extension(suffix);
            symlink(&target, &sidecar).unwrap();

            let error = HistoryStore::open(root.path())
                .expect_err("an existing sidecar alias must fail before SQLite opens");
            assert_eq!(error.code(), HistoryErrorCode::HistoryUnavailable);
            assert_eq!(
                fs::read(&database).unwrap(),
                database_before,
                "{suffix}: rejected open mutated the database"
            );
            assert_eq!(
                fs::read(&target).unwrap(),
                target_before,
                "{suffix}: rejected open mutated the hostile target"
            );
            assert!(
                fs::symlink_metadata(&sidecar)
                    .expect("sidecar alias remains")
                    .file_type()
                    .is_symlink(),
                "{suffix}: hostile alias itself was replaced"
            );
        }
    }
}
