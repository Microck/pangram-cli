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
```

## Ownership and invariants

`input_json` and every `analysis_checks.result_json` /
`analysis_checks.error_json` body contain canonical schema-major-1 values.
`analysis_checks` is the sole authoritative reconstruction surface: exactly
one row exists per check, `check_index` is contiguous from zero, and the
ordered kinds must satisfy the canonical AI-detection-before-plagiarism rule.
The parent `check_count` records the authoritative cardinality so a deleted
terminal check row cannot be mistaken for a valid one-check analysis.
Missing, duplicate, malformed, or order-disagreeing rows fail closed as
`history_corrupt`; reads never repair them. The legacy parent body columns
are not consulted for reconstruction. For every locally authored manual or
automatic save, `input_json` durably retains submitted text, or the original
path and provider-extracted text for a binary file, even when the command's primary
projection omitted `--include-input`; this is permitted only because the
manual `--save` or enabled automatic-history gate made retention explicit
under the first-enable plaintext warning. A resumed remote read that has no
locally known input never fabricates plaintext. `history show` removes the
retained `text`, `path`, and `extracted_text` fields unless
`--include-input` is supplied. `upstream_version` stores validated provider-version provenance and is
nullable when an observation does not report it. A present incoming version
refreshes the stored value; an absent incoming version preserves it.
An absent optional input descriptor is stored as JSON `null` internally but
the canonical reconstructed object omits the `input` member entirely. All
other optional canonical members are likewise omitted rather than emitted as
JSON `null`. For a present descriptor, its type and SHA-256 must agree with
the typed columns. If retained text exists, canonical reconstruction verifies
its SHA-256, UTF-8 byte count, and Unicode-whitespace word count.
`submitted_at` independently stores the canonical submission timestamp
reported by a locally authored analysis. It is nullable because resumed
observations do not imply local authorship. Reads reproduce the stored value
exactly: they never substitute `created_at`, erase a prior value during
reconciliation, or fabricate one for a resumed observation.
`completed_at` is present for remotely observed terminal analyses and absent
for queued or running analyses. The one terminal-status exception is a failed
`acceptance_unknown` analysis carrying `submission_outcome_unknown`: its local
failure records an ambiguous issued POST, not a remote terminal observation,
so history preserves the reconciliation record with no fabricated completion
time.
`result_json` is an immutable terminal snapshot. Current remote
observation lives in `upstream_tasks`. Observation-refresh and bulk-child
reconciliation writes coalesce `result_json`, `error_json`, and
`completed_at`. Once a valid terminal body is stored, a non-terminal or
body-empty refresh cannot regress the parent status, submission outcome,
authoritative checks, terminal body, completion timestamp, result-derived FTS
metadata, or validated provenance. It may refresh only legitimate task
observation evidence and the observation timestamp. These rules are identical
for standalone tasks and bulk children and are order-, repetition-, and
concurrency-independent (contracts.md 14.2 durable-authorship invariance).

The nullable bulk-membership columns form one value: both `bulk_id` and
`bulk_index` are absent for a standalone analysis, or both are present for a
child. A present index is nonnegative and strictly less than the parent
collection's `total_items`. Partial, invalid-ID, missing-parent, and
out-of-range membership is `history_corrupt`, never a standalone downgrade.

Every `upstream_tasks` row must correspond one-to-one by `analysis_id` and
`check_kind` with an authoritative `analysis_checks` row. A task row for an
unknown or absent check kind, malformed identity, duplicate logical evidence,
or any task/check cardinality disagreement fails full reconstruction as
`history_corrupt`; task evidence is never silently omitted.

`HistoryStore` owns transactional synchronization between typed columns, JSON,
and FTS. Other modules must not write these tables directly.

History reads use an existing-only open: list/search return empty and clear is
an empty successful mutation when the database file does not exist; show and
delete surface the canonical missing-row error. No read creates the history
directory or database. The effective `history.enabled` value is deliberately
not consulted by reads, because disabling automatic retention neither hides
nor deletes existing rows.

FTS search never accepts raw FTS5 syntax. The adapter tokenizes literal user
text with SQLite FTS5's contracted `unicode61` tokenizer itself, quotes and
escapes every resulting token, and combines tokens with `AND` before binding
the expression to `MATCH`. This preserves all-literal-terms semantics while
making normalization, combining marks, punctuation, and non-Latin text match
the index tokenizer exactly.
Summary pages are ordered by `created_at DESC, id ASC`; their default limit is
50 and their hard maximum is 1,000.

Every list and search read validates, in the same deferred SQLite read
snapshot as the returned page, that each analysis owns exactly one
`analysis_search` row and that no orphan or malformed search row exists.
Missing, duplicate, or malformed search rows fail closed as
`history_corrupt`; reads never repair the index. The same snapshot also
validates the complete authoritative `analysis_checks` rows with one bounded
set query: contiguous indexes, canonical kind ordering, exact parent
cardinality, known statuses, status-specific and kind-correct result/error
bodies, and a parent status derived from those rows. It never validates a
summary through one follow-up query per row. Full analysis reads likewise hold
one deferred snapshot across the parent, search payload, ordered checks, task
evidence, and bulk provenance. WAL writers remain unblocked while that
snapshot is held.

History export is a raw primary stdout stream. Failure to write or flush that
stream exits 1 through the primary output-error lane. It is not a history
storage write (`history_write_failed`, exit 7), and no secondary envelope is
written after a possibly partial raw export.

Uniqueness and concurrent reconciliation are database-enforced, never
lookup-before-insert outside a transaction:

- `bulk_collections.upstream_bulk_id` is `UNIQUE`: one remote bulk job owns at
  most one stored row, and a concurrent insert of the same job is rejected at
  commit, never duplicated. The `(check_kind, upstream_task_id)` pair of
  `upstream_tasks` is `UNIQUE`: one upstream task reconciles onto exactly one
  analysis per check kind, so two processes reading the same task can never
  persist two analysis rows for it.
- Reconciliation is atomic: `HistoryStore` resolves
  prior-row lookup, merge, and insert-or-refresh inside one `IMMEDIATE`
  transaction (a write transaction that serializes concurrent writers for the
  whole reconcile) and commits it before returning. An adapter hands the store
  its fresh projections and never performs the lookup itself. A conflicting
  write rolls the whole batch back (no half-committed analysis, collection,
  child, or observation), and the store retries a busy commit a bounded number
  of times before surfacing `history_write_failed`.
- In-flight uniqueness is unchanged and lives at the adapter: the fresh local
  `anl_`/`bulk_` identity minted for the current read is preserved for output,
  and the persisted row keeps its first-recorded identity, authorship, local
  input/FTS payload, and creation time exactly as the reconciliation rules
  below require.

Every SQLite connection enables `PRAGMA foreign_keys = ON` before reading or
writing application tables. Tests MUST prove foreign-key rejection,
`ON DELETE CASCADE` for owned task/check rows, and `ON DELETE SET NULL` for
lineage against the real database. Deleting an original analysis preserves
dependent analyses and clears their `retry_of` / `rerun_of` values in the same
transaction.

Persistence initiated by `detect` accepts completed envelopes only. Explicit
`detect --save` is therefore incompatible with `--detach` and is rejected as
a usage error before credentials, network, or history work. With automatic
history enabled, a detached accepted queued/running snapshot remains
ephemeral: no history open or write is attempted and no history warning is
emitted. A later terminal `task status` or `task wait` observation may persist
or reconcile the completed evidence under the automatic gate.

Resumed task observations obey the same completed-envelope boundary.
Automatic history persists a `task status` or `task wait` analysis only when
its canonical parent state is `succeeded`, `failed`, or `partial`. A queued or
running status observation remains `ephemeral`, does not open the history
directory, and emits no automatic-save warning. `task wait` persists only the
terminal observation it returns. This does not change explicit manual
`detect --save`.

## Filesystem protection

The platform data directory, database, WAL, and shared-memory sidecars contain
plaintext submitted content and results. On Unix, the directory requires mode
`0700` and files require mode `0600`. On Windows, each requires an owner-only
ACL. `PANGRAM_DATA_DIR` does not weaken these requirements.
The database path and any existing sidecar path must be real regular files,
never symbolic links or any Windows object carrying
`FILE_ATTRIBUTE_REPARSE_POINT`. Type, alias, and owner-only permission checks
for every existing `-wal` and `-shm` sidecar happen before SQLite opens the
database and must not mutate the database or hostile target. Sidecars that
appear in the race after that pre-open check must not let SQLite follow a
terminal symbolic-link or reparse alias. Unix relies on the bundled SQLite
VFS's no-follow sidecar opens. Windows creates or opens both sidecars with
`FILE_FLAG_OPEN_REPARSE_POINT`, rejects reparse and non-disk/non-file objects,
applies or verifies the owner-only ACL, excludes delete sharing, and retains
those handles until immediately before the SQLite connection closes. During
that sequential handoff, SQLite's already-open sidecar handles, which also
exclude delete sharing, preserve object identity through final cleanup.


The SQLite connection for the protected filesystem database opens the exact
already-validated path as an absolute literal filename with the per-connection
URI flag disabled. Relative data directories are made absolute lexically
before protection and open, without changing their literal components. Names
beginning with `file:` and URI metacharacters such as `?` and `#` therefore
have no SQLite URI semantics even when the bundled SQLite library was compiled
with URI support. URI-capable connections are permitted only when no
user-controlled filesystem path exists, such as the isolated in-memory
expected-schema catalog connection.
This alias boundary is scoped to terminal database and sidecar names inside
the already-required owner-only history directory. It does not claim to
defend against arbitrary operations by the same owner or pre-existing hard
links.

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
Before either destructive transaction performs its first write, it certifies
the complete logical store through the canonical collection and analysis
validators, including exact FTS cardinality and content. Corruption anywhere
returns `history_corrupt`, preserves every logical row, and skips the
post-commit checkpoint.

