# Pangram CLI MCP contract

Status: approved for implementation
Transport: stdio

This file is the contract owner for the MCP interface. Input schemas set
`additionalProperties: false`.

## Protocol

The shipping server implements MCP `2026-07-28` over stdio with RMCP exactly
`3.1.2`. It supports `server/discover` and requires the protocol version and
client capabilities in every request `_meta`. Client information is optional.
The server does not implement the removed
`initialize`/`notifications/initialized` lifecycle, negotiate an older
protocol version, or expose the experimental Tasks extension.

Every ordinary success and tool execution failure includes
`resultType: "complete"`. Tool-list, resource-list, and resource-read results
also include `ttlMs: 0` and `cacheScope: "private"`. Their ordering and bytes
are deterministic. The v1 server does not open a subscription stream because
its tool and resource inventories are immutable for the server lifetime.

Invalid startup capability configuration or file roots fail before the server
reads stdin. Startup failure writes no stdout, writes exactly one sanitized
diagnostic line to stderr, and exits 1.

## Phase-owned tool inventory

The generated inventory is ordered and immutable for one server process. Phase
6 advertises only operations whose shared analysis and persistence behavior is
already implemented:

| Tool | Required input | Optional input |
| --- | --- | --- |
| `detect_text` | `text`, `max_billable_units` | `save`, `public_link`, `include_input` |
| `check_plagiarism` | `text`, `max_billable_units` | `save`, `include_input` |
| `analyze_text` | `text`, `max_billable_units` | `save`, `public_link`, `include_input` |
| `get_task` | exactly one of `analysis_id`, `upstream_task_id` | none |
| `wait_task` | exactly one of `analysis_id`, `upstream_task_id` | `timeout_ms` |
| `submit_bulk` | exactly one of `items`, `jsonl_path`; `max_billable_units` | `save` |
| `get_bulk` | exactly one of `bulk_id`, `upstream_bulk_id` | none |
| `wait_bulk` | exactly one of `bulk_id`, `upstream_bulk_id` | `timeout_ms` |
| `get_bulk_results` | exactly one of `bulk_id`, `upstream_bulk_id`; `offset`, `limit` | none |
| `check_update` | none | none |

The gated history and configuration tools in this contract join that inventory
only when their startup gate is enabled.

Phase 7 advertises inline-text `check_plagiarism` and `analyze_text` after live
conformance resolved their upstream contracts. It does not advertise
`detect_files`: MCP billable tools require a pre-submission ceiling, and
Pangram publishes no estimator for binary documents before server extraction.
Phase 8 advertises the nonbillable `check_update` tool. `0.x` builds
perform no update-network access and return the canonical typed
`update_unavailable` failure. Public builds run only an explicit check and do
not install. The server has no alias, compatibility shim, hidden tool, or
rejection-only placeholder for a later-phase operation.

Every billable text submission requires positive `max_billable_units`. The
server estimates locally and rejects the call before submission when the
estimate is above the ceiling. Pangram 4 text estimates use one unit per
started 100-word block, with a minimum of one.
`check_plagiarism` uses the published fixed 5-unit plagiarism estimate.
`analyze_text` sums those 5 units with the AI-detection text estimate.

## Task tools

`get_task` and `wait_task` are ordinary Pangram tools, not the experimental
`io.modelcontextprotocol/tasks` extension. The server does not expose
`tasks/get`, `tasks/update`, or `tasks/cancel`, and it does not generate or own
an MCP Task schema. `wait_task.timeout_ms`, when present, is positive.

An `upstream_task_id` resolves through Pangram without history. An
`analysis_id` resolves only through the concrete history store and therefore
requires `--history`. The MCP adapter has no transient or hidden task ledger.

## Bulk tools

`submit_bulk` requires exactly one of inline `items` or `jsonl_path`, plus a
positive `max_billable_units`. Pangram's Bulk API does not document a public
dashboard link, so no bulk tool accepts `public_link`.

The request uses one job-wide JSON `model` field set to `pangram-4`, with no
per-item selector. Each valid item costs one unit per started 100-word block,
with a minimum of one. The job estimate is their sum. The input schema and
preflight enforce Pangram's 1,000-unit request limit as well as the smaller
caller-supplied `max_billable_units` ceiling.

Inline items work without an approved file root. A `jsonl_path` must be inside
one of the directories approved with `--allow-file-root`. With no approved
root, a `jsonl_path` call is rejected before submission.

`get_bulk`, `wait_bulk`, and `get_bulk_results` accept either the local
`bulk_id` or the upstream `upstream_bulk_id`. An upstream ID works without
history. A local ID resolves only through history and therefore requires
`--history`. `wait_bulk.timeout_ms`, when present, is positive.
`get_bulk_results` requires explicit `offset` and `limit`; `limit` is from 1
through 1,000.

## History and configuration tools

Read tools, exposed only with `--history`:

- `history_list`
- `history_search`
- `history_get`

`history_get` omits content unless `include_content: true`. Its
`structuredContent` is the canonical `history_show` success or failure
envelope, not a `history_get` command variant.

Mutation tools, exposed only with both `--history` and
`--allow-history-mutations`:

- `history_rerun`
- `history_delete`
- `history_clear`

`save: true` on `detect_text`, `check_plagiarism`, `analyze_text`, or
`submit_bulk` is also a history mutation. It
requires both `--history` and `--allow-history-mutations`. Omitting `save`, or
using `save: false`, performs no history write.

Configuration mutation, exposed only with `--allow-config-mutations`:

- `update_config`

`update_config` rejects credential, endpoint, public-link, and unknown keys.
`--allow-history-mutations` without `--history` is invalid startup
configuration.

## Public links

Any tool request with `public_link: true` fails with
`mcp_capability_required` unless the server started with
`--allow-public-links`.

## Tool annotations

| Tool class | readOnly | destructive | idempotent | openWorld |
| --- | --- | --- | --- | --- |
| Billable analysis submission | false | false | false | true |
| Upstream status/wait/results | true | false | true | true |
| Explicit update check | true | false | true | true |
| Local history reads | true | false | true | false |
| Local rerun | false | false | false | true |
| Local delete/clear | false | true | false | false |
| Configuration update | false | false | false | false |

Annotations are hints, not authorization. Startup capability gates remain
authoritative.

## Results and errors

Each Phase 6 tool maps to one canonical command:

| Tool | Canonical command |
| --- | --- |
| `detect_text` | `detect` |
| `check_plagiarism` | `plagiarism` |
| `analyze_text` | `analyze` |
| `get_task`, `wait_task` | `task_status`, `task_wait` |
| `submit_bulk`, `get_bulk`, `wait_bulk`, `get_bulk_results` | `bulk_submit`, `bulk_status`, `bulk_wait`, `bulk_results` |
| `check_update` | `update_check` |
| `history_list`, `history_search`, `history_rerun`, `history_delete`, `history_clear` | matching `history_*` command |
| `history_get` | `history_show` |
| `update_config` | `config_set` |

Successful tools return:

- `resultType: "complete"`
- exactly one schema-valid canonical command success envelope in
  `structuredContent`
- a concise text summary in text content

Malformed arguments use MCP validation errors. Domain failures return
`isError: true`, `resultType: "complete"`, and exactly one canonical command
failure envelope in `structuredContent`. `structuredContent` never contains a
bare analysis or bare error object. The generated tool inventory closes each
input schema and specializes each output schema to its command constant and
corresponding `data` root.

## Cancellation

JSON-RPC cancellation of an active call stops local observation only. It does
not send or claim an upstream cancellation. After cancellation, the server
sends no JSON-RPC response for the cancelled request. It may write one
sanitized diagnostic to stderr; when identifiers are known, that diagnostic
identifies the local analysis or bulk ID and upstream task or bulk ID without
including submitted content.

## File roots and file opening

Capability flags and approved file roots are immutable for the server lifetime.
`--allow-file-root PATH` is repeatable. Each value must be an absolute, existing
directory. The server validates and pre-opens every configured root with the
required permissions and no-follow guarantees before reading stdin.

For `jsonl_path`, the server selects the matching pre-opened root and opens
only relative path segments beneath that directory handle. File access is
root-relative and handle-based, rejects symlink and reparse-point traversal,
and verifies the opened object rather than a pathname checked earlier. A path
is never authorized by canonicalizing a string and reopening it later.

Installers never add file roots or optional capability flags automatically.

## Resources and generated ownership

Static resources return the exact embedded build-time bytes and MIME types:

| URI | Embedded owner | MIME type |
| --- | --- | --- |
| `pangram://schema/output/v1` | `contracts/output.schema.json` | `application/schema+json` |
| `pangram://schema/errors/v1` | `generated/error-reference.json` | `application/json` |
| `pangram://skills/pangram` | `skills/pangram/SKILL.md` | `text/markdown` |

There are no MCP prompts or history resources.

The Rust-owned generator commits:

- `generated/mcp-tools.json`, the ordered immutable tool descriptors with
  closed input schemas and command-specialized output schemas
- `generated/agent-reference.md`, the compact embedded agent reference

Per-tool schemas live inside `generated/mcp-tools.json`; the generator does not
create separate schema files for each tool. `skills/pangram/SKILL.md` and the
two generated files are embedded in the binary at build time. Runtime startup
does not read them from disk.

The adjacent CLI guidance surface uses those exact embedded bytes:

- `pangram agent` and `pangram skills get pangram` emit
  `generated/agent-reference.md`
- `pangram skills get pangram --full` emits `skills/pangram/SKILL.md`
- `pangram skills list` emits ``# Embedded skills\n\n- `pangram`\n``
- `pangram skills path` emits `embedded://skills\n`
- `pangram skills path pangram` emits
  `embedded://skills/pangram/SKILL.md\n`

