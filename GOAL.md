# Pangram CLI completion goal

Status: complete
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
5. the project owner has confirmed Pangram authorization for public third-party
   API use and the approved terminal fox-logo artwork
6. artifacts bound by the signed manifest install and update on every supported
   target
7. the landing page is deployed at `pangram.micr.dev/` and the Fumadocs site
   is deployed at `pangram.micr.dev/docs`
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
- Documentation routing: `pangram.micr.dev/` is the product landing page and
  Fumadocs begins at `pangram.micr.dev/docs`.
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
  right inspector, and a restrained contextual command bar. Sparse separators
  replace dashboard-style card chrome.
- Intro: the approved Pangram fox GIF, one generated 72x16 terminal mark, a
  four-cycle timeline whose final eight frames dissolve, 56 frames at
  20 fps, a 2.8-second fox sequence, a 300 ms Analyze fade, a 100x28 floor,
  and `once` by default.
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
| 4 | Complete |
| 5 | Complete |
| 6 | Complete |
| 7 | Complete |
| 8 | Complete |
| 9 | Complete |

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
- MCP product protocol checks and applicable official conformance checks
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
- The project owner confirmed Pangram authorization for public API and approved
  fox-logo use. The artwork and derivatives remain outside the MIT grant.
- Live conformance may use at most 10 current Pangram billable units across
  securely supplied test keys. Credentials MUST NOT appear in chat, repository
  files, logs, fixtures, diagnostics, or retained command output.
- Live credentials are supplied to the conformance harness at runtime from a
  private operator-controlled source.
- Phase 6 pins RMCP 3.1.2 at upstream commit
  `02c62aef2e331e5cf79c06c744eb1eb052cc8ebd`. Its Rust 1.88 minimum raises the
  package MSRV from 1.87 to 1.88 when the dependency enters the lockfile.
- Phase 6 pins the official MCP conformance applicability reference to
  `@modelcontextprotocol/conformance` 0.2.0-alpha.11. Upstream issue 258
  prevents stdio, and the frozen full-server profile requires optional
  capabilities outside Pangram's advertised surface. Phase 6 therefore uses
  compiled `pangram mcp` stdio tests and does not claim that a broader
  test-only HTTP fixture proves the shipping handler.

### Unknown knowns

- The exact terminal fox rendering quality requires inspection of the generated
  TUI frames in a real terminal.
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

Historical deviations are retained in the linked
[deviation log](docs/goal-deviation-log.md). This goal remains the source of
truth for destination, authority, policy, and current completion criteria.

## Progress log

Historical progress through Phase 4 is retained in the
[progress archive](docs/goal-progress-history.md).

- 2026-08-24: Phase 7 moves to Complete. After the Developer API wallet was
  funded, four minimum private synthetic requests made no automatic retry:
  one plagiarism request and one RTF, PDF, and DOCX file request. All returned
  HTTP 200. They confirmed the numeric `plagiarized_sentences` field, the rich
  ordered file array, and file windows that omit humanizer fields. No key,
  submitted content, response value, match, or public link was logged or
  retained. Contract-first implementation now exposes PDF, DOCX, and RTF AI
  detection through the CLI and text plagiarism and combined analysis through
  CLI, TUI, and MCP. Binary plagiarism and binary combined analysis fail
  before submission. Combined text analysis starts both checks concurrently,
  returns checks in canonical order, and preserves either successful half.
  History reruns retain the original text check set. On remote Box
  `bx_mvuw4wga`, Rust 1.97.1 passed formatting, strict all-target Clippy, and
  the full locked all-feature suite; the declared Rust 1.88 MSRV passed the
  same full suite from a separate target directory. No local Cargo command or
  further Pangram live request was used.
- 2026-08-24: Phase 8 remains in progress with the private `0.x` update policy:
  `pangram update --check`, `pangram update`, and `pangram update --yes` resolve
  to the canonical typed `update_unavailable` response before any prompt,
  release-network access, state read, or mutation. Compiled-binary and
  generated-contract tests cover all three forms and the conflicting-flag
  negative case. The signed-updater trust-order review resolved the stale
  architecture list in favor of the normative update contract: verify the
  detached signature over the exact downloaded manifest bytes first, then
  parse and validate the signed manifest. Runtime implementation follows that
  single order.
