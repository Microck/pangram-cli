# Pangram CLI deviation log

This historical log records deviations from [the completion goal](../GOAL.md).
The completion goal remains the source of truth for destination, authority,
policy, and current completion criteria.

- 2026-07-28: The GitHub repository was already public when implementation
  began, although repository visibility is listed as a Phase 9 publication
  action. No visibility change was made. Implementation pull requests remain
  authorized, while releases, packages, and production documentation remain
  gated.
- 2026-07-28: The roadmap's final loopback detection slice is treated as a
  Phase 0-through-2 milestone. Phase 0 retains its explicit network-free exit
  criterion.
- 2026-07-28: Phase 0 transfer review found that the baseline seed output
  schema did not reserve `data` and `error` across envelope branches or require
  the canonical duplicate-billing recovery warning. Preserving those defects
  for literal seed equivalence would violate the observable contract. They
  were corrected before ownership transfer completed and are covered by
  current-only regressions.
- 2026-07-29: Pangram 4 launched before its rendered public REST reference and
  SDK release documented the selection field and updated bulk billing
  contract. The initial correction targeted Pangram 4 only and blocked text
  and bulk submission rather than relying on Pangram 3 default routing or old
  billing assumptions.
- 2026-07-31: The Pangram SDK v1.0.0 tag (`ca42297`) documented the Pangram 4
  text and job-wide bulk selection field (`model` set to `pangram-4`) while the
  rendered REST reference still omitted it. Text submission was unblocked
  contract-first and pinned to the explicit selector, the no-default/no-fallback
  rule was kept, and the domain gained the canonical started-100-word text
  billable-unit rule (`text_billable_units`) used later by CLI preflight and
  the Analyzer. The bulk billing source had not yet been located in this
  research pass.
- 2026-07-31: The locked v1 output-projection contract includes TOON, but the
  pinned `toon-format 0.5.0` requires Rust 1.87 (its decode parser uses
  `unsigned_is_multiple_of`, stabilized in 1.87.0, and fails on 1.85 with
  `E0658`). Applying the architecture's lowest-dependency-compatible
  `rust-version` rule, the package MSRV rose from 1.85 to the exact minimum
  1.87.0 rather than to the RMCP 1.88 prerelease floor, and the CI MSRV leg
  moved with it. Current stable remains 1.97.1.
- 2026-08-12: Phase 6 selected stable RMCP 3.1.2 and its Rust 1.88 minimum, so
  the dependency-driven package MSRV rises from 1.87 to 1.88 when the pin is
  implemented. The official conformance suite cannot drive stdio while
  upstream issue 258 is open. The release gate therefore combines official
  conformance against a non-shipping RMCP HTTP fixture with independent
  compiled-server stdio contract tests, without treating either transport as
  proof of the other.
- 2026-08-13: Implementation inspection of the pinned conformance source found
  that its frozen `2026-07-28` server requirements mandate a full diagnostic
  server instead of selecting tests from advertised capabilities. The planned
  HTTP fixture would therefore test added fixture-only tools, prompts,
  templates, resources, and interaction flows rather than Pangram's shipping
  handler. Phase 6 removed that false proof path, kept the compiled stdio suite
  as the product gate, and retained the exact official pin for re-evaluation
  when stdio and capability-aware selection are available.
