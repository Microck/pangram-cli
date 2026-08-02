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
| 2 | Complete |
| 3 | Complete |
| 4 | In progress |
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
  result semantics. The Pangram SDK v1.0.0 tag documents the text and job-wide
  bulk model-selection request field (`model` set to `pangram-4`). Pangram's
  official API reference documents per-item started-100-word bulk billing and
  the 1,000-unit request limit.
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
  property and overflow-boundary tests. Phase 3 remained out of this Phase 2
  packet, so Phase 2 stayed In progress.
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
- 2026-07-31: Phase 2 moves to Complete. The compiled CLI completes Pangram 4
  text detection against the real loopback fixture server through `detect`
  and bare input, every adapter-visible result renders from the canonical
  typed envelope (JSON, JSONL, TOON, Markdown, pretty), and no adapter
  contains Pangram protocol logic, proving the roadmap's Phase 2 exit
  criteria. Independent review returned READY with no P0, P1, or P2 findings;
  remote Yoga current/MSRV/gate, generated drift, hygiene, supply-chain,
  gitleaks, and the no-network policy are green, and the native Windows ACL
  and generated/supply-chain CI gates are exercised on the delivery pull
  request. Phase 3 remains separate planned work rather than a Phase 2 gap.
- 2026-07-31: Pangram's official Mintlify API source at `eb214f4` resolves the
  Phase 3 external entry contract. A Pangram 4 bulk job uses one selector for
  the whole request, bills each valid item in started 100-word units with a
  minimum of one, and accepts at most 1,000 billable units. No separate item
  count limit is documented. Phase 3 may proceed with loopback implementation;
  public support still requires live conformance.
- 2026-08-01: The first bounded Phase 3 packet corrects the bulk-submit seed
  grammar. The Rust-owned grammar had drifted from the normative contract by
  seeding a `--public-link` bulk-submit flag, while contracts.md 14.3 and
  docs/mcp-contract.md lock bulk against Pangram's Bulk API, which documents no
  public-dashboard-link request or response field. The contradictory seed is
  removed contract-first and test-first; bulk and task surfaces remain planned
  (the compiled help/runtime still exposes none of them), detect's contracted
  `--public-link` is unchanged, and the generated reference was regenerated
  through the official generator. No MCP tool schemas exist yet, so docs/
  mcp-contract.md remains the sole MCP bulk surface contract. Phase 3 stays In
  progress; public support still requires implementation plus live
  conformance.
- 2026-08-01: The second bounded Phase 3 packet locks the official bulk
  wire/domain contract and lands the real Axum loopback fixture foundation.
  The official Bulk API source `eb214f4` was re-verified as the latest commit
  on `api-reference/bulk-api.mdx` on 2026-08-01, and contracts.md gained
  section 9.1 pinning the exact documented wire shapes: submit `items`/`text`
  plus one job-wide `model` (`pangram-4`, no per-item selector, no
  public-link field), the 202 accepted/failed item lists, the status
  counters, items/results pages (offset/limit, max limit 1,000), epoch-second
  string timestamps, 48-hour terminal retention, and the 401/402/403/404/413/
  422/500/503 error matrix. `src/domain/bulk.rs` adds the typed,
  constructor-validated `BulkSubmissionItem`/`BulkSubmissionPlan` (ordered
  items, unique caller IDs, the min(caller ceiling, 1000) effective ceiling,
  checked estimate, whole-file JSONL validation) and the deserialization
  fixture wire types for submit/status/items/results responses.
  `tests/support/protocol_loopback/bulk.rs` extends the real Axum fixture
  with the four documented `/bulk` routes, scripted queues, and a loopback
  `BulkProbeClient` (real reqwest against the fixture, decoding 2xx bodies
  into the domain wire types); eleven `bulk_protocol` integration tests prove
  the request grammar, 413/no-replay, terminal failure, partial child
  results, page offset/limit/query, and stalled/safe-retry surfaces. The
  route URL derivation lives behind a `dev-tools`-gated
  `UpstreamEndpoints::bulk_*` accessor; no production endpoint constants or
  production bulk client exist yet. The compiled CLI, README availability,
  and MCP surface are unchanged (no capability Tegami). Generated contracts
  show no drift: the new wire types are fixture spines, not public output
  schema types. Phase 3 stays In progress; public support still requires the
  production analysis client and live conformance.
