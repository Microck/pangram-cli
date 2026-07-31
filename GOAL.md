# Pangram CLI completion goal

Status: implementation
Destination: public v1
Created: 2026-07-23

## Destination

Build, verify, document, and release Pangram CLI as a public MIT-licensed
product for humans, shell automation, and AI agents.

The finished product provides one behavioral implementation through:

- a JSON-first command-line interface
- an interactive Ratatui terminal interface
- a typed stdio MCP server
- searchable Fumadocs documentation
- signed native releases and supported package-manager distribution

## Completion criteria

This goal is complete only when:

1. every in-scope capability in the normative specifications works through its
   planned adapters
2. every contract, unit, integration, PTY, conformance, documentation, package,
   installation, and update check passes
3. the agent-owned TUI baseline and autonomous acceptance suite pass, and the
   user has approved the quality of the final product
4. live Pangram conformance has resolved the documented file and plagiarism
   response conflicts
5. Pangram has granted written permission for public third-party API use and
   terminal fox-logo artwork
6. artifacts bound by the signed manifest install and update on every supported
   target
7. the Fumadocs site is deployed at `pangram.micr.dev`
8. npm, Homebrew, and Scoop distribution is live
9. the GitHub repository is public under MIT
10. no unresolved P0 or P1 test, security, or maintainability finding remains

All locally controllable work MUST continue while an external gate is blocked.
The same external blocker repeating twice ends that execution cycle with a
clear blocked report; it does not broaden authority or permit a workaround.

## Normative sources

Read these before implementation:

1. [Product specification](docs/product-spec.md)
2. [Observable contracts](docs/contracts.md)
3. [Architecture specification](docs/architecture-spec.md)
4. [Testing and release plan](docs/testing-release-plan.md)
5. [Implementation roadmap](docs/implementation-roadmap.md)
6. [Documentation plan](docs/documentation-plan.md)
7. [History contract](docs/history-contract.md)
8. [MCP contract](docs/mcp-contract.md)
9. [Update contract](docs/update-contract.md)
10. [Local setup contract](docs/local-setup-contract.md)
11. [Architecture decisions](docs/adr/)

The observable contracts take precedence over implementation. A discovered
contract flaw requires a contract-first correction, not a compatibility path
or local exception.

## Decisions already locked

- Runtime: Rust 2024 in one package until another package boundary earns its
  cost.
- Documentation: TypeScript, Next.js, Fumadocs, and Tegami.
- Executable: `pangram`.
- Product surfaces: CLI, TUI, and stdio MCP over one analysis module.
- Primary workflow: AI detection.
- Secondary workflow: plagiarism.
- Noninteractive default output: canonical JSON, with JSONL as the repeated-file
  default.
- Persistence: local history disabled by default, with explicit manual and
  automatic save paths.
- Authentication: persistent `pangram auth set --api-key VALUE`, environment
  override, and interactive first-launch setup.
- TUI: Analyze, Active, History, and Settings with regular and Vim keymaps.
- TUI visual direction: compact route rail, dominant center workspace, stable
  right inspector, and a restrained contextual command bar. The resolved fox
  shrinks after the intro; sparse separators replace dashboard-style card
  chrome.
- Intro: Pangram fox mark, precomputed 56-frame terminal sequences, 20 fps,
  2.8-second nominal timeline, and `once` by default.
- Updates: signed direct updates, manager-owned install detection, and an
  optional TUI startup check.
- Privacy: no telemetry and no default public links or retained analysis
  history.
- License: MIT.
- Public destination: `Microck/pangram-cli`.
- Feature parity: match Pangram's web analysis capabilities when they have a
  documented public interface. Mirror useful web workflows and presentation,
  but do not depend on undocumented dashboard behavior.
- Release safety: no undocumented Pangram routes, browser automation, session
  cookies, or copied Droid material.
- TUI acceptance: deterministic Ratatui snapshots and native PTY tests remain
  the correctness gates; a pinned Terminal Control harness drives the compiled
  TUI, captures settled text and cell frames, exercises input and resize, and
  produces sanitized visual evidence. The agent selects and maintains the
  baseline from the locked design rubric; the user reviews only final product
  quality.

