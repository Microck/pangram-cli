//! Phase 4 Packet C remediation: the deterministic mixed-outcome proof that
//! one repeated-file member's real SQLite insert failure drops none of the
//! ordered tail (contracts.md 14.2 note, docs/history-contract.md).
//!
//! Unlike the poisoned-data-directory suites (where the store fails closed
//! before ever opening SQLite), this suite opens the real store and then
//! makes ONE chosen member's observation insert fail at the database itself
//! through the contracted `UNIQUE (check_kind, upstream_task_id)` constraint.
//! A pre-existing real row owns the first member's task identity, so that
//! member's transaction genuinely rolls back while the second member, with a
//! distinct task identity, commits. No mock, test-only production bypass,
//! uncontracted schema object, or fake member is involved.
//!
//! Locked semantics proven here:
//! - the trigger-matched member renders honest `ephemeral` in invocation
//!   order while the later member still persists its own row and renders its
//!   committed `saved_manual`
//! - one exit-7 failure envelope (`history_write_failed`, category
//!   `local_history`) closes the full series after every member rendered
//! - aside from the pre-existing constraint owner, the store holds exactly the
//!   later member's `analyses` row plus its own `analysis_search` FTS payload
//!   and `upstream_tasks` observation row; the failed member left no trace

#![cfg(feature = "dev-tools")]
#![cfg(unix)]

#[path = "support/history_save_env.rs"]
mod harness;

use harness::fixture::{ProtocolFixture, Step, TASK_ID};
use harness::{Isolated, analyses_rows, assert_no_leak, search_payload, stderr_text, task_rows};

