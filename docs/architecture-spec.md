# Pangram CLI architecture specification

Status: approved for implementation
Runtime language: Rust 2024
Documentation language: TypeScript
Architecture style: one package, deep core module, thin adapters

## 1. Architectural goals

The architecture MUST:

- expose one behavioral implementation to CLI, TUI, and MCP
- represent asynchronous Pangram work as typed state rather than string maps
- keep billable submission and retry behavior in one place
- normalize upstream contracts once
- preserve successful partial work
- keep JSON as the canonical output model
- support deterministic loopback protocol testing without mocks
- remain a single Rust package until a second package boundary earns its cost
- prevent UI, protocol, persistence, and release concerns from leaking into one
  another

The architecture MUST NOT begin with:

- a generated Pangram SDK
- a generic HTTP transport trait
- a generic repository layer
- one endpoint wrapper per file
- a public Rust SDK promise
- a daemon
- a multi-crate workspace
- duplicated CLI, TUI, and MCP orchestration

## 2. System context

```text
                         +----------------------+
                         | Pangram documented   |
                         | REST endpoints       |
                         +----------+-----------+
                                    ^
                                    | HTTPS + x-api-key
                                    |
 +-------------+   +-------------+  |  +----------------+
 | CLI adapter |-->|             |--+->| Text / bulk    |
 +-------------+   |             |     +----------------+
                   | Analysis    |
 +-------------+   | module      |---->| File           |
 | TUI adapter |-->|             |     +----------------+
 +-------------+   |             |
                   |             |---->| Plagiarism     |
 +-------------+   +------+------+     +----------------+
 | MCP adapter |----------+
 +-------------+          |
                          v
                  +-------+--------+
                  | Config/history |
                  | Output/events  |
                  +----------------+
```

Only the analysis module may call Pangram endpoints. Adapters call typed
operations and consume typed progress events and results.

## 3. Repository layout

The initial implementation should follow this ownership layout:

```text
.
|-- Cargo.toml
|-- Cargo.lock
|-- src/
|   |-- lib.rs
|   |-- main.rs
|   |-- domain.rs
|   |-- analysis.rs
|   |-- config.rs
|   |-- history.rs
|   |-- output.rs
|   |-- cli.rs
|   |-- tui.rs
|   |-- mcp.rs
|   |-- update.rs
|   `-- diagnostics.rs
|-- tools/
|   |-- generate-contracts.rs
|   |-- generate-intro-frames.rs
|   `-- tui-acceptance/
|       |-- package.json
|       `-- tui-acceptance.test.ts
|-- skills/
|   `-- pangram/
|       `-- SKILL.md
|-- contracts/
|   |-- output.schema.json
|   |-- config.schema.json
|   |-- tui-state.schema.json
|   `-- update-manifest.schema.json
|-- tests/
|   |-- fixtures/
|   |-- cli-contract.rs
|   |-- protocol-contract.rs
|   |-- mcp-contract.rs
|   |-- tui-pty.rs
|   `-- update-contract.rs
|-- docs/
|-- scripts/
|   `-- tegami.mts
`-- docs-app/
```

This is an ownership map, not a demand for tiny files. A module may start as
one file and move to a directory only when cohesive submodules are large enough
to justify it.

`docs-app/` is the Fumadocs TypeScript workspace. It MUST NOT be part of a Rust
workspace.

## 4. Package and binary boundaries

The Rust package is named `microck-pangram-cli` and provides:

- library target used by the shipped binary and integration tests
- `pangram` binary
- feature-gated `generate-contracts` development binary

The library target is an internal architecture boundary and is not published to
crates.io in v1. Public documentation MUST NOT advertise a stable Rust SDK.

The contract generator requires a `dev-tools` feature and is absent from normal
release artifacts.

## 5. Module ownership

| Module | Owns | Must not own |
| --- | --- | --- |
| `domain` | IDs, inputs, checks, status, results, events, errors | HTTP, terminal, SQLite |
| `analysis` | HTTP submission, polling, retry, normalization, running handles | CLI flags, TUI state, RMCP |
| `config` | paths, precedence, TOML validation, credential permissions | remote validation |
| `history` | SQLite schema, transactions, FTS, exports | remote submission |
| `output` | canonical envelopes and projections | command execution |
| `cli` | Clap grammar, input resolution, dispatch, exit mapping | HTTP and SQL |
| `tui` | reducer, event loop, screens, keymaps, terminal lifecycle | HTTP and SQL internals |
| `mcp` | RMCP protocol, tools, file scopes, capability gates, installers | subprocess CLI execution |
| `update` | manifest signature, receipts, atomic replacement | release production |
| `diagnostics` | sanitized local checks and environment reporting | billable validation |

## 6. Domain model

The authoritative serialized model is defined in [contracts.md](contracts.md).
Rust domain types enforce its invariants.

Core types:

```rust
pub struct AnalysisId(/* UUIDv7 */);
pub struct BulkId(/* UUIDv7 */);
pub struct UpstreamTaskId(String);
pub struct UpstreamBulkId(String);

