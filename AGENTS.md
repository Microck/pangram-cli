# Pangram CLI agent instructions

These instructions are project-specific. Global agent rules still apply.

## Product state

The repository contains a corrected pre-implementation specification, not a
working runtime. Do not describe planned commands as available until compiled
contract tests prove that they exist.

The intended public product is an unofficial, MIT-licensed Pangram CLI, TUI,
and MCP server. The repository may remain private during development, but
public open-source distribution is the destination.

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

The committed schemas are seed contracts until Phase 0 imports them into
Rust-owned types and proves equivalent regeneration. After that bootstrap,
Rust-owned types generate:

- JSON Schemas
- CLI reference and help fixtures
- MCP tool schemas
- error and exit-code reference
- embedded skill content
- Fumadocs reference inputs

Generated artifacts are committed. CI must fail when regeneration changes the
tree. During the documented bootstrap only, seed schemas are specification
owned. After ownership transfers, do not hand-edit a generated file; edit its
owning type or generator.

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