## Execution policy

Implementation MUST:

1. follow the phase order in the implementation roadmap, including its narrow
   external-blocker exception
2. update observable contracts before behavior
3. add a failing test before non-trivial implementation
4. use real loopback servers, temporary files, SQLite, subprocesses, and PTYs
   instead of mocks
5. keep the analysis module deep and adapters thin
6. preserve successful partial results
7. log material deviations and newly discovered unknowns in this file
8. verify focused behavior before moving to the next phase
9. run the full release-candidate suite before requesting publication
10. stop for user input only when a choice changes scope, architecture,
    security, cost, public behavior, data semantics, or external authority

Parallel implementation may use subagents only for isolated tasks with
disjoint file or module ownership. Every subagent receives the relevant
contracts, constraints, expected output, and verification gate. The primary
agent remains responsible for reviewing, integrating, and testing all returned
work.

Version-control checkpoints use jj. Before Phase 0, create one baseline
specification commit after validating the complete planning diff. After that,
create one or more logical commits only when the owned phase behavior and its
relevant verification pass. Inspect `jj status` and `jj diff` before every
commit. Commit authority by itself does not permit a push; the implementation
pull-request workflow below grants that narrower authority.

Implementation delivery uses phase-sized pull requests:

1. create a semantic bookmark from the current default branch
2. push only verified implementation commits
3. open the pull request without waiting for a separate draft confirmation
4. verify the current head, required checks, reviews, comments, and unresolved
   threads
5. trigger available CodeRabbit, Greptile, and Codex reviewers once per head
6. fix valid findings, rerun relevant verification, push, and request fresh
   review only when the head changed
7. merge when required checks pass and no actionable finding or unresolved
   thread remains
8. continue from the updated default branch

Use the repository-required merge method. If none is required, squash each
phase pull request. Never push implementation commits directly to the default
branch. Automated review-trigger comments are authorized for this repository.
The implementation PR authority ends when the verified release candidate is
merged. It does not include Phase 9 publication actions.

Routine implementation decisions stay with the implementer when they fit the
approved contracts.

## Work map

| Phase | Status |
| --- | --- |
| 0 | Complete |
| 1 | Complete |
| 2 | In progress |
| 3 | Not started |
| 4 | Not started |
| 5 | Not started |
| 6 | Not started |
| 7 | Not started |
| 8 | Not started |
| 9 | Blocked by release authority |

The detailed work and phase exit criteria remain in the
[implementation roadmap](docs/implementation-roadmap.md). This table records
progress without duplicating the roadmap.

## Verification gates

A release candidate requires:

- formatting and lint checks
- all Rust tests
- generated-contract drift checks
- loopback Pangram protocol fixtures
- CLI stdout, stderr, exit, help, and format contracts
- real SQLite history tests
- Ratatui reducer and rendering snapshots
- real PTY startup, resize, interruption, skip, and restoration tests
- Terminal Control settled-screen, keyboard, resize, and failure-artifact
  acceptance tests on GNU/Linux and macOS
- MCP protocol and official conformance checks
- updater signature, receipt, tamper, and atomic replacement tests
- Fumadocs typecheck, build, search, generated references, and link checks
- target build and clean-install smoke tests
- dependency, license, secret, and artifact provenance audits
- thermo-nuclear maintainability review with no unresolved P0 or P1 finding

Publication additionally requires live Pangram conformance, written permission,
visual intro acceptance, signing material, registry ownership, and explicit
production authority. One authorization covers one exact version and the
complete named destination set: repository visibility, GitHub Release,
production documentation, npm, Homebrew, and Scoop. It does not authorize a
different version, destination, release channel, or later release. A partial
failure may retry only the authorized version and destinations.

## Authority boundaries

This file defines the destination. It does not by itself authorize every
external action needed to reach it.

Allowed once implementation starts:

- edit files in this repository
- install development dependencies with the repository package manager
- run local builds, tests, servers, PTYs, browsers, and package smoke tests
- make reversible local implementation decisions inside the locked contracts
- use bounded parallel subagents with disjoint ownership and primary-agent
  integration
- create a validated baseline specification commit and logical verified phase
  commits with jj
- push verified semantic implementation bookmarks
- open, update, review, and merge implementation pull requests
- trigger CodeRabbit, Greptile, and Codex review, then fix valid findings
- use securely supplied Pangram test credentials for at most 10 current
  Pangram billable units across the complete live-conformance run, with at
  most one unit per scenario and no automatic billable retry

Requires explicit authority:

- close, delete, or otherwise mutate standalone GitHub issues
- create or mutate pull requests unrelated to this implementation goal
- purchase Pangram billable units, enable auto-refill, or exceed the
  10-billable-unit live-conformance ceiling
- publish packages or formulas
- deploy production documentation
- create releases or tags
- change repository visibility

## Unknowns ledger

### Known knowns

- The product, command, output, persistence, update, documentation, release,
  and architecture contracts have completed their pre-Phase-0 correction pass.
- File and plagiarism response shapes need live confirmation.
- Pangram 4 is the only intended text model. Its published model card defines
  result semantics. The Pangram SDK v1.0.0 tag documents the text
  model-selection request field (`model` set to `pangram-4`), while the bulk
  billing contract remains undocumented.
- The Pangram 4 text request selector is resolved. The Pangram SDK v1.0.0 tag
  (`ca42297`, 2026-07-29) documents `POST
  https://text.external-api.pangram.com/task` with JSON field `model` set to
  `pangram-4`, optional `public_dashboard_link`, `x-api-key` auth, poll `GET
  /task/{id}`, success version `4.0`, and `is_humanized` plus `humanizer_score`
  on every Pangram 4 window.
- The intro behavior is locked. The agent owns its initial visual baseline and
  autonomous acceptance. The user reviews the quality of the final product,
  not individual baseline snapshots.
- `@kitlangton/terminal-control 0.6.0` is the development-only TUI acceptance
  harness. It does not ship with Pangram CLI and does not create a semantic UI
  adapter.
- Public API and logo use require written Pangram permission.
- Live conformance may use at most 10 current Pangram billable units across
  securely supplied test keys. Credentials MUST NOT appear in chat, repository
  files, logs, fixtures, diagnostics, or retained command output.
- Live credentials are supplied to the conformance harness at runtime from a
  private operator-controlled source.

### Known unknowns

1. What is the SHA-256 and provenance of the replacement fox source geometry?
2. Which stable RMCP 3 release and resulting minimum Rust version are current
   when Phase 6 begins?
3. What billing unit and request ceiling apply to Pangram 4 bulk work?

### Unknown knowns

- The exact terminal fox rendering quality is a taste decision that requires a
  generated prototype.
- Final TUI density, hierarchy, and motion feel require inspection in a real
  terminal even though their behavioral contracts are fixed.
- Documentation voice may need a final editorial reaction after the generated
  reference and tutorial pages exist.

### Suspected unknown unknowns

- Live Pangram responses may differ from the documented examples beyond the
  two known conflicts.
- Supported-target terminal capabilities may expose rendering or restoration
  behavior not visible through Ratatui snapshots.
- Package registries or signing systems may impose current constraints that
  differ from the research snapshot.
- Pangram permission may constrain naming, logo treatment, screenshots, or
  required attribution.

## Decision queue

No unresolved design decision blocks Phase 0. Add a decision here only when a
newly discovered choice changes the destination or observable behavior.

Low-risk implementation details discovered later receive a documented default.
Any newly discovered choice that changes the destination or observable
behavior enters this queue before implementation continues through that seam.

## Deviation log

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
- 2026-07-29: Pangram 4 launched before its public REST reference and SDK
  documented the selection field and updated bulk billing contract. Pangram
  CLI now targets Pangram 4 only and blocks text and bulk submission rather
  than relying on Pangram 3 default routing or old billing assumptions.
