# Pangram CLI testing and release plan

Status: approved for implementation

## 1. Verification strategy

Tests follow observable boundaries rather than internal function count.

```text
Domain invariants
      |
      v
Loopback Pangram protocol
      |
      v
Compiled CLI contracts
      |
      +--> TUI reducer and PTY
      +--> MCP stdio and conformance
      +--> History and export
      `--> Signed updater
```

No test uses a mocking framework or module mock.

## 2. Test-first rule

For non-trivial behavior:

1. add the observable failing test
2. implement the smallest cohesive behavior
3. make the test pass
4. inspect the diff and generated contracts
5. run the relevant broader suite

Do not replace or weaken a contract test when implementation is difficult.

## 3. Domain tests

Test:

- UUIDv7-prefixed local identity
- parent status derivation
- check ordering
- one-of input invariants
- retry and rerun lineage
- omission versus explicit content inclusion
- word and billable-unit estimation
- bulk source ordering
- timestamp normalization
- unknown upstream enum rejection
- partial success preservation
- error category and exit-code mapping

Use property tests for:

- any terminal child-state combination derives the documented parent state
- Pangram 4 text billable units equal one started 100-word block with a minimum
  of one
- Pangram 4 bulk billable units equal the sum of each valid item's started
  100-word units, with a minimum of one per item
- bulk preflight rejects an estimate above either the caller's ceiling or the
  1,000-unit upstream request limit
- pagination visits each item once in order
- accepted UUIDv7 IDs round-trip
- malformed IDs do not parse
- output envelopes never contain both top-level `data` and `error`

## 4. Pangram protocol fixtures

Use a real Axum server bound to loopback. The fixture server implements:

- text task submission
- task polling
- bulk submission
- bulk status
- bulk items
- bulk result pages
- file multipart upload
- plagiarism

Fixture scenarios:

- documented success
- in-progress stages
- terminal upstream failure
- 400, 401, 402, 403, 404, 413, 415, 422, 429, and 500 responses
- `Retry-After`
- delayed responses
- connection reset before send
- connection reset after ambiguous POST send
- malformed JSON
- missing required fields
- unknown stage, classification, and confidence
- missing or invalid Pangram 4 humanizer fields
- additive optional fields
- duplicate and out-of-order bulk pages
- partial bulk results
- required integer `plagiarized_sentences`; list or missing values fail
  upstream contract validation
- both conflicting official file-response shapes

Tests assert exact request method, path, header, JSON, multipart fields, and
whether retry occurred.

Text and bulk request fixtures must assert Pangram's documented Pangram 4
selector. No fixture may encode an inferred field or accept a request that
relies on Pangram's temporary Pangram 3 default. Bulk fixtures must also assert
the per-item 100-word billing calculation, the 1,000-unit request limit, and
the absence of per-item model selectors.

Ambiguous-send tests assert `submission_outcome_unknown`, `retryable: false`,
the sanitized local operation reference, and zero automatic POST replay.

No normal CI test calls Pangram.

## 5. Live conformance

Live conformance is a manual workflow requiring:

- dedicated `PANGRAM_API_KEY`
- explicit workflow dispatch
- synthetic, non-sensitive text
- maximum one estimated billable unit per scenario
- no public dashboard links
- sanitized retained fixture output

Required live gates:

1. Pangram 4 text task request, selected upstream version, humanizer fields,
   and complete result
2. text failure shape
3. Pangram 4 bulk submission, billed-unit behavior, status, and paginated result
4. binary file response
5. plagiarism result, especially `plagiarized_sentences`

If live output changes a required field, update the contract artifact first.

## 6. CLI contract tests

Run the compiled `pangram` binary with isolated config and data directories.

Verify:

- bare TTY versus piped dispatch
- positional, stdin, `-`, and file input
- every command and help page
- incompatible flags fail before fixture requests
- JSON, JSONL, TOON, Markdown, and pretty projection
- default multi-file JSONL
- explicit multi-file JSON array
- progress JSONL on stderr
- canonical error output
- stdout/stderr separation
- every exit code
- `NO_COLOR`
- interruption exit 130
- no noninteractive prompts
- completion output contains no diagnostics
- agent and skill output contain no decoration

Prefer readable inline snapshots. Inspect every updated snapshot diff.

## 7. Configuration and credential tests

Use real temporary files and platform permission checks.

Verify:

- precedence
- missing and unknown keys
- rate ceiling
- omitted first-run update preference
- credential environment override
- masked status
- credential rejection through `config set`
- atomic writes
- Unix `0600`
- failed restrictive persistence
- logout behavior with an environment key still present
- no credential in Debug, logs, JSON errors, or panic diagnostics

Windows ACL behavior requires a Windows CI test rather than a Unix simulation.

## 8. History tests

Use real SQLite with bundled FTS5.

Verify:

- disabled default
- manual save while automatic history is disabled
- automatic save when enabled
- successful analysis with failed history write
- concurrent reader and writer processes
- active-to-terminal transaction
- bulk child ordering and linkage
- retry and rerun lineage
- FTS over every specified field
- list and filter indexes
- redacted and full export
- delete one and clear all
- no import
- corrupt database preservation
- incompatible `user_version` failure
- no silent database replacement
- 10,000-analysis search performance budget

## 9. TUI tests

### Reducer

Feed `AppState` and `AppEvent` values directly. Verify:

- route and focus transitions
- responsive layout derivation
- composer and file queue
- check selector cannot become empty
- public-link and save toggles reset per new analysis
- progress and partial results
- active-session lifecycle
- history search and selection
- regular and Vim keymaps
- printable-key behavior in text fields
- destructive confirmation
- first-launch sequence
- update banner focus behavior
- `once`, `always`, and `off` intro frequency
- full, reduced, and off motion
- intro skip-key consumption
- one-time state is recorded after completion, skip, and reduced rendering
- suppressed and ineligible launches do not consume one-time state
- elapsed-time frame selection skips stale frames
- 2,799 ms selects frame 55 and 2,800 ms completes playback

### Rendering

Use Ratatui `TestBackend` snapshots at:

- 120x40
- 100x30
- 80x24
- below minimum size

Test wide and narrow segment presentation, errors, settings, help, and
unauthenticated state. Intro snapshots cover frame 0, each phase boundary,
frame 55, full and compact geometry, truecolor, ANSI, no-color, and ASCII
fallbacks. Generated frame fixtures MUST be deterministic.

### PTY

Use real PTYs to verify:

- empty TTY launches TUI
- piped input does not
- terminal resize
- Ctrl+C
- quit with active polling
- panic restoration
- cursor restoration
- alternate-screen cleanup
- no input escape injection
- intro skip keys do not reach Analyze
- no intro under `CI` or `TERM=dumb`
- resize from below 80x24 starts an unconsumed eligible intro
- stdout and stderr redirection prevent automatic TUI launch
- handled I/O errors restore the terminal
- unwind panics use the idempotent restoration path
- no guarded TUI path calls `process::exit`

### Autonomous acceptance

Use a separate Vitest harness with
`@kitlangton/terminal-control 0.6.0`. It launches the compiled `pangram` binary,
not a test renderer or semantic adapter.

Each scenario MUST:

1. start with isolated config, data, and home directories
2. disable inherited environment variables and use synthetic, non-sensitive
   content
3. set the viewport and terminal capabilities explicitly
4. wait for visible text or a settled capture instead of sleeping
5. drive exact keyboard input through the PTY
6. assert settled visible text and cell-frame snapshots
7. save sanitized text, JSON, SVG, logs, and metadata on failure
8. dispose the session and verify process exit and terminal cleanup

The acceptance matrix covers:

- first launch with skipped and stored authentication
- regular and Vim navigation
- Analyze, Active, History, Settings, help, and confirmation flows
- full, reduced, and off intro motion
- intro completion and every skip key
- 120x40, 100x30, 80x24, below-minimum, and live-resize layouts
- truecolor, ANSI, no-color, and ASCII fallbacks
- success, partial, upstream failure, and local-history failure presentation
- Ctrl+C and normal quit while idle and while polling

Intro timing remains a synthetic-clock reducer test. Terminal Control confirms
that the compiled application shows the expected real-terminal frames and
transitions, but wall-clock video timing is not the sole correctness oracle.

The agent establishes the initial text and cell-frame baselines by checking the
generated result against the locked geometry, palette, timing, capability,
layout, contrast, and terminal-integrity contracts. PNG, SVG, and an optional
MP4 recording support agent review; they do not replace semantic assertions.
The user reviews the quality of the final product rather than approving
individual baselines. Recordings are never produced for credentialed or live
Pangram scenarios.

The Terminal Control lane runs on GNU/Linux and macOS. Windows uses the native
PTY suite for startup, input, resize, interruption, and restoration because the
pinned package does not publish a Windows binary.

## 10. MCP tests

Spawn the actual stdio MCP server.

Verify:

- `server/discover` and protocol version `2026-07-28`
- required protocol version and client capabilities in request `_meta`, with
  client identity accepted as optional
- rejection of the removed initialization lifecycle and older protocol versions
- exact Phase 6 tool discovery: text detection, ordinary task get/wait, bulk
  submit/get/wait/results, and no later-phase tools
- ordinary `get_task` and `wait_task` tools without the Tasks extension
- structured and text result content
- `resultType: "complete"` on success and tool execution failure
- canonical `isError` failures
- billable tool annotations
- bulk pagination
- no credentials in schemas
- no generic CLI tool
- no history tools by default
- each capability gate
- `save: true` rejection unless both `--history` and
  `--allow-history-mutations` are enabled
- local analysis and bulk ID rejection without history, while upstream IDs
  remain usable
- public-link rejection
- absolute, existing, repeatable file-root startup validation
- installer refusal to approve file roots automatically
- inline bulk items without roots, and `jsonl_path` rejection without a root
- symlink and Windows reparse-point escape
- path replacement between authorization and open
- handle-relative open never escapes a pre-opened root
- invalid startup configuration fails before stdin read, writes no stdout,
  writes one sanitized stderr diagnostic, and exits 1
- stderr logging without stdout frame corruption
- exact static resource bytes and MIME types from
  `contracts/output.schema.json`, `generated/error-reference.json`, and
  `skills/pangram/SKILL.md`
- no prompts and no history resources
- deterministic tool and resource ordering
- `ttlMs: 0` and `cacheScope: "private"` on list and resource-read results
- no subscription stream for immutable inventories
- exact per-tool success and failure envelope schemas
- absence of `io.modelcontextprotocol/tasks`, `tasks/get`, `tasks/update`, and
  `tasks/cancel`
- JSON-RPC cancellation stops local observation, produces no response for the
  cancelled request, and does not claim upstream cancellation
- `--allow-history-mutations` startup rejection without `--history`
- `history_get` returns the canonical `history_show` envelope
- generated `mcp-tools.json` is the one ordered descriptor and per-tool schema
  inventory, and `agent-reference.md` matches the embedded bytes

Pin RMCP exactly to 3.1.2 and verify its locked source/archive identity, Rust
1.88 minimum, and Apache-2.0 license transition notice. Pin
`@modelcontextprotocol/conformance` exactly to 0.2.0-alpha.11 with npm
`gitHead` `c321dd32035556e6769d3724a8ee97d87c3faaac` and integrity
`sha512-imPK9tx5gQsL6ZKQq4MrsyDYfSaIwpRmX6+ogjbeAXs9LGvxkBxWcY7KcS7TvwaBk/ZiVWl6b/naF4q83UwDRA==`.

The official suite cannot drive stdio while upstream issue 258 remains open.
Its frozen `2026-07-28` server requirements also mandate diagnostic `test_*`
tools, prompts, templates, binary resources, completion, SSE streams,
DNS-rebinding checks, sampling, elicitation, roots, and input-required flows
regardless of the server's advertised capabilities. Do not add those surfaces
to a test-only HTTP server and call that Pangram conformance. Individual
overlapping scenarios may exercise shared handler code, but the expanded
fixture cannot prove Pangram's full-profile or stdio conformance. Prove the
selected product contract through the compiled `pangram mcp` stdio subprocess
suite. Re-run the applicability review when the official suite supports stdio
and capability-aware scenario selection. Verify that HTTP transport and
conformance-only `test_*` tools/resources are absent from normal builds and
release artifacts.

Installer tests use real temporary client config files for every supported
client whose current path and schema have been pinned from authoritative
evidence. Verify explicit target selection, parsed atomic writes, idempotence,
dry run, preservation of unrelated fields and bytes where the format permits,
exact server-name ownership, conflicting-entry rejection, exact matching owned
uninstall, and malformed-config failure without replacement. Verify installers
add no root or optional capability flag by default. Do not enable a client
target until its path, schema, and exact owned-entry match rule are pinned.

## 11. Updater tests

Use a local HTTPS-capable or loopback fixture appropriate to the test layer and
real Ed25519 keys generated for fixtures.

Verify:

- detached manifest signature
- exact downloaded-byte verification without JSON reserialization
- unknown key ID
- wrong signature
- overlap key rotation and removed key rejection
- unsupported schema and channel
- running updater below minimum updater version
- equal-version no-op and downgrade rejection
- target selection
- HTTPS production URL enforcement
- archive and expanded-executable size mismatch
- SHA-256 mismatch
- duplicate target rejection
- absolute, parent, symlink, hardlink, device, duplicate-executable, and
  unexpected archive entries
- direct-install receipt path match
- receipt creation only after version smoke test
- receipt finalization retry after successful replacement
- stale and moved receipt
- package-manager advisory behavior
- interrupted download
- atomic replacement
- executable permission
- Windows replacement behavior
- current binary preserved on every failure
- no automatic check outside the TUI
- 24-hour TUI check interval and ETag
- 200, 304, stale ETag, network failure, and clock rollback state transitions

Private development builds perform no release-network request.

## 12. Documentation tests

Verify:

- generated references match Rust
- examples validate against schemas
- commands and flags exist
- blocked features are labeled
- links return expected content
- Fumadocs typecheck and build
- local search index
- `llms.txt` and `llms-full.txt`
- individual page Markdown
- no real secrets or sensitive content
- no analytics
- ASCII punctuation in edited source and docs

## 13. Performance acceptance

Release build targets on supported desktop hardware:

| Operation | Target |
| --- | ---: |
| TUI first frame before intro | under 100 ms |
| Intro nominal duration | 2,800 ms |
| Intro frame cadence | 50 ms |
| Warm `--help` | under 150 ms |
| Warm `--version` | under 150 ms |
| Warm `agent` or `skills` | under 150 ms |
| History FTS at 10,000 analyses | under 100 ms |

Network analysis latency is upstream-dependent and has no product performance
claim.

These values are advisory budgets until a benchmark job records:

- runner image and CPU model
- release binary hash
- five warm-up runs
- at least 30 measured runs
- median and 95th percentile
- failure only when the 95th percentile exceeds the budget by more than 10
  percent in two consecutive release-candidate runs

History FTS uses a committed synthetic 10,000-analysis fixture. Intro cadence is
a deterministic synthetic-clock assertion, not a wall-clock CI benchmark.

Bulk input and result pages SHOULD stream so memory growth is bounded by page
size plus output buffering.

## 14. Static checks

Required:

- `cargo fmt --check`
- Clippy with warnings denied for project code
- `cargo test`
- MSRV build
- current stable build
- dependency license policy
- Rust vulnerability audit
- TypeScript typecheck
- compiled-TUI Terminal Control acceptance on GNU/Linux and macOS
- docs lint and build
- generated-contract clean-tree check

Do not introduce a custom type-aware lint tool when normal Clippy or frontend
lint rules can enforce the invariant.

## 15. Release version ownership

One SemVer value is shared by:

- Rust package
- main npm package
- platform npm packages
- embedded skill
- schema bundle
- docs reference
- release manifest
- GitHub Release

Stable `0.x` releases use the same exact-version authorization and public gates
as later releases. Their built-in updater remains network-free until `1.0.0`.

## 16. Tegami workflow

Tegami owns:

- `.tegami/*.md` fragments
- explicit bump computation
- Cargo and npm version updates
- cross-registry dependency ranges
- version pull request
- publish lock
- npm publication

Configure the GitHub plugin with:

```ts
github({
  repo: "Microck/pangram-cli",
  createTags: false,
  release: false,
});
```

Pin Tegami exactly as a development dependency. It currently requires Node.js
24 or later.

Do not run `tegami init-agent`.

## 17. Release ordering

```text
Merge user changes with Tegami fragments
                 |
                 v
Tegami opens version pull request
                 |
                 v
Merge version and publish-lock changes
                 |
                 v
Build/test every native target
                 |
                 v
Assemble npm platform packages and installers
                 |
                 v
Verify hashes, manifest signature, SBOM, and smoke tests
                 |
                 v
Create the immutable tag at the built workflow commit
and the private draft GitHub Release
                 |
                 v
Upload every artifact, manifest, signature, and provenance file
                 |
                 v
Prepare matching documentation without publishing it
                 |
                 v
Acquire the Tegami publish lock
                 |
                 v
With version-scoped authority, make the repository public
                 |
                 v
Publish the GitHub Release and matching documentation
                 |
                 v
Tegami publishes npm packages
                 |
                 v
Update Homebrew and Scoop metadata
                 |
                 v
Run public clean-install and update smoke tests
```

The GitHub Release and matching documentation form the first coherent public
availability point. No registry publication begins until their URLs are public
and every required build artifact passes.

The release job extracts the exact requested-version section from Tegami's
root `CHANGELOG.md` into a temporary notes file. It fails before tag or release
creation when the changelog or version section is absent. `gh release create`
receives the workflow commit through `--target`, so a concurrent default-branch
advance cannot move the release tag away from the source used for every build.

Production publication requires one explicit authorization that names the
exact version and the complete destination set: repository visibility, GitHub
Release, production documentation, npm, Homebrew, and Scoop. The authorization
cannot be reused for a different version, destination, release channel, or
later release.

If registry publication partially fails, retain the Tegami publish lock and
retry. Do not unpublish already published package versions. GitHub Release,
documentation, npm, Homebrew, and Scoop each have an idempotent retry step and a
recorded operator-visible recovery state. Recovery may retry only the version
and destinations named by the existing authorization.

## 18. Release artifacts

Targets:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

Windows aarch64 is excluded until a real executable can be smoke-tested.

Each archive includes:

- `pangram` executable
- README
- MIT license
- shell completions
- man page

Release also includes:

- SHA-256 checksum file
- signed update manifest and detached signature
- SPDX SBOMs
- generated third-party license notices
- GitHub artifact provenance

## 19. Signing

Use Ed25519 for the detached update-manifest signature.

- private key exists only in a protected GitHub Actions release environment
- pull requests cannot access it
- public keys are embedded in the updater
- the detached manifest signature identifies `key_id`
- rotation adds a new public key in an earlier release before signing only
  with it

Never print private signing material or pass it through pull-request artifacts.

## 20. Distribution channels

Public channels:

- GitHub Releases
- POSIX shell installer
- PowerShell installer
- Homebrew tap `Microck/pangram-cli`
- Scoop bucket `Microck/scoop-pangram-cli`
- npm `@microck/pangram-cli`
- platform npm packages under `@microck`

The npm main package selects an exact supported platform package through
optional dependencies and fails clearly on unsupported targets.

Package managers retain update ownership.

pnpm and Bun consume the npm package; they are not separate publication
channels.

## 20.1 Platform support baseline

| Target | Minimum runtime baseline | Required release evidence |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | Linux kernel 3.2 and glibc 2.17 | build plus native smoke test in a glibc 2.17 environment |
| `aarch64-unknown-linux-gnu` | Linux kernel 4.1 and glibc 2.17 | build plus native ARM64 smoke test in a glibc 2.17 environment |
| `x86_64-apple-darwin` | macOS 10.12 | native x86_64 smoke test at the declared deployment target |
| `aarch64-apple-darwin` | macOS 11.0 | native Apple Silicon smoke test |
| `x86_64-pc-windows-msvc` | Windows 10 or Windows Server 2016 | native Windows x64 smoke test |

These are Rust target baselines, not proof of Pangram CLI support. A target is
publicly supported only after its native release evidence passes. If the
required minimum-version environment is unavailable, narrow the published
claim rather than substituting a cross-build.

## 21. Public release blockers

The release workflow MUST require:

- repository release variable authorizing public release
- Pangram written permission for API distribution and terminal fox-logo use
- live file conformance
- live plagiarism conformance
- owned registry names
- accepted generated intro frame art
- hashed source geometry, provenance, and written redistribution terms for
  derived intro frames
- public landing page ready at `pangram.micr.dev/` and Fumadocs ready at
  `pangram.micr.dev/docs`
- unofficial-project disclosure on the README, documentation site, and package
  metadata
- every required check passing
- no unresolved P0 or P1 finding

This gate is intentional. Making the GitHub repository public does not by
itself authorize publishing billable integrations or release artifacts.

## 22. Maintainability review

Before public v1, run the thermo-nuclear review across:

- abstraction depth
- duplicate adapter logic
- giant modules
- state ownership
- validation duplication
- error quality
- dead code
- test realism
- documentation-to-code enforcement
- release complexity

The review produces a checked artifact containing the reviewed commit, named
review lanes, command outputs, findings by severity, disposition for every
finding, and a pass only when no actionable P0 or P1 remains. The review name
alone is not acceptance evidence.

## 23. Security fuzz and property corpora

Bounded fuzz or property suites cover:

- upstream JSON normalization and unknown values
- Unicode and terminal control-string sanitization
- Markdown and external-link escaping
- update manifest and detached-signature parsing
- archive layout and extraction
- MCP root-relative path handling under concurrent filesystem changes

Every discovered crash or invariant violation becomes a minimized committed
regression fixture.

Files near 800 lines require review. Files over 1,000 lines block release
without an ADR.