- 2026-08-25: Phase 8 locally controllable implementation now includes the
  signed-manifest verifier, fixed production checker with ETag state, closed
  archive validation, direct-install receipt ownership, atomic Unix
  replacement, receipt-finalization recovery, manager-install advice, and the
  narrowly scoped Windows replacement helper. The CLI exposes deterministic
  completion generation for all five contracted shells and private `0.x`
  update forms remain network-free. The release toolchain builds deterministic
  five-target archives, verifies archive metadata before signing, emits the
  detached Ed25519 manifest signature, renders Homebrew and Scoop metadata,
  stages exact npm platform packages, writes checksums, an SPDX SBOM,
  third-party license notices, and provenance, and keeps Tegami versioning,
  npm publication, cargo-dist planning, native archives, and GitHub Releases
  under their documented owners. Separate manual workflows configure the
  version pull request, private draft release, and environment-gated npm
  publication without running any external action in this execution cycle.
  The Fumadocs application generates 41 document pages plus site routes,
  individual Markdown, local search, schemas, `llms.txt`, and
  `llms-full.txt`; typecheck, production build, links/source checks, package
  metadata checks, accessibility/keyboard browser smoke, actionlint 1.7.12,
  and npm critical audit pass. Remote Box `bx_mvuw4wga` passed strict
  all-target Clippy and the complete locked all-feature Rust suite on current
  Rust 1.97.1 and MSRV Rust 1.88.0. Release-packaging tests prove the real
  packager, signer, shipping verifier, and extractor agree on exact bytes.
  Phase 8 cannot close until production signing material supplies an embedded
  public key, the public updater and direct installers can be built against
  that key, and native Linux-baseline, macOS, and Windows install/update smoke
  tests pass. Phase 9 remains blocked by the same external release gates:
  written Pangram API and fox-art permission, approved/provenanced intro art,
  production signing and registry ownership, native target evidence, final
  user quality approval, production docs deployment, exact-version release
  authority, and public publication. No Pangram request, push, pull request,
  deployment, package publication, tag, draft release, visibility change, or
  production network action occurred.
- 2026-08-25: The final locally controllable Phase 8 and Phase 9 readiness
  pass is complete. The thermo-nuclear maintainability review found and fixed
  the only structural blocker: `src/analysis/handle.rs` no longer carries a
  weak waiver for growing to 1,284 lines. Synchronous file/plagiarism work and
  upstream-authored task observation now live in cohesive child modules, the
  owner remains one `Analyzer`, and the core orchestration file is 689 lines.
  Updater test fixtures moved out of the 812-line contract root, reducing it
  to 708 lines without splitting behavior from its negative cases. On remote
  Box `bx_mvuw4wga`, the exact refactored tree passed formatting, strict
  all-target all-feature Clippy, all generated-contract drift tests, and the
  complete locked all-feature suite on current Rust 1.97.1 and MSRV Rust
  1.88.0. Fumadocs typecheck and production build generated all 45 routes;
  package checks, npm audit (zero vulnerabilities), checksum-verified
  actionlint 1.7.12, checksum-verified cargo-dist 0.32.0 planning for the five
  native archives, and checksum-verified cargo-about 0.9.2 generation of the
  224,851-byte third-party notice also passed. All disposable remote Cargo
  targets and downloaded validation tools were removed after proof. No P0 or
  P1 maintainability finding remains. Phase 8 and Phase 9 remain externally
  blocked for the second consecutive readiness audit by the same exact gates:
  Pangram's written API and fox-art permission, an approved and provenanced
  intro source plus final user quality approval, production Ed25519 signing
  material and embedded public key, registry ownership and trusted-publishing
  setup, native glibc 2.17 Linux x64/arm64 plus macOS x64/arm64 and Windows x64
  install/update evidence, production docs deployment, exact-version public
  release authority, and publication to the public repository, GitHub Release,
  npm, Homebrew, and Scoop. This repeated external blocker ends the current
  autonomous execution cycle. No Pangram request, push, pull request,
  deployment, publication, release, tag, registry mutation, or visibility
  change occurred.