pub enum CheckKind {
    AiDetection,
    Plagiarism,
}

pub enum AnalysisStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Partial,
}

pub enum AnalysisInput {
    Text(TextInput),
    File(FileInput),
}

pub enum CheckResult {
    AiDetection(AiDetectionResult),
    Plagiarism(PlagiarismResult),
}

pub enum StartRequest {
    Analysis(AnalysisRequest),
    Bulk(BulkRequest),
}

pub enum OperationReference {
    Analysis(AnalysisReference),
    Bulk(BulkReference),
}
```

The actual implementation may refine names, but serialized names and
invariants MUST match the contract.

### 6.1 Validation boundary

Validate:

- CLI and MCP arguments at adapter boundaries
- configuration when loaded or changed
- Pangram response shape once during normalization
- history JSON once when read
- update manifests before use

Do not revalidate the same invariant inside hot loops or every projection.

### 6.2 Unknown upstream values

Do not use catch-all enum variants for required Pangram states. Unknown
required states fail as `upstream_contract_changed` and include a sanitized
field path and value.

Optional additive upstream fields may be ignored until adopted.

## 7. Analysis module

### 7.1 Public operation surface

The shared module needs a small interface equivalent to:

```text
start(request: StartRequest) -> RunningOperation
snapshot(reference: OperationReference) -> OperationState
observe(reference: OperationReference, options: WaitOptions) -> RunningOperation
bulk_results(reference: BulkReference, page: Page) -> BulkResultPage
```

`AnalysisRequest` carries input, an ordered non-empty check set, public-link
choice, and persistence choice. It represents detection, plagiarism, and
combined analysis without separate orchestration methods.

The request does not carry a user-selectable Pangram model. The production
analyzer is fixed to Pangram 4 and must serialize Pangram's documented model
selector, JSON request field `model` with the exact value `pangram-4`. It
must not omit `model`, depend on the temporary Pangram 3 default, or retain a
second model path.

`BulkRequest` carries validated ordered items and the mandatory billable-unit
ceiling. Reference enums represent local and upstream identities without
nullable ID pairs.

These are behavior names, not a mandate for a trait. Begin with a concrete
`Analyzer`. Endpoint-specific functions remain private implementation details.

### 7.2 Running handle

`RunningOperation` centralizes:

- typed local and upstream identity
- ordered progress events
- final analysis or bulk result
- local wait timeout
- stop-observing signal

It SHOULD expose an event receiver and an awaitable completion method rather
than adapter-specific callbacks.

Stopping the handle cancels local polling with a `CancellationToken`. It MUST
NOT send a remote cancellation request.

`snapshot` performs one remote observation and returns. `observe` repeats that
same private observation operation until a terminal state, local timeout, or
local cancellation. This prevents status and wait commands from growing
separate protocol paths.

### 7.3 Combined analysis

Combined analysis runs:

```text
AI submit/poll -------+
                      +--> derive combined terminal state
