# Pangram CLI product specification

Status: approved for implementation
Decision date: 2026-07-23
Public product destination: `https://github.com/Microck/pangram-cli`

## 1. Product statement

Pangram CLI is a terminal client for Pangram AI detection and plagiarism
checking. It serves three interaction modes through one behavioral
core:

1. a JSON-first command-line interface for scripts and shell pipelines
2. an interactive terminal user interface for people
3. a typed stdio MCP server for AI agents

AI detection is the primary workflow. Plagiarism is secondary and opt-in.

The product targets parity with analysis capabilities exposed through Pangram's
documented APIs. It does not target undocumented dashboard administration or
browser-only behavior.

## 2. Product principles

### 2.1 One behavior, three adapters

CLI, TUI, and MCP MUST expose the same analysis semantics. An adapter may
change presentation or interaction, but it MUST NOT invent a separate request,
normalization, retry, billing, or persistence path.

### 2.2 Agent-native, not agent-tolerant

Automation MUST have stable structured output, deterministic errors, explicit
side-effect gates, and machine-readable schemas. Agents MUST NOT need to scrape
terminal decoration or shell out through a generic MCP escape hatch.

### 2.3 Human-friendly without changing defaults

The TUI and `--format pretty` provide human presentation. JSON remains the
canonical noninteractive output. Configuration MUST NOT silently change that
default.

### 2.4 Privacy through explicit retention

Local history and public Pangram dashboard links are disabled by default.
Submitting content is inherent to the service, but retaining or publishing it
requires a separate choice.

### 2.5 Evidence, not verdict

Detection output MUST retain Pangram's classifications, fractions, confidence,
and explanatory text. Product copy MUST describe the result as probabilistic
evidence and MUST NOT reduce it to a boolean accusation.

### 2.6 Documented interfaces only

The product MUST use API-key-authenticated documented REST endpoints. It MUST
NOT use session cookies, private dashboard routes, browser automation,
extension protocols, or scraping.

## 3. Users and primary outcomes

| User | Primary outcome |
| --- | --- |
| Terminal user | Submit text or documents and understand the result |
| Shell author | Compose Pangram analysis with pipes and stable JSON |
| AI agent | Call typed tools with bounded cost and explicit permissions |
| Reviewer | Inspect, search, export, and rerun optional local history |
| Maintainer | Release signed native artifacts from one versioned contract |

The first successful human outcome is an AI detection result in the TUI. The
first successful automation outcome is a JSON AI detection result from stdin.
The first successful agent outcome is one `detect_text` MCP tool call.

## 4. Domain language

### Analysis

An `Analysis` is the local umbrella record for one input and one or more
ordered checks. Every analysis receives a local opaque identifier, even when
history is disabled.

### Check

A `Check` is one of:

- `ai_detection`
- `plagiarism`

AI detection is first in presentation and ordering.

### Input

An input is either:

- `TextInput`: UTF-8 text provided literally, through stdin, or from a text file
- `FileInput`: a PDF, DOCX, or RTF document sent to Pangram's file endpoint

Multiple files produce independent analyses.

### Segment

`Segment` is the public term for Pangram response objects called `windows`.
Segments remain in source order and retain text, label, AI-assistance score,
confidence, upstream indices, word count, and token length.

### Bulk collection

A `BulkCollection` is a local grouping of ordinary AI-detection analyses
submitted through Pangram's asynchronous bulk API. It is not a different type
of analysis result.

### Local and upstream identity

Local analysis and bulk identifiers are distinct from Pangram task and bulk
identifiers. Presentation MUST label each identity rather than calling every
identifier a task ID.

## 5. Analysis capabilities

### 5.1 AI detection

AI detection MUST preserve:

- overall short classification: AI, human, or mixed
- Pangram headline and long prediction
- AI, AI-assisted, and human fractions
- AI, AI-assisted, and human segment counts
- ordered segment evidence
- segment humanizer score and thresholded humanized decision
- Pangram's upstream version value
- upstream task identity and timing
- public dashboard link when explicitly requested