- 2026-08-25: The installed Claude CLI confirmed `claude-opus-5` and completed
  an adversarial TUI review. All ten findings were fixed contract-first: the
  interface now uses Pangram orange as its primary accent, preserves true
  black and terminal-safe no-color behavior, removes duplicate narrow
  headings and actions, aligns focus gutters and route markers, shows no
  estimate for an empty composer, and sizes result viewports from rendered
  preamble rows. The stale Terminal Control selectors and snapshots now match
  the compact layout. Tuistory captures at 80, 100, and 120 columns confirmed
  visible color, spacing, placeholder text, and stable hierarchy. The compiled
  acceptance suite passed all 12 journeys. The remote Rust gate passed 103
  focused TUI tests, the complete locked all-feature/all-target suite, and
  strict Clippy after two PTY tests were corrected to reconstruct complete
  delta-rendered frames. A final read-only release audit confirmed that the
  GitHub repository is public, npm is authenticated as `microck`, and the
  protected release environment contains the signing secret and key ID that
  match the embedded production public key. Remaining external gates are the
  actual approved fox source files and provenance record, native five-target
  install/update evidence, deployment and DNS for `pangram.micr.dev`, the
  Homebrew and Scoop repositories, npm trusted publishing, and final user TUI
  approval. No Pangram request or billable unit was used.
- 2026-08-25: The exact current TUI binary was rebuilt on remote Box
  `bx_mvuw4wga` and left running with truecolor in the focused Herdr workspace
  `pangram-cli` for final user inspection. Its review credential is synthetic,
  so the pane cannot spend Pangram credits unless the user replaces it. The
  protected GitHub release environment now admits deployments only from
  `main`, and the canonical public package-manager destinations
  `Microck/homebrew-pangram-cli` and `Microck/scoop-pangram-cli` now exist with
  initialized `main` branches. npm authentication identifies `microck`, but
  the current credential cannot read account/package administration and none
  of the six `@microck/pangram-cli*` packages exists yet. npm trusted
  publishing therefore remains blocked until the authorized first package
  publication creates those package records. Native Crabbox discovery found
  no usable release-evidence lane: the macOS provider is unreachable, the
  Windows x64 Hyper-V provider failed while preparing the remote worktree, and
  the Windows 10 QEMU provider is ARM64 despite advertising `amd64` and carries
  no Rust toolchain. Both disposable Windows VMs were removed. No Pangram
  request or billable unit was used.
- 2026-08-25: The OpenCode-inspired vertical-rhythm pass is complete
  contract-first. Analyze and Settings now use deliberate blank rows between
  groups, the composer has breathing room below its rule, overlays have one
  row and two columns of inner padding, and mouse hit targets come from the
  same named geometry that renders each control. Overlay actions now measure
  terminal cell width and clicks in the visual gap perform no action. The
  rendering root was split along existing ownership boundaries from 843 to
  693 lines, with layout geometry and overlay projection in cohesive child
  modules. Remote Box `bx_86v2x32q` passed 109 focused TUI tests, strict
  all-target all-feature Clippy, the complete locked all-feature suite on
  current Rust, and the complete suite at the declared Rust 1.88 MSRV. The
  Fumadocs typecheck and production build, package drift check, and all 12
  Terminal Control journeys pass. The golden update was reviewed as spacing,
  padding, wrapping, and stable frame-coordinate changes; settled snapshots
  now derive their text from the authoritative structured frame instead of a
  briefly stale delta-rendered convenience string. The exact ARM64 binary is
  running with truecolor in focused Herdr pane `w8:p3` with a synthetic key.
  A read-only release audit reconfirmed the public source and package-manager
  repositories, the protected signing environment, and npm authentication.
  `pangram.micr.dev` still has no DNS answer and the six npm package records do
  not exist. The remaining external gates are the approved fox source files
  and provenance record, native five-target install/update evidence, docs
  deployment and DNS, authorized first npm publication and trusted-publishing
  setup, and final user TUI approval. No Pangram request, billable unit, push,
  pull request, deployment, publication, release, tag, or registry mutation
  occurred.