These are byte-exact raw stdout surfaces. The generator owns their mapping and
newline termination. Both Markdown owner files end with one newline.

## Client install and uninstall

An install or uninstall command requires one or more explicit selected targets
or `--all`. `--dry-run` reports the exact planned changes and writes nothing.
The operation owns only the exact selected server name, preserves unrelated
fields and entries, and preserves untouched bytes when the client format
permits it.

Install rejects a malformed configuration, a conflicting existing entry, or
an entry with the selected name that Pangram CLI does not own. Uninstall
removes only an exact matching entry previously owned by Pangram CLI; it does
not remove a same-named entry with different command or arguments. Installers
do not add file roots or optional capability flags by default.

The command preflights every selected target before its first write. An
unavailable target, malformed file, ambiguous location, ownership conflict, or
unsafe plan returns one canonical failure envelope and writes no target. This
is a preflighted multi-target plan, so a preflight failure has no partial
success to preserve.

A successful dry run and a successful normal operation return the same closed
`McpMutationReport` data shape:

```json
{
  "dry_run": true,
  "targets": [
    {
      "client": "windsurf",
      "path": "/home/example/.config/devin/mcp_config.json",
      "action": "create"
    }
  ]
}
```

`dry_run` records whether writes were suppressed. `targets` follows explicit
`--target` order after duplicate targets keep their first position; `--all`
uses the generated client inventory order. Every successful target record has
the closed client identifier, its exact resolved absolute path, a closed action
of `create`, `update`, `remove`, or `unchanged`, and optional sanitized
`reason`. Install uses `create`, `update`, or `unchanged`; uninstall uses
`remove` or `unchanged`. A normal operation returns the report only after every
planned atomic write succeeds. If an unexpected write failure occurs after an
earlier per-target atomic write commits, the canonical failure preserves that
change. Its sanitized recovery message identifies the successfully changed
client and path records, and it does not claim that later targets changed.
`unavailable` is not a report action because unavailable targets fail preflight
and produce no success report.

`pangram mcp status` remains a read-only `McpStatus` payload. It does not use
`McpMutationReport`.

Each client target remains disabled until its current configuration path,
schema, and exact owned-entry match rule are pinned from authoritative client
evidence. The common invariants above do not guess those client-specific
details.

### Client-specific target rules

`windsurf` means the current Windsurf editor integration backed by Devin Local.
It does not mean the legacy Cascade configuration, and the installer MUST NOT
read or edit a Cascade MCP file as a compatibility path. Devin Local uses:

- Linux and macOS: `~/.config/devin/mcp_config.json`
- Windows: `%APPDATA%\devin\mcp_config.json`

The file is JSON with comments. Under the default server name, the owned entry
is `mcpServers.pangram`, with `command` set to the absolute `pangram`
executable path and `args` exactly `["mcp"]`. A custom server name changes only
the key beneath `mcpServers`.

Paths beginning with `~` or `%APPDATA%` are resolved to absolute paths before
preflight and before they enter `McpMutationReport`.

The `claude-desktop` Linux path is
`${XDG_CONFIG_HOME:-$HOME/.config}/Claude/claude_desktop_config.json`. The
installer resolves the environment expression to an absolute path before it
constructs the mutation report.

`roo-code` may use custom or host-managed storage. The installer proceeds only
when exactly one supported configuration path is discoverable and its schema
matches the pinned target contract. No match is unavailable; more than one
match is ambiguous. Either condition fails the full preflight with zero writes.
The installer MUST NOT select a location by precedence or edit every candidate.

## Conformance proof

Product transport tests spawn the compiled `pangram mcp` process and exercise
its real stdio framing, startup failure behavior, cancellation, tool calls,
resources, stderr separation, and shutdown.

The official `@modelcontextprotocol/conformance` suite is pinned exactly to
`0.2.0-alpha.11` as the applicability reference. It cannot drive stdio while
upstream issue 258 remains open. Its frozen `2026-07-28` server requirements
also assume a full diagnostic server, including named `test_*` tools, prompts,
resource templates, binary resources, completion, SSE streams, DNS-rebinding
checks, sampling, elicitation, roots, and input-required flows. Pangram does
not advertise those optional capabilities.

The official suite therefore has no capability-aware profile that can be
scoped to the shipping Pangram server in this phase. Individual overlapping
scenarios could exercise a shared handler over HTTP, but a test-only server
that adds the full required inventory would prove neither full-profile nor
stdio conformance for Pangram. It MUST NOT be reported as Pangram conformance.
Phase 6 uses the compiled stdio subprocess suite as its product protocol proof.
Re-evaluate the pinned official suite when it can drive stdio and select
scenarios from the server's advertised capabilities. HTTP transport and every
conformance-only `test_*` tool or resource remain absent from normal builds and
release artifacts.