Plagiarism request ---+
```

The two futures run concurrently under one local analysis. Completion preserves
each check result or error independently.

### 7.4 Bulk

Bulk submission validates all input and estimated units before sending the
POST. After acceptance:

- the collection retains caller IDs and source ordering
- upstream status is normalized
- result pages stream into ordinary child analysis values
- fetching all results iterates pages rather than requesting an undocumented
  aggregate

The module MUST NOT automatically retry bulk submission after an ambiguous
network outcome.

## 8. HTTP client

### 8.1 Endpoints

Production endpoint constants are:

```text
https://text.external-api.pangram.com/task
https://text.external-api.pangram.com/bulk
https://file-external.api.pangram.com/
https://plagiarism.api.pangram.com/
```

All requests use `x-api-key`.

Production configuration cannot override these values. Test-only constructors
accept loopback endpoint sets.

The Pangram SDK v1.0.0 tag documents the exact text and job-wide bulk request
selector (`model` set to `pangram-4`) even though the rendered public REST
reference still describes the Pangram 3-era request. Production text
submission sends that explicit selector and never relies on default routing;
do not infer the selector from dashboard traffic. A future bulk submission
uses the same job-wide selector and never places a selector on individual
items. Each valid item contributes one unit per started 100-word block, with a
minimum of one; the job estimate is their sum. The upstream request limit is
1,000 billable units with no separate item-count limit. Do not reuse the
Pangram 3-era 1,000-word unit. Bulk remains unimplemented until Phase 3, and
both paths must be represented in exact loopback request fixtures before their
adapters use them. Live conformance remains a public-support gate.

### 8.2 Reqwest configuration

Use Reqwest with:

- rustls
- system proxy discovery
- JSON
- multipart
- streaming
- gzip, Brotli, and deflate where applicable

Do not provide an insecure TLS option.

### 8.3 Rate and retry policy

Use one shared time-based issue gate with a hard maximum of 5 requests per
second. Every Pangram request (submit and poll) is issued only after the gate
releases it, so request issue times are spaced at least `1/rate` apart and no
burst can exceed the pacing. Rate limiting is enforced on request issue
timing, not request completion. Configuration may lower the rate but never
raise it above 5. The gate schedule is shared across all callers of one
client (one owner, not a per-request or per-sender limiter).

Retry policy:

| Operation | Automatic retry |
| --- | --- |
| GET task/status/page | Yes, for bounded transient failures |
| POST text task | No after ambiguous send |
| POST bulk | No after ambiguous send |
| POST file | No after ambiguous send |
| POST plagiarism | No after ambiguous send |

Honor `Retry-After`. Use bounded exponential backoff with jitter for eligible
GET requests. Poll intervals and HTTP retry backoff are separate concepts.

Safe-GET retry chains observe the caller's wait deadline and a cumulative
retry-time budget in addition to the per-attempt cap. The total time spent
sleeping between retry attempts must not exceed the cumulative budget, and a
caller wait/cancellation deadline interrupts the retry prompt sleep promptly
(even through repeated `429`/`503` responses carrying `Retry-After` hints).
Retry attempts remain bounded; the budget and deadline add interruption
semantics, they do not extend the attempt count. The jitter used to
decorrelate backoff schedules advances once per draw (atomically per caller)
so concurrent callers can never produce identical lockstep retry schedules.

### 8.4 Error mapping

HTTP and protocol errors map once to canonical categories:

- usage
- authentication
- permission
- payment
- rate_limit
- network
- upstream
- upstream_contract
- local_config
- local_history
- update

Do not expose raw response bodies by default.

## 9. Output architecture

Domain results serialize to one canonical envelope. Projections consume the
canonical typed value:

```text
Domain result
    |
    +--> canonical JSON value
             |
             +--> JSON
             +--> JSONL
             +--> TOON
             +--> Markdown
             `--> pretty terminal
```

Projections MUST NOT re-run domain logic or recalculate status.

JSON field ordering may be deterministic for fixtures, but consumers MUST NOT
rely on object order.

Pretty and TUI rendering sanitize control characters. Markdown rendering
escapes structural content and unsafe links.

## 10. Configuration

### 10.1 Paths

Use platform directories through `directories`:

- configuration file in the platform config directory
- history and runtime state in the platform data directory

Overrides:

- `PANGRAM_CONFIG`
- `PANGRAM_DATA_DIR`

### 10.2 Precedence

General precedence:

```text
command flags
> environment variables
> explicit config file
> default config file
> built-in defaults
```

Credential precedence is specifically:

```text
PANGRAM_API_KEY > stored key
```