An unknown or incompatible `user_version` fails with recovery instructions.
The application must not silently replace, rewrite, or migrate the database.

First-use creation is serialized by SQLite itself. Every opener takes an
`IMMEDIATE` transaction before it classifies `user_version` or creates schema
objects. Starting from an absent database path, exactly one opener creates and
commits schema v1; concurrent openers wait for that transaction, then validate
the committed exact-v1 schema. A protected, zero-byte SQLite file left by a
process that stopped after secure file creation but before schema commit is
the only `user_version = 0` state treated as fresh: it is initialized by the
same transaction. Any `user_version = 0` database that already contains a
schema object is incompatible and fails closed as `history_corrupt`, without
repair or mutation. Schema creation and `user_version = 1` commit atomically,
so a failed or interrupted initializer cannot expose a partial schema.

`user_version = 1` alone is not sufficient: a v1 database carries the exact
schema-locked structure above, so every open (fresh or existing) also
verifies that structure before any application statement runs. The required
v1 surface is the exact set of tables (`bulk_collections`, `analyses`,
`upstream_tasks`, `analysis_checks`, the `analysis_search` FTS5 virtual table), the contracted
indexes (`analyses_status_created`, `analyses_bulk_index`), and the
contracted uniqueness and referential rules:

- `bulk_collections.upstream_bulk_id` carries a `UNIQUE` constraint
  (surfaced as its owning unique index)