- 2026-08-01: The third Phase 3 packet review remediation landed
  contract-first and test-first. The bulk core and upstream client are
  decomposed below the 1,000-line hygiene gate (the bulk observation/paging
  pipeline into `src/analysis/bulk/{mod,assemble}.rs`, the bulk submit/page
  client into `src/analysis/upstream/bulk.rs`), the unrequested implementation
  diary is removed, and the bulk surface is folded behind the single
  adapter-facing `Analyzer` so adapters never own a second protocol client.
  The wire core pins bulk submit success to exactly HTTP 202 (any other 2xx
  is never replayed and surfaces the ambiguous `submission_outcome_unknown`),
  validates the 202 acceptance `status` token against the closed `queued`
  value, normalizes documented `result: null` results-page entries to the
  canonical `running` state, treats per-item `stage` as sanitized
  diagnostic-only evidence, and bounds every coverage allocation by the
  validated plan count (or, for a resumed remote handle, by the documented
  job cap); the fetch-all walk uses the conservative bounded 100-item page
  while explicit one-page reads keep `1..=1,000`. New loopback tests cover
  the documented GET status/error matrix with retry/no-retry proof, the
  202-only and undecodable-202 ambiguity, a failed index 0 plus succeeded
  index 1 window, and the hostile `u64::MAX`/plan-mismatch allocation guards.
  contracts.md and the shared fragment were updated contract-first. Phase 3
  stays In progress; public bulk support still requires live conformance.
- 2026-08-01: A fourth Phase 3 remediation packet landed contract-first and
  test-first, addressing the review findings on the CLI bulk/task activation.
  The bulk/task CLI adapter decomposed from `src/cli/bulk.rs` into the
  cohesive `src/cli/bulk/` modules (`mod`, `policy`, `plan`, `submit`,
  `status_wait`, `results`, `task`), each below the hygiene threshold and
  sharing the detection preparation, async runtime, and projection owners. A
  Rust-owned typed dry-run schema, closed `bulk_submit` union
  (`BulkDryRun`/`BulkSubmitOutput`), and projection now own the dry-run
  reconciliation shape, refreshed through the official generator with no
  drift beyond the intended singular closed union. Dozens of compiled-binary
  loopback tests across `tests/bulk-task-cli-loopback.rs`,
  `tests/bulk-task-status-results-loopback.rs`, and the shared
  `tests/support/bulk_cli_env.rs` lock the exact exit, stdout-envelope,
  stderr-separation, help, one-POST no-replay, and loopback grammar of the
  bulk and task surfaces. A non-JSON `--format` on `bulk submit` (submitted
  or dry-run) is rejected as `unsupported_combination` before any source
  read, plan validation, credential resolution, or network access. The real
  documented normalization is kept and tested: an upstream terminal
  `STAGE_FAILED` exits 6 per its upstream category. The minor Tegami bump is
  set for both packages. Phase 3 stays In progress; public bulk/task support
  still requires live conformance.
- 2026-08-01: The final Phase 3 remediation packet landed contract-first and
  test-first over the packet-3/4 surface. Observed bulk child analyses are
  now `accepted`, never `terminal` (contracts.md 4.6): the resumed plan=None
  results/items builders in `src/analysis/bulk/assemble.rs` emit observed
  success and observed failed children with the attested upstream identity, so
  a valid failed child or a text-less succeeded child no longer fails as
  `upstream_contract_changed`. `bulk submit` without `--wait` projects the
  validated HTTP 202 acceptance snapshot (truthful
  accepted/failed counters and derived collection status) instead of
  fabricating an all-queued-zero state, and any successfully normalized 202
  exits 0. A successful `bulk results` page or fetch-all read exits 0
  regardless of failed children on the returned window (one page is not
  authoritative for whole-job terminal state), and fetch-all reassembles one
  canonical aggregate window (`offset: 0`, `limit: max(1, total_items)`
  bounded by 1,000, no `next_offset`). The README lists bulk and task as
  compiled and available with the live-conformance caveat, and a compiled
  contract test pins the README availability list to the Rust-owned grammar.