### 10.3 Credentials

The stored key lives in dedicated versioned `credentials.toml` under the
default platform configuration directory. `PANGRAM_CONFIG` never relocates it.
Unix permissions MUST be `0600`. Windows MUST apply an owner-only ACL.

If restrictive persistence cannot be established, credential storage fails
closed. Ephemeral environment authentication remains available.

The in-memory value SHOULD use `secrecy` and MUST never implement ordinary
debug display.

## 11. History storage

### 11.1 Technology

Use `rusqlite` with bundled SQLite and FTS5. Use transactions for state changes
and enable WAL mode where supported. Every connection enables foreign keys and
secure deletion before accessing application tables.

No asynchronous database abstraction is required. Short SQLite operations may
run through `spawn_blocking` when called from async adapters.

### 11.2 Storage shape

Store canonical JSON for input and result bodies plus typed columns needed for
identity, filtering, and lifecycle. Do not normalize every segment into a
table.

The normative schema is in [contracts.md](contracts.md).

### 11.3 Ownership

One `HistoryStore` owns:

- database open and schema validation
- permission checks
- analysis create/update
- task identity
- bulk relationships
- FTS maintenance
- list, get, search, delete, clear, and export

Do not introduce a repository trait until a second backend exists.

The platform data directory requires owner-only access. The database and its
WAL and shared-memory sidecars require owner-only file permissions. History
fails closed before opening SQLite when those restrictions cannot be
established.

### 11.4 Failures and corruption

History failures return local warnings or local errors according to operation:

- an automatic write accompanying successful analysis is a warning
- an explicit `--save` or history command failure is an error
- corruption preserves the original file and blocks writes

Delete and clear are logical deletion, not a secure-erasure guarantee. The
store updates FTS transactionally and truncates the WAL before reporting an
explicit destructive operation complete.

There is no silent repair, replacement, or development-state compatibility
bridge.

## 12. CLI adapter

`main.rs` performs only:

1. process-level tracing and panic setup
2. argument parsing
3. dependency construction
4. dispatch
5. exit-code mapping

Clap definitions and dispatch may share `cli.rs` initially but must separate
when the file approaches the module review threshold.

Input resolution occurs before the analysis module can make a billable call.
Unsupported combinations are local usage errors.

The CLI progress renderer consumes the same analysis events as the TUI and
MCP adapter.

## 13. TUI adapter

### 13.1 Reducer

The TUI uses one state transition boundary:

```text
AppState + AppEvent -> AppState + Effects
```

Effects request analysis, history, update, URL, or terminal actions. Async work
returns typed events through a channel.

Screen code renders state and emits user-intent events. It does not own
networking or SQLite.

### 13.2 State

The root state owns:

- active route
- focus
- composer and file queue
- current analysis identity and progress
- selected history record
- settings draft
- overlays
- terminal size
- intro phase

Derived values such as parent analysis status, responsive layout, and enabled
actions SHOULD be computed from this state instead of copied into additional
mutable fields.

### 13.3 Layout

The wide layout derives three areas from root state:

```text
+-------------+---------------------------+------------------+
| Route rail  | Center workspace          | Inspector        |
| compact fox | input, progress, results  | state and actions|
+-------------+---------------------------+------------------+
| Contextual command bar                                     |
+------------------------------------------------------------+
```

The center workspace receives most available width. The inspector contains
checks, privacy, cost, result filters, and actions that apply to the active
state. The route rail contains only the compact resolved mark and primary
routes. It is not an intro canvas.

At narrower supported sizes, navigation becomes top tabs and inspector content
joins the center flow. Layout is derived from terminal size and current state;
wide and narrow screens do not maintain separate mutable UI models.

Rendering uses sparse separators, focus markers, and a restrained command bar.
It does not reproduce web-style cards, oversized buttons, or a persistent
large brand mark.

### 13.4 Intro renderer

The intro is an internal deep module in the TUI adapter. It has no trait and no
public interface. Callers provide resolved intro policy, terminal capabilities,
whether the one-time state has been seen, and monotonic elapsed time. The
module returns either no intro, the reduced resolved mark, or the frame to
render.

The implementation uses two generated constant frame tables:

- 56 frames at 32x16 cells
- 56 frames at 20x10 cells

Both tables share the same 20 fps timeline. A development-only generator
converts approved fox vector geometry into styled terminal cells. Generated
tables are committed so normal builds and runtime startup need no SVG, video,
image decoder, floating-point rasterizer, or filesystem asset lookup.

The generator samples one timeline:

- transform settle: 0 through 800 ms
- orange center unfold: 150 through 700 ms
- pink facet unfold: 280 through 920 ms with 60 ms stagger
- resolved hold: 920 through 2,380 ms
- linear density dissolve: 2,380 through 2,800 ms

Transform phases use `cubic-bezier(0.175, 0.885, 0.32, 1.1)`.

For elapsed time below 2,800 ms, frame selection uses
`floor(elapsed_ms / 50)`. At 2,800 ms or later, the module returns completion
instead of another frame. The event loop does not enqueue one event per missed
frame. This keeps the nominal duration at 2.8 seconds when terminal rendering
stalls.

Glyph capability and color capability are resolved once before playback.
Fallback selection is deterministic:

1. full or compact terminal-cell table based on terminal size
2. truecolor, ANSI approximation, or no-color styling
3. Unicode cell glyphs or the generated ASCII `#`, `*`, `.`, and space table

`Escape`, `Enter`, and `Space` produce one skip event that transitions directly
to Analyze. The input event is consumed before normal routing.

For `tui.intro = "once"`, the TUI atomically writes its `intro_seen` marker
after completion, skip, or reduced rendering. The marker lives in TUI state
at `PANGRAM_DATA_DIR/tui-state.json`, not in configuration. A failed write
reports a non-blocking diagnostic and does not stop Analyze. Suppressed startup
does not write it.

The generator and derived frame tables may enter the public tree only after
the Pangram logo-use release gate is satisfied. Droid source, frames, and
marketing assets are research evidence only and MUST NOT enter the repository.

### 13.5 Terminal lifecycle

One terminal guard owns:

- raw mode
- alternate screen and bracketed-paste mode
- cursor visibility
- mouse capture when enabled
- panic restoration

The guard's restoration operation is idempotent and restores state on drop.
Process-level unwind panic handling and supported signal handling invoke the
same operation before printing diagnostics. Guarded code returns exit intent
and MUST NOT call `process::exit`. The build uses `panic = "unwind"`.

The release guarantee covers normal return, handled I/O failure, Ctrl+C,
supported catchable termination signals, and unwind panic. Process abort,
SIGKILL, and equivalent uncatchable termination are outside the guarantee.

### 13.6 Autonomous acceptance boundary

The compiled TUI is exercised through a development-only Terminal Control
harness. The harness launches the real binary in a real PTY and drives the same
keyboard and resize paths a person uses. It does not link into the application
and does not ship in release artifacts.

The harness uses `@kitlangton/terminal-control 0.6.0` with Vitest. Tests set an
isolated config directory, data directory, home directory, locale, terminal
type, and viewport. They do not inherit the operator's environment. Settled
text and cell frames are source-controlled snapshots. Text, JSON, SVG, logs,
and metadata are retained as failure evidence. PNG captures and recordings are
opt-in review artifacts and use synthetic, credential-free scenarios because
typed input and terminal streams may contain secrets.

The acceptance harness runs on GNU/Linux and macOS, matching Terminal Control's
published native packages. Windows keeps its native platform PTY and terminal
restoration tests. A platform-independent Ratatui `TestBackend` layer remains
the source of deterministic renderer snapshots.

Do not implement Terminal Control's optional OpenTUI semantic protocol. It is
not a Ratatui contract, and adding it would create a second machine-only UI
surface that could pass while the visible terminal is broken.

## 14. MCP adapter

### 14.1 Protocol lifecycle

The stdio adapter implements MCP `2026-07-28` only. `server/discover` reports
the server identity, protocol version, and immutable capabilities. Each request
carries protocol version, client identity, and client capabilities in `_meta`.
The adapter does not implement the removed initialization handshake or a legacy
protocol fallback.

RMCP owns wire-level protocol types. Pangram-owned types begin at tool
arguments, canonical command envelopes, and startup capability configuration.
Every ordinary result has `resultType: "complete"`. Tool and resource list
results are deterministic, private, and non-cacheable with `ttlMs: 0`.

