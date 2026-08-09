---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Added

Landed the concrete `HistoryStore` under `src/history/` (`store`,
`operations`, `records`) implementing the local SQLite history
foundation (architecture-spec 11, docs/history-contract.md schema v1). The
store owns open, schema validation, transactions, FTS synchronization, and
every mutation of the `bulk_collections`, `analyses`, `upstream_tasks`,
and `analysis_search` tables. On Unix the history directory requires mode
`0700` and the database file plus its WAL/SHM sidecars `0600` with
fail-closed enforcement on every open; on Windows the Phase 1 owner-only
ACL policy is reused through the existing `config::windows_acl` machinery.
Each connection enables WAL, `foreign_keys = ON`, and `secure_delete = ON`
before use, verified by reading the runtime `journal_mode` back. Schema
version `1` is recorded in `user_version`, and an unknown or newer value
or a `quick_check` failure fails with `history_corrupt` while preserving
the original file. Stored input kinds are typed as the closed
`text`/`file` set, with unknown persisted values rejected as corruption.
Terminal updates and their search payload advance in the same transaction
through the typed `TerminalResult` snapshot, and delete/clear update the
FTS index in the same transaction and finish with
`wal_checkpoint(TRUNCATE)` per the deletion contract. Stored rows carry
typed identifiers and canonical schema-major-1 JSON as opaque strings so
no upstream content parsing, plaintext logging, or schema normalization
leaks into the module. Twenty new real-SQLite integration tests under
`tests/history-store.rs` exercise schema equality, pragmas, foreign-key
rejection and `ON DELETE CASCADE`, transactional create/update, FTS5
search and search payload replacement, delete/clear semantics,
version/corruption handling, structural inconsistency detection, and the
Unix owner-only protection fail-closed paths including WAL/SHM sidecars.
No CLI, TUI, or MCP surface is activated and no `--save` integration
exists: history remains disabled by default and invisible to users.

