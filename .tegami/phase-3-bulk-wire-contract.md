---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Changed

Pangram CLI's bulk analysis domain, its internal loopback protocol fixture,
and the shared bulk analysis core now lock the officially documented Pangram
4 bulk wire contract. Submission sends one job-wide `model` set to
`pangram-4` with no per-item selector and no public-dashboard-link field;
each valid item is estimated at one billable unit per started 100-word block
with a minimum of one; the job estimate is compared against the smaller of
the caller-supplied ceiling and Pangram's 1,000-unit request limit before any
credential or network work; and the typed submit/status/items/results
responses follow the documented shapes (epoch-second string timestamps,
48-hour terminal metadata retention, and the documented error matrix).

The bulk analysis core owns submit, one observation loop, and typed page
reads over the single adapter-facing `Analyzer`, so CLI/TUI/MCP share one
pacemaker and HTTP stack. The core pins the bulk submit success to exactly
HTTP 202 (any other 2xx is never replayed and surfaces the ambiguous
`submission_outcome_unknown`), validates the 202 acceptance `status` token
against the closed `queued` value, normalizes documented `result: null`
results-page entries to the canonical `running` item state, treats per-item
`stage` as sanitized diagnostic-only evidence, and bounds every coverage
allocation by the validated plan count (or, for a resumed remote handle, by
the documented job cap) rather than an unchecked upstream count. The
fetch-all walk requests a conservative bounded page size (100) instead of the
1,000 maximum, while explicit one-page items/results requests may still use
any `limit` from 1 to 1,000.

The bulk and task commands remain planned for Phase 3. No public CLI, MCP, or
README capability is enabled, and public bulk support still requires live
conformance. This fragment uses `patch`, matching the existing
contract-and-domain-foundation fragments in this repository.
