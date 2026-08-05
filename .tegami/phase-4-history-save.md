---
packages:
  "cargo:microck-pangram-cli": minor
  "npm:@microck/pangram-cli": minor
---

## Added

Activated local history persistence for completed detection, bulk, and task
workflows (Phase 4 Packet C; contracts.md 14.2 note, docs/history-contract.md,
product-spec 10). `pangram detect --save` now persists the completed analysis
envelope - including the submitted plaintext whenever manual save or enabled
automatic history authorizes retention, regardless of whether
`--include-input` was used for the primary output - together with the terminal
result or canonical check error, typed lifecycle state, and current
observation identity. It works while automatic history is disabled and
reports `save_state: saved_manual`. The automatic gate (`config set
history.enabled true`) saves every completed detect analysis as
`saved_history`, and additionally records bulk submissions (the collection
plus its plan children in bulk-index order, each carrying its `(bulk_id,
bulk_index)` membership link and caller ID) and terminal task status/wait
observations (a repeated read of one remote task refreshes the one saved row
instead of duplicating it). Queued and running task observations remain
ephemeral and do not open history. A manual save failure surfaces as a
canonical local-history error (exit 7) after the honest envelopes render in
order with their own truthful save states; an automatic failure produces
exactly one sanitized stderr warning per invocation and never fails the
remote result. When neither path applies, no history directory or database is
opened or created.
Enabling automatic history for the first time (`config set history.enabled
true` after it was unset or false) acknowledges ADR 0004 with exactly one
direct plaintext warning on stderr: history stores submitted content and
results unencrypted in the local data directory. Saved plaintext is redacted
from `history show` by default but is exposed by `history show
--include-input`; `history export` includes retained content unless
`--redact-content` is requested. The bulk and task surfaces carry no `--save`,
and `history save` is rejected as an unknown argument. History data remains
subject to the owner-only local filesystem boundary; it is not encrypted.

## Fixed

Repeated-file `detect --save` now preserves the ordered tail after one
member's save failure: every completed member persists or renders `ephemeral`
exactly once, in invocation order, and later members still save. One bulk
submission or observation write (the collection, its children, and the
observation rows) commits or rolls back atomically, reconciled by
`upstream_bulk_id` without duplicates, keeping local input text, filename,
caller ID, membership, and the original submission outcome and creation time.
Repeated `task status`/`task wait` reads refresh the one saved row's
observation and terminal fields only; they never rewrite its original
submission outcome, save state, local input, or creation time, and the fresh
read output keeps its own fresh identity and save outcome rather than the
prior row's. A primary render failure exits 1 even when an explicit save also
failed, so a closed stdout never renders behind a history exit 7. An
automatic bulk save now returns the truthful children series instead of
dropping them. Upstream-identity reconciliation is now atomic and
database-enforced: the history schema (still user_version 1) declares
`bulk_collections.upstream_bulk_id UNIQUE` and `upstream_tasks UNIQUE
(check_kind, upstream_task_id)`, and the store runs every task or bulk
reconcile (prior-row lookup, merge, insert-or-refresh) inside one immediate
SQLite write transaction, so two Pangram CLI processes observing the same
remote task or bulk job concurrently converge on exactly one stored row with
its children and observations exactly once instead of risking duplicate
durable rows. One bulk command (`bulk submit --wait`, `bulk status`, `bulk
wait`) now shares one invocation-scoped automatic-history warning across its
observed-children read and persistence phases, so a run in which both fail
emits exactly one `warning:` line.

A stored `user_version = 1` alone no longer opens the history database: the
store now verifies the exact schema-v1 structure on every open and fails
closed as `history_corrupt` with recovery guidance when any element is
absent or different, preserving the original file byte-for-byte and never
repairing or migrating it in place. The structural probe itself is exact
(this remediation): every base table's full ordered column list with
declared types, nullability, defaults, and `PRAGMA table_xinfo` hidden flags
(so an extra generated or hidden column cannot evade the check); every
table's exact primary key;
the exact uniqueness surface including specifically
`bulk_collections.upstream_bulk_id` (verified by its owning unique index,
never by name presence); the two named indexes' ordered columns,
uniqueness, origin, non-partial status, `BINARY` collation, key-only rows,
and sort direction; every primary-key and unique-identity index receives
the same full `PRAGMA index_list`/`index_xinfo` validation, so partial,
expression, extra-key, `NOCASE`/custom-collation, wrong-origin, and
wrong-direction near misses are rejected; every foreign key with its exact
actions (including the `ON DELETE CASCADE` from `upstream_tasks` and both
analysis-lineage references); and
`analysis_search` as the exact FTS5 virtual table with its contracted
column list and the `unicode61` tokenizer. The store additionally derives
the complete expected `sqlite_master.sql` catalog by executing its compiled
schema-v1 body in an isolated in-memory connection to the same bundled SQLite
engine, then compares deterministic normalized catalog entries before
mutating the real database. Normalization admits only harmless keyword case,
whitespace/comments, trailing semicolons, and exact-identifier quoting; it
does not discard semantic clauses. Hidden foreign-key `MATCH`,
`DEFERRABLE`/`INITIALLY`, primary-key or unique `ON CONFLICT`, and extra or
altered FTS5 options therefore fail closed even where PRAGMAs omit them, as
do extra or missing SQLite-owned catalog objects. A near-miss v1 body (a wrong
nullability or type drift, a missing or wrong foreign key, a wrong FTS5
column set or tokenizer, or an extra sneaked-in unique index) is rejected
exactly like a grossly incompatible one.