- 2026-08-26: The approved fox source is now locked at
  `fa806f95e5775e9bc4ffda599a540910edd2042115eae80729308b02d89a542e`.
  Its recorded permission and provenance, generator contract, and non-MIT art
  notice accompany the source. The deterministic development-only generator
  converts the 1772x709 nine-frame GIF into 22 embedded 72x16 terminal frames
  and a 56-frame, 2.8-second four-cycle sequence whose final eight frames
  dissolve. Its backdrop reaches the TUI canvas during playback, then the real
  Analyze screen fades in over 300 ms. The runtime uses Pangram orange as the
  dominant color, supports truecolor, ANSI, and `NO_COLOR`, skips cleanly on
  input or resize, honors
  reduced or disabled motion, and suppresses the intro below 100x28. Tuistory
  review confirmed a centered, recognizable fox with orange, pink, cream, and
  shadow-orange facets. Remote Box `bx_hwbqzag2` passed generator drift and
  unit tests, strict all-target all-feature Clippy, the real-PTY intro suite,
  the complete locked all-feature suite on current Rust 1.97.1, and the same
  complete suite on MSRV Rust 1.88.0. After the canvas handoff refinement,
  fresh Box `bx_skmv6zgq` passed 16 focused intro tests, strict all-target
  all-feature Clippy, and the complete locked all-feature suite on both Rust
  1.97.1 and MSRV Rust 1.88.0. A clean locked npm install exposed and
  corrected the stale workspace lockfile; package checks, docs drift,
  TypeScript, and the 45-route production Fumadocs build then passed. A second
  clean locked install on `bx_skmv6zgq` reconfirmed those documentation and
  package gates. Tuistory truecolor captures and a 75-frame rapid sequence
  confirmed the orange fox, shared charcoal canvas, fade ordering, and stable
  final TUI. The exact stripped glibc 2.17 ARM64 binary is open for review in
  full-width Herdr tab
  `w8:t3`, pane `w8:p8`, with a synthetic non-billable preview key. Phase 8
  remains in progress pending native five-target install and update evidence.
  Phase 9 remains blocked by the remaining native platform evidence, docs
  deployment and DNS, first npm publication and trusted publishing, final TUI
  approval, and explicit exact-version publication authority. No Pangram
  request, billable
  unit, commit, push, pull request, deployment, publication, release, tag, or
  registry mutation occurred.
- 2026-08-26: The project owner approved the final TUI quality and authorized
  the exact public `v0.1.0` release to the existing public GitHub repository,
  GitHub Release, `pangram.micr.dev`, npm, Homebrew, and Scoop after every
  release gate passes. The production web contract reserves
  `pangram.micr.dev/` for the product landing page and places Fumadocs at
  `pangram.micr.dev/docs`. This authorization does not waive native artifact,
  install, update, signing, documentation, registry, or final verification
  gates.
- 2026-08-26: Phase 8 direct installers are complete for POSIX and PowerShell.
  The generated scripts select the current five-target archive from signed
  release metadata, verify its exact size and SHA-256, and delegate the trust
  boundary to the downloaded Pangram candidate. That candidate verifies the
  detached Ed25519 manifest signature, version, target, closed archive shape,
  and its own executable bytes before an atomic initial install or
  receipt-owned replacement. Unowned executables fail closed, failed smoke
  tests preserve prior state, and receipts publish only after a successful
  smoke test. Release automation renders both scripts after signing and adds a
  five-target signed direct-install smoke matrix before release creation. On
  remote Box `bx_vg2graus`, current Rust passed formatting, the complete
  locked all-feature suite, and strict all-target Clippy; Rust 1.88.0 passed
  the same complete suite from an isolated target directory. A stale PTY click
  coordinate exposed by the approved command-bar shift was corrected to hit
  the visible Quit target, and the real-PTY contract passes on both
  toolchains. Clean npm install, zero-vulnerability audit, docs generation,
  TypeScript, the 45-route Fumadocs production build, package checks,
  checksum-verified actionlint 1.7.12, and gitleaks diff scanning all pass.
  The pull-request CI now owns the five native archive build and clean-install
  smoke jobs. No Pangram request or billable unit was used. Phase 8 remains in
  progress until that native matrix supplies its target evidence.

