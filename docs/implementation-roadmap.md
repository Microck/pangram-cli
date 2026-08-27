# Pangram CLI implementation roadmap

Status: ready for implementation
Date: 2026-07-23

## 1. Completion target

Implementation is complete when:

- every supported documented Pangram workflow is available through its planned
  CLI, TUI, and MCP adapter
- canonical JSON and errors pass contract tests
- live conformance has resolved the file and plagiarism response conflicts
- signed release artifacts install and update on every supported target
- the Fumadocs site and README describe only behavior enforced by tests
- written Pangram permission permits public third-party distribution
- written Pangram permission covers terminal use of its fox logo
- the agent-owned intro baseline passes its visual rubric and autonomous suite
- the user approves the quality of the final product
- no P0 or P1 finding remains

The same external blocker repeating twice ends an execution cycle with a
reported blocked status. It does not authorize a private API workaround.

## 2. Delivery rules

Every phase follows this order:

1. update the observable contract
2. add a failing test at the owning module's interface
3. implement the smallest cohesive vertical slice
4. regenerate committed artifacts
5. run focused and broader verification
6. update the matching documentation

Do not build every internal type before the first vertical slice. Do not add a
generic transport, repository interface, or multi-crate workspace in
anticipation of future variants.

The phase numbers express dependency order. Work inside a phase can run in
parallel only when file ownership and contract ownership do not overlap.

If Phase 7 live conformance is blocked only by credentials, authorization, or
upstream evidence, independent Phase 8 work may proceed after Phase 6. That
work MUST keep file and plagiarism behavior labeled blocked and MUST NOT
publish, enable update networking, or claim conformance. Phase 7 remains open
until its own exit criteria pass.

## 3. Phase 0: Scaffold and executable contracts

Goal: create a compiling package whose generated schemas and command skeleton
make contract drift visible.

### Files

- `Cargo.toml`, `Cargo.lock`
- `src/lib.rs`, `src/main.rs`
- `src/domain.rs`, `src/output.rs`, `src/cli.rs`
- `tools/generate-contracts.rs`
- `tests/cli-contract.rs`
- `contracts/*.schema.json`
- CI workflow files

### Work

1. Create one Rust 2024 package named `microck-pangram-cli` with the
   `pangram` binary.
2. Add the minimum approved runtime and development dependencies from the
   architecture specification.
3. Encode the canonical envelope, error category, status, input, check, and
   result types from the committed seed contracts.
4. Build only the visible command entries implemented in this phase. Keep the
   complete grammar in generator input without advertising unsupported runtime
   commands as available.
5. Generate JSON Schema from Rust-owned contract types behind `dev-tools`, run
   a locked differential transfer corpus against every seed and generated
   schema, document each intentional correction, add generated ownership
   headers, and transfer artifact ownership to the generator.
6. Add CI checks for formatting, lint, tests, contract regeneration, forbidden
   Unicode, file-size thresholds, and secret scanning.
7. Set the jj identity locally if a commit is later requested.

### Tests

- envelope one-of invariants
- UUIDv7 identity parsing and serialization
- error to exit-code mapping
- implemented command help snapshots and full generated grammar reference
- generated schema diff check
- one-package `cargo metadata` assertion

### Exit criteria

- `cargo build --locked` produces `pangram`
- `pangram --help` lists only implemented command entries
- generated contracts match committed files
- no command can perform a network request yet

## 4. Phase 1: Configuration, credentials, and diagnostics

Goal: make startup behavior, credential precedence, and local diagnostics safe
before adding billable operations.

### Files

- `src/config.rs`
- `src/diagnostics.rs`
- configuration sections of `src/cli.rs`
- `tests/cli-contract.rs`
- platform-specific CI jobs

### Work

1. Resolve platform config and data paths.
2. Load strict TOML with the documented precedence.
3. Implement dedicated atomic restrictive `credentials.toml` persistence that
   cannot be relocated through `PANGRAM_CONFIG`.
4. Implement `auth`, masked TTY input, `auth set --api-key VALUE`,
   `auth set --api-key-stdin`, `auth status`, and `auth logout`.
5. Implement `config list|get|set|path` while rejecting credential keys.
6. Implement non-billable `doctor`.
7. Provide the credential setup operation and state needed by the Phase 5
   first-launch TUI overlay. Do not add a TUI prompt in this phase.