### 14.2 In-process execution

The RMCP server constructs and calls the same `Analyzer`,
`HistoryStore`, `ConfigStore`, and `UpdateChecker` used by other adapters.

It MUST NOT spawn `pangram` as a subprocess and MUST NOT expose a generic CLI
tool.

### 14.3 Capability gates

Server startup resolves immutable capabilities:

- history reads
- history mutations
- configuration mutations
- public links
- approved file roots

Tools consult the resolved capability set. Installers never enable optional
capabilities automatically.

`--allow-history-mutations` requires `--history`. Each
`--allow-file-root PATH` must name an absolute, existing directory that can be
pre-opened safely. Invalid capability combinations or file roots fail before
the server begins reading JSON-RPC messages. Installers never configure file
roots.

### 14.4 File roots

Approved roots are fixed startup configuration, not the deprecated MCP Roots
feature. With no approved roots, the adapter omits `detect_files`.

The adapter pre-opens approved root directories and delegates root-relative,
handle-based file opening to one concrete filesystem module. That module uses
no-follow semantics, rejects symlink and Windows reparse-point traversal, and
verifies the opened object. For each absolute tool path, it selects the matching
pre-opened root and resolves only the relative segments beneath that handle. It
MUST NOT authorize a canonicalized path string and reopen that path later.

### 14.5 Long-running Pangram work

The adapter exposes ordinary typed `get_task` and `wait_task` tools over the
same underlying analysis handles used by the CLI and TUI. It does not implement
the experimental `io.modelcontextprotocol/tasks` extension.

Local analysis IDs and Pangram task IDs remain separate fields. JSON-RPC
cancellation stops local observation only.

### 14.6 Errors

Schema-invalid calls use protocol validation errors. Domain-valid operations
that fail return `isError: true` with one canonical failure envelope in
`structuredContent` and `resultType: "complete"`. Success returns one canonical
success envelope with `resultType: "complete"`. Generated per-tool schemas fix
the command and `data` root.

All logs go to stderr to preserve JSON-RPC framing.

## 15. Updater

### 15.1 Trust

The updater embeds Ed25519 public keys. It accepts a release only after:

1. manifest schema validation
2. detached signature verification over the exact downloaded manifest bytes
3. target and version-policy match
4. archive byte-size validation
5. archive SHA-256 validation
6. safe archive-layout and executable-size validation

Failure leaves the running executable untouched.

The signed manifest is the single updater trust document. It binds artifact
size and SHA-256. Key rotation requires an overlap release; downgrade is
rejected; an equal version is no update.

### 15.2 Ownership

An installation receipt is required for mutation. Its install method is always
`direct`; it also includes the absolute executable path, version, target, and
manifest identity. Package-manager detection never creates or adopts a
receipt.

The updater compares the current executable to the receipt before replacement.
Known package-manager paths are advisory only.

The direct installers own initial receipt creation. They verify the same
manifest and archive contract, install atomically, smoke-test the version, then
write the receipt. They do not edit shell profiles or PATH.

### 15.3 Replacement

Download to the destination filesystem, verify, set executable permissions,
and atomically replace. Platform-specific replacement behavior remains behind
the updater module.

Windows replacement uses the verified new executable as a narrowly scoped
post-parent helper. Receipt state advances only after replacement and a version
smoke test. There is no installed-version collection or automatic rollback.

## 16. Documentation and generated contracts

The Rust domain and CLI definitions own:

- JSON Schemas
- command reference
- error and exit-code reference
- MCP tool schemas
- TUI shortcut reference
- update manifest, signature, state, and receipt schemas
- embedded skill inputs

The feature-gated generator writes committed artifacts. CI runs it and fails if
the working tree changes.

Fumadocs consumes the committed artifacts. It MUST NOT invoke Rust code at
request time.

`contracts/output.schema.json` is allowed to exceed 1,000 generated lines
because one schema ID owns the discriminated command-envelope union and shared
domain definitions. Splitting it would weaken direct validation and create
cross-file resolver requirements. The Rust source types and generator remain
subject to normal module-size limits. ADR 0008 covers the same exception during
the specification-seed bootstrap.

