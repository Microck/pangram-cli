# Pangram CLI progress history through Phase 4

This file archives completed progress entries that no longer need to occupy the
live completion goal. `GOAL.md` remains authoritative for current status,
policy, authority, and completion criteria.

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
- 2026-08-02: The third bounded Phase 4 packet activates explicit `--save`
  and automatic history-save integration at the adapter surface
  (contracts.md 14.2 note), landing its independent-review remediation
  contract-first and test-first in the same change. The observable surface:
  `pangram detect --save` (manual, `saved_manual`) and the
  `history.enabled = true` automatic gate (`saved_history`) persist every
  completed detection (the terminal snapshot, its FTS payload, and one
  `upstream_tasks` observation row per check) atomically, in one
  transaction each (`HistoryStore::save_analysis_atomic`). Repeated-file
  runs preserve the ordered tail after any one member's manual save
  failure: every completed member renders exactly once in invocation order
  with its own honest save state, and a later member still persists before
  the canonical exit-7 envelope closes the series. An automatic save
  failure emits exactly one sanitized `warning:` line per invocation and
  never degrades a remote outcome; a primary render failure reports exit
  1, never masked behind the history exit 7. Remotely observed reads
  reconcile by upstream identity: repeated `task status`/`task wait` reads
  refresh the one stored analysis row (`history::save::observation_merge`
  + `update_observation_snapshot`), preserving the row's original
  submission outcome, save state, local input/filename/FTS payload, and
  creation time, while each fresh read's output keeps its own fresh `anl_`
  identity and its own save outcome. Bulk submissions and observations
  reconcile one `bulk_collections` row by `upstream_bulk_id`
  (`find_bulk_collection_by_upstream` distinguishes SQL not-found from a
  failure) with children and observation rows refreshed atomically
  (`upsert_bulk_collection_atomic`) and local metadata (identity, caller
  ID, input payload, creation time) preserved; accepted 202 children
  persist truthfully with their attested task IDs, failed children with
  their canonical check error, and unattested items stay `not_submitted`.
  First enablement of durable plaintext history
  (`config set history.enabled true` after unset/false) acknowledges
  ADR 0004 with exactly one direct plaintext warning on stderr while the
  command still exits 0; repeats, disables, and failed sets print nothing.
  The remediation decomposed `src/history/operations.rs` into
  `analysis_writes.rs`/`collections.rs`/`reads.rs`/`wire.rs` and split the
  history-save loopback suite into persistence
  (`history-save-detect-loopback.rs`, 404 lines), the detect
  failure/render-precedence semantics
  (`history-save-failures-loopback.rs`, 381 lines), the task/bulk
  reconciliation and ADR 0004 transitions
  (`history-save-reconciliation-loopback.rs`, 559 lines), and the
  deterministic mixed-outcome proof
  (`history-save-mixed-outcome-loopback.rs`, 350 lines), all over a shared
  real harness (`tests/support/history_save_env.rs`) and all under the
  800-line hygiene threshold. Five new real-SQLite store tests
  (`tests/history-store-atomic.rs`, `tests/history-store-collections.rs`)
  prove row+FTS+observation atomicity, whole-batch rollback, upstream-id
  dedupe, and authorship preservation; seven detect failure semantics
  loopback tests prove the manual ordered tail, one-warning automatic
  failures (a single direct `warning:` line, never `note: warning:`), and
  render precedence on `/dev/full`; seven reconciliation loopback tests
  prove task refresh invariance with fresh output identity, bulk
  dedupe/truthful children, and the ADR 0004 warning transitions; and the
  deterministic mixed-outcome loopback test proves one member's real
  SQLite insert failure leaves the ordered tail intact (the later member
  persists with its row, FTS payload, and observation; exit 7 closes the
  series). The package bump is minor for both packages (a new
  user-visible capability). The history read, search, export, and
  mutation commands remain planned for a later packet; MCP history stays
  ungated.