Unknown upstream stages, classifications, confidence values, or required field
shapes MUST fail with `upstream_contract_changed`.

Pangram 4 is the only production text model. The product does not expose model
selection and does not retain a Pangram 3 compatibility path. Requests MUST
include Pangram's documented Pangram 4 selector, JSON request field `model`
set to `pangram-4`, rather than omitting `model` or relying on temporary
default routing that Pangram retires on 2026-09-30.

Pangram 4 is intended for long-form natural-language writing of at least 50
words. The product documents that limitation and may warn for shorter input,
but it does not reject an otherwise valid request unless the upstream contract
defines a hard minimum.

### 5.2 Plagiarism

Plagiarism MUST preserve:

- whether Pangram reports plagiarism
- plagiarized percentage
- total and plagiarized sentence counts
- source URLs
- matched text
- similarity scores

The current official API reference defines `plagiarized_sentences` as an
integer count. The implementation follows that contract. A live authorized
response must still confirm the shape before public conformance.

### 5.3 Combined analysis

Combined analysis runs AI detection and plagiarism concurrently and combines
the results locally. It MUST be described as a Pangram CLI combined report,
not as a Pangram dashboard report.

If one check succeeds and the other fails, the analysis is `partial` and
retains the successful result.

Combined analysis always waits. It does not support detach because Pangram's
plagiarism endpoint is synchronous and the product has no daemon.

### 5.4 Bulk AI detection

Bulk supports AI detection only. It MUST NOT implement client-side bulk
plagiarism fanout.

Submission accepts items with optional caller-provided IDs. The product MUST:

- preserve input order
- validate the entire local JSONL input before submission
- estimate billable units before submission
- require a maximum billable-unit ceiling
- reject requests above Pangram's documented current limit
- page upstream item and result endpoints at no more than 1,000 entries
- expose queued, running, succeeded, failed, and partial state
- warn that upstream bulk metadata and results expire 48 hours after terminal
  completion

The Pangram SDK documents Pangram 4 bulk selection as one job-wide JSON
`model` field with the exact value `pangram-4`; per-item selectors are not
supported. Each valid item costs one unit per started 100-word block, with a
minimum of one unit per item. A job costs the sum of its item units and cannot
exceed 1,000 billable units. There is no separate item-count limit, but normal
request-body limits still apply. The 1,000-entry page limit is a separate
response-pagination contract.

### 5.5 Files

UTF-8 text files use the text endpoint and support every applicable check.
PDF, DOCX, and RTF files use the file endpoint for AI detection.

Binary document implementation remains behind live conformance because the
current official sources conflict. The Mintlify API reference documents an
array containing only `public_dashboard_link`, while the Pangram SDK v1.0.0
README documents extracted text, prediction fields, windows, `filename`, and
an optional `dashboard_link`.

The product MUST NOT invent client-side PDF, DOCX, or RTF text extraction as a
fallback. Plagiarism and combined analysis for binary documents remain blocked
until Pangram documents or confirms a supported contract.

## 6. Status model

Normalized analysis and check states are:

- `queued`
- `running`
- `succeeded`
- `failed`
- `partial`

Parent status is derived in this order:

| Child state | Parent state |
| --- | --- |
| Any check is `running` | `running` |
| Otherwise, any check is `queued` | `queued` |
| Every check succeeded | `succeeded` |
| Mixed terminal success and failure | `partial` |
| Every check failed | `failed` |

Exact Pangram stages remain available as diagnostic provenance but MUST NOT
become the public state machine.

Interrupting a wait stops local observation only. It does not cancel Pangram
work. The last known remote state and identifiers remain available to the
caller.

## 7. CLI behavior

The normative command grammar is in [contracts.md](contracts.md).

Product-level rules:

- Bare `pangram` launches the TUI only when stdin, stdout, and stderr are TTYs.
- Bare `pangram` with provided text or piped stdin runs AI detection.
- `detect` runs AI detection only.
- `plagiarism` runs plagiarism only.
- `analyze` always runs both checks.
- `bulk` submits and inspects asynchronous AI-detection collections.
- JSON is the default noninteractive representation. Repeated files default to
  JSONL unless the caller selects a format.