- `analyses` carries the `UNIQUE (bulk_id, bulk_index)` constraint
- `upstream_tasks` carries its `PRIMARY KEY (analysis_id, check_kind)` and
  the `UNIQUE (check_kind, upstream_task_id)` constraint
- `upstream_tasks.analysis_id` carries the `ON DELETE CASCADE` foreign key
  to `analyses(id)`
- `analysis_checks` carries `PRIMARY KEY (analysis_id, check_index)`,
  `UNIQUE (analysis_id, check_kind)`, and an `ON DELETE CASCADE` foreign key
  to `analyses(id)`

A database whose stored `user_version` is 1 but whose structure does not
carry every one of those rules is an incompatible v1 and fails closed as
`history_corrupt` with the same recovery guidance, leaving the original
file untouched. The application never repairs, upgrades, or rewrites such
a database in place.

### Exact v1 catalog surface

Name-and-kind presence alone is never sufficient. Every one of the catalog
checks below must hold for a v1 database to open; any deviation is the
incompatible-v1 `history_corrupt` failure above. Column names are compared
case-sensitively against the exact schema-v1 spelling; declared types are
compared after SQLite's catalog case normalization, and an
affinity-compatible but different declaration or an unknown type is
rejected.

- Before the real database can be accepted, the store executes the compiled
  schema-v1 body in an isolated in-memory connection to the same bundled
  SQLite engine and reads that connection's complete deterministic
  `sqlite_master` catalog. The real catalog must have the same ordered
  `(type, name, table name, sql)` entries, including SQLite-owned FTS5 shadow
  objects. DDL comparison tokenizes the complete SQL and normalizes only
  SQLite-insensitive unquoted word/identifier case, insignificant whitespace
  and comments, an optional trailing semicolon, and equivalent quoting around
  exact-case identifiers. Quoted identifiers and string literals remain
  byte-sensitive, and the comparison never drops semantic tokens. A hidden
  `MATCH` clause, a
  `DEFERRABLE` / `NOT DEFERRABLE` or `INITIALLY` clause, a primary-key or
  unique-constraint `ON CONFLICT` policy, an extra or altered FTS5 option, or
  any other altered declaration is incompatible even when a PRAGMA reports
  the same reduced semantics. Extra or missing application, index,
  virtual-table, autoindex, or FTS5 shadow objects are incompatible too.