use rusqlite::Connection;
use serde_json::Value;
/// Opens (creating if necessary) the test store at `data_dir` with the exact
/// same protection, pragmas, and schema version the production open applies,
/// seeded with a real-schema mirror plus one row that owns the first member's
/// upstream task identity. Returns the already-open store connection (still
/// owned by the test; the compiled binary opens its OWN second connection when
/// the command runs, after this handle is dropped).
///
/// This mirrors the store's `SCHEMA_V1` body on purpose: `store.rs` keeps
/// `SCHEMA_V1` private, and docs/history-contract.md locks it at
/// `user_version = 1`, so the fixture duplicates the five schema statements
/// verbatim and sets `user_version = 1` exactly as production does. It is a
/// fixture of the real schema under test, kept next to the one consumer so a
/// contract change to the schema forces an intentional decision here (the
/// production open would then fail its own version/`quick_check` probe).
fn prepared_store(data_dir: &std::path::Path) -> Connection {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn establish_directory(directory: &Path) {
        if !directory.exists() {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mode = std::fs::metadata(directory).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o700, "history directory must be owner-only");
    }

    let directory = data_dir.join("history");
    establish_directory(&directory);
    let database = directory.join("pangram-history.db");

    let connection = Connection::open(&database).expect("open test history database");
    // A brand-new file must be owner-only before any schema page is written,
    // matching the store's `restrict_file` before its first write.
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600))
        .expect("restrict the fresh database file to owner-only");

    // The pragma contract the store applies and reads back on every open.
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .expect("WAL mode");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys");
    connection
        .pragma_update(None, "secure_delete", true)
        .expect("secure delete");

    // The real schema body (mirror of store.rs SCHEMA_V1, user_version = 1).
    connection
        .execute_batch(
            "
CREATE TABLE bulk_collections (
  id TEXT PRIMARY KEY,
  upstream_bulk_id TEXT UNIQUE,
  status TEXT NOT NULL,
  submission_outcome TEXT NOT NULL,
  total_items INTEGER NOT NULL,
  accepted INTEGER NOT NULL,
  succeeded INTEGER NOT NULL,
  failed INTEGER NOT NULL,
  estimated_billable_units INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT
);

CREATE TABLE analyses (
  id TEXT PRIMARY KEY,
  bulk_id TEXT REFERENCES bulk_collections(id),
  bulk_index INTEGER,
  caller_id TEXT,
  status TEXT NOT NULL,
  submission_outcome TEXT NOT NULL,
  save_state TEXT NOT NULL,
  input_type TEXT NOT NULL,
  input_sha256 TEXT NOT NULL,
  display_name TEXT,
  input_json TEXT NOT NULL,
  check_count INTEGER NOT NULL DEFAULT 1 CHECK (check_count BETWEEN 1 AND 2),
  result_json TEXT,
  error_json TEXT,
  upstream_version TEXT,
  retry_of TEXT REFERENCES analyses(id) ON DELETE SET NULL,
  rerun_of TEXT REFERENCES analyses(id) ON DELETE SET NULL,
  submitted_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT,
  UNIQUE (bulk_id, bulk_index)
);

CREATE TABLE upstream_tasks (
  analysis_id TEXT NOT NULL REFERENCES analyses(id) ON DELETE CASCADE,
  check_kind TEXT NOT NULL,
  upstream_task_id TEXT NOT NULL,
  last_stage TEXT,
  observed_at TEXT NOT NULL,
  PRIMARY KEY (analysis_id, check_kind),
  UNIQUE (check_kind, upstream_task_id)
);

CREATE TABLE analysis_checks (
  analysis_id TEXT NOT NULL REFERENCES analyses(id) ON DELETE CASCADE,
  check_index INTEGER NOT NULL,
  check_kind TEXT NOT NULL,
  status TEXT NOT NULL,
  result_json TEXT,
  error_json TEXT,
  PRIMARY KEY (analysis_id, check_index),
  UNIQUE (analysis_id, check_kind)
);

CREATE VIRTUAL TABLE analysis_search USING fts5(
  analysis_id UNINDEXED,
  input_text,
  filename,
  headline,
  source_urls,
  tokenize = 'unicode61'
);

CREATE INDEX analyses_status_created
  ON analyses(status, created_at DESC);

CREATE INDEX analyses_bulk_index
  ON analyses(bulk_id, bulk_index);
",
        )
        .expect("apply real schema");

    connection
        .execute(
            "INSERT INTO analyses (
               id, status, submission_outcome, save_state, input_type,
               input_sha256, input_json, created_at, updated_at
             ) VALUES (
               'seed-constraint-owner', 'running', 'accepted', 'saved_history',
               'text', 'seed-sha256', '{}',
               '2026-08-04T00:00:00Z', '2026-08-04T00:00:00Z'
             )",
            [],
        )
        .expect("seed the constraint-owning analysis");
    connection
        .execute(
            "INSERT INTO upstream_tasks (
               analysis_id, check_kind, upstream_task_id, observed_at
             ) VALUES (
               'seed-constraint-owner', 'ai_detection', ?1,
               '2026-08-04T00:00:00Z'
             )",
            [TASK_ID],
        )
        .expect("seed the first member's conflicting task identity");

    connection
        .pragma_update(None, "user_version", 1u32)
        .expect("pin schema user_version = 1");

    // Read back the runtime state exactly as the store does (WAL is proven,
    // never assumed).
    let journal: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .expect("read back journal_mode");
    assert!(
        journal.eq_ignore_ascii_case("wal"),
        "WAL is proven, not assumed"
    );
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("read back user_version");
    assert_eq!(version, 1, "schema version is pinned to 1");
    let quick_check: String = connection
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .expect("quick_check integrity probe");
    assert_eq!(quick_check, "ok", "the seeded store is SQLite-healthy");

    connection
}