- stdout contains primary output only.
- stderr contains progress, warnings, diagnostics, and errors.
- unsupported flag combinations fail before a billable request.
- commands MUST NOT prompt when stdin, stdout, or stderr indicates
  noninteractive execution.

Supported projections are JSON, JSONL, TOON, Markdown, and pretty terminal
output. CSV is not supported.

## 8. TUI behavior

### 8.1 Application structure

The top-level navigation is:

```text
Analyze | Active | History | Settings
```

`Analyze` is the default destination.

At 120 columns or wider, the application uses three stable areas:

1. a compact route rail with the resolved fox mark and top-level navigation
2. a dominant center workspace for input, progress, and results
3. a right inspector for checks, privacy, cost, filters, and contextual actions

A restrained bottom command bar shows only actions valid for the current route
and focus. The resolved fox shrinks after the intro and MUST NOT consume
persistent workspace. Sparse separators define structure; the interface MUST
NOT box every value into dashboard-style cards.

At 80 to 119 columns, the route rail becomes top tabs and the inspector moves
into the center flow below its primary content. Below 80x24, a resize overlay
preserves application state.

### 8.2 Analyze workflow

The Analyze screen contains:

- check selector: AI detection, plagiarism, or both
- input selector: text or files
- multiline text composer
- validated file path field and pre-submission file queue
- public-link toggle, unchecked for each submission
- manual-save toggle, unchecked unless the user selects it
- estimated word and billable-unit summary
- submit control

After submission, the composer collapses to an input summary and progress or
result takes over the center workspace. The inspector retains status, evidence
controls, and valid actions. `New analysis` restores the composer.

### 8.3 Result hierarchy

Results appear in this order:

1. overall classification
2. AI, AI-assisted, and human fractions
3. ordered segment evidence
4. plagiarism evidence
5. provenance and task identity
6. actions

Wide terminals show synchronized inline highlighting and a segment list.
Narrow terminals show the list only. Color supplements text labels and MUST
NOT carry meaning alone.

### 8.4 Active and history

Active shows in-session operations and saved unfinished analyses. Ephemeral
analyses disappear after process exit.

History provides search, filters, detail, rerun, export, delete, and save-state
information. It MUST always identify itself as local Pangram CLI history.

### 8.5 Settings

Settings contains:

- authentication
- history
- intro frequency
- keymap
- motion
- updates
- diagnostics

There is no theme marketplace, project profile, or endpoint configuration.

### 8.6 Keymaps

Regular mode is the default. It uses arrows, Tab, Shift+Tab, Enter, Escape,
Home, End, PageUp, PageDown, `?`, and contextual `/`.

Vim mode adds `h`, `j`, `k`, `l`, `gg`, `G`, `Ctrl+u`, `Ctrl+d`, `n`, and
`N`. It does not create a modal Vim editor. Printable keys always behave
normally in an active text field.

Destructive actions require an action menu and confirmation. A single `d`
keypress MUST NOT delete data.

Mouse input may supplement keyboard input but MUST NOT be required.

### 8.7 Intro

The intro is a terminal-native recreation of the supplied Pangram fox-mark
motion reference. It uses precomputed terminal-cell frames, not bundled video,
an image decoder, or Droid frame data.

The default `tui.intro = "once"` plays the intro on the first eligible
full-motion launch. `always` replays it on every eligible launch, and `off`
opens Analyze immediately. Intro frequency and motion level are separate
settings.

The full-motion sequence is exactly 56 frames at 20 frames per second for a
nominal 2.8 seconds:

1. from 0 to 800 ms, the fox mark settles from 90 percent scale, 24 source
   pixels low, and 2.2 degrees counterclockwise
2. from 150 to 700 ms, the orange center unfolds upward from a bottom pivot
3. from 280 to 920 ms, the pink facets unfold outward with a 60 ms stagger
4. from 920 to 2,380 ms, the resolved mark holds without looping
5. from 2,380 to 2,800 ms, the mark dissolves by decreasing terminal-cell
   density and reveals Analyze