- 2026-07-31: The Pangram SDK v1.0.0 tag (`ca42297`) documented the Pangram 4
  text selection field (`model` set to `pangram-4`) while the rendered REST
  reference still omitted it. Text submission was unblocked contract-first and
  pinned to the explicit selector, the no-default/no-fallback rule was kept,
  and Phase 3 bulk submission remains blocked on the undocumented bulk billing
  contract. The domain gained the canonical started-100-word text billable-unit
  rule (`text_billable_units`) used later by CLI preflight and the Analyzer.
- 2026-07-31: The locked v1 output-projection contract includes TOON, but the
  pinned `toon-format 0.5.0` requires Rust 1.87 (its decode parser uses
  `unsigned_is_multiple_of`, stabilized in 1.87.0, and fails on 1.85 with
  `E0658`). Applying the architecture's lowest-dependency-compatible
  `rust-version` rule, the package MSRV rose from 1.85 to the exact minimum
  1.87.0 rather than to the RMCP 1.88 prerelease floor, and the CI MSRV leg
  moved with it. Current stable remains 1.97.1.

## Progress log

- 2026-07-23: Locked feature parity to documented public Pangram analysis
  interfaces. Undocumented dashboard routes and browser scraping remain out of
  scope.
- 2026-07-23: Authorized bounded parallel subagents for isolated work with
  disjoint ownership and primary-agent review.
- 2026-07-23: Authorized a validated baseline specification commit and logical
  jj commits after verified implementation phases.
- 2026-07-23: Authorized phase-sized implementation pull requests, automated
  CodeRabbit, Greptile, and Codex review loops, fixes, and autonomous merge to
  the default branch after all gates pass. This authority ends at the merged
  release candidate.
- 2026-07-23: Authorized at most 10 free-tier Pangram billable units for live
  conformance, with one billable unit per scenario and no automatic billable
  retry. Credentials pasted into chat are treated as compromised and require
  replacement through secure local secret injection before use.
- 2026-07-23: Locked Terminal Control 0.6.0 as the development-only autonomous
  TUI acceptance harness. Ratatui snapshots and native PTY tests remain the
  lower-level correctness gates; no OpenTUI semantic adapter will be added.
- 2026-07-23: Assigned initial TUI and intro baseline selection to the agent.
  The user may choose a concept direction and reviews only final product
  quality.
- 2026-07-29: Adopted Pangram 4 as the only production text model. Added its
  humanizer evidence and segment-offset semantics to the canonical result
  contract, and changed text estimates to one billable unit per started 100
  words.
- 2026-07-29: Deferred image detection until Pangram publishes and generally
  opens a documented Image API. Invitation-only preview access, private
  dashboard routes, and compatibility code remain out of scope.
- 2026-07-23: Locked the TUI direction to Concept B's three-area information
  architecture with Concept C's restrained chrome and command bar.
- 2026-07-27: Locked production publication to one explicit authorization for
  one exact version and its complete named destination set. Retries remain
  scoped to that authorization.
- 2026-07-28: Started Phase 0. The pre-implementation audit made parent-state
  precedence explicit and tightened the seed output schema to enforce UTC `Z`
  timestamps, AI-first two-check ordering, and command-specific single versus
  repeated analysis results.
- 2026-07-28: Revalidated the Rust toolchain and Phase 0 dependency baseline.
  Current stable is Rust 1.97.1, and the lowest selected direct-dependency MSRV
  is Rust 1.87 (raised from 1.85 on 2026-07-31 when `toon-format 0.5.0` set the
  floor; see the deviation log and evidence ledger).
- 2026-07-28: Retargeted the planned stdio server to MCP 2026-07-28 before MCP
  implementation began. File access now requires explicit startup-approved
  roots, the removed initialization lifecycle has no compatibility path, and
  the experimental Tasks extension remains out of v1.
- 2026-07-28: Phase 0 passed its local exit gates. The compiled binary exposes
  only help and version, the Rust-owned generator reproduces the committed
  contract set, and a shared transfer corpus passes against every baseline seed
  and generated schema. Current-only regressions record the two documented
  contract-first seed corrections. Formatting, strict Clippy, Rust 1.85
  compatibility (the MSRV at that time, now 1.87), repository hygiene,
  dependency audit, license policy, secret scanning, and workflow linting are
  green.