- 2026-08-01: Phase 3 moves to Complete. The compiled CLI activates every
  contracted bulk and task surface against the real loopback fixture server
  (`bulk submit` with required `--max-billable-units` whole-file preflight,
  `bulk status|wait|results` with safe-GET paging, and `task status|wait`), so
  no bulk request starts without a validated cost ceiling, task and bulk
  waiting reuse the one analysis progress model behind the single
  adapter-facing `Analyzer`, a terminal `partial` result exits 3 through the
  status/wait surfaces while a successful page/fetch-all read stays
  machine-readable at exit 0 regardless of failed children (the documented
  exit mapping), proving the roadmap's Phase 3 exit criteria.
  Independent complete-chain review returned READY with no unresolved P0, P1,
  or warranted P2 findings; the safe static gates (fmt, hygiene/ASCII,
  one-package, audit/deny, gitleaks, workflow and no-network policy, Tegami
  shape, GOAL/contracts/evidence coherence) are green. The authoritative broad
  gates (current Rust, MSRV 1.87, generated drift, native Windows ACL, supply
  chain) are exercised on the delivery pull request because the remote Yoga
  lease is held by another checkout and must not be reclaimed. Public bulk
  support, live Pangram bulk conformance, and any public release stay gated.
  Phase 4 remains planned, separate work.