8. Explain how to obtain a key at `https://www.pangram.com/apikey`.

### Tests

- real temporary config files
- Unix mode `0600`
- Windows ACL job
- environment credential override
- masked local status
- no credential in Debug, diagnostics, errors, or panic output
- no noninteractive prompt

### Exit criteria

- configuration failures are canonical and actionable
- credential persistence refuses unsafe writes
- `doctor` performs no Pangram request

## 5. Phase 2: AI text detection vertical slice

Goal: ship the first complete behavior through the shared analysis module and
CLI before expanding breadth.

Entry gate: Pangram must document the Pangram 4 request selector. The exact
field and selected upstream version must be captured in the evidence ledger
before protocol implementation starts. Resolved 2026-07-31: the Pangram SDK
v1.0.0 tag documents the selector (`model` set to `pangram-4`); see the
evidence ledger.

### Files

- `src/analysis.rs`
- `src/domain.rs`
- `src/output.rs`
- `src/cli.rs`
- `tests/fixtures/`
- `tests/protocol-contract.rs`
- `tests/cli-contract.rs`

### Work

1. Build a real loopback Pangram fixture server.
2. Implement explicit Pangram 4 text submission and task polling in `Analyzer`.
3. Normalize documented responses into the canonical result.
4. Centralize rate limiting, safe retries, timeout, and local cancellation.
5. Implement bare-command dispatch and `detect`.
6. Implement JSON, JSONL, TOON, Markdown, and pretty projections.
7. Implement JSONL progress on stderr and stable exit mapping.
8. Support `detect --detach` for UTF-8 text.

### Tests

- exact methods, paths, headers, and bodies
- explicit Pangram 4 selection with no default-model path
- required humanizer fields and zero-based half-open segment offsets
- all documented HTTP status categories
- malformed and additive responses
- `Retry-After` and ambiguous POST failure
- TTY, pipe, positional, `-`, and file input resolution
- stdout and stderr separation
- interruption and terminal restoration

### Exit criteria

- the compiled CLI completes text detection against the loopback server
- every adapter-visible result uses the canonical envelope
- no adapter contains Pangram protocol logic

## 6. Phase 3: Bulk and remote task workflows

Goal: add long-running and batch behavior without creating a second polling
path.

Entry gate resolved 2026-07-31: official Pangram sources document one job-wide
JSON `model` field set to `pangram-4`, one billable unit per started 100-word
block per valid item with a minimum of one, and a 1,000-unit request limit.
There is no per-item selector or separate item-count limit. Do not reuse the
Pangram 3-era 1,000-word unit. Live conformance remains required before public
support, but it does not block loopback implementation.

### Files

- `src/analysis.rs`
- `src/domain.rs`
- `src/cli.rs`
- `tests/protocol-contract.rs`
- `tests/cli-contract.rs`

### Work

1. Add typed bulk submission, status, wait, item, and result-page models.
2. Derive billable-unit estimates before submission.
3. Require `--max-billable-units` for every bulk submission.
4. Remove the planned bulk `--public-link` flag and `submit_bulk.public_link`
   input before implementation. The official Bulk API documents no public
   dashboard request or response field; do not infer parity with text or file
   submission.
5. Preserve input ordering and caller IDs.
6. Implement `bulk submit|status|wait|results`.
7. Implement `task status|wait`.
8. Preserve successful child results when a bulk operation is partial.

### Tests

- JSONL whole-file validation
- billable-unit properties
- absence of bulk public-link inputs in CLI and MCP schemas
- pagination ordering and duplicate-page rejection
- local timeout without remote cancellation
- partial and terminal parent-state derivation
- upstream and local ID resolution

### Exit criteria

- no bulk request starts without a validated cost ceiling
- task and bulk waiting reuse the analysis progress model
- partial results remain machine-readable and exit with the documented code

## 7. Phase 4: Optional local history

Goal: add explicit local persistence without changing the default privacy
posture.

### Files

- `src/history.rs`
- history integration in `src/analysis.rs` and `src/cli.rs`
- `tests/history-contract.rs`

### Work

1. Create schema version 1 with SQLite and bundled FTS5.
2. Implement transactional active-to-terminal writes.
3. Implement manual `--save` and disabled-by-default automatic history.
4. Implement list, show, search, delete, clear, export, and rerun.
5. Preserve retry and rerun lineage.
6. Fail on incompatible or corrupt databases without replacing them.

