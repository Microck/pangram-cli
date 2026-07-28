# Pangram CLI history contract

Status: approved for implementation
Schema version: 1

This file is the contract owner for the local SQLite schema. Changes to this
schema are observable persistence changes and must update this artifact before
implementation.

## SQLite schema

The initial schema is equivalent to:

```sql
PRAGMA foreign_keys = ON;
PRAGMA user_version = 1;

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
  completed_at TEXT,
  UNIQUE (bulk_id, bulk_index)
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
```

## Ownership and invariants

`input_json`, `result_json`, and `error_json` contain canonical schema-major-1
values. `result_json` is an immutable terminal snapshot. Current remote
observation lives in `upstream_tasks`.

`HistoryStore` owns transactional synchronization between typed columns, JSON,
and FTS. Other modules must not write these tables directly.

Every SQLite connection enables `PRAGMA foreign_keys = ON` before reading or
writing application tables. Tests MUST prove foreign-key rejection and
`ON DELETE CASCADE` behavior against the real database.

## Filesystem protection

The platform data directory, database, WAL, and shared-memory sidecars contain
plaintext submitted content and results. On Unix, the directory requires mode
`0700` and files require mode `0600`. On Windows, each requires an owner-only
ACL. `PANGRAM_DATA_DIR` does not weaken these requirements.

If protection cannot be established or verified:

- automatic history persistence is disabled and reports one sanitized warning
- explicit `--save` and history commands fail with
  `insecure_history_permissions`
- the process does not open or create the database

## Deletion semantics

Delete and clear remove logical accessibility through Pangram CLI. They do not
promise forensic secure erasure from filesystem snapshots, backups, flash
translation layers, or prior database copies.

Connections enable SQLite secure deletion. After an explicit delete or clear,
`HistoryStore` updates FTS in the same transaction and performs a
`wal_checkpoint(TRUNCATE)` before reporting success. Failure to truncate is
reported, but the logical deletion remains committed.

An unknown or incompatible `user_version` fails with recovery instructions.
The application must not silently replace, rewrite, or migrate the database.