Transform phases use `cubic-bezier(0.175, 0.885, 0.32, 1.1)`. The final
dissolve is linear.

The terminal renderer uses a 32x16 full mark and a 20x10 compact mark. It uses
Pangram orange `#FF6106` and pink `#FECAB9` when truecolor is available,
nearest ANSI colors otherwise, and density alone when color is disabled.
Terminals without the required glyph support use `#`, `*`, `.`, and spaces.

Frame selection derives from monotonic elapsed time in 50 ms steps. A delayed
render skips stale frames instead of extending the sequence.

The intro MUST NOT display fake classifications, scores, or analysis progress.
Escape, Enter, or Space skips it and the skip key is consumed.

Reduced mode performs no timed animation. It places the resolved fox mark in
the first interactive Analyze render, which is already usable, then uses the
normal application header on the next input or state change. Motion off skips
directly to Analyze.
The intro is also suppressed outside an interactive TTY, under `CI`, with
`TERM=dumb`, or until the terminal reaches 80x24.

The supplied media is a design reference, not a runtime asset. Public source
or release artifacts MUST include the fox artwork or derived frames only after
written Pangram permission covers logo and trademark use.

The normative source-art, provenance, rights, and visual acceptance requirements
are in [intro-art-contract.md](intro-art-contract.md). Missing source geometry
blocks frame generation, not the core CLI, TUI reducer, or terminal lifecycle.

### 8.8 First launch

First launch is one reducer-owned Analyze route with onboarding substates:

1. full motion, when enabled, completes before the Analyze route
2. Analyze opens with an optional masked API-key overlay
3. the update-check preference overlay follows using `[Y/n]`
4. onboarding completes and Analyze remains usable

Reduced motion places the resolved mark in the first usable Analyze frame
behind the onboarding overlay. The next input or state change replaces it with
the normal header. Automatic update checking starts only after the preference
is resolved, onboarding is complete, and one usable Analyze frame has rendered.

API-key setup is skippable. Unauthenticated users retain access to settings,
history, help, and update checks.

### 8.9 Terminal integrity

The TUI uses the alternate screen and MUST restore the terminal, cursor, and
input mode on normal return, handled I/O error, Ctrl+C, supported termination
signals, and unwind panic. Restoration is idempotent. Guarded code MUST NOT call
`process::exit`; it returns an exit intent to `main`. Uncatchable termination
and process abort are explicitly outside the guarantee. This is release
blocking.

Upstream text is untrusted. Control sequences MUST be escaped before terminal
rendering.

## 9. Authentication

Authentication commands are:

- `pangram auth`
- `pangram auth set --api-key VALUE`
- `pangram auth set --api-key-stdin`
- `pangram auth status`
- `pangram auth logout`

The guided flow stores a key locally without a billable validation request.
The first real analysis validates it.

Credential precedence is:

```text
PANGRAM_API_KEY > stored API key
```

Persistent setup is preferred for local agents. The environment variable is
the ephemeral CI alternative.

Persistent credentials live in a dedicated protected file that
`PANGRAM_CONFIG` cannot relocate. They never appear in general configuration,
configuration output, or generated configuration examples.

Passing `--api-key VALUE` is intentionally supported for agent usability even
though argv and shell history can expose it. Help and documentation MUST warn
about that risk.

There is no `auth login` or `auth check`. Pangram documents no safe,
non-billable authentication validation endpoint.

## 10. Local history

### 10.1 Default and retention

History is disabled by default. Enabling it displays a warning that full input
and result content will be stored locally in plaintext.

Records remain until explicit deletion. Disabling history stops future
automatic writes but does not hide or delete existing records.

The data directory, database, WAL, and shared-memory files require owner-only
access. History fails closed before opening SQLite when that protection cannot
be established.

### 10.2 Saved states

An analysis reports one of:

- `ephemeral`
- `saved_manual`
- `saved_history`

Manual save is available even when automatic history is disabled.

### 10.3 Stored data

Saved analyses may contain:

- local and upstream identifiers
- input text or extracted text returned by Pangram
- filename, original path, size, and SHA-256
- canonical normalized results
- check and parent state
- timestamps and timing
- retry and rerun lineage
- bulk relationship

