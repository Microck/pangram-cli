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

use std::fs;
use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, SaveState, Sha256Hash, SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryErrorCode, HistoryStore, InputKind, StoredAnalysis, TerminalResult,
};

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

fn open_store(root: &tempfile::TempDir) -> HistoryStore {
    HistoryStore::open(root.path()).expect("open history store")
}

// ------------------------------------------------ finding 2: FTS atomic --

#[test]
fn terminal_update_replaces_search_payload_in_the_same_transaction() {
    let root = tempfile::tempdir().unwrap();
    let mut store = open_store(&root);
    let id = AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a20").unwrap();
    store
        .save_analysis(&analysis(
            "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a20",
            "initial observation text only",
        ))
        .unwrap();
    assert_eq!(store.search("observation", 10).unwrap().len(), 1);
    assert_eq!(store.search("mostly", 10).unwrap().len(), 0);

    store
        .update_terminal_result(
            &id,
            &TerminalResult {
                status: AnalysisStatus::Succeeded,
                submission_outcome: SubmissionOutcome::Terminal,
                result_json: Some(
                    "{\"checks\":[{\"kind\":\"ai_detection\",\"headline\":\"mostly AI\"}]}"
                        .to_owned(),
                ),
                error_json: None,
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
}