### Tests

- real SQLite, concurrent readers and writers
- owner-only data directory, database, WAL, and shared-memory files
- fail-closed behavior before SQLite open
- foreign-key rejection and cascade behavior on every connection
- FTS coverage over all documented fields
- failed history write after successful remote analysis
- redacted and full export
- corruption preservation
- logical deletion, FTS synchronization, and WAL truncation
- 10,000-analysis search budget

### Exit criteria

- a default installation writes no analysis history
- history mutation is atomic
- remote success is never hidden by a local persistence failure

## 8. Phase 5: TUI and motion prototype

Goal: make the primary human workflow usable while keeping all behavior in the
shared modules.

### Files

- `src/tui.rs`, promoted to `src/tui/` only if size and ownership justify it
- `tests/tui-pty.rs`
- `contracts/tui-state.schema.json`
- `tools/generate-intro-frames.rs`
- `tools/tui-acceptance/`
- generated 72x16 intro frame table and playback sequence

### Work

1. Implement a pure reducer for `Analyze`, `Active`, `History`, and `Settings`.
2. Build the locked route-rail, dominant-workspace, inspector, and restrained
   command-bar layout with its documented narrow-terminal transformation.
3. Add regular and Vim keymaps with text-field precedence.
4. Add the reducer-owned first-launch credential, update-check, and history
   overlays on the Analyze route.
5. Surface progress, partial results, public-link consent, and save state.
6. Implement `once`, `always`, and `off` intro frequency with a separate,
   atomic one-time state marker.
7. Verify the approved GIF against `intro-art-contract.md`, then generate the
   locked 14-frame 72x16 cycle, eight dissolve frames, and 56-entry 20 fps
   playback sequence.
8. Implement monotonic frame selection, color and ASCII fallbacks, skip-key
   consumption, and full, reduced, and off motion behavior.
9. Add the pinned Terminal Control Vitest harness and drive the compiled binary
   through first launch, navigation, input, resize, motion, failure, interrupt,
   and quit scenarios.
10. Produce sanitized text, cell, SVG, and optional video evidence, then have
    the agent verify one initial baseline against the intro art contract without
    copying Droid frames or runtime code.
11. Implement one idempotent terminal restoration guard for normal return,
    handled I/O error, Ctrl+C, supported signals, and unwind panic.

### Tests

- reducer transitions
- Ratatui `TestBackend` snapshots
- PTY startup, resize, interruption, panic restoration, and intro skipping
- Terminal Control settled-screen, keyboard, resize, and failure-artifact
  acceptance on GNU/Linux and macOS
- deterministic cycle, dissolve, and fallback frame fixtures
- one-time state completion, skip, suppression, and failed-write cases
- full, reduced, and off motion
- narrow-terminal fallback

### Exit criteria

- all network work still enters through `Analyzer`
- printable keys edit fields under both keymaps
- the accepted intro runs once by default on a 2.8-second nominal timeline
- delayed rendering skips stale frames and adds no missed-frame delay
- the accepted intro stops after four source-cycle repetitions
- reduced and off motion paths are first-class
- the agent-owned baseline and autonomous TUI acceptance matrix pass before the
  final product quality review
- public artifacts carry the separate approved-artwork notice and no
  unapproved third-party material
- the source hash, rights reference, generator hash, and objective acceptance
  record are complete

