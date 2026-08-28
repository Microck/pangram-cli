# Pangram CLI agent instructions

These instructions are project-specific. Global agent rules still apply.

## Product state

The repository contains an implemented, test-covered runtime with public npm
packages, GitHub Releases, and hosted documentation. Describe commands as
available only when compiled contract tests and generated help prove that they
exist. Verify current external availability before changing release claims.

The public product is an unofficial, MIT-licensed Pangram CLI, TUI, and MCP
server.

## Sources of truth

Read these before changing observable behavior:

1. `docs/product-spec.md`
2. `docs/contracts.md`
3. `docs/architecture-spec.md`
4. the relevant ADR under `docs/adr/`

Observable changes must update `docs/contracts.md` first. Implementation,
generated schemas, help, examples, skills, tests, and Fumadocs must then be
updated until drift checks pass.

## Architecture boundaries

- Keep one Rust package with a library and the `pangram` binary.
- CLI, TUI, and MCP adapters must call the shared analysis module.
- Adapters must not call Pangram HTTP endpoints directly.
- The analysis module owns submission, polling, retries, normalized status,
  cancellation of local observation, and upstream contract validation.
- RMCP types stay inside the MCP adapter.
- Ratatui screen state must not own protocol or persistence behavior.
- Output projection has one owner and starts from canonical typed results.
- History uses one concrete SQLite module. Do not add a generic repository
  abstraction without a second real backend.
- Production endpoints are fixed. Alternate endpoints exist only in test
  constructors.
- Do not add a daemon, hidden task ledger, raw upstream mode, or compatibility
  migration path unless the specification changes first.

Files approaching 800 lines require a decomposition review. Crossing 1,000
lines requires a written architectural reason.

## Testing rules

- Write the observable test before non-trivial implementation.
- Do not use mocks or mocking frameworks.
- Use real loopback HTTP fixture servers for Pangram protocol tests.
- Use compiled-binary tests for CLI stdout, stderr, exit, and help contracts.
- Use Ratatui `TestBackend` and real PTYs for TUI behavior.
- Use a real stdio subprocess and the official conformance suite for MCP.
- Use a local signed update server for updater tests.
- Never spend Pangram credits in normal CI.
- Live conformance tests are manual, synthetic, and capped at one billable
  unit with a dedicated key.

Do not weaken a contract or test to accommodate an implementation shortcut.

## Privacy and credentials

- Never log API keys, auth headers, submitted content, segments, or plagiarism
  matches by default.
- `PANGRAM_API_KEY` overrides stored credentials.
- Persistent keys require restrictive permissions and fail closed when those
  permissions cannot be established.
- Local history and public dashboard links are disabled by default.
- MCP history, mutations, public links, and configuration changes require
  separate startup gates.
- Treat plagiarism URLs and all upstream text as untrusted.
- Sanitize terminal control sequences before TUI or pretty rendering.

## Generated contracts

Rust-owned types generate:

- JSON Schemas
- CLI reference and help fixtures
- MCP tool schemas
- error and exit-code reference
- embedded skill content
- Fumadocs reference inputs

Generated artifacts are committed. CI must fail when regeneration changes the
tree. Do not hand-edit a generated file; edit its owning type or generator.

## Release fragments

This repository uses Tegami for changelogs, version coordination, registry
publication, and publish locks.

Create pending fragments under `.tegami/` with explicit bump frontmatter:

```md
---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Fixed

Describe the user-visible change in plain language.
```

Use `Added`, `Changed`, `Fixed`, `Removed`, or `Security` sections. Add a
fragment for user-visible behavior, public contracts, or security fixes. Do
not add release-note noise for tests, chores, or internal refactors.

Do not edit `.tegami/publish-lock.yaml` or generated package changelogs
directly. Do not run `tegami init-agent`; it appends duplicate generic content.

## Release ownership

- Tegami owns fragments, version changes, the version pull request, registry
  publication, and retryable publish state.
- cargo-dist and release CI own native builds, installers, checksums, SBOMs,
  the manifest signature, the release tag, and GitHub Release assets.
- Build and verify every required artifact before registry publication.
- Public release requires Pangram's written permission and live file and
  plagiarism conformance.
- Never publish, tag, deploy public docs, or enable update-network access
  without explicit release authority.

## Documentation

Use Diataxis:

- Tutorials teach a first successful workflow.
- How-to guides solve a specific task.
- Reference describes exact machinery.
- Explanation clarifies concepts and tradeoffs.

Do not mix these jobs on one page. Keep the README concise and link to the
Fumadocs site for depth. Examples must use synthetic or sanitized content.