- 2026-08-02: The Packet C independent-review remediation folded
  contract-first and test-first into the same change. The normative docs
  (contracts.md 14.2 note, docs/history-contract.md) lock the durable
  authorship invariance (a non-terminal refresh never erases an attested
  terminal body), the accepted-children honesty rule (the HTTP 202 truth,
  never fabricated all-queued children), the observed-children refresh for
  every armed bulk read, and the narrow render-loss precedence (a failed
  primary render always owns exit 1; the history attachment can never
  overwrite it with exit 7). `RunningBulk::acceptance_children` plus the
  new `src/analysis/bulk/children.rs` (split out of `assemble.rs` to stay
  under the 1,000-line hygiene gate) now project the honest children of
  the validated acceptance, and `Analyzer::bulk_observed_children` keeps
  `bulk submit --wait`, `bulk status`, and `bulk wait` refreshing the same
  `(bulk_id, bulk_index)` children from the documented results window. The
  store coalesces terminal bodies on every refresh, and
  `find_analysis_by_task` resolves deterministically (`ORDER BY
  analysis_id`). The render layer tracks `primary_ok` and is exercised
  through scripted faulting sinks plus a compiled `/dev/full` invocation.
  The FileOrigin is honest per source (`file` with the JSONL basename for
  a real file submission; `stdin` otherwise). Compiled loopback and
  real-SQLite coverage gained the JSONL mixed-acceptance blocked-save
  proof, the armed-history children refresh and one-warning fallback, the
  deterministic task-lookup and bulk-child coalesce proofs, the text- and
  JSON-surface render-precedence attribution, and three fetch-all drift
  guards (a duplicated source position, an out-of-total position, and a
  non-advancing empty page). Validation is complete on the disposable
  Oracle Paris Crabbox box (slug `p4c-p2u-7d4b6b71-1595`): `cargo fmt
  --check`, strict all-target Clippy under `-D warnings`, and the full
  locked all-feature suite pass on stable Rust 1.97.1 (463 tests across
  30 result groups, 0 failures) and under MSRV Rust 1.87.0 (build plus
  the same 463 tests); generated contracts show no drift; `cargo audit`
  reports no vulnerabilities over the 323-crate lockfile; the CI
  `cargo deny check licenses` allow-list reports licenses ok;
  `tools/check-hygiene.rs` passes with no errors.

## Packet C remediation history (2026-08-02 to 2026-08-03)

Rounds 1 through 5 folded independent-review findings into Packet C before
its final certification pass:

- Real SQLite and compiled-loopback coverage locked ordered-tail persistence,
  one-warning automatic failures, render-failure precedence, and truthful bulk
  acceptance children. History and bulk modules were decomposed below the
  source-size gates without changing behavior.
- Typed `history.enabled` transitions now warn exactly once for every accepted
  true spelling, and acceptance-failed children preserve caller-owned JSONL
  origin, basename, plaintext, and FTS data without inventing task identity.
- Task and bulk identity reconciliation moved into bounded-retry `IMMEDIATE`
  transactions with schema-enforced uniqueness. Concurrent real-store tests
  prove deduplication, rollback, authorship preservation, and one invocation
  warning across children-read and persistence phases.
- The first exact-schema remediation rejected incompatible `user_version = 1`
  bodies before any write pragma, preserving their bytes, and primary render
  failures began carrying `primary_ok = false` through both JSON and text
  outcome chains. The final certification below tightens that schema probe to
  the complete catalog and closes the remaining task-first ordering gap.

All five folds used disposable Oracle Paris aarch64 containers with isolated
workroots and targets; stable and MSRV tests, strict Clippy, generated drift,
audit/deny, hygiene, inventory, and secret/privacy checks passed for each
recorded fold. One round-3 `crabbox warmup` accidentally claimed and immediately
released `static_yoga` before any command ran; no Yoga workroot or cache was
read or written. AGENTS.md remained excluded, and no credits, push, or PR were
used.

### Packet C remediation fold, round 6 (2026-08-03)

Certification of the folded Packet C change `f7732c07` found two P1
correctness defects; both are fixed contract-first and test-first in the
same Packet C change with no observable scope change beyond the contracts
themselves:

- Exact schema-v1 catalog validation (P1). The round-5 structural probe
  verified only name/kind presence plus four spot uniqueness/cascade
  rules, so near-miss v1 bodies passed falsely: a wrong
  `bulk_collections` unique column, a missing or wrong primary key, a
  named index with wrong ordered columns (or a sneaked-in
  `CREATE UNIQUE INDEX` reusing a contracted name), column nullability or
  declared-type drift, a missing or retargeted foreign key, or an FTS5
  table with wrong columns or tokenizer all opened as if they carried
  the exact v1 surface. docs/history-contract.md now locks the exact v1
  catalog surface section (every base table's full ordered columns with
  declared types, nullability, and defaults; the exact primary keys; the
  exact uniqueness index surface by origin and ordered columns, including
  specifically `bulk_collections.upstream_bulk_id`; the two named indexes
  with their ordered columns, uniqueness, and `created_at` direction and
  a global named-index count that rejects extras; every foreign key with
  its exact actions; and `analysis_search` as the exact FTS5 virtual
  table with its contracted columns and `unicode61` tokenizer), and the
  probe lives in the cohesive sibling `src/history/schema_v1.rs` (`store.rs`
  at 594 lines and the probe at 635 lines stay under the
  thresholds). The probe runs on every open strictly before any write
  pragma, so a rejection still leaves the original file byte-for-byte
  preserved. Real-SQLite regressions
  (`tests/history-store-schema.rs`) build eighteen named incompatible
  variants derived from the exact production body (wrong bulk unique
  column; missing and wrong analyses primary keys; swapped status-index
  columns; a unique bulk index masquerading under the contracted name;
  nullability, declared-type, and default drift; the missing
  pair-uniqueness, a missing or extra named index, an extra application
  table, and the wrong cascade action; missing or retargeted lineage
  foreign keys; and wrong FTS5 columns and tokenizer)
  and prove each fails closed as `history_corrupt` with the original
  bytes preserved, while the exact current body still opens and reopens
  cleanly.
- Order-independent task/bulk reconciliation (P1). A standalone task
  observation saved before a bulk read of the same upstream task made the
  bulk child insert collide with `UNIQUE (check_kind, upstream_task_id)`
  and roll its whole batch back. contracts.md 14.2 and
  docs/history-contract.md now lock the order-independence semantics:
  inside the one immediate write transaction each bulk child resolves by
  its membership AND every attested `(check_kind, upstream_task_id)` key
  together, reusing the one existing durable row when they agree (a
  standalone row gains its membership link through a refresh-only UPDATE
  (never an INSERT), but a row already belonging to another collection or
  position, or carrying only half a membership, fails closed rather than
  being moved). A task-less bulk child cannot be correlated from a later
  standalone task read alone. If it creates a distinct task-owning row, a
  later bulk observation of that task against the occupied task-less membership
  fails as ambiguous `history_corrupt` without merging evidence. Without that
  competing row, the bulk read attaches the task to the membership. Reuse
  preserves identity, authorship, save state, local input/FTS payload, and
  creation time, and failing closed
  as `history_write_failed` with the whole batch rolled back when the
  candidates conflict (two task keys resolving two different rows, a
  task-key row different from the membership row, or a membership holder
  attesting an overlapping but different task set, including a replacement
  task ID for the same check kind, or one candidate attesting two task IDs
  for one check kind). The standalone task reconcile resolves across ALL
  attested task keys, preserving bulk-first and failing contradictory reads
  closed instead of picking or replacing a row. Real-SQLite coverage in
  `tests/history-store-reconcile-{order,ambiguity}.rs` proves task-first adoption,
  evidence-driven bulk-refresh-then-task reconciliation, no fabricated
  direct adoption, cross-position and cross-collection rollback, conflicting
  task-identity and occupied-membership rollback (whole batch, unrelated rows untouched),
  same-key convergence before a later bulk read, no duplicate rows, and
  preserved authorship/local input; compiled loopback coverage
  `tests/history-save-order-loopback.rs` (split to stay under 800 lines) proves both
  directions end to end through the compiled binary with no automatic-save
  warning on an honest read.
- Terminal body reconciliation is branch-aware on standalone, membership,
  and adoption refreshes: body-empty terminal observations preserve the
  stored body even when `completed_at` is present, a result clears a stale
  error, an error clears a stale result, and both-present input fails the
  atomic write closed.