- 2026-07-30: Phase 1 implementation is in place. Strict configuration,
  atomic credential persistence with Unix `0600` and owner-only protected
  Windows ACL enforcement, the `auth`, `config`, and non-billable `doctor`
  commands, and a `windows-latest` CI gate running the real Win32 credential
  ACL integration tests are implemented with focused current-Rust tests green.
  Phase 1 remains In progress until the separately delegated full validation
  suite passes on the integrated tree.
- 2026-07-30: Final-review remediation resolved both P1 findings
  contract-first and test-first. `config get`/`config list` now report one
  effective configuration: absent keys resolve to documented built-in defaults
  (typed bool/number/string), and the pre-onboarding
  `updates.check_on_tui_start` reports `null` rather than the never-documented
  `(unset)` sentinel. `doctor` exits 7 (the canonical local-state code) when
  any check is `fail` while still emitting the complete typed checks payload
  in both JSON and pretty projections; pass/warn-only reports remain exit 0
  and stdout render failures remain general failure 1.
- 2026-07-30: Phase 1 final validation passed on the integrated tree:
  formatting, strict Clippy, full current-Rust tests, generated-contract
  drift, repository hygiene, cargo audit, cargo deny licenses+bans, gitleaks
  secret scanning, and CI workflow structural checks were green, and the
  Windows credential ACL integration tests were validated locally on the
  cross-compiled target with the `windows-latest` native CI gate configured.
  Independent review returned READY with no unresolved P0 or P1 findings, so
  Phase 1 moves to Complete.
- 2026-07-31: Phase 2 contract and domain foundation lands. The Pangram SDK
  v1.0.0 tag documented the Pangram 4 text selector, so the normative
  contract, architecture, and product text were unblocked for text only and
  pinned to `model` = `pangram-4` with the no-default/no-fallback rule kept,
  the rendered-docs staleness was recorded as a caveat rather than a protocol
  unknown, and the evidence ledger gained sourced protocol and bulk-blocker
  rows. The domain gained the canonical `text_billable_units` rule with
  property and overflow-boundary tests. Phase 3 bulk submission and the bulk
  billing unknown remain open, so Phase 2 stays In progress.
- 2026-07-31: Phase 2 independent-review remediation landed contract-first
  and test-first. Bare piped stdin now detects (the `--help` fallback moved
  behind bare dispatch; empty or whitespace-only pipes return
  `input_required`), the undocumented 300-second default wait ceiling was
  removed in favor of the documented unbounded wait, and cancellation of an
  issued billable POST now reports the canonical `submission_outcome_unknown`
  reconciliation outcome instead of a false definite no-remote-action claim
  while SIGINT still exits 130. Repeated files under an explicit
  single-document format render one ordered array envelope instead of failing
  after billable work; an upstream terminal `STAGE_FAILED` exits 6 per its
  upstream category; explicit `--format pretty` failures surface as sanitized
  stderr text with empty stdout; the `--timeout` grammar rejects whitespace,
  exponent, non-finite, zero, and out-of-range forms; `--save` is planned in
  the generated reference and rejected by the runtime until Phase 4 history;
  the README reflects the compiled surface; and the protocol suite was
  decomposed into submission/observation/contract-matrix modules below the
  hygiene threshold. Remote Yoga smoke, MSRV (1.87), and gate (fmt, full
  tests, strict clippy) passes are green together with drift, hygiene,
  audit/deny, gitleaks, and tegami-shape checks.

## Out of scope

v1 excludes:

- undocumented Pangram dashboard behavior
- account, credit, or billing administration
- a daemon or remote HTTP MCP server
- a public Rust SDK stability promise
- remote history synchronization or import
- telemetry
- endpoint overrides or insecure TLS
- pre-release compatibility shims
- copied Droid code, frames, or artwork
- invitation-only Pangram Image API integration
