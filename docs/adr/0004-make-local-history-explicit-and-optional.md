# ADR 0004: Make local history explicit and optional

Status: accepted
Date: 2026-07-23

## Context

Saved checks make the TUI more useful and let agents inspect or rerun prior
work. The analyzed text and files may also contain private or regulated
content. Silent persistence would violate user expectations, while omitting
history would remove an important local workflow.

Pangram receives submitted content regardless of local history. This decision
only governs what the CLI stores on the user's machine.

## Decision

Use local SQLite history with these rules:

- automatic history is disabled by default
- `--save` can persist one check while automatic history remains disabled
- saved input and results are plaintext
- first enablement presents a direct plaintext warning
- the configured data directory owns the database
- history schema incompatibility fails with recovery instructions
- the application never silently replaces or migrates an unknown database

The history module owns transactions, FTS synchronization, typed JSON
snapshots, and schema version checks. Other modules do not write its tables.

MCP history reads and mutations require separate startup capabilities.

## Consequences

- Users get searchable history without a cloud account.
- Users must opt into durable plaintext storage.
- Backups and disk encryption remain the user's responsibility.
- The hard-cut product policy avoids migration shims before a public installed
  base exists.

## Enforcement

- Default configuration sets `history.enabled = false`.
- Tests use real SQLite and assert transaction, FTS, permission, and corruption
  behavior.
- Diagnostics never print saved content.
- MCP conformance verifies that disabled capabilities remove the related tools.