- The exact column probe uses `PRAGMA table_xinfo`, not `table_info`, so
  generated and hidden columns cannot evade validation. Every contracted
  base-table column below has `hidden = 0`; any extra column or any nonzero
  hidden flag is incompatible schema drift.
- `bulk_collections` has exactly 12 columns, in this exact declaration
  order, with this exact declared type, nullability, and default for each:
  1. `id` TEXT nullable (primary key; SQLite `PRAGMA table_xinfo` reports
  `notnull = 0` for this rowid-table spelling), 2. `upstream_bulk_id` TEXT
  nullable, 3. `status` TEXT `NOT NULL`, 4. `submission_outcome` TEXT
  `NOT NULL`, 5. `total_items` INTEGER `NOT NULL`, 6. `accepted` INTEGER
  `NOT NULL`, 7. `succeeded` INTEGER `NOT NULL`, 8. `failed` INTEGER
  `NOT NULL`, 9. `estimated_billable_units` INTEGER `NOT NULL`,
  10. `created_at` TEXT `NOT NULL`, 11. `updated_at` TEXT `NOT NULL`,
  12. `completed_at` TEXT nullable. Its primary key is exactly the single
  column `id`. Aside from that `pk` primary key it owns exactly one `u`
  (unique-origin) index, and that index covers exactly the single column
  `upstream_bulk_id`; it owns no `name`-origin constraints and exactly the
  two named indexes `analyses_status_created` and `analyses_bulk_index`
  live on `analyses` below (no extra `c`-origin index exists anywhere in
  the v1 surface).