- 2026-08-01: The first bounded Phase 4 packet pins the SQLite dependency
  baseline evidence-first, before any `HistoryStore` exists, after the
  official-source research reversed the initial 0.40.1 selection. The newest
  `rusqlite 0.40.1`/`libsqlite3-sys 0.38.1` pair uses stable `cfg_select!`
  (stabilized in Rust 1.95) in rusqlite source and in the sys build script,
  so it effectively requires Rust 1.95 and exceeds the locked package MSRV of
  1.87. The locked selection is therefore `rusqlite = "=0.39.0"` with
  `default-features = false` and only `features = ["bundled"]`: the smallest
  feature selection satisfying architecture-spec 11.1 (bundled SQLite + FTS5
  + transactions) while excluding the default `cache` (hashlink) feature
  (rusqlite 0.39.0 declares no other default feature). rusqlite 0.39.0
  requires `libsqlite3-sys ^0.37.0`, which under Cargo's 0.x caret semantics
  resolves only to >=0.37.0, <0.38.0 and so can never select the
  incompatible 0.38.1; Cargo.lock pins the transitive sys crate at 0.37.0, so
  no direct sys dependency is carried. Its build script unconditionally
  compiles the vendored SQLite 3.51.3 amalgamation with
  `-DSQLITE_ENABLE_FTS5` plus FTS3, RTREE, JSON1, column metadata,
  `SQLITE_THREADSAFE=1`, `SQLITE_USE_URI`, and
  `SQLITE_DEFAULT_FOREIGN_KEYS=1`. A focused compiled probe
  (`tests/history-sqlite-baseline.rs`) proves the runtime reports
  `ENABLE_FTS5` through `PRAGMA compile_options` and SQLite 3.51.3 through
  `sqlite_version()`, executes the exact FTS5 virtual-table statement from
  the history contract (`tokenize = 'unicode61'`), and honors foreign-key
  enforcement. Both crates are MIT, and the vendored amalgamation is public
  domain. The evidence ledger gained the sourced version/feature/FTS5/
  MSRV/license rows.

  Compiled validation is now complete on a separate, disposable Crabbox box
  on Oracle Paris (the `oracle-paris` VPS, `100.96.124.15` via Tailscale)
  through the external `crabbox-paris-provider.sh` provider that runs
  per-lease Docker containers there (lease `cbx_d30368ca0f82`, slug
  `pangram-p4-sqlite-a73kf`, container `6645f5f9c9d9`, `crabbox:full` image,
  aarch64 Ubuntu 22.04.5, SSH port 22102). The run used stable `rustc`/`cargo`
  1.97.1 plus a rustup-installed `1.87.0` MSRV toolchain, an isolated
  workroot `/home/ubuntu/work/p4sv-a73kf/pangram-cli`, isolated stable and
  MSRV `CARGO_TARGET_DIR`s under `/home/ubuntu/cargo-target/p4sv-a73kf/`, and
  an isolated toolbuild target. The local Docker host (`local-container`),
  the `static_yoga` host, and the externally held
  `remoteuse-t7-profiles-policy` lease all stayed untouched. Remote Paris
  execution was verified from inside the box (`hostname` = `6645f5f9c9d9`,
  `uname -m` = `aarch64`, separate `overlay` root on the Paris VPS). The
  committed tree at HEAD was
  synced (excluding `.git`, `.jj`, caches, `target`, `.crabbox`, `tmp`), and
  `Cargo.lock` was regenerated by Cargo after the direct `libsqlite3-sys`
  edge removal (never hand-edited): the regenerated lockfile drops the stale
  direct sys edge, keeps `rusqlite 0.39.0` and `libsqlite3-sys 0.37.0`
  pinned, and carries only the compatible-semver transitive bumps Cargo's
  newer index resolves (`icu_* 2.2.0`, `idna_adapter 1.2.2`, `wasip2 1.0.4`,
  `wit-bindgen 0.57.1`, `displaydoc 0.2.7`, `hybrid-array 0.4.14`). All
  gates are green: the smoke equivalent (`cargo metadata`) passes; the MSRV
  equivalent under Rust 1.87.0 (`cargo build --locked --all-features` then
  the SQLite probe) passes with the baseline test 3/3; the focused SQLite
  probe also passes 3/3 on current stable 1.97.1 with a compiled
  `sqlite_version()=3.51.3` and `ENABLE_FTS5`/`DEFAULT_FOREIGN_KEYS`/
  `THREADSAFE=1` assertion; the gate equivalent passes (`cargo fmt --check`,
  `cargo test --locked --all-features` = 400 passed across 20 binaries,
  `cargo clippy --locked --all-features --all-targets -- -D warnings`); the
  generated-contract generator reproduces the committed set with no drift;
  `cargo audit` reports no vulnerabilities; and `cargo deny check licenses`
  against the regenerated lockfile and the authoritative CI allow-list
  reports `licenses ok`. No history schema, `HistoryStore`, runtime
  behavior, CLI surface, generated artifact, or capability activation
  changed: history remains unimplemented and disabled by default, all
  privacy and live-release gates stand, and the package bump is patch
  (foundation only). Phase 4 stays In progress; the HistoryStore core,
  history commands, and their real-SQLite contract suite are subsequent
  packets gated on this now-verified baseline.