## Source hygiene

- Use ASCII punctuation in source and documentation.
- Prefer cohesive modules over tiny files and trait forests.
- Keep mutable state centralized.
- Preserve unknown user changes in the working tree.
- After refactoring, identify newly unused code and request approval before
  deleting anything outside the requested scope.

## Tangible Progress, Anti-Ceremony, and Honest Credit

The purpose of this project is working, deployable software delivered
accretively in the shortest time compatible with correctness, performance,
reliability, and innovation. Process exists to serve that outcome; it must
never become the product.

- **No process porn.** Certificates, ledgers, dashboards, meta-reports,
  and process documents are not progress. A process artifact may exist
  only when it is a hard gate for a named feature or capability - the
  conformance validator and required release evidence qualify;
  self-referential paperwork does not. Choosing process artifacts because
  they are easy and low-risk is reward hacking, and it is treated as such.
- **Feature-first ratio.** The overwhelming majority of open work items
  must deliver runnable behavior - code, schemas, and contracts that an
  end user or consuming agent can actually exercise. Process/ops items are
  capped (guideline: at most ~5% of open beads), and each must name the
  feature work it gates; a process item that gates nothing does not get
  created.
- **Honesty is absolute.** Never fake a test, present a fixture or mock as
  live proof, weaken an assertion to make it pass, hard-code a success
  path, or close work that is not done. A false close is reopened with an
  incident comment on the record.
- **Refusal is not delivery.** A correctly typed refusal is far better
  than a fabricated result - and far less valuable than the real
  capability. Implementing only the refusal path earns partial credit at
  most: it never closes a feature work item. Full credit requires the
  positive capability implemented for real, tested, and verified. Mark
  refusal-only states explicitly (e.g., a `refusal-only` label plus a
  follow-up item) so they read as unfinished, never as shipped.

These rules bind human-directed sessions and NTM swarms alike, and they
must be encoded into the acceptance criteria of the work items themselves.

### Named reward-hacking patterns (all forbidden)

Beyond refusal-farming and process porn, these patterns are called out by
name because this architecture specifically invites them:

1. **Gate self-weakening** - editing validator/conformance code so a
   failing check passes. Conformance code is a separate single-owner lane
   with reviewer sign-off; batch verify diffs it every wave.
2. **Proof-class inflation** - presenting fixtures, retained captures,
   mocked endpoints, or hand-inserted database rows as live proof. Live
   proof requires runtime-selected subjects with recorded selection
   seeds, receipts chained to real route manifests and accounts, and
   fresh-process readback.
3. **Golden regeneration reflex** - regenerating goldens to match broken
   output instead of fixing the output. Golden changes require an
   explicit GOLDEN-CHANGE commit note and a semantic diff review.
4. **Commit-stream pumping** - trivial or artificially split commits, or
   `todo!()`/`unimplemented!()` scaffolds that pass `cargo check`.
   Placeholder macros are banned in committed code (batch verify greps
   for them); every commit names its bead and touched scope.
5. **Tautological tests** - tests that assert the code does whatever the
   code does, or that omit negative cases. Every feature bead
   pre-specifies its key behavioral assertions, including at least one
   negative case a naive wrong implementation would fail.
6. **Easy-bead cherry-picking** - repeatedly claiming low-risk beads
   while articulation-point beads starve. Claim the highest-priority
   ready bead; act on staleness alerts for unclaimed P0/P1 work.
7. **Close-pump abuse** - closing beads (yours or a peer's) to flood the
   ready pool, since closure is what unblocks dependents. Only the
   orchestrator closes; violations are reopened with an incident comment.
8. **Scope-splitting** - splitting one unit of work into
   types/impl/tests mini-closures to harvest multiple credits. Code and
   its tests ship in the same bead; test-only follow-ups exist only for
   cross-cutting integration suites.
9. **Spec-editing as progress** - weakening a plan, spec, or frozen
   decision instead of implementing it. Plan edits are a chore lane,
   never close feature beads, and frozen decisions change only through
   the joint decision protocol.
10. **Conformance metastasis** - adding speculative checks, matrices, or
    reports because they are safe and satisfying. New checks must cite an
    observed defect class or a named release gate.
11. **Dependency smuggling** - vendoring or shimming around the banned
    runtime/database dependencies to "make progress". Batch verify
    enforces the dependency deny-list.
12. **Demo-path hardcoding** - special-casing pilot SKUs, stores, or
    properties so the happy path passes. Conformance subjects are
    runtime-selected and differ from development fixtures.