- `analyses` has exactly 21 columns, in this exact declaration order, with
  this exact declared type and nullability for each: 1. `id` TEXT nullable
  (primary key; SQLite `PRAGMA table_xinfo` reports `notnull = 0` for this
  rowid-table spelling), 2. `bulk_id` TEXT nullable,
  3. `bulk_index` INTEGER nullable, 4. `caller_id` TEXT nullable,
  5. `status` TEXT `NOT NULL`, 6. `submission_outcome` TEXT `NOT NULL`,
  7. `save_state` TEXT `NOT NULL`, 8. `input_type` TEXT `NOT NULL`,
  9. `input_sha256` TEXT `NOT NULL`, 10. `display_name` TEXT nullable,
  11. `input_json` TEXT `NOT NULL`, 12. `check_count` INTEGER `NOT NULL`
  with default `1` and `CHECK (check_count BETWEEN 1 AND 2)`,
  13. `result_json` TEXT nullable, 14. `error_json` TEXT nullable,
  15. `upstream_version` TEXT nullable, 16. `retry_of` TEXT nullable,
  17. `rerun_of` TEXT nullable, 18. `submitted_at` TEXT nullable,
  19. `created_at` TEXT `NOT NULL`, 20. `updated_at` TEXT `NOT NULL`,
  21. `completed_at` TEXT nullable. Its
  primary key is exactly the single column `id`. Aside from that primary
  key it owns exactly one `u` index, covering exactly `(bulk_id,
  bulk_index)` in that column order. It owns exactly the two contracted
  `c`-origin named indexes and no others: `analyses_status_created` is
  non-unique over exactly the ordered columns `(status, created_at)` with
  `created_at` descending, and `analyses_bulk_index` is non-unique over
  exactly the ordered columns `(bulk_id, bulk_index)`.
- `upstream_tasks` has exactly 5 columns, in this exact declaration order:
  1. `analysis_id` TEXT `NOT NULL`, 2. `check_kind` TEXT `NOT NULL`,
  3. `upstream_task_id` TEXT `NOT NULL`, 4. `last_stage` TEXT nullable,
  5. `observed_at` TEXT `NOT NULL`. Its primary key is exactly the ordered
  pair `(analysis_id, check_kind)`. Aside from that primary key it owns
  exactly one `u` index, covering exactly `(check_kind, upstream_task_id)`
- `analysis_checks` has exactly 6 columns in this exact declaration order:
  `analysis_id` TEXT `NOT NULL`, `check_index` INTEGER `NOT NULL`,
  `check_kind` TEXT `NOT NULL`, `status` TEXT `NOT NULL`, `result_json` TEXT
  nullable, and `error_json` TEXT nullable. Its primary key is exactly
  `(analysis_id, check_index)` and its sole additional unique constraint is
  exactly `(analysis_id, check_kind)`.
  in that order. Its foreign-key list is exactly one entry: the
  single-column key from `analysis_id` to `analyses(id)` with
  `ON UPDATE NO ACTION`, `ON DELETE CASCADE`, and `MATCH NONE`. `analyses`
  in turn owns exactly three single-column foreign keys: `bulk_id` to
  `bulk_collections(id)` uses `NO ACTION` / `NO ACTION` / `MATCH NONE`;
  `retry_of` and `rerun_of` to `analyses(id)` use `NO ACTION` /
  `SET NULL` / `MATCH NONE`.
- Every contracted primary-key (`pk`), unique-constraint (`u`), and named
  schema-created (`c`) index is validated through both `PRAGMA index_list`
  and `PRAGMA index_xinfo`, not by column names alone. Its `unique` and
  `origin` flags must match the role above, `partial` must be `0`, and its
  complete ordered set of `key = 1` rows must contain only the contracted
  columns (never an expression or an extra key), each with `coll = BINARY`
  and the contracted ascending/descending direction. SQLite's auxiliary
  `key = 0` rowid payload is not part of the index key. A `NOCASE` or custom
  collation, partial predicate, expression key, extra key, wrong direction,
  wrong uniqueness, or wrong origin is incompatible v1 schema drift.
- `analysis_search` is an exact FTS5 virtual table created with the
  `unicode61` tokenizer, whose catalog column list is exactly
  `(analysis_id, input_text, filename, headline, source_urls)` in that
  order, each with `hidden = 0`, followed by SQLite's expected hidden
  implementation columns `analysis_search` and `rank`, each with
  `hidden = 1`. Its `USING fts5` creation options, as recorded through
  SQLite's FTS5 vocabulary, are exactly: one `UNINDEXED` column
  `analysis_id` and the `tokenize = 'unicode61'` tokenizer option.

### Task-first/bulk-second child reconciliation

