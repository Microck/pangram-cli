---
packages:
  "cargo:microck-pangram-cli": minor
  "npm:@microck/pangram-cli": minor
---

## Added

Added local history list, show, literal-text search, delete, clear, export, and
rerun commands with typed summaries, retained-input privacy controls,
consistent redacted exports, exact ordered check reconstruction, rerun
lineage, and confirmation gates.

## Fixed

List and search now validate complete authoritative check rows and parent
status consistency inside the same SQLite snapshot before returning a
summary. History export stdout write or flush failures now use the primary
output-failure exit 1 and never append a misleading history-write error to a
possibly partial raw stream.

`task status` and `task wait` now accept saved local `anl_` identities in
addition to upstream task IDs. Local resolution validates existing SQLite
history before credentials or network access, works when automatic history is
disabled, and fails locally without creating missing storage when the record
cannot resolve exactly one task. Only a complete canonical local `anl_` UUIDv7
ID enters that lookup; an opaque upstream task ID that starts with `anl_`
passes through unchanged. History rerun now applies the same canonical text
eligibility check as fresh detection, including Unicode-whitespace-only input
rejection before credentials or billable work.
