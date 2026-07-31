# Pangram CLI MCP contract

Status: approved for implementation
Transport: stdio

This file is the contract owner for the MCP interface. Input schemas set
`additionalProperties: false`.

## Protocol

The server implements MCP `2026-07-28` over stdio. It supports
`server/discover` and requires the protocol version and client capabilities in
request `_meta`. It does not implement the removed
`initialize`/`notifications/initialized` lifecycle or negotiate an older
protocol version.

All results include `resultType`. Ordinary success and tool execution failure
use `resultType: "complete"`. List and resource-read results use `ttlMs: 0` and
`cacheScope: "private"`; list ordering is deterministic. The v1 server does not
open a subscription stream because its tool and resource inventories are
immutable for the server lifetime.

## Analysis tools

| Tool | Required input | Optional input |
| --- | --- | --- |
| `detect_text` | `text`, `max_billable_units` | `save`, `public_link`, `include_input` |
| `detect_files` | `paths` | `max_billable_units`, `confirm_unknown_cost`, `save`, `public_link`, `include_input` |
| `check_plagiarism` | `text`, `max_billable_units` | `save`, `include_input` |
| `analyze_text` | `text`, `max_billable_units` | `save`, `public_link`, `include_input` |

`paths` is a non-empty array of absolute filesystem paths inside directories
approved with `--allow-file-root`. The server selects the matching pre-opened
root and opens only the relative path beneath that handle. Every billable text
tool requires positive `max_billable_units`. The server estimates locally and
rejects the call before submission when the estimate is above the ceiling.
Pangram 4 text estimates use one unit per started 100-word block, with a
minimum of one.
Binary file cost remains `unknown` until Pangram publishes an authoritative
rule; a binary file call additionally requires `confirm_unknown_cost: true`.

## Task tools

`get_task` and `wait_task` require exactly one:

```json
{"analysis_id":"anl_..."}
```

or:

```json
{"upstream_task_id":"task-123"}
```

`wait_task` optionally accepts positive `timeout_ms`.

These are ordinary Pangram tools, not the experimental
`io.modelcontextprotocol/tasks` extension. The server does not expose
`tasks/get`, `tasks/update`, or `tasks/cancel`, and it does not generate or own
an MCP Task schema.

JSON-RPC cancellation of an active call stops local observation only and does
not claim to cancel Pangram work. When known, the cancellation diagnostic
identifies the local analysis and upstream task without including submitted
content.

## Bulk tools

`submit_bulk` requires:

- exactly one of `items` or `jsonl_path`
- positive `max_billable_units`

The optional field is `save`. Pangram's Bulk API does not document a
public-dashboard-link request or response field, so `submit_bulk` has no
`public_link` input.

The server does not advertise `submit_bulk` until Phase 3 implements it and
live conformance validates the response contract. The request uses one
job-wide JSON `model` field set to `pangram-4`, with no per-item selector. Each
valid item costs one unit per started 100-word block, with a minimum of one;
the job estimate is their sum. The input schema and preflight enforce
Pangram's 1,000-unit request limit as well as the smaller caller-supplied
`max_billable_units` ceiling.

`get_bulk` and `wait_bulk` require exactly one of local `bulk_id` or
`upstream_bulk_id`. `wait_bulk` optionally accepts `timeout_ms`.

`get_bulk_results` also requires explicit `offset` and `limit`. `limit` is from
1 through 1,000.

## History and configuration tools

Read tools, exposed only with `--history`:

- `history_list`
- `history_search`
- `history_get`

`history_get` omits content unless `include_content: true`.

Mutation tools, exposed only with `--allow-history-mutations`:

- `history_rerun`
- `history_delete`
- `history_clear`

`--allow-history-mutations` requires `--history`; an invalid combination fails
server startup. An active in-process analysis ID can resolve without history.
Resolving an ID from persisted state requires `--history`.

Configuration mutation, exposed only with `--allow-config-mutations`:

- `update_config`

`update_config` rejects credential, endpoint, public-link, and unknown keys.

Update check is always safe to expose:

- `check_update`

It checks only and never installs.

## Public links

Any tool request with `public_link: true` fails with
`mcp_capability_required` unless the server started with
`--allow-public-links`.

## Tool annotations

| Tool class | readOnly | destructive | idempotent | openWorld |
| --- | --- | --- | --- | --- |
| Billable analysis submission | false | false | false | true |
| Upstream status/wait/results | true | false | true | true |
| Local history reads | true | false | true | false |
| Local rerun | false | false | false | true |
| Local delete/clear | false | true | false | false |
| Configuration update | false | false | false | false |
| Update check | true | false | true | true |

Annotations are hints, not authorization. Startup capability gates remain
authoritative.

## Results and errors

Each tool maps to one canonical command:

| Tool | Canonical command |
| --- | --- |
| `detect_text`, `detect_files` | `detect` |
| `check_plagiarism` | `plagiarism` |
| `analyze_text` | `analyze` |
| `get_task`, `wait_task` | `task_status`, `task_wait` |
| `submit_bulk`, `get_bulk`, `wait_bulk`, `get_bulk_results` | `bulk_submit`, `bulk_status`, `bulk_wait`, `bulk_results` |
| `history_list`, `history_search`, `history_get`, `history_rerun`, `history_delete`, `history_clear` | matching `history_*` command |
| `update_config` | `config_set` |
| `check_update` | `update_check` |

Successful tools return:

- `resultType: "complete"`
- exactly one schema-valid canonical command success envelope in
  `structuredContent`
- a concise text summary in text content

Malformed arguments use MCP validation errors. Domain failures return
`isError: true`, `resultType: "complete"`, and exactly one canonical command
failure envelope in `structuredContent`. `structuredContent` never contains a
bare analysis or bare error object. Generated per-tool output schemas constrain
the command constant and corresponding `data` root.

## File roots and file opening

Capability flags and approved file roots are immutable for the server lifetime.
`--allow-file-root PATH` is repeatable. Each value must be an absolute, existing
directory. The server fails startup before reading JSON-RPC messages when it
cannot validate and pre-open every configured root with the required
permissions and no-follow guarantees.

With no configured roots, the server omits `detect_files`. Installers never add
file roots automatically.

File access is root-relative and handle-based, rejects symlinks and
reparse-point traversal, and verifies the opened object rather than a pathname
checked earlier. A path is never authorized by canonicalizing a string and
reopening it later.

## Resources

Static resources:

- `pangram://schema/output/v1`
- `pangram://schema/errors/v1`
- `pangram://skills/pangram`

There are no MCP prompts or history resources.
