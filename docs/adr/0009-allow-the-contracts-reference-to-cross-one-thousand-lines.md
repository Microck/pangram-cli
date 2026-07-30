# ADR 0009: Allow the contracts reference to cross one thousand lines

Status: accepted
Date: 2026-07-31

## Context

`docs/contracts.md` is the single normative reference for the wire contract.
Phase 0 review (PR #14) required documenting invariants that consumers must not
derive from implementation behavior: the analysis parent-status derivation in
section 4.1 and the bulk counter and collection status relationships in section
9, including the `failed`-includes-rejection rule and the constructor-owned
arithmetic bounds that JSON Schema Draft 2020-12 cannot express.

The file already approached the 1,000-line hygiene ceiling in the baseline.
Adding the documented invariants crosses it, even after folding prose. ADR 0008
granted a line-count exception only to the generated output schema and stated
that documentation receives no exception without another architecture decision.

Splitting the contracts reference would scatter one closed contract across
pages, breaking the single canonical citation that review, generated-schema
drift checks, and consumer documentation already link to.

## Decision

`docs/contracts.md` may cross the 1,000-line threshold. The hygiene gate counts
its lines but does not error on the count alone for this file; forbidden
characters and other checks still apply. The exception applies only to this one
reference document.

## Consequences

- The normative contract stays on one page with one canonical citation.
- Reviewers see required invariants documented where consumers already look.
- The file is longer than the normal documentation limit.
- Any further split or exception requires another architecture decision.

## Enforcement

- `tools/check-hygiene.rs` exempts `docs/contracts.md` from the line-count
  error only; all other hygiene checks continue to run against it.
- Review continues to prefer folding prose over growth, and any new section
  must be justified against this decision.
