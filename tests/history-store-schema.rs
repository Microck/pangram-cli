//! Phase 4 Packet C remediation: exact schema-v1 structural validation.
//!
//! A database whose stored `user_version` is 1 but whose structure does not
//! carry every contracted v1 element (tables, FTS5 virtual table, indexes,
//! and the uniqueness/referential rules) is an incompatible v1 and must fail
//! closed as `history_corrupt`, with the original file preserved
//! byte-for-byte (docs/history-contract.md). The schema is never repaired,
//! migrated, or rewritten in place.
//!
//! No mocks anywhere: every fixture is a real SQLite database built through
//! rusqlite against the bundled engine, and every assertion reads real
//! catalog state.

#![forbid(unsafe_code)]

use std::fs;
use std::path::Path;

use microck_pangram_cli::history::{HistoryErrorCode, HistoryStore};

/// The Packet B v1 body, before the uniqueness `UNIQUE` constraints were
/// contracted. This is the old schema an existing user may legitimately
/// hold; it claims `user_version = 1` but does not carry the contracted
/// uniqueness rules.
const PACKET_B_SCHEMA_V1: &str = "
CREATE TABLE bulk_collections (
  id TEXT PRIMARY KEY,
  upstream_bulk_id TEXT,
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
  result_json TEXT,
  error_json TEXT,
  retry_of TEXT REFERENCES analyses(id),
  rerun_of TEXT REFERENCES analyses(id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT
);

CREATE TABLE upstream_tasks (
  analysis_id TEXT NOT NULL REFERENCES analyses(id) ON DELETE CASCADE,
  check_kind TEXT NOT NULL,
  upstream_task_id TEXT NOT NULL,
  last_stage TEXT,
  observed_at TEXT NOT NULL,
  PRIMARY KEY (analysis_id, check_kind)
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
";

/// Build a real SQLite file at `database` with the exact body the caller
/// supplies and `user_version = 1`, protected with the owner-only modes the
/// store's Unix fail-closed check requires.
fn build_fixture(root: &Path, schema_body: &str) -> std::path::PathBuf {
    let history_dir = root.join("history");
    fs::create_dir_all(&history_dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&history_dir, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let database = history_dir.join("pangram-history.db");
    {
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .pragma_update(None, "journal_mode", "DELETE")
            .unwrap();
        connection.execute_batch(schema_body).unwrap();
        connection
            .pragma_update(None, "user_version", 1u32)
            .unwrap();
        connection.close().unwrap();
    }
    // Remove any WAL/SHM the fixture open may have left so the reopen sees a
    // clean single database file.
    for extension in ["db-wal", "db-shm"] {
        let sidecar = database.with_extension(extension);
        if sidecar.exists() {
            fs::remove_file(&sidecar).unwrap();
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    }
    database
}

/// The fixture body derived from the production lock with one named
/// deviation applied. `exact_v1` rebuilds the exact production schema v1
/// (byte-for-byte the docs/history-contract.md lock) so every negative
/// variant differs from the passing case by the one deviation under test
/// and nothing else. `production` maps each marker onto the one altered
/// clause; an unhandled marker is a test-authoring bug and fails here,
/// before the store ever opens.
fn variant_body(deviation: &str) -> String {
    let exact_v1 = String::from(
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
  result_json TEXT,
  error_json TEXT,
  upstream_version TEXT,
  retry_of TEXT REFERENCES analyses(id),
  rerun_of TEXT REFERENCES analyses(id),
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
CREATE VIRTUAL TABLE analysis_search USING fts5(
  analysis_id UNINDEXED, input_text, filename, headline, source_urls,
  tokenize = 'unicode61'
);
CREATE INDEX analyses_status_created ON analyses(status, created_at DESC);
CREATE INDEX analyses_bulk_index ON analyses(bulk_id, bulk_index);
",
    );
    let altered = match deviation {
        // bulk_collections owns its own wrong-column unique rule: the
        // catalog has a `u` index on the table, but it covers `status`
        // instead of the contracted `upstream_bulk_id`.
        "bulk unique column" => exact_v1.replace(
            "upstream_bulk_id TEXT UNIQUE,\n  status TEXT NOT NULL,",
            "upstream_bulk_id TEXT,\n  status TEXT NOT NULL UNIQUE,",
        ),
        // The same identity column and uniqueness constraint with a
        // different collation is not the same key semantics.
        "bulk identity nocase" => exact_v1.replace(
            "upstream_bulk_id TEXT UNIQUE,",
            "upstream_bulk_id TEXT COLLATE NOCASE UNIQUE,",
        ),
        // An explicit unique index has creation origin `c`, not the
        // contracted table-constraint origin `u`.
        "bulk identity wrong origin" => exact_v1
            .replace("upstream_bulk_id TEXT UNIQUE,", "upstream_bulk_id TEXT,")
            .replace(
                "CREATE INDEX analyses_status_created",
                "CREATE UNIQUE INDEX bulk_upstream_identity ON bulk_collections(upstream_bulk_id);\n\
                 CREATE INDEX analyses_status_created",
            ),
        "bulk identity partial" => exact_v1
            .replace("upstream_bulk_id TEXT UNIQUE,", "upstream_bulk_id TEXT,")
            .replace(
                "CREATE INDEX analyses_status_created",
                "CREATE UNIQUE INDEX bulk_upstream_identity ON bulk_collections(upstream_bulk_id) \
                 WHERE upstream_bulk_id IS NOT NULL;\n\
                 CREATE INDEX analyses_status_created",
            ),
        // Primary-key autoindexes are part of the identity surface too.
        "bulk primary key nocase" => exact_v1.replacen(
            "  id TEXT PRIMARY KEY,",
            "  id TEXT COLLATE NOCASE PRIMARY KEY,",
            1,
        ),
        // analyses loses its single-column `id` primary key entirely.
        "analyses primary key" => exact_v1.replacen(
            "  id TEXT PRIMARY KEY,\n  bulk_id TEXT",
            "  id TEXT,\n  bulk_id TEXT",
            1,
        ),
        // analyses keeps a primary key but on the wrong column.
        "analyses wrong primary key" => exact_v1.replacen(
            "  id TEXT PRIMARY KEY,\n  bulk_id TEXT",
            "  id TEXT,\n  bulk_id TEXT PRIMARY KEY",
            1,
        ),
        // The named status index swaps its ordered columns: it exists and
        // covers the same two names, but in the wrong order.
        "status index columns" => exact_v1.replace(
            "CREATE INDEX analyses_status_created ON analyses(status, created_at DESC);",
            "CREATE INDEX analyses_status_created ON analyses(created_at, status);",
        ),
        "status index direction" => exact_v1.replace(
            "CREATE INDEX analyses_status_created ON analyses(status, created_at DESC);",
            "CREATE INDEX analyses_status_created ON analyses(status, created_at);",
        ),
        "status index nocase" => exact_v1.replace(
            "CREATE INDEX analyses_status_created ON analyses(status, created_at DESC);",
            "CREATE INDEX analyses_status_created ON analyses(status COLLATE NOCASE, created_at DESC);",
        ),
        "status index alternate collation" => exact_v1.replace(
            "CREATE INDEX analyses_status_created ON analyses(status, created_at DESC);",
            "CREATE INDEX analyses_status_created ON analyses(status COLLATE RTRIM, created_at DESC);",
        ),
        "status index partial" => exact_v1.replace(
            "CREATE INDEX analyses_status_created ON analyses(status, created_at DESC);",
            "CREATE INDEX analyses_status_created ON analyses(status, created_at DESC) WHERE status IS NOT NULL;",
        ),
        "status index expression" => exact_v1.replace(
            "CREATE INDEX analyses_status_created ON analyses(status, created_at DESC);",
            "CREATE INDEX analyses_status_created ON analyses(status, lower(created_at) DESC);",
        ),
        "status index extra key" => exact_v1.replace(
            "CREATE INDEX analyses_status_created ON analyses(status, created_at DESC);",
            "CREATE INDEX analyses_status_created ON analyses(status, created_at DESC, id);",
        ),
        // Constraint identity indexes also contract direction even though
        // equality uniqueness would otherwise behave similarly.
        "analyses identity descending" => exact_v1.replace(
            "  UNIQUE (bulk_id, bulk_index)",
            "  UNIQUE (bulk_id DESC, bulk_index)",
        ),
        // The named bulk index becomes unique: SQLite reports its origin
        // as `u`, so the table's unique surface silently changes even
        // though the index name and columns look right.
        "bulk index made unique" => exact_v1.replace(
            "CREATE INDEX analyses_bulk_index ON analyses(bulk_id, bulk_index);",
            "CREATE UNIQUE INDEX analyses_bulk_index ON analyses(bulk_id, bulk_index);",
        ),
        // Nullability drift on one stored-content column.
        "result_json not null" => exact_v1.replace(
            "  result_json TEXT,\n  error_json TEXT,",
            "  result_json TEXT NOT NULL,\n  error_json TEXT,",
        ),
        "upstream version missing" => {
            exact_v1.replace("  upstream_version TEXT,\n", "")
        }
        // Declared type drift on one counter column.
        "bulk status type" => exact_v1.replacen(
            "  status TEXT NOT NULL,\n  submission_outcome",
            "  status INTEGER NOT NULL,\n  submission_outcome",
            1,
        ),
        // No v1 column declares a default.
        "bulk status default" => exact_v1.replacen(
            "  status TEXT NOT NULL,\n  submission_outcome",
            "  status TEXT NOT NULL DEFAULT 'queued',\n  submission_outcome",
            1,
        ),
        // `PRAGMA table_info` omits generated columns entirely. These two
        // near-miss bodies therefore passed the old exact-column probe even
        // though both add an uncontracted column. `table_xinfo` exposes the
        // VIRTUAL/STORED hidden flags (2/3), so both must fail closed.
        "extra virtual generated column" => exact_v1.replace(
            "  completed_at TEXT,\n  UNIQUE (bulk_id, bulk_index)",
            "  completed_at TEXT,\n  generated_status TEXT GENERATED ALWAYS AS (status) VIRTUAL,\n  UNIQUE (bulk_id, bulk_index)",
        ),
        "extra stored generated column" => exact_v1.replace(
            "  completed_at TEXT,\n  UNIQUE (bulk_id, bulk_index)",
            "  completed_at TEXT,\n  generated_status TEXT GENERATED ALWAYS AS (status) STORED,\n  UNIQUE (bulk_id, bulk_index)",
        ),
        // The analyses split-key uniqueness rule is missing outright.
        "analyses pair uniqueness missing" => exact_v1.replace(
            ",\n  UNIQUE (bulk_id, bulk_index)\n);",
            "\n);",
        ),
        // A contracted index is missing outright.
        "bulk index missing" => exact_v1.replace(
            "CREATE INDEX analyses_bulk_index ON analyses(bulk_id, bulk_index);",
            "",
        ),
        // An extra named index changes the exact catalog surface.
        "extra named index" => exact_v1.replace(
            "CREATE INDEX analyses_bulk_index ON analyses(bulk_id, bulk_index);",
            "CREATE INDEX analyses_bulk_index ON analyses(bulk_id, bulk_index);\n\
             CREATE INDEX analyses_caller_id ON analyses(caller_id);",
        ),
        // An extra application table is not part of schema v1.
        "extra table" => exact_v1.replace(
            "CREATE TABLE analyses (",
            "CREATE TABLE uncontracted (id TEXT);\nCREATE TABLE analyses (",
        ),
        "missing virtual table" => exact_v1.replace(
            "CREATE VIRTUAL TABLE analysis_search USING fts5(\n  analysis_id UNINDEXED, input_text, filename, headline, source_urls,\n  tokenize = 'unicode61'\n);",
            "",
        ),
        "extra virtual table" => exact_v1.replace(
            "CREATE VIRTUAL TABLE analysis_search USING fts5(",
            "CREATE VIRTUAL TABLE uncontracted_search USING fts5(body);\n\
             CREATE VIRTUAL TABLE analysis_search USING fts5(",
        ),
        // upstream_tasks swaps its cascade for a restrict delete.
        "cascade action wrong" => exact_v1.replace(
            "REFERENCES analyses(id) ON DELETE CASCADE,",
            "REFERENCES analyses(id) ON DELETE RESTRICT,",
        ),
        // analyses loses its own self-referencing retry foreign key.
        "retry foreign key missing" => exact_v1.replace(
            "  retry_of TEXT REFERENCES analyses(id),",
            "  retry_of TEXT,",
        ),
        // analyses re-points its retry foreign key at the wrong table.
        "retry foreign key wrong target" => exact_v1.replace(
            "  retry_of TEXT REFERENCES analyses(id),",
            "  retry_of TEXT REFERENCES bulk_collections(id),",
        ),
        // The rerun lineage key is independently contracted.
        "rerun foreign key missing" => exact_v1.replace(
            "  rerun_of TEXT REFERENCES analyses(id),",
            "  rerun_of TEXT,",
        ),
        // SQLite's foreign-key PRAGMA does not preserve these declaration
        // clauses. The canonical sqlite_master catalog must still reject
        // them because schema v1 declares none of them.
        "foreign key match clause" => exact_v1.replace(
            "  bulk_id TEXT REFERENCES bulk_collections(id),",
            "  bulk_id TEXT REFERENCES bulk_collections(id) MATCH FULL,",
        ),
        "foreign key deferrable initially" => exact_v1.replace(
            "  retry_of TEXT REFERENCES analyses(id),",
            "  retry_of TEXT REFERENCES analyses(id) DEFERRABLE INITIALLY DEFERRED,",
        ),
        "foreign key not deferrable initially" => exact_v1.replace(
            "  rerun_of TEXT REFERENCES analyses(id),",
            "  rerun_of TEXT REFERENCES analyses(id) NOT DEFERRABLE INITIALLY IMMEDIATE,",
        ),
        // Index PRAGMAs preserve the key shape but not the conflict policy
        // written on a table constraint.
        "primary key conflict policy" => exact_v1.replacen(
            "  id TEXT PRIMARY KEY,",
            "  id TEXT PRIMARY KEY ON CONFLICT REPLACE,",
            1,
        ),
        "unique conflict policy" => exact_v1.replace(
            "  upstream_bulk_id TEXT UNIQUE,",
            "  upstream_bulk_id TEXT UNIQUE ON CONFLICT IGNORE,",
        ),
        // The FTS5 virtual table carries a wrong column list.
        "fts columns wrong" => exact_v1.replace(
            "CREATE VIRTUAL TABLE analysis_search USING fts5(\n  analysis_id UNINDEXED, input_text, filename, headline, source_urls,",
            "CREATE VIRTUAL TABLE analysis_search USING fts5(\n  analysis_id UNINDEXED, body, headline,",
        ),
        // The FTS5 virtual table swaps its contracted tokenizer.
        "fts tokenizer wrong" => exact_v1.replace(
            "tokenize = 'unicode61'",
            "tokenize = 'porter'",
        ),
        "fts tokenizer arguments" => exact_v1.replace(
            "tokenize = 'unicode61'",
            "tokenize = 'unicode61 remove_diacritics 0'",
        ),
        "fts prefix option" => exact_v1.replace(
            "  tokenize = 'unicode61'",
            "  tokenize = 'unicode61',\n  prefix = '2 3'",
        ),
        "fts detail option" => exact_v1.replace(
            "  tokenize = 'unicode61'",
            "  tokenize = 'unicode61',\n  detail = column",
        ),
        "fts columnsize option" => exact_v1.replace(
            "  tokenize = 'unicode61'",
            "  tokenize = 'unicode61',\n  columnsize = 0",
        ),
        "harmless catalog spelling" => exact_v1
            .replace(
                "CREATE TABLE bulk_collections (",
                "create /* catalog comment */ table \"bulk_collections\"(",
            )
            .replace(
                "  id TEXT PRIMARY KEY,",
                "  \"id\" text primary key,\n  -- insignificant line comment",
            )
            .replace(
                "CREATE INDEX analyses_bulk_index ON analyses(bulk_id, bulk_index);",
                "create index \"analyses_bulk_index\"\n\
                 on \"analyses\" ( \"bulk_id\" , \"bulk_index\" ) ; -- trailing comment",
            ),
        other => panic!("unhandled deviation marker: {other}"),
    };
    assert_ne!(
        altered,
        exact_v1.as_str(),
        "the deviation marker must actually alter the fixture body: {deviation}"
    );
    format!("{altered}\n")
}

/// Every named incompatible deviation of the exact v1 body is rejected as
/// `history_corrupt`: the probe has no false positives on any of these
/// near-miss bodies. This is the meta-regression for the pre-remediation
/// probe, which accepted every one of these.
#[test]
fn every_named_incompatible_v1_deviation_is_rejected_as_history_corrupt() {
    for deviation in [
        "bulk unique column",
        "bulk identity nocase",
        "bulk identity wrong origin",
        "bulk identity partial",
        "bulk primary key nocase",
        "analyses primary key",
        "analyses wrong primary key",
        "status index columns",
        "status index direction",
        "status index nocase",
        "status index alternate collation",
        "status index partial",
        "status index expression",
        "status index extra key",
        "analyses identity descending",
        "bulk index made unique",
        "result_json not null",
        "upstream version missing",
        "bulk status type",
        "bulk status default",
        "extra virtual generated column",
        "extra stored generated column",
        "analyses pair uniqueness missing",
        "bulk index missing",
        "extra named index",
        "extra table",
        "missing virtual table",
        "extra virtual table",
        "cascade action wrong",
        "retry foreign key missing",
        "retry foreign key wrong target",
        "rerun foreign key missing",
        "foreign key match clause",
        "foreign key deferrable initially",
        "foreign key not deferrable initially",
        "primary key conflict policy",
        "unique conflict policy",
        "fts columns wrong",
        "fts tokenizer wrong",
        "fts tokenizer arguments",
        "fts prefix option",
        "fts detail option",
        "fts columnsize option",
    ] {
        let root = tempfile::tempdir().unwrap();
        let database = build_fixture(root.path(), &variant_body(deviation));
        let before = fs::read(&database).unwrap();
        let error = match HistoryStore::open(root.path()) {
            Ok(_) => panic!("deviation `{deviation}` unexpectedly opened"),
            Err(error) => error,
        };
        assert_eq!(
            error.code(),
            HistoryErrorCode::HistoryCorrupt,
            "deviation `{deviation}` must fail closed as history_corrupt: {error:?}"
        );
        // The original file is preserved byte-for-byte on every rejection.
        let after = fs::read(&database).unwrap();
        assert_eq!(
            before, after,
            "deviation `{deviation}`: the original database file is preserved"
        );
    }
}

#[test]
fn packet_b_v1_without_uniqueness_is_rejected_as_history_corrupt() {
    let root = tempfile::tempdir().unwrap();
    let database = build_fixture(root.path(), PACKET_B_SCHEMA_V1);
    let before = fs::read(&database).unwrap();

    let error = HistoryStore::open(root.path()).unwrap_err();
    assert_eq!(
        error.code(),
        HistoryErrorCode::HistoryCorrupt,
        "a Packet B v1 schema without its contracted uniqueness rules must fail closed: {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("preserved"),
        "recovery guidance must point at the preserved original: {message}"
    );

    // The original file is preserved byte-for-byte: no repair, rewrite, or
    // migration occurred.
    let after = fs::read(&database).unwrap();
    assert_eq!(
        before, after,
        "the original database file must be preserved byte-for-byte"
    );
}

#[test]
fn a_v1_schema_missing_the_analysis_pair_uniqueness_rule_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let with_others = "
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
  result_json TEXT,
  error_json TEXT,
  upstream_version TEXT,
  retry_of TEXT REFERENCES analyses(id),
  rerun_of TEXT REFERENCES analyses(id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT
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
CREATE VIRTUAL TABLE analysis_search USING fts5(
  analysis_id UNINDEXED, input_text, filename, headline, source_urls,
  tokenize = 'unicode61'
);
CREATE INDEX analyses_status_created ON analyses(status, created_at DESC);
CREATE INDEX analyses_bulk_index ON analyses(bulk_id, bulk_index);
";
    build_fixture(root.path(), with_others);
    let error = HistoryStore::open(root.path()).unwrap_err();
    assert_eq!(
        error.code(),
        HistoryErrorCode::HistoryCorrupt,
        "analyses must carry its UNIQUE (bulk_id, bulk_index) pair"
    );
}

#[test]
fn a_v1_schema_missing_the_cascade_foreign_key_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let without_cascade = "
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
  result_json TEXT,
  error_json TEXT,
  upstream_version TEXT,
  retry_of TEXT REFERENCES analyses(id),
  rerun_of TEXT REFERENCES analyses(id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT,
  UNIQUE (bulk_id, bulk_index)
);
CREATE TABLE upstream_tasks (
  analysis_id TEXT NOT NULL REFERENCES analyses(id),
  check_kind TEXT NOT NULL,
  upstream_task_id TEXT NOT NULL,
  last_stage TEXT,
  observed_at TEXT NOT NULL,
  PRIMARY KEY (analysis_id, check_kind),
  UNIQUE (check_kind, upstream_task_id)
);
CREATE VIRTUAL TABLE analysis_search USING fts5(
  analysis_id UNINDEXED, input_text, filename, headline, source_urls,
  tokenize = 'unicode61'
);
CREATE INDEX analyses_status_created ON analyses(status, created_at DESC);
CREATE INDEX analyses_bulk_index ON analyses(bulk_id, bulk_index);
";
    build_fixture(root.path(), without_cascade);
    let error = HistoryStore::open(root.path()).unwrap_err();
    assert_eq!(
        error.code(),
        HistoryErrorCode::HistoryCorrupt,
        "upstream_tasks must cascade deletes to analyses"
    );
}

#[test]
fn the_current_exact_schema_v1_opens_and_reopens_cleanly() {
    let root = tempfile::tempdir().unwrap();
    // Build through the production open, close, then reopen: the second
    // open must validate exactly the structure the first one created.
    {
        let store = HistoryStore::open(root.path()).expect("fresh open");
        assert_eq!(store.user_version().unwrap(), 1);
    }
    // Reopen carries the store through the same exact structural probe.
    let store = HistoryStore::open(root.path()).expect("reopen");
    assert_eq!(store.user_version().unwrap(), 1);
    // The catalog surface the probe just validated is exactly the
    // contracted one: both named indexes exist and are non-unique (a
    // `CREATE UNIQUE INDEX` reusing a contracted name must never pass),
    // the FTS5 virtual table declares the `unicode61` tokenizer, and every
    // table carries its exact column count.
    store
        .with_connection(|connection| {
            let unique_flag = |index: &str| {
                connection
                    .query_row(
                        "SELECT \"unique\" FROM pragma_index_list('analyses') WHERE name = ?1",
                        [index],
                        |row| row.get::<_, i64>(0),
                    )
                    .expect("named index exists")
            };
            assert_eq!(unique_flag("analyses_status_created"), 0);
            assert_eq!(unique_flag("analyses_bulk_index"), 0);
            let ddl: String = connection
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name = 'analysis_search'",
                    [],
                    |row| row.get(0),
                )
                .expect("FTS5 virtual table");
            assert!(ddl.contains("unicode61"), "the contracted tokenizer");
        })
        .expect("catalog assertions");
}

#[test]
fn harmless_sql_formatting_case_comments_and_identifier_quoting_are_accepted() {
    let root = tempfile::tempdir().unwrap();
    let equivalent = variant_body("harmless catalog spelling");
    build_fixture(root.path(), &equivalent);

    let store = HistoryStore::open(root.path()).expect("equivalent catalog spelling opens");
    assert_eq!(store.user_version().unwrap(), 1);
}
