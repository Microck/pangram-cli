# ADR 0008: Keep the output schema whole

Status: accepted
Date: 2026-07-28

## Context

The canonical output schema owns one closed union of success and failure
envelopes. It also owns the shared analysis, check, result, error, bulk, and
command-status definitions referenced by that union.

The complete schema exceeds 1,000 lines before Phase 0 because it is a
specification-owned seed. Phase 0 transfers the same contract to generated
Rust ownership, where the generated artifact remains above the normal source
file threshold.

Splitting the schema would require cross-file JSON Schema resolution for direct
validation. It would also make the command-to-data discrimination harder to
inspect and weaken the single-artifact contract used by shell and MCP
consumers.

## Decision

Keep `contracts/output.schema.json` as one file during the seed bootstrap and
after generated ownership transfers.

The exception applies only to this generated contract artifact. Rust source,
tests, documentation, and other generated files remain subject to the normal
decomposition thresholds.

## Consequences

- Consumers can validate a canonical envelope with one schema document.
- Shared definitions stay local to the discriminated envelope union.
- The generated file is longer than the normal source limit.
- Reviews focus on the Rust owner, generator, and drift tests instead of
  treating generated line count as source complexity.

## Enforcement

- Phase 0 proves the generated schema matches the seed contract before
  ownership transfers.
- CI regenerates the file and rejects drift.
- No second generated file receives this exception without another
  architecture decision.