Validation for round 6 ran on the dedicated disposable Oracle Paris aarch64
container `1d798b846d63` (lease `cbx_18e74e5a40ba`, slug
`p4c-final-p1-7f3a9c`, SSH port 22896) through the explicit
`crabbox-paris-provider.sh` provider and then its direct Oracle SSH workflow
after the local warmup client timed out while the healthy container continued
starting. Isolation was verified inside the box (`uname -m = aarch64`,
`overlay` root), with unique workroot and stable/MSRV/tool targets. The Yoga
Windows SSH host, static `local-container`, repo jobs, and `remoteuse-*`
leases were never used.

Stable Rust 1.97.1 formatting, 489 locked all-feature tests across 37 result
groups, strict all-target Clippy, generated-contract regeneration with no
drift, the 323-crate inventory, and the hygiene gate passed. Rust 1.87.0
passed the locked all-feature build and the same 489 tests. Pinned
`cargo-audit` 0.22.2 found no vulnerabilities in the 323-crate lockfile,
`cargo-deny` 0.20.2 reported `licenses ok` under the CI allow-list, and the
checksum-verified gitleaks 8.30.1 aarch64 directory scan found no leaks.
No Pangram credits were spent; AGENTS.md remains the user's byte-exact
uncommitted local edit; no push, no PR. Phase 4 stays In progress.

### Packet C final certification remediation (2026-08-03)

Certification of Packet C `083fe8b0` found two P1 correctness gaps and one P2
maintainability gap, fixed without expanding the public history surface:

- Exact-v1 validation now checks each identity and named index's uniqueness,
  origin, `partial = 0`, ordered real key rows, `BINARY` collation, and sort
  direction. Real SQLite collation, partial, expression, extra-key, origin,
  and direction near misses fail before mutation with bytes preserved.
- Standalone task reconciliation compares every incoming key with all evidence
  owned by the selected row before mutation. Add and exact-refresh are allowed;
  same-kind replacement rolls back; omitted kinds survive. Real SQLite tests
  cover selection by another key, add/same/omitted cases, and concurrency.
- The 885-line reconcile module is split into task, bulk, and common modules
  below 800 lines, preserving one `HistoryStore` and its public API.

Final exactness validation used explicit Oracle Paris external-provider
container `cbx-pangram-p4c-ddl-recovery` (lease `cbx_e9fdd7257e90`), isolated
workroots, its per-lease key, and an SSH `ProxyCommand`. Stable 1.97.1 passed
formatting, 506 tests, Clippy, drift, inventory, and hygiene; MSRV 1.87.0
passed the locked all-feature build and tests. Audit 0.22.2, deny 0.20.2, and
checksum-verified gitleaks 8.30.1 passed. No credits, Yoga/static/repo jobs,
local Docker, push, or PR were used; `AGENTS.md` stayed excluded and byte-exact.
- 2026-08-04: Packet C first-open schema creation serializes under one SQLite `IMMEDIATE` transaction; real thread/process and rollback races pass.
- 2026-08-04: Packet C closes the final exact-schema P1 by deriving a complete
  canonical `sqlite_master` catalog from compiled `SCHEMA_V1` in isolated
  memory before mutation, with PRAGMAs retained as a second probe. Real SQLite
  tests reject hidden foreign-key, conflict-policy, and FTS5-option clauses
  byte-preservingly while harmless SQL spelling and first open stay green.
- 2026-08-09: Phase 4 moves to Complete. Packet D delivers optional local history with exact ordered per-check persistence, typed list/show/literal-search/delete/clear, redacted and full export, fresh-identity reruns with durable lineage, narrowly scoped canonical certification, bounded 10,000-analysis search under 100 ms, linear fresh-bulk FTS writes, pre-merge evidence validation, and bounded retry for SQLite WAL activation locks. Remote strict all-target Clippy and the full locked all-feature suite pass, including generated drift, four consecutive first-open races, and the performance gate. No local Cargo build or Pangram credit was used. Rust 1.87, native Windows, supply-chain, and secret gates remain required on the delivery PR; `AGENTS.md` stays excluded as the user's uncommitted change.