A bulk refresh identifies each observed child by two attested identities at
once: its `(bulk_id, bulk_index)` membership inside the collection and any
upstream task keys the read attested for it. A standalone `task status` /
`task wait` observation of one of those task identities may already have
reconciled the one stored row for that task before the bulk read ever ran
(and the bulk row lookup by membership alone would collide with that
row's `upstream_tasks UNIQUE (check_kind, upstream_task_id)` constraint and
roll the whole batch back). Reconciliation is therefore
order-independent: inside the bulk write transaction the store resolves
each candidate child by its membership AND by every attested
`(check_kind, upstream_task_id)` key together, and then:

- **Adopt.** When the task keys resolve at most one stored analysis row,
  and that row is the same row the membership resolves to (or the
  membership is currently unoccupied), the bulk refresh may reuse that one
  existing durable row only when its membership columns are both NULL
  (standalone) or already exactly equal the incoming `(bulk_id,
  bulk_index)`. A row belonging to another collection or position, or
  carrying a partial membership, is a conflict and the entire transaction
  fails closed; adoption never moves or rekeys a member. The row keeps its first-recorded identity,
  authorship (`input_json`, `input_sha256`, input kind, display name,
  caller ID when the bulk read carries none of its own), `save_state`,
  `created_at`, and its local search/FTS payload (the refresh coalesces
  onto stored values); the refresh adds the `(bulk_id, bulk_index)`
  membership link to the pre-existing standalone row and moves only
  observation fields (`status`, `submission_outcome`, terminal bodies, and
  the refresh stamps) under the durable-authorship coalescing rules. No
  second analysis row is ever inserted for the child, and the observation
  rows rebind onto the reused identity. A locally observed (in-flight)
  standalone child adopts with its local authorship intact; a locally
  authored terminal body is never erased by a body-less bulk refresh.
- **No fabricated reverse match.** When a prior bulk refresh recorded a
  child at its membership without an attested task identity, a standalone
  task observation has no bulk membership or other documented evidence that
  can correlate it to that child. It therefore remains a separate durable
  row; status, order, content, and timing are never matching keys. If a later
  bulk observation attests a task identity already owned by that distinct
  standalone row, the provenance is ambiguous: the refresh fails as
  `history_corrupt` and rolls back without deleting, transferring, merging,
  or rekeying either row or its task evidence.
- **Fail closed.** When the task keys resolve more than one distinct
  stored analysis, or the row a task key resolves to is a different row
  than the one occupying the membership, or the membership resolves to an
  overlapping set of other attested task keys that is not identical to the
  set this read attests (including a different upstream task ID for the
  same check kind), or one candidate itself attests two different task IDs
  for one check kind, the candidates disagree. The write then fails
  with `history_write_failed` and the whole enclosing batch (collection,
  children, and observations) rolls back atomically; the store never
  deletes, merges, or rekeys an unrelated row to force a fit.

Terminal-body reconciliation is independent from completion time on every
standalone, membership, and adoption refresh. When neither body is present,
the stored `result_json` and `error_json` survive even if the incoming row
has `completed_at`; a supplied result replaces `result_json` and clears
`error_json`, while a supplied error replaces `error_json` and clears
`result_json`. Supplying both is an impossible observation and fails the
atomic write as `history_write_failed`.

A blank membership (`bulk_index` absent) resolves only through the task
keys: an unresolved task key falls back to a fresh insert keyed on the
read's fresh identity, exactly as a first observation would.

For a standalone task reconciliation that resolves an existing durable row,
selection by one incoming task key never authorizes replacement of that row's
other evidence. Before any refresh mutation, every incoming
`(check_kind, upstream_task_id)` pair is compared with every task key already
owned by the selected row. A check kind absent from the row may be added; the
same kind and same upstream ID may refresh its stage and timestamp; the same
kind with a different upstream ID is a conflict and fails the whole immediate
transaction closed as `history_write_failed`. Incoming observations that omit
a previously recorded kind leave that evidence untouched. These rules remain
true under concurrent standalone refreshes because comparison and mutation
share the same immediate transaction.
