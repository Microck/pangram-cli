//! Phase 4 dependency-foundation probe for the contracted SQLite baseline.
//!
//! Architecture-spec 11.1 locks one concrete rusqlite backend with bundled
//! SQLite and FTS5. These tests prove the pinned `rusqlite = "=0.39.0"`
//! dependency (transitive `libsqlite3-sys 0.37.0`, bundled SQLite 3.51.3)
//! satisfies that contract on the real compiled library before
//! `HistoryStore` exists:
//!
//! - the bundled runtime reports `SQLITE_ENABLE_FTS5` through
//!   `PRAGMA compile_options`, matching the pinned build script's
//!   unconditional `-DSQLITE_ENABLE_FTS5` flag
//! - the exact FTS5 virtual-table statement from docs/history-contract.md
//!   executes against an in-memory database
//! - the foreign-key pragma the history contract demands on every connection
//!   is honored by the runtime (the bundled build additionally compiles with
//!   `SQLITE_DEFAULT_FOREIGN_KEYS=1`, so enforcement is on by default)
//!
//! No schema, `HistoryStore`, or persistence behavior is implemented here;
//! this is the smallest evidence-first guard for the locked dependency
//! selection and must keep passing after the store lands.

#![forbid(unsafe_code)]

use rusqlite::Connection;

#[test]
fn bundled_sqlite_reports_fts5_compile_option() {
    let connection = Connection::open_in_memory().expect("open in-memory database");

    let mut statement = connection
        .prepare("PRAGMA compile_options")
        .expect("prepare compile_options pragma");
    let options: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .expect("query compile_options")
        .collect::<Result<_, _>>()
        .expect("collect compile_options");

    assert!(
        options.iter().any(|option| option == "ENABLE_FTS5"),
        "bundled SQLite MUST be compiled with ENABLE_FTS5; got {options:?}"
    );
}

#[test]
fn history_contract_fts5_virtual_table_statement_executes() {
    let connection = Connection::open_in_memory().expect("open in-memory database");

    // The exact FTS5 shape locked by docs/history-contract.md.
    connection
        .execute_batch(
            "CREATE VIRTUAL TABLE analysis_search USING fts5(
              analysis_id UNINDEXED,
              input_text,
              filename,
              headline,
              source_urls,
              tokenize = 'unicode61'
            );",
        )
        .expect("create the contracted FTS5 virtual table");

    let table_name: String = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'analysis_search'",
            [],
            |row| row.get(0),
        )
        .expect("FTS5 virtual table is registered");
    assert_eq!(table_name, "analysis_search");
}

#[test]
fn foreign_keys_pragma_is_honored_by_the_runtime() {
    let connection = Connection::open_in_memory().expect("open in-memory database");

    // The history contract requires `PRAGMA foreign_keys = ON` on every
    // connection before touching application tables. The pinned bundled
    // SQLite 3.51.3 build also compiles with `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`
    // (libsqlite3-sys 0.37.0 build.rs), so this runtime reports enforcement
    // from the start; the explicit pragma remains the HistoryStore's
    // contract.
    let foreign_keys: i64 = connection
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .expect("read foreign_keys pragma");
    assert_eq!(
        foreign_keys, 1,
        "bundled SQLite MUST enforce foreign keys by default"
    );

    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("enable foreign_keys pragma");
    let foreign_keys: i64 = connection
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .expect("read foreign_keys pragma after enabling");
    assert_eq!(foreign_keys, 1, "foreign_keys pragma MUST report ON");
}