- 2026-08-02: The second bounded Phase 4 packet lands the concrete
  `HistoryStore` under `src/history/` (`store`, `operations`, `records`),
  owning exactly the docs/history-contract.md schema v1 and the
  architecture-spec 11 responsibilities. The store fails closed on
  protection: on Unix it requires `0700` on the `history/` directory and
  `0600` on the database file created fresh before any SQLite handle exists,
  and it fails closed as `insecure_history_permissions` when an existing
  file or directory does not carry the exact owner-only mode; the Windows
  path delegates to the Phase 1 `windows_acl` machinery through a cfg seam
  so the same owner-only ACL policy covers history. Every connection
  enables WAL (verified by reading the runtime `journal_mode` back),
  `foreign_keys = ON`, `secure_delete = ON`, and a 5-second busy timeout.
  Schema creation/validation runs in one step and records
  `SCHEMA_VERSION = 1` in `user_version`; an unknown or newer version fails
  as `history_corrupt` with recovery guidance, and a file that fails
  SQLite's `quick_check` probe (including the lazy `SQLITE_NOTADB` open
  path) also fails as `history_corrupt` with the original file left
  untouched. `HistoryErrorCode` maps one-to-one onto the closed
  `local_history` output codes (`InsecureHistoryPermissions`,
  `HistoryCorrupt`, `NotFound` plus `HistoryWriteFailed`,
  `HistoryUnavailable`) through `ErrorCode::canonical()`. The operations
  module owns inserts and upserts of `analyses`, `bulk_collections`, and
  `upstream_tasks` rows, transactional terminal-result updates, FTS
  synchronization of `input_text`/`filename`/`headline`/`source_urls`,
  `delete_analysis` (which cascades `upstream_tasks` through the
  `ON DELETE CASCADE` foreign key and drops the FTS row in the same
  transaction), and `clear` (which empties every table). Both destructive
  operations run `wal_checkpoint(TRUNCATE)` after the commit so the logical
  deletion is reported even if the truncate fails, per the deletion
  semantics clause. Stored rows are plain Rust structs holding the typed
  domain IDs (`AnalysisId`, `BulkId`, `Sha256Hash`, `UtcTimestamp`), closed
  enums (`AnalysisStatus`, `SubmissionOutcome`, `SaveState`, `CheckKind`),
  and the canonical JSON bodies as opaque strings; the opaque-JSON rule
  keeps the store free of any upstream or submitted content parsing.

  Independent review remediation (closed `InputKind` with no
  `String::leak`, FTS replacement inside the terminal-update transaction,
  missing FTS rows failing closed as `history_corrupt`, WAL/SHM sidecar
  owner-only enforcement through the existing Unix/Windows protection
  machinery, and panic-free `user_version` handling) is folded into this
  same packet. Final validation is complete on the disposable Crabbox
  remediation box on Oracle Paris (lease `cbx_367852b6572e`, slug
  `pangram-p4-hrem-445a01`, container `11e07c22ee42`, SSH port 22837),
  separate from the static `local-container` host, the Yoga Windows SSH
  host, and the externally held `remoteuse-t7-profiles-policy` lease, with
  an isolated workroot `/home/ubuntu/work/p4rem-445a01/pangram-cli` and
  isolated stable and MSRV `CARGO_TARGET_DIR`s under
  `/home/ubuntu/cargo-target/p4rem-445a01/`. Twenty
  `tests/history-store*.rs` real-SQLite integration tests (16 core in
  `tests/history-store.rs`, 4 in `tests/history-store-hardening.rs`) prove
  the exact schema v1 (every column, index, and the contracted FTS5
  virtual table), the per-connection pragma set (WAL + foreign keys +
  secure delete read back through the runtime), foreign-key rejection and
  `ON DELETE CASCADE` against the live database, transactional
  save/observation/update roundtrips, recent-first listing and FTS5
  search, FTS-consistent terminal updates (delete/reinsert in one
  transaction), a structurally missing FTS row failing closed as
  `history_corrupt`, structured-corruption tracking with original-file
  guidance, owner-only Linux modes on both the database file and its
  `*-wal`/`-shm` sidecar companions with fail-closed reopen on an insecure
  sidecar, and sanitized error surfaces. `cargo fmt --check` and the
  strict `cargo clippy --locked --all-features --all-targets --
  -D warnings` gate are clean on stable Rust 1.97.1. The full locked
  all-feature suite passes on stable Rust 1.97.1 (420 tests across 23
  result groups, 0 failures) and under the MSRV Rust 1.87.0 (`cargo build
  --locked --all-features` plus the same 420 passing tests). The
  generated-contract generator reproduces the committed artifacts with no
  drift, and the committed `tests/generated-contracts.rs` drift test
  passes (9/9). `cargo audit` reports no new advisories against the
  locked 323-crate graph, and the authoritative `cargo deny 0.20.2 check
  licenses` with CI's exact inline allow-list configuration (Unicode-3.0
  included) reports `licenses ok`. The repository
  `tools/check-hygiene.rs` binary reports no errors across 142 files (its
  warnings on pre-existing 800-to-1000-line source files are unchanged),
  and a `gitleaks` 8.30.1 repository scan (189 commits, 32.97 MB) reports
  no leaks. History remains unimplemented at the adapter surface: no CLI,
  TUI, or MCP grammar is activated, no `--save` integration exists, and
  the package bump is patch (foundation only). Phase 4 stays In progress;
  history commands, the analysis `--save` seam, and their adapter contract
  tests are subsequent packets.

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