Concurrent first use now serializes schema classification and creation with
an immediate SQLite transaction. Starting from an absent database path, one
opener atomically commits the exact schema and `user_version = 1`, while
concurrent threads or processes wait and validate the committed schema
instead of falsely reporting a transient version zero as `history_corrupt`.
A protected zero-byte file left before schema creation resumes through this
same path; a version-zero database containing any schema object still fails
closed without mutation.

Task and bulk reconciliation are now order-independent. A standalone `task
status`/`task wait` observation saved before a bulk read of the same
upstream task no longer collides with the `upstream_tasks UNIQUE
(check_kind, upstream_task_id)` constraint: inside the one immediate write
transaction the store resolves each bulk child by its `(bulk_id,
bulk_index)` membership AND every attested upstream task key together,
reusing the one existing durable row when they agree (a previously
standalone row gains its membership link only when both membership columns
were NULL, and a row already assigned to another collection or position,
or carrying a partial membership, fails closed rather than being moved)
while preserving the row's first-recorded identity, authorship, save state,
local input/FTS payload, and creation time. A task-less bulk child cannot be
correlated from a direct task read alone: that read remains separate, and a
later bulk observation that would assign the direct row's task identity to
the distinct membership now fails closed as `history_corrupt` with a full
rollback instead of transferring or deleting either row's evidence. Other
candidate conflicts - two task keys resolving
two different rows, a task-key row different from the membership row, or a
membership holder attesting an overlapping but different task set
(including a different task ID for the same check kind), or one candidate
attesting two IDs for one check kind - the whole batch (collection,
children, observations) fails closed as `history_write_failed` and rolls
back atomically; no unrelated row is ever deleted, merged, or rekeyed, and
no duplicate analysis or observation ever persists.

A standalone task refresh selected through one matching upstream key now
compares every incoming key with all evidence already owned by that durable
row before mutation. Missing check kinds may be added and exact keys may
refresh, while a different upstream ID for an already-owned check kind fails
closed and rolls back the snapshot and observation updates; omitted evidence
remains untouched. Real SQLite tests cover selected-by-another-key
replacement, allowed add/same/omitted cases, and two-connection concurrency.
The reconciliation implementation is split into cohesive task, bulk, and
common modules, each below the source-size threshold, while retaining one
concrete `HistoryStore` owner.

Terminal-body refreshes now follow the same branch-aware merge in standalone,
membership, and adoption paths: no incoming body preserves the stored body
even when `completed_at` is present, a result replaces the result and clears
a stale error, an error replaces the error and clears a stale result, and
both-present input fails the atomic reconciliation closed.

Remote bulk result children now retain the validated terminal input
descriptor, upstream version, task identity, and sanitized last stage through
the canonical output and history projection; a held JSONL plan adds its truthful filename and local
plaintext without replacing upstream evidence. Terminal refreshes replace
stale result-owned headline and source URL search metadata while preserving
durable input text and filename authorship. Typed terminal updates now
preflight the exact synchronized FTS row and fail closed as `history_corrupt`
without mutation when it is missing, duplicated, or malformed instead of
silently repairing it. Validated upstream-version provenance is now durable
in schema v1: a present refresh replaces it and an absent refresh preserves
the stored value.

History opening now rejects symbolic links at the database path or existing
SQLite sidecar paths before changing permissions or opening SQLite. The link
target and any target-adjacent files remain untouched. Raced terminal WAL and
shared-memory aliases are now covered too: the bundled Unix VFS's no-follow
opens are regression-tested, while Windows pins both sidecar names with
no-follow reparse-point handles that exclude rename and deletion until
immediately before the SQLite connection closes. SQLite's already-open
sidecar handles preserve identity through the sequential handoff, concurrent
WAL use, and last-close cleanup.

History opening now also disables SQLite URI parsing for the protected
filesystem database path, so data-directory names beginning with `file:` or
containing URI metacharacters are opened exactly as the already-validated
literal path and cannot redirect database or sidecar access.

`detect --save --detach` is now rejected as a usage conflict before
credentials, network, or history work because manual save persists completed
envelopes only. With automatic history enabled, `detect --detach` leaves its
accepted queued/running snapshot ephemeral without opening SQLite or emitting
a history warning; a later terminal `task status` or `task wait` can persist
the completed evidence.

A primary render failure now clears `primary_ok` in the very outcome
`emit_primary` returns (JSON and text surfaces), so a later post-primary
history attachment can never replace the render failure's exit 1 with the
save failure's exit 7 even on the JSON deferred-write surface.