The product does not copy original uploaded binary files.

### 10.4 Search and export

History supports full-text search over stored input text, extracted text,
filenames, result headlines, and plagiarism source URLs.

Export supports JSONL and Markdown. JSONL is the default and includes content
unless `--redact-content` is supplied. Import and synchronization are out of
scope for v1.

### 10.5 Failures

Automatic history write failure produces a warning and does not turn a
successful remote analysis into failure. Failure of explicit `--save` or a
history command is a canonical local error.

Delete and clear remove logical access through Pangram CLI and truncate the
active WAL. They are not a forensic secure-erasure guarantee for backups,
snapshots, storage media, or prior copies.

Corrupt databases are preserved. The product MUST NOT silently wipe, replace,
or recreate them.

## 11. Privacy and safety

### 11.1 Public dashboard links

Public links are off for every request and cannot be configured on globally.
Existing links open only after explicit user action.

### 11.2 Logging

Default logs MUST exclude:

- API keys and authentication headers
- full submitted content
- segment text
- plagiarism matches

Normal output omits full input unless `--include-input` is requested. History
and explicit export follow their separate retention contracts.

### 11.3 Telemetry

The runtime and documentation site include no telemetry or analytics in v1.

### 11.4 URLs and terminal content

Plagiarism source URLs and dashboard links are untrusted external destinations.
The product does not fetch them and opens them only after explicit action.

Pretty and TUI renderers sanitize control characters. Markdown exports escape
content so it cannot restructure the report unexpectedly.

## 12. Billing and network behavior

Pangram's documented research snapshot states:

- realtime detection limit: 5 requests per second
- Pangram 4 text API price: USD 0.05 per started 100 words
- each valid Pangram 4 text request costs at least one unit

The Pangram SDK v1.0.0 tag documents the exact text and job-wide bulk
model-selection request field (`model` set to `pangram-4`). Pangram's official
API reference defines each valid Pangram 4 bulk item as one billable unit per
started 100-word block, with a minimum of one, and caps a request at 1,000
units. The product MUST NOT reuse the Pangram 3 default or its 1,000-word
estimator. Text and bulk submission send the explicit selector. Bulk preflight
sums item units and enforces both the caller's ceiling and Pangram's limit.

Pricing is documentation, not a hard-coded monetary promise. The product
estimates word counts and units but MUST NOT report an exact charged amount
unless Pangram returns one authoritatively.

The client enforces a 5 QPS ceiling and permits only lower configured rates.
It honors `Retry-After`.

Automatic retry is limited to safe GET operations. The client MUST NOT replay
an ambiguous POST because doing so may duplicate billable work.

System proxy settings and normal TLS verification are supported. There is no
insecure TLS switch and no production endpoint override.

## 13. Updates

Automatic update checks:

- occur only in the interactive TUI
- run after the first frame
- happen at most once every 24 hours
- never steal focus
- never install without a separate explicit action

CLI, piped, CI, MCP, and agent execution perform no automatic checks.

Self-update is allowed only for an executable matching a direct-install
receipt. Package-manager installations receive the exact manager command and
are never mutated by Pangram CLI.

Before public release, update networking is disabled.

## 14. Agent behavior

The MCP server is stdio-only in v1 and implements MCP `2026-07-28`. It exposes
typed tools for analysis, waiting, bulk retrieval, optional history, and update
checks. It does not implement the experimental Tasks extension, MCP Apps, or a
legacy protocol path.

Default MCP behavior:

- no history tools
- no history writes
- no destructive tools
- no configuration mutation
- no public links
- no file tools without explicit startup-approved roots
- file access only within directories approved by repeated
  `--allow-file-root PATH`
- no API key arguments

History, history mutation, configuration mutation, public links, and file roots
require separate server flags. Installers do not enable any optional capability
or approve file roots automatically.

MCP installation is explicit. Authentication and TUI onboarding do not prompt
users to install it.

The embedded skill tells agents to:

- prefer AI detection unless plagiarism is needed
- set a bulk billing ceiling
- avoid public links and retention unless requested
- use canonical structured output
- treat mutations as explicit user actions

The complete MCP tool contract is in [mcp-contract.md](mcp-contract.md).

## 15. Supported and unsupported parity

| Pangram capability | Product status |
| --- | --- |
| Single text AI detection | Blocked pending Phase 2 implementation against the documented Pangram 4 text selector |
| AI-assistance fractions, segments, and humanizer evidence | Blocked pending Pangram 4 REST conformance |
| Multilingual model behavior | Blocked pending Pangram 4 REST conformance |
| Text public dashboard link | Blocked with text submission, then opt-in |
| Bulk AI detection | Planned for Phase 3 against the documented Pangram 4 contract; blocked from public support pending implementation and live conformance |
| PDF, DOCX, and RTF AI detection | Blocked pending live response conformance |
| Plagiarism text check | Blocked from public conformance pending field validation |
| Combined AI and plagiarism report | Local composition of supported checks |
| Local history | Local substitution, not Pangram account history |
| Local exports | Local substitution, not Pangram-rendered reports |
| Pangram account history management | Unavailable through documented APIs |
| Remote task cancellation | Unavailable through documented APIs |
| API-key, credit, and billing administration | Unavailable through documented APIs |
| Playback | Unavailable through documented APIs |
| Feed Scanner | Unavailable through documented APIs |
| Browser-extension workflows | Unavailable through documented APIs |
| LMS administration and grading | Unavailable through documented APIs |
| AI image detection | Blocked until Pangram publishes and generally opens a documented Image API |

The public parity page MUST use the categories Supported, Local substitution,
Blocked by upstream contract, and Unavailable through documented APIs.

## 16. Distribution and licensing

The intended public project is open source under the MIT license.

Planned public channels:

- GitHub Releases
- shell installer
- PowerShell installer
- Homebrew
- Scoop
- npm package `@microck/pangram-cli`
- pnpm and Bun through the npm package

The executable name is `pangram`.

Two unrelated or minimal third-party tools already use that executable name.
Namespaced registry packages and clear repository ownership mitigate package
collision while preserving the best command experience.

The README carries the full unofficial-project disclosure. The documentation
site footer and package metadata carry a concise disclosure and link to the
README. Users must not need to discover the repository before learning that the
project is unofficial.

## 17. Public release gates

Public v1 requires:

1. written Pangram confirmation that a third-party CLI and MCP server may use
   the documented APIs and the fox logo in terminal artwork
2. documented Pangram 4 text and job-wide bulk contract (resolved: `model` set
   to `pangram-4`, 100-word per-item billing units, and a 1,000-unit bulk
   request limit)
3. live authorized Pangram 4 text, bulk, and file-response conformance
4. live authorized plagiarism-response conformance
5. passing CLI, TUI, MCP, storage, update, and release contracts
6. artifacts bound by the signed manifest and verified installers for every
   required target
7. complete Fumadocs deployment at `pangram.micr.dev`
8. owned registry and package-manager names
9. accepted generated intro frame art
10. no unresolved P0 or P1 test, security, or maintainability findings

Private `0.x` development milestones perform no public distribution or update
networking.

## 18. Explicit non-goals

v1 does not include:

- a public Rust SDK commitment
- a daemon or background service
- remote HTTP MCP transport
- cancellation of Pangram work
- account, credit, or billing administration
- project profiles
- history import, synchronization, or diffing
- external editor integration
- full terminal file browser
- clipboard integration
- output templates or CSV
- endpoint overrides or insecure TLS
- telemetry
- undocumented web parity
- invitation-only or reverse-engineered image detection
- compatibility shims for pre-release local state

## 19. Remaining blockers

The intro behavior, timing, rendering strategy, and fallbacks are specified.
The agent must establish the generated frame-art baseline against the locked
visual rubric and pass the autonomous acceptance suite before release. The user
reviews only final product quality.

The following are external release blockers rather than unresolved design:

- Pangram distribution permission
- Pangram 4 text and bulk response conformance
- file response conformance
- plagiarism response conformance
