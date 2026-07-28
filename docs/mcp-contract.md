# Pangram CLI MCP contract

Status: approved for implementation
Transport: stdio

This file is the contract owner for the MCP interface. Input schemas set
`additionalProperties: false`.

## Analysis tools

| Tool | Required input | Optional input |
| --- | --- | --- |
| `detect_text` | `text`, `max_billable_units` | `save`, `public_link`, `include_input` |
| `detect_files` | `paths` | `max_billable_units`, `confirm_unknown_cost`, `save`, `public_link`, `include_input` |
| `check_plagiarism` | `text`, `max_billable_units` | `save`, `include_input` |
| `analyze_text` | `text`, `max_billable_units` | `save`, `public_link`, `include_input` |

`paths` is a non-empty array of filesystem paths inside current MCP roots.
Every billable text tool requires positive `max_billable_units`. The server
estimates locally and rejects the call before submission when the estimate is
above the ceiling. Binary file cost remains `unknown` until Pangram publishes
an authoritative rule; a binary file call additionally requires
`confirm_unknown_cost: true`.

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

Task-augmented tool calls use the MCP 2025-11-25 Task object without a
Pangram-specific wrapper. The generated contract is
[`mcp-task.schema.json`](../contracts/mcp-task.schema.json). `tasks/result`
returns the same `CallToolResult` that a non-task call would return.

MCP cancellation stops local observation only. It transitions the MCP task to
`cancelled` and does not claim to cancel Pangram work. When known, the final
status message identifies the local analysis and upstream task without
including submitted content.

## Bulk tools

`submit_bulk` requires:

- exactly one of `items` or `jsonl_path`
- positive `max_billable_units`, at most 1,000

Optional fields are `public_link` and `save`.

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

- exactly one schema-valid canonical command success envelope in
  `structuredContent`
- a concise text summary in text content

Malformed arguments use MCP validation errors. Domain failures return
`isError: true` and exactly one canonical command failure envelope in
`structuredContent`. `structuredContent` never contains a bare analysis or bare
error object. Generated per-tool output schemas constrain the command constant
and corresponding `data` root.

For a failed task-augmented call, the MCP task reaches `failed`; `tasks/result`
returns the same `isError: true` tool result.

## Roots and file opening

Capability flags are immutable for the server lifetime. Roots are not a
startup capability. The server maintains a versioned snapshot obtained through
the MCP roots protocol and refreshes it on the protocol's root-change event.
Each tool invocation captures one immutable snapshot for its full duration.

The server pre-opens approved root directories. File access is root-relative
and handle-based, rejects symlinks and reparse-point traversal, and verifies the
opened object rather than a pathname checked earlier. A path is never
authorized by canonicalizing a string and reopening it later.

## Resources

Static resources:

- `pangram://schema/output/v1`
- `pangram://schema/errors/v1`
- `pangram://skills/pangram`

There are no MCP prompts or history resources.