/// One repeated-file member's real SQLite insert fails while a later member
/// still persists and renders in order. The failure is a genuine database
/// rejection of the first member's observation (its task identity is already
/// owned by another row), so the covariance is faithful: the first member
/// renders `ephemeral`, the second persists its row with `saved_manual`, and
/// one exit-7 `history_write_failed` envelope closes the series.
#[tokio::test(flavor = "multi_thread")]
async fn manual_mixed_outcome_one_member_insert_fails_later_member_persists_in_order() {
    let first_text = "First completed member whose own row insert the store rejects";
    let second_text = "Second completed member whose row persists after the failure";

    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(harness::fixture::pangram4_success(first_text)));
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": "task-456"})));
    fixture.on_poll(Step::Json(harness::fixture::pangram4_success(second_text)));

    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.txt");
    let second = root.path().join("second.txt");
    std::fs::write(&first, first_text).unwrap();
    std::fs::write(&second, second_text).unwrap();

    let isolated = Isolated::new();
    // Seed the real store with the member-selective unique-key conflict, then
    // drop the test handle so the compiled binary opens its own connection.
    let store_connection = prepared_store(&isolated.data);
    drop(store_connection);

    let output = isolated
        .command(fixture.base_url())
        .args([
            "detect",
            "--save",
            "--file",
            first.to_str().unwrap(),
            "--file",
            second.to_str().unwrap(),
        ])
        .output()
        .expect("run pangram detect --save with one member's insert rejected by the real store");

    // Exit: the first member's save failure is the canonical local error.
    assert_eq!(
        output.status.code(),
        Some(7),
        "one member's real store failure exits 7"
    );

    // Ordered output: three envelopes (first, second, then the failure) with
    // the failed member honest `ephemeral` and the later member honestly
    // `saved_manual`, in invocation order, in full.
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "two completed envelopes then the failure envelope"
    );
    let first_env: Value = serde_json::from_str(lines[0]).unwrap();
    let second_env: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(
        first_env["data"]["status"], "succeeded",
        "the first member's remote result is preserved, never dropped"
    );
    assert_eq!(
        first_env["data"]["save_state"], "ephemeral",
        "the trigger-matched member stays honestly ephemeral"
    );
    assert_eq!(
        second_env["data"]["status"], "succeeded",
        "the later member's remote result is preserved"
    );
    assert_eq!(
        second_env["data"]["save_state"], "saved_manual",
        "the later member still persisted its own row"
    );
    let failure: Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(failure["command"], "detect");
    assert_eq!(failure["error"]["category"], "local_history");
    assert_eq!(
        failure["error"]["code"], "history_write_failed",
        "the real SQLite rejection surfaces as history_write_failed"
    );

    // The JSON error surface keeps stderr clean and nothing leaks.
    let stderr = stderr_text(&output);
    assert!(
        stderr.is_empty(),
        "JSON error surface keeps stderr clean: {stderr}"
    );
    assert_no_leak(&output);

    // Store state: the pre-existing constraint owner and exactly ONE new
    // `analyses` row (the later member), its FTS payload, and its observation
    // row. The failed member left no trace.
    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(
        rows.len(),
        2,
        "only the seed and later member exist (the rejected member committed nothing)"
    );
    let surviving = rows
        .iter()
        .find(|row| row.3 == "saved_manual")
        .expect("the later member is the one newly saved row");
    assert_eq!(
        surviving.3, "saved_manual",
        "the surviving row carries the manual save state"
    );
    assert_eq!(
        surviving.1, "succeeded",
        "the surviving row carries the terminal outcome"
    );
    let payloads = search_payload(&connection);
    assert_eq!(
        payloads.len(),
        1,
        "the later member's FTS payload exists; the failed member has none"
    );
    let tasks = task_rows(&connection);
    assert_eq!(
        tasks.len(),
        2,
        "the seed and later member task observations persist"
    );
    assert!(
        tasks.iter().any(|task| task.2 == "task-456"),
        "the later member owns its distinct task observation"
    );
    let collections = harness::bulk_collection_rows(&connection);
    assert!(
        collections.is_empty(),
        "a plain detect run persists no collection rows"
    );
    drop(connection);

    fixture.shutdown().await;
}