## 17. Dependency baseline

Phase 0 revalidates and pins exact versions only for the roles used by the
network-free scaffold: Clap, Serde and JSON, Schemars, Jiff, thiserror, UUIDv7,
SHA-256, JSON Schema validation, property tests, and temporary directories.
Each later phase revalidates and pins its newly introduced roles before first
use. Future roles such as Tokio, Reqwest, RMCP, Ratatui, SQLite, and Ed25519 do
not become dependencies before their owning phase.

RMCP must support MCP `2026-07-28` and the official conformance suite before it
enters Phase 6. Its dependency-compatible Rust version becomes the package
`rust-version` in that phase. The 2026-07-28-capable RMCP 3 prerelease requires
Rust 1.88, but a prerelease is evidence, not an approved dependency pin.

Phase 2 applies the same dependency-driven `rust-version` rule to the locked v1
TOON projection. `toon-format 0.5.0` requires Rust 1.87: its decode parser uses
`unsigned_is_multiple_of` (stabilized in Rust 1.87.0) and fails to compile on
Rust 1.85 with `E0658`. Because TOON is part of the locked projection contract
and the lowest selected direct-dependency-compatible toolchain becomes the
package `rust-version`, Phase 2 raises `rust-version` from 1.85 to 1.87 (not to
the RMCP 1.88 prerelease floor, which is not yet a selected dependency).

Later tests add real Axum loopback servers, snapshots, PTYs, and the pinned
Terminal Control harness when the corresponding behavior exists. Exact
research snapshots live in [evidence-ledger.md](evidence-ledger.md), not in
this normative architecture.

The workspace pins current stable Rust for development and records the lowest
dependency-compatible Rust 2024 toolchain as `rust-version`. Each phase MUST
prove both the current stable toolchain and the pinned `rust-version`
toolchain before its manifest is accepted. Direct dependency upgrades are
intentional changes.

## 18. Documentation workspace baseline

Phase 8 revalidates and pins one compatible Fumadocs Core, Fumadocs MDX,
`@fumadocs/base-ui`, Next.js, React, and TypeScript set. The production build,
not the prior research snapshot, accepts the set.

The repository package manager is selected by the committed docs lockfile and
MUST NOT be mixed.

## 19. Observability

Use structured tracing with a default sanitized filter.

Useful fields:

- command or MCP tool
- local analysis or bulk ID
- check kind
- normalized and upstream stage
- attempt number for safe GET retries
- elapsed milliseconds
- HTTP status
- history operation
- update target and version

Excluded fields:

- API keys and auth headers
- submitted content
- segment and plagiarism match text
- raw response bodies

No telemetry leaves the process.

## 20. Security boundaries

| Boundary | Control |
| --- | --- |
| Credential persistence | restrictive permissions, environment override |
| Pangram POST | no ambiguous automatic replay |
| Terminal | control-character sanitization and restoration guard |
| MCP filesystem | versioned roots and handle-relative no-follow access |
| MCP mutations | immutable startup gates |
| History | disabled default, explicit plaintext warning |
| Public links | per-request opt-in |
| Updates | signed manifest, hashes, receipt ownership |
| External URLs | explicit open only |
| Markdown | structural escaping |
| SQLite | parameterized queries and transactions |

## 21. Complexity controls

Before adding an abstraction, answer:

1. Which current duplication or invariant does it own?
2. Which callers become simpler?
3. What behavior is impossible to express without it?
4. Can a cohesive concrete module solve the same problem?

Reject:

- endpoint-per-trait designs
- repositories over one SQLite backend
- factories for one implementation
- adapter-specific result types
- duplicate state caches without a synchronization owner
- validators repeated after an invariant boundary

Files approaching 800 lines receive decomposition review. A file over 1,000
lines blocks merge unless an ADR explains why cohesion outweighs navigation
cost.

## 22. Implementation sequence

The [implementation roadmap](implementation-roadmap.md) is the sole owner of
phase order and PR boundaries. This architecture specifies dependencies, not a
second sequence: contracts precede implementation, the text-analysis vertical
slice precedes secondary workflows, and an adapter exposes only operations the
shared analysis module already implements.