- 2026-08-27: Phase 8 is complete. The release candidate merged through
  PRs [#22](https://github.com/Microck/pangram-cli/pull/22),
  [#23](https://github.com/Microck/pangram-cli/pull/23),
  [#24](https://github.com/Microck/pangram-cli/pull/24),
  [#25](https://github.com/Microck/pangram-cli/pull/25),
  [#26](https://github.com/Microck/pangram-cli/pull/26),
  [#28](https://github.com/Microck/pangram-cli/pull/28),
  [#29](https://github.com/Microck/pangram-cli/pull/29),
  [#30](https://github.com/Microck/pangram-cli/pull/30),
  [#31](https://github.com/Microck/pangram-cli/pull/31),
  [#32](https://github.com/Microck/pangram-cli/pull/32),
  [#33](https://github.com/Microck/pangram-cli/pull/33),
  [#34](https://github.com/Microck/pangram-cli/pull/34),
  [#35](https://github.com/Microck/pangram-cli/pull/35), and
  [#36](https://github.com/Microck/pangram-cli/pull/36), ending at merged
  commit `8aefdbd0279d21f631628c0aa9e4830382166291`. The documentation gate
  now validates repository links, deployed Fumadocs routes, public files, and
  external prose links before the site build. The final protected
  [native CI matrix](https://github.com/Microck/pangram-cli/actions/runs/33079144873)
  passed all 13 jobs. The non-publishing
  [signed release workflow](https://github.com/Microck/pangram-cli/actions/runs/33080258523)
  built all five native targets and proved an exact glibc 2.17 baseline for
  both Linux archives before signing and auditing the artifacts. Native support
  claims now match this evidence: Linux claims glibc 2.17 without an untested
  kernel floor, both macOS targets require macOS 15, and Windows x64 requires
  Windows Server 2025 on the pinned `windows-2025` runner. The workflow ran the
  generated public installer end to end twice and installed the staged npm
  packages offline on Linux x64, Linux ARM64, macOS x64, macOS ARM64, and
  Windows x64. It also installed and tested the generated Homebrew formula on
  all four POSIX targets and the generated Scoop manifest on Windows. Those
  tests proved native target selection, download, size and hash checks, the
  default destination, initial receipt ownership, receipt-owned replacement,
  and package-manager execution. The workflow ran with `publish=false`; the
  release-creation job was skipped, and no tag, GitHub Release, registry
  publication, documentation deployment, or Pangram API request occurred.
  Phase 9 remains blocked pending its separately authorized production
  publication actions and final release gates.

- 2026-08-27: Phase 9 and the public v1 completion goal are complete. The
  authorized `v0.1.0` release was built from merged commit
  `28b131dc7b44f544179a10eefa38aed063ced20b`. The protected release workflow
  [built, signed, audited, and smoke-tested all five native
  targets](https://github.com/Microck/pangram-cli/actions/runs/33113399509)
  before publishing the [GitHub Release with 14 checksum-bound
  assets](https://github.com/Microck/pangram-cli/releases/tag/v0.1.0). The
  production [site](https://pangram.micr.dev/) and
  [Fumadocs](https://pangram.micr.dev/docs) are live, and the public installer
  completed a fresh Linux ARM64 installation that reported `pangram 0.1.0`.
  [Homebrew](https://github.com/Microck/homebrew-pangram-cli/commit/8c186db0ffbc4945ee9499b082cd9870bfd03734)
  and [Scoop](https://github.com/Microck/scoop-pangram-cli/commit/2bb0dc82e7b453293401796081a647220c6ea28a)
  publish the exact generated metadata that passed the release matrix. All six
  [npm packages](https://www.npmjs.com/package/@microck/pangram-cli) use GitHub
  OIDC trusted publishing through the protected `pangram-public-release`
  environment, resolve `latest` to `0.1.0`, and pass a clean public install and
  binary-version smoke test. The repository is public under MIT, the TUI and
  fox intro are approved, live Pangram conformance is recorded, and no
  unresolved P0 or P1 finding remains. The signed update manifest was validated
  during the release workflow; the `0.x` binary correctly returns
  `update_unavailable` without network access under the approved update
  contract.

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