Open implementation and acceptance tracker:
[#7](https://github.com/Microck/pangram-cli/issues/7). Its stale request for
three design variants is superseded by the one-baseline intro art contract.

## 9. Phase 6: MCP and embedded agent guidance

Goal: expose the same operations to agents with explicit capability and billing
signals.

### Files

- `src/mcp.rs`
- `tests/mcp-contract.rs`
- `skills/pangram/SKILL.md`
- `generated/mcp-tools.json`
- `generated/agent-reference.md`

### Work

1. Pin stable RMCP exactly to 3.1.2, raise the dependency-driven MSRV to Rust
   1.88, and implement MCP `2026-07-28` over stdio with `server/discover`,
   required protocol version and client capabilities in request `_meta`,
   optional client information, typed tools, and `additionalProperties: false`.
2. Publish exact canonical success and failure envelopes as
   `structuredContent` through command-specialized output schemas in the one
   generated tool inventory, with `resultType: "complete"`.
3. Advertise only Phase 6 tools: `detect_text`, ordinary task get/wait, bulk
   submit/get/wait/results, and gated history/configuration tools. Leave file
   detection, plagiarism, and combined analysis absent until Phase 7, and
   update checks absent until Phase 8, without compatibility shims.
4. Add repeated `--allow-file-root PATH` startup configuration and
   handle-relative, no-follow opening for bulk `jsonl_path`; keep inline bulk
   items usable without roots.
5. Apply separate history, history-mutation, config-mutation, and public-link
   startup gates. Require history plus history mutations for `save: true`, and
   resolve local analysis/bulk IDs only from history. Do not add a transient
   ledger.
6. Add correct read-only, destructive, idempotent, and open-world annotations.
7. Implement safe, idempotent client install and uninstall plans after each
   target's current path, schema, and exact owned-entry match rule are pinned
   from authoritative evidence. Preflight every selected target before writes,
   return one typed mutation report for dry-run and normal success, never edit
   legacy Cascade for `windsurf`, and refuse ambiguous `roo-code` storage.
8. Expose ordinary `get_task` and `wait_task` tools without implementing the
   experimental Tasks extension or owning MCP protocol types.
9. Emit deterministic private list and resource results with `ttlMs: 0`, and
   return the exact embedded output schema, error reference, and skill bytes
   with their contracted MIME types.
10. Generate `mcp-tools.json` as the one ordered tool/schema inventory and
    `agent-reference.md`, then embed both and the Pangram skill in the binary.
    Map `agent`, compact/full skill reads, skill listing, and skill locators to
    their exact contracted newline-terminated bytes.
11. Pin the official conformance applicability reference exactly to
    `@modelcontextprotocol/conformance` 0.2.0-alpha.11. Record that its frozen
    full-server profile cannot be scoped to Pangram's selected capabilities or
    stdio transport, and do not fabricate a broader HTTP fixture. Keep compiled
    `pangram mcp` stdio tests as the required Phase 6 product proof. Re-evaluate
    the official suite when it supports stdio and capability-aware selection.

### Tests

- `server/discover`, required per-request metadata, optional client identity,
  protocol-version rejection, and MCP 2026-07-28 behavior
- every tool schema and annotation
- bulk `jsonl_path` root validation, traversal, symlink, reparse-point, and
  path-race cases; inline bulk with no roots
- capability combinations
- history-only local ID resolution and upstream-ID operation without history
- no hidden task ledger and no later-phase tool names
- no billable retry hidden behind idempotent metadata
- required result type, cache metadata, and deterministic inventory ordering
- cancellation sends no response for the cancelled request
- exact embedded resource bytes and MIME types
- compiled stdio contract tests plus a pinned official-suite applicability
  audit that prevents a fixture-only pass from being claimed as product proof
- installer explicit target, dry-run, idempotency, preservation, ownership,
  conflict, exact uninstall, zero-write preflight failure, typed report, Devin
  Local-only Windsurf, Claude Desktop Linux, and Roo ambiguity behavior
- byte-exact compact/full agent guidance, skill list, and embedded locators

### Exit criteria

- agents need no TTY and receive no decorative output
- billable operations are visibly non-idempotent
- disabled capabilities remove or reject the protected operations
- HTTP and conformance-only `test_*` inventory are absent from shipping builds
- compiled product stdio tests pass, and the official-suite incompatibility is
  recorded without a fabricated conformance claim

## 10. Phase 7: File, plagiarism, and combined analysis

Goal: complete documented parity only after upstream conflicts are measured.

### Prerequisites

- dedicated Pangram test key
- manual live-conformance authorization
- synthetic non-sensitive fixtures

### Files

- contract artifacts first
- `src/domain.rs`
- `src/analysis.rs`
- adapters that expose the new supported inputs
- sanitized protocol fixtures
- live-conformance workflow

### Work

1. Run one billable-unit live file scenario for each supported file family.
2. Resolve the two documented file response shapes.
3. Run one plagiarism scenario that confirms the documented numeric
   `plagiarized_sentences` field.
4. Update canonical contracts before implementation.
5. Add file detection, plagiarism, and combined text analysis.
6. Keep binary plagiarism and binary combined analysis rejected before send
   unless Pangram documents and verifies support.

### Tests

- multipart names, content types, limits, and response normalization
- plagiarism evidence and provenance
- combined partial success
- file queue ordering
- no unsupported request leaves the process

### Exit criteria

- sanitized live fixtures agree with committed contracts
- the parity matrix links every enabled workflow to passing evidence
- unresolved upstream shapes remain disabled, not guessed

## 11. Phase 8: Documentation, distribution, and updater

Goal: make the verified product installable, explainable, and safely
updatable.

### Files

- `docs-app/`
- `scripts/tegami.mts`
- release workflows and cargo-dist configuration
- `src/update.rs`
- `tests/update-contract.rs`
- `README.md`

### Work

1. Build the Fumadocs site from the documentation plan.
2. Generate CLI, configuration, error, schema, and MCP references.
3. Add `llms.txt` and `llms-full.txt`.
4. Configure Tegami for change entries, version PRs, changelog, and lock.
5. Configure target builds, archives, checksums, the Ed25519-signed update
   manifest, and artifact provenance.
6. Implement direct-install receipts and signed atomic self-update.
7. Add package-manager formulas or manifests without letting the self-updater
   take ownership of manager installs.

### Tests

- Fumadocs build, links, accessibility, and generated drift
- exact-byte manifest signature verification
- tamper, rollback, interruption, and atomic replacement
- direct versus manager-owned update behavior
- clean-machine install smoke tests for every supported channel

### Exit criteria

- README and docs claim only tested behavior
- all external links pass validation, except the planned docs domain before
  deployment
- each install method reports the same version
- manager-owned installs receive instructions rather than binary replacement

## 12. Phase 9: Public release readiness

Goal: convert the private development repository into a supportable public
project.

### Required evidence

- written Pangram permission for a public third-party CLI, MCP server, and
  terminal fox-logo artwork
- live file and plagiarism conformance
- accepted generated intro frame art
- passing Linux, macOS, and Windows matrix
- passing CLI, TUI, MCP, history, docs, and updater suites
- successful install and update smoke tests
- dependency, license, secret, and artifact provenance audit
- no P0 or P1 review finding

### Release sequence

The testing and release plan is the sole owner of publication order. Phase 9
follows its staged private draft, explicit visibility authority, coherent
GitHub Release and documentation availability point, then npm, Homebrew, and
Scoop publication.

### Exit criteria

- the release checklist contains evidence links for every gate
- the repository is public under MIT
- no credential, private fixture, or proprietary Pangram material is present
- the released update manifest and signature validate independently, while the
  `0.x` `pangram update --check` command returns `update_unavailable` without a
  network request as required by `docs/update-contract.md`

## 13. Issue disposition

| Issue | Decision | Disposition |
| --- | --- | --- |
| [#1](https://github.com/Microck/pangram-cli/issues/1) | Umbrella product and architecture specification | Close after all specification artifacts are recorded |
| [#5](https://github.com/Microck/pangram-cli/issues/5) | Canonical language is in the product and contract specs | Close |
| [#6](https://github.com/Microck/pangram-cli/issues/6) | CLI grammar, output, errors, and exits are locked | Close |
| [#7](https://github.com/Microck/pangram-cli/issues/7) | Behavior is locked; track the hashed source, rights gate, one generated baseline, and acceptance evidence | Keep open until Phase 5 passes; remove stale three-variant wording |
| [#8](https://github.com/Microck/pangram-cli/issues/8) | One deep analysis module with thin adapters | Close |
| [#9](https://github.com/Microck/pangram-cli/issues/9) | Credential, config, privacy, and history rules are locked | Close |
| [#10](https://github.com/Microck/pangram-cli/issues/10) | MCP tools, roots, annotations, and gates are locked | Close |
| [#11](https://github.com/Microck/pangram-cli/issues/11) | Loopback, live conformance, and drift tests are defined | Close |
| [#12](https://github.com/Microck/pangram-cli/issues/12) | Fumadocs, Tegami, cargo-dist, updater, and release ownership are locked | Close |
| [#13](https://github.com/Microck/pangram-cli/issues/13) | Parity gates and implementation order are explicit | Close |

## 14. First implementation slice

Start with Phase 0 and stop after this demonstrable path:

```text
literal text
    -> CLI input resolution
    -> typed detection request
    -> loopback Pangram fixture
    -> typed progress
    -> canonical JSON result
    -> stable exit code
```

That slice proves the deepest seam before TUI, MCP, history, updater, and
documentation work build on it.
