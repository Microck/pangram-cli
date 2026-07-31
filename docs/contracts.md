# Pangram CLI observable contracts

Status: normative implementation contract
Schema major: `"1"`
Configuration version: `1`

This document defines behavior visible outside an implementation module. When
code and this document disagree, update this document first or fix the code.

### Generated contract ownership

Phase 0 imported the seed contracts into Rust-owned types and transferred
artifact ownership to the Rust contract generator. The baseline seed set
remains traceable at commit `8b5149013cb231e5aae099320f296bb3576841b1`.
A locked differential transfer corpus passes against every retained seed and
generated schema. The MCP 2025-11-25 Task seed was retired when the MCP
contract moved to 2026-07-28 because Tasks left the core protocol.

Transfer review found two defects in the seed output schema: envelope branches
did not reserve the opposite payload field, and unknown-submission errors did
not require one canonical duplicate-billing warning. The observable contract
was corrected before ownership transfer completed. Current-only regressions
record these intentional differences from the seed.

The 2026-07-29 Pangram 4 correction removed the never-shipped `ai_assisted`
document classification, renamed the never-shipped `api_version` provenance
field to `upstream_version`, and added required humanizer evidence. No runtime
or public schema had been released, so schema major `"1"` remains the initial
public contract.

Generated artifacts are now read-only outputs. Observable changes update this
document first, then the Rust owner and generated artifacts in the same change.
CI rejects regeneration drift and stale generated files.

## 1. Compatibility rules

Within schema major `"1"`:

- fields may be added
- enum values MUST NOT be added unless the field is documented as open text
- consumers MUST ignore unknown object fields
- fields MUST NOT be removed
- field types and meanings MUST NOT change
- array ordering rules MUST remain stable

A public object is extensible unless its schema explicitly closes unknown
fields for a security or integrity boundary. Consumers MUST reject unknown
fields in an explicitly closed object. Extending a closed object is a
compatibility change governed by that contract's version.

A removal, type change, semantic change, or closed-enum expansion requires a
new schema major.

JSON object ordering is not semantic. The implementation may serialize fields
deterministically for fixtures.

### 1.1 MCP protocol compatibility

The stdio MCP server targets protocol version `2026-07-28` without a legacy
protocol path or the experimental Tasks extension. File access requires an
explicit repeatable `--allow-file-root PATH`. The complete normative interface,
including discovery, result metadata, and file-opening rules, lives in
[mcp-contract.md](mcp-contract.md).

## 2. Primitive conventions

| Concept | Contract |
| --- | --- |
| Field names | `snake_case` |
| Schema version | String major, initially `"1"` |
| Timestamps | RFC 3339 UTC strings ending in uppercase `Z` |
| Durations | Non-negative integer milliseconds |
| Hashes | Lowercase hexadecimal SHA-256 |
| Fractions | Number from 0.0 through 1.0 |
| Percentages | Number from 0.0 through 100.0 |
| Missing optional field | Omitted, not `null` |
| Local analysis ID | `anl_` plus lowercase UUIDv7 |
| Local bulk ID | `bulk_` plus lowercase UUIDv7 |
| Upstream IDs | Opaque non-empty strings |

Explicit `null` is reserved for cases where absence and not-yet-known have
different meanings. The initial public model does not require it.

## 3. Canonical command envelopes

### 3.1 Success

```json
{
  "schema_version": "1",
  "command": "detect",
  "data": {
    "id": "anl_01983c20-0180-7a80-a001-000000000001",
    "status": "running",
    "submission_outcome": "accepted",
    "input": {
      "type": "text",
      "origin": "literal",
      "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "byte_count": 15,
      "word_count": 3
    },
    "checks": [
      {
        "kind": "ai_detection",
        "status": "running",
        "upstream": {
          "task_id": "task-123"
        }
      }
    ],
    "save_state": "ephemeral",
    "provenance": {
      "provider": "pangram"
    },
    "created_at": "2026-07-23T12:00:00Z",
    "updated_at": "2026-07-23T12:00:00Z"
  },
  "meta": {
    "started_at": "2026-07-23T12:00:00Z"
  }
}
```

`command` is the resolved command, not necessarily the literal argv spelling.
Bare piped detection uses `detect`.
A success envelope contains `data` and MUST NOT contain `error`, even when the
extra `error` value would not validate as a canonical error.

JSON envelope commands and their `data` roots are closed:

| Commands | `data` root |
| --- | --- |
| `detect`, `plagiarism`, `analyze` | one analysis, or an ordered analysis array for repeated files |

For every repeated-file run the analyses are one canonical ordered series.
Single-document success formats (JSON, TOON, Markdown, and pretty) place the
whole series inside one success envelope whose `data` is the ordered analysis
array, so an explicit non-JSONL format never performs billable work and then
fails to render. JSONL is the only repeated-file streaming projection: it
emits one ordered success envelope per analyzed file, one per line. The
ordered-series rule is stable for schema major `"1"` (section 1).
| `task_status`, `task_wait`, `history_show`, `history_rerun` | one analysis |
| `bulk_submit`, `bulk_status`, `bulk_wait` | one bulk collection |
| `bulk_results` | one ordered bulk-item page |
| `history_list`, `history_search` | an ordered analysis summary page |
| `history_delete`, `history_clear`, `auth_set`, `auth_logout`, `config_set`, `mcp_install`, `mcp_uninstall` | mutation acknowledgement |
| `auth_status`, `config_list`, `config_get`, `config_path`, `doctor`, `mcp_status`, `update_check`, `update_install` | the command-specific typed status object |

Commands that emit Markdown, shell completion source, JSONL export, or start the
stdio MCP server do not use the JSON envelope. The generated schema MUST
discriminate the command and corresponding `data` root. It MUST NOT accept an
arbitrary command or unconstrained `data`.

### 3.2 Failure

```json
{
  "schema_version": "1",
  "command": "detect",
  "error": {
    "code": "missing_api_key",
    "category": "authentication",
    "message": "No Pangram API key is configured.",
    "retryable": false,
    "recovery": {
      "message": "Configure a persistent key or set PANGRAM_API_KEY.",
      "command": "pangram auth"
    }
  },
  "meta": {
    "failed_at": "2026-07-23T12:00:00Z"
  }
}
```

A failure envelope contains `error` and MUST NOT contain `data`, even when the
extra `data` value would not validate for the resolved command.

### 3.3 Partial success

Partial combined and bulk output uses the success envelope. `data.status` is
`partial`, successful data remains present, and failed checks or items contain
canonical errors. The process exits 3.

## 4. Analysis model

An analysis serializes as:

```json
{
  "id": "anl_01983c20-0180-7a80-a001-000000000001",
  "status": "succeeded",
  "submission_outcome": "terminal",
  "input": {
    "type": "text",
    "origin": "stdin",
    "sha256": "2f77668a9dfbf8d5848b9eeb4a7145ca94c6ed9236e4a773f6dcafa5132b2f91",
    "byte_count": 15,
    "word_count": 3
  },
  "checks": [
    {
      "kind": "ai_detection",
      "status": "succeeded",
      "upstream": {
        "task_id": "task-123",
        "last_stage": "STAGE_SUCCESS"
      },
      "result": {
        "classification": "human",
        "headline": "Human-written",
        "prediction": "The document appears to be human-written.",
        "fraction_ai": 0.0,
        "fraction_ai_assisted": 0.0,
        "fraction_human": 1.0,
        "num_ai_segments": 0,
        "num_ai_assisted_segments": 0,
        "num_human_segments": 1,
        "segments": [
          {
            "text": "The text to analyze",
            "label": "Human Written",
            "ai_assistance_score": 0.0,
            "confidence": "high",
            "start_index": 0,
            "end_index": 19,
            "word_count": 4,
            "token_length": 4,
            "humanizer_score": 0.0,
            "is_humanized": false
          }
        ]
      }
    }
  ],
  "save_state": "ephemeral",
  "provenance": {
    "provider": "pangram"
  },
  "created_at": "2026-07-23T12:00:00Z",
  "updated_at": "2026-07-23T12:00:01Z",
  "completed_at": "2026-07-23T12:00:01Z"
}
```

### 4.1 Status

Closed values:

- `queued`
- `running`
- `succeeded`
- `failed`
- `partial`

The parent status derives from its checks; a disagreeing status is invalid. In
precedence order: any `running` check, then any `queued` check, then all
`succeeded`, then all `failed`, else (mixed succeeded and failed) `partial`.

### 4.2 Save state

Closed values:

- `ephemeral`
- `saved_manual`
- `saved_history`

### 4.3 Input

Text input:

```json
{
  "type": "text",
  "origin": "literal",
  "name": "notes.txt",
  "sha256": "...",
  "byte_count": 1024,
  "word_count": 180,
  "text": "Present only when explicitly included."
}
```

Closed `origin` values:

- `literal`
- `stdin`
- `file`

`name` is present for a text file. `text` is omitted unless the caller
explicitly requests input content or reads a full saved export.

Binary file input:

```json
{
  "type": "file",
  "filename": "paper.pdf",
  "media_type": "application/pdf",
  "sha256": "...",
  "size_bytes": 42000,
  "path": "/path/present/only/in/explicit/full/output",
  "extracted_text": "Present only if Pangram returns it and content is included."
}
```

Default command output omits `path` and `extracted_text`.

### 4.4 Check

```json
{
  "kind": "ai_detection",
  "status": "failed",
  "upstream": {
    "task_id": "task-123",
    "last_stage": "STAGE_FAILED"
  },
  "error": {
    "code": "upstream_analysis_failed",
    "category": "upstream",
    "message": "Pangram could not analyze the submitted text.",
    "retryable": false
  }
}
```

Closed `kind` values:

- `ai_detection`
- `plagiarism`

A terminal check has exactly one of `result` or `error`.

The check variant is tagged by `kind`. An `ai_detection` check can contain only
an AI-detection result. A `plagiarism` check can contain only a plagiarism
result. An analysis contains at most one check of each kind. Queued and running
checks contain neither `result` nor `error`; succeeded checks contain `result`;
failed checks contain `error`.

### 4.5 Submission outcome

Every billable submission has one closed local outcome:

- `not_submitted`: no request body may have reached Pangram
- `accepted`: Pangram returned an upstream identifier
- `terminal`: Pangram returned the final result synchronously
- `acceptance_unknown`: the request may have reached Pangram but no acceptance
  response was obtained

`acceptance_unknown` is never automatically retryable. Its error details contain
the local analysis or bulk ID, request SHA-256, any known upstream IDs, and the
last observed state. They contain no submitted content. Its recovery object is
exactly `{"message":"A manual retry may create a second billable operation."}`.

Every timeout or handled interruption after acceptance reports the local ID,
known upstream identifiers, and last observed state before exit. With history
disabled, the process retains this identity only in its final output; it does
not create a hidden task ledger.

## 5. AI-detection result

The complete result shape appears in the analysis example in section 4.
Closed `classification` values:

- `ai`
- `human`
- `mixed`

The classification maps Pangram 4's `prediction_short` values. AI-assisted
evidence remains in fractions, segment counts, and segment labels; it is not a
fourth document classification.

Closed `confidence` values:

- `high`
- `medium`
- `low`

`label` is provider-authored descriptive text and is intentionally open. It is
not used as a state discriminator.

`humanizer_score` is Pangram 4's estimate from 0.0 through 1.0 that a humanizer
modified the segment. `is_humanized` preserves Pangram's thresholded decision.
The client MUST NOT derive it from a local threshold. `start_index` and
`end_index` preserve zero-based, half-open upstream character offsets. Pangram
does not define the character unit precisely enough to use them as UTF-8 byte
indices.

`dashboard_link` appears only when the request explicitly asked Pangram to
create one.

## 6. Plagiarism result

```json
{
  "plagiarism_detected": true,
  "total_sentences": 4,
  "plagiarized_sentence_count": 1,
  "percent_plagiarized": 25.0,
  "matches": [
    {
      "source_url": "https://example.com/source",
      "matched_text": "Matched text",
      "similarity_score": 0.95
    }
  ]
}
```

The canonical field is `plagiarized_sentence_count`. The initial response
normalizer accepts a numeric upstream `plagiarized_sentences`. A list triggers
`upstream_contract_changed` until the live contract is resolved.

Source URLs preserve the raw provider string and are not required to validate as
a URI. The runtime MUST NOT fetch them automatically. It offers an open action
only after separately parsing a valid HTTP or HTTPS destination and explicit
user confirmation.

## 7. Provenance

```json
{
  "provider": "pangram",
  "upstream_version": "4.0",
  "upstream_task_ids": ["task-123"],
  "upstream_bulk_id": "blk-123",
  "submitted_at": "2026-07-23T12:00:00Z",
  "completed_at": "2026-07-23T12:00:01Z"
}
```

Only applicable fields appear. `provider` is the closed value `pangram` in
schema major 1. `upstream_version` preserves Pangram's top-level `version`
value without claiming it identifies only an API or only a model.

## 8. Lineage

Retry and rerun create new analyses:

```json
{
  "retry_of": "anl_...",
  "rerun_of": "anl_..."
}
```

At most one lineage field appears on an analysis.

- `retry_of` means retry of failed or partial checks with the same input and
  options.
- `rerun_of` means a fresh user-requested analysis from any saved record.

Public-link creation always resets to false.

## 9. Bulk model

Bulk collection:

```json
{
  "id": "bulk_01983c20-0180-7a80-a001-000000000001",
  "upstream_bulk_id": "blk_123",
  "status": "partial",
  "submission_outcome": "accepted",
  "total_items": 3,
  "accepted": 2,
  "succeeded": 2,
  "failed": 1,
  "estimated_billable_units": 3,
  "created_at": "2026-07-23T12:00:00Z",
  "updated_at": "2026-07-23T12:00:20Z",
  "completed_at": "2026-07-23T12:00:30Z"
}
```

Bulk item:

```json
{
  "index": 0,
  "caller_id": "row-001",
  "analysis_id": "anl_...",
  "upstream_task_id": "task-123",
  "status": "succeeded",
  "analysis": {}
}
```

Results preserve ascending source `index`. `analysis` is omitted until a
result exists. Failed items contain `error`.

Bulk item status is one of `queued`, `running`, `succeeded`, or `failed`.
Queued and running items contain neither `analysis` nor `error`. Succeeded
items contain `analysis` and no `error`. Failed items contain `error` and no
`analysis`.

`total_items` is the locally validated input count. `accepted` is the count for
which Pangram returned an accepted item and upstream task ID. `failed` includes
immediate upstream rejection and terminal analysis failure, not local
whole-request validation failures. A rejected item counts in `failed` without
entering `accepted`, so `succeeded + failed` may exceed `accepted`; the
committed example (`accepted: 2`, `succeeded: 2`, `failed: 1`) is valid. The
only counter bounds are `accepted <= total_items`, `succeeded <= accepted`,
and `succeeded + failed <= total_items`. At terminal state:

```text
accepted <= total_items
succeeded + failed = total_items
```

The collection `status` agrees with its counters; a disagreeing status is
invalid. `queued` and `running` are not terminal (`succeeded + failed <
total_items`). `succeeded` requires `succeeded = total_items` and `failed = 0`;
`failed` requires `failed = total_items` and `succeeded = 0`; `partial`
requires a terminal count with both `succeeded > 0` and `failed > 0`.

`output.schema.json` encodes the status-driven counter relations Draft 2020-12
can express (a `succeeded` collection has `failed: 0` and a positive
`succeeded`). No standard Draft 2020-12 keyword expresses the cross-field
arithmetic bounds or the exact terminal equation over unbounded integers, so
the canonical `BulkCounters` and `BulkCollection` constructors remain
authoritative for them, exactly as the update contract leaves non-expressible
manifest invariants to the Rust verifier.

`updated_at` is required and changes whenever counters or state change.
`estimated_billable_units` records the estimate calculated under the
documented billing rule for the selected Pangram operation. Pangram has not
published a Pangram 4 bulk rule or confirmed the earlier 1,000-unit maximum.
Bulk remains blocked until then. Estimates MUST NOT be presented as exact
charges.

Bulk timestamps from Pangram are Unix epoch seconds encoded as strings. The
normalizer converts them to RFC 3339 UTC.

## 10. Progress events

`--progress jsonl` writes one event per stderr line:

```json
{
  "schema_version": "1",
  "type": "progress",
  "analysis_id": "anl_...",
  "check": "ai_detection",
  "status": "running",
  "upstream_stage": "STAGE_PREPROCESSING",
  "observed_at": "2026-07-23T12:00:00Z"
}
```

Progress events MUST NOT include input or result content.

Bulk events use `bulk_id` and may include counters.

## 11. Error object

```json
{
  "code": "rate_limited",
  "category": "rate_limit",
  "message": "Pangram is rate limiting requests.",
  "retryable": true,
  "retry_after_ms": 2000,
  "recovery": {
    "message": "Wait before retrying or lower network.max_requests_per_second.",
    "command": "pangram config set network.max_requests_per_second 2"
  },
  "details": {
    "http_status": 429
  }
}
```

Closed categories:

- `usage`
- `authentication`
- `permission`
- `payment`
- `rate_limit`
- `network`
- `upstream`
- `upstream_contract`
- `local_config`
- `local_history`
- `update`

Initial stable codes:

| Code | Category | Retryable |
| --- | --- | --- |
| `input_required` | usage | no |
| `input_conflict` | usage | no |
| `unsupported_input` | usage | no |
| `unsupported_combination` | usage | no |
| `bulk_limit_exceeded` | usage | no |
| `missing_api_key` | authentication | no |
| `invalid_api_key` | authentication | no |
| `permission_denied` | permission | no |
| `payment_required` | payment | no |
| `rate_limited` | rate_limit | yes |
| `network_unavailable` | network | yes |
| `network_timeout` | network | depends |
| `wait_timeout` | network | yes |
| `submission_outcome_unknown` | network | no |
| `upstream_error` | upstream | depends |
| `upstream_analysis_failed` | upstream | depends |
| `upstream_not_found` | upstream | no |
| `upstream_contract_changed` | upstream_contract | no |
| `invalid_config` | local_config | no |
| `insecure_config_permissions` | local_config | no |
| `insecure_history_permissions` | local_history | no |
| `history_disabled` | local_history | no |
| `history_unavailable` | local_history | depends |
| `history_corrupt` | local_history | no |
| `history_write_failed` | local_history | depends |
| `local_task_unresolvable` | local_history | no |
| `mcp_capability_required` | permission | no |
| `mcp_root_required` | permission | no |
| `mcp_path_outside_root` | permission | no |
| `update_unavailable` | update | no |
| `update_not_owned` | update | no |
| `update_verification_failed` | update | no |
| `update_replace_failed` | update | depends |

`details` is sanitized and code-specific. It MUST NOT contain credentials,
auth headers, submitted content, segment text, plagiarism matches, or raw
response bodies.

Upstream-reported messages (for example `upstream_message` on
`upstream_analysis_failed`) are reduced before they enter `details`: terminal
control sequences are stripped, non-printable and non-ASCII characters are
removed, and the retained text is truncated to a short bounded prefix.
Provider messages are untrusted and can echo submitted content; callers MUST
NOT surface raw upstream text to the terminal or serialized error output.

`network_timeout` is retryable only when no billable body was sent or the
operation is a safe read. A timeout after an ambiguous billable send maps to
`submission_outcome_unknown`.

The client paces every Pangram request with one shared time-based issue gate:
request issue times are spaced at least `1/network.max_requests_per_second`
apart, so no burst exceeds the hard 5-requests-per-second ceiling. This is
enforced on request issue timing (not completion). Safe-GET retry chains are
bounded by the attempt cap, a cumulative retry-time budget, and the caller's
wait deadline: a wait-timeout or cancellation interrupts pending retry sleeps
promptly, and the cumulative budget prevents bounded-but-large `Retry-After`
hints from delaying interruption indefinitely.

## 12. Exit codes

| Code | Meaning |
| ---: | --- |
| 0 | Success, accepted asynchronous work, or no update needed |
| 1 | General operation failure not covered below |
| 2 | Usage or local input error |
| 3 | Partial combined or bulk result |
| 4 | Authentication or permission failure |
| 5 | Payment, quota, or rate-limit failure |
| 6 | Network or upstream failure |
| 7 | Local configuration, history, or update-state failure |
| 130 | Interrupted by the user |

An accepted detached or bulk submission exits 0.

A failure exit derives from the canonical error object's category everywhere
it surfaces: as a top-level command failure envelope, and as the terminal
check `error` carried inside a canonical failed analysis. The category-to-exit
mapping is identical in both positions. In particular:

- an upstream terminal task failure (`STAGE_FAILED`) is an upstream analysis
  failure: the check carries `upstream_analysis_failed` (category `upstream`),
  and the command exits 6 (network or upstream failure), never general exit 1
- a failed analysis whose check error is a local usage or authentication
  failure exits per that error's category instead
- process interruption stays exit 130 per section 19

### 12.1 Cancellation and the billable-submission boundary

Local cancellation before the billable submit request is issued completes no
remote action and reports exactly that (a local stop; with history disabled the
process exits per the mapping above). Once the submit request is issued, the
send is ambiguous: the body may have reached Pangram. Cancellation or any
failure after issue therefore reports the canonical `submission_outcome_unknown`
acceptance (section 4.5) with the local analysis ID, the request SHA-256, any
known upstream IDs, the last observed state, and the fixed reconciliation
recovery. An ambiguous billable send is never replayed, and the process must
not claim either certain delivery or certain non-delivery.

Signal-driven interruption of the CLI (Ctrl+C/SIGINT) still exits 130 as
locked; the distinction above governs the identity reported alongside that
exit and the canonical outcome recorded in the final output.

## 13. Output projections

### JSON

One canonical envelope. For repeated files, explicit `--format json` returns an
ordered array inside `data`. The same one-envelope ordered-series rule applies
to every other single-document format (TOON, Markdown, pretty); only JSONL
streams one envelope per analyzed file.

### JSONL

One complete canonical envelope per line. Repeated files default to JSONL when
the user does not specify a format.

### TOON

A projection of the canonical JSON value using `toon`. It has no independent
contract semantics.

### Markdown

A human report with escaped input, result, evidence, and provenance. It MUST
not contain terminal color sequences.

### Pretty

Terminal-oriented rendering with optional color. It is not stable for parsing.

## 14. CLI grammar

### 14.1 Global

```text
pangram [GLOBAL] [TEXT]

GLOBAL:
  --config PATH
  --data-dir PATH
  --error-format json|text
  --no-color
  -V, --version
  -h, --help
```

Resolution:

- no command, no input, and stdin, stdout, and stderr all TTYs: TUI (the TUI
  arrives in Phase 5; before it is compiled, that otherwise-TUI launch falls
  back to successful help text exactly as bare `pangram --help`)
- no command, literal text: detect. Every bare token that is not a compiled
  subcommand and does not begin with `-` is literal text, including tokens
  that spell planned (not yet compiled) command names; those are analyzed as
  text. Only a hyphen-leading unknown remains a Clap usage error.
- no command, non-TTY stdin (a pipe or redirection) with content: detect, and
  the resolved `command` is `detect`
- no command, non-TTY stdin that decodes to no detectable text (an empty or
  whitespace-only pipe): the canonical `input_required` usage error (exit 2),
  never the help surface
- no command, a TTY stdin, and any other redirected stream: `input_required`
- literal `-`: stdin

Bare dispatch evaluates the source before any help or usage surface: a bare
piped stdin never prints help, and only the all-TTY bare launch uses the
pre-TUI successful-help fallback.

Source-category and content rules:

- `word_count` is the adapter-computed count of Unicode whitespace-separated
  tokens (`str::split_whitespace`), the canonical count shown in input
  summaries and used for the text billing estimate.
- Empty or whitespace-only literal text and empty or whitespace-only piped
  stdin are the canonical `input_required` usage error: no content was
  supplied to detect. An executable stdin (TTY, or a pipe that decodes to
  nothing) is also `input_required`.
- `--file` reads UTF-8 text files only in schema major 1. A path that cannot
  be read is `input_required`; a file whose bytes are not UTF-8, or whose
  decoded text carries no detectable words, is `unsupported_input`, both
  before any submission. Binary document (PDF, DOCX, RTF) detection is a
  later-phase workflow and is not inferred client side.

Defaults:

- noninteractive commands default to JSON success and JSON errors
- an explicitly selected pretty format defaults to text errors
- `--error-format` overrides those defaults
- the resolved error surface applies to every failure of the invocation,
  including failures raised before billable work (plan validation,
  credential resolution, and client construction): an explicit
  `--format pretty` surfaces a sanitized text message on stderr with empty
  stdout and the category-derived exit, unless `--error-format json`
  overrides it back to a stdout JSON envelope
- `--progress auto` emits human progress only when stderr is a TTY and the
  selected output is pretty; otherwise it emits no progress
- JSONL progress requires explicit `--progress jsonl`

### 14.2 Analysis commands

```text
pangram detect [TEXT] [--file PATH]... [--detach]
pangram plagiarism [TEXT] [--file PATH]...
pangram analyze [TEXT] [--file PATH]...
```

Common applicable flags:

```text
--format json|jsonl|toon|markdown|pretty
--include-input
--save
--public-link
--timeout DURATION
--progress auto|never|jsonl
--max-billable-units N
```

`--save` is a later-phase flag: local history arrives in Phase 4. It remains in
the normative grammar so the Phase 4 surface is fixed now, but until history is
compiled the flag is not advertised in runtime help or the generated reference,
and passing it is rejected as an unknown-argument usage error (exit 2) before
any billable work. It must not be presented as an available Phase 2 capability.

Rules:

- exactly one source category: positional text, stdin, or files
- repeated files are allowed
- `--detach` is an explicit `detect` flag and is invalid for other analysis
  commands
- `--public-link` is invalid for plagiarism-only work
- `detect --detach` is allowed for UTF-8 text only
- `analyze --detach` does not exist
- plagiarism does not detach
- binary file plagiarism and combined analysis fail before submission
- timeout stops waiting, not upstream work
- `--max-billable-units` rejects a locally estimated request above the ceiling
  before submission; MCP billable tools require the same field. Each analyzed
  text contributes its own started-100-word estimate, so repeated `--file`
  inputs are summed and compared against the single ceiling before any
  submission.
- text detection estimates one billable unit per started 100-word block, with
  a minimum of one

Wait and completion:

- `--timeout DURATION` accepts a non-negative decimal count of seconds (`30`,
  `0.5`), optionally followed by exactly one ASCII unit suffix `s`, `ms`,
  `m`, or `h` (`500ms`, `2m`, `1h`). A missing unit means seconds. The grammar
  is exact: no whitespace is allowed between the count and the suffix, an
  exponent or non-finite form (`1e2`, `inf`, `nan`) is rejected, the count
  must not be negative, and a count of `0` (or any value that truncates to
  zero) is rejected as a usage error because it would not bound any wait. The
  scaled value must fit the supported duration range.
- when `--timeout` is not supplied, `detect` waits for the analysis to reach a
  terminal state without a local wait deadline; there is no hidden wait
  ceiling. A caller bounds an observation only by passing `--timeout`.

Pangram 4 is the only production text model. The CLI has no model-selection
flag. The analysis module MUST send Pangram's documented Pangram 4 selector,
JSON request field `model` with the exact value `pangram-4`, and MUST NOT
omit `model` or otherwise rely on the temporary Pangram 3 default routing
that Pangram retires on 2026-09-30.

There is no image-detection command, schema, or MCP tool. Invitation-only
preview access does not qualify as a public documented API contract.

### 14.3 Bulk

```text
pangram bulk submit [JSONL_PATH|-]
  --max-billable-units N
  [--dry-run]
  [--wait]
  [--public-link]
  [--format FORMAT]
  [--progress auto|never|jsonl]

pangram bulk status ID
pangram bulk wait ID [--timeout DURATION] [--progress ...]
pangram bulk results ID [--offset N] [--limit N] [--format FORMAT]
```

JSONL item:

```json
{"id":"optional-caller-id","text":"Text to analyze"}
```

Unknown item fields and duplicate caller IDs fail whole-file validation.

### 14.4 Task

```text
pangram task status ID
pangram task wait ID [--timeout DURATION] [--progress ...]
```

`ID` may be a local analysis ID when history resolves it or a Pangram task ID.

### 14.5 History

```text
pangram history list [--status STATUS] [--check CHECK] [--limit N]
pangram history show ID [--include-input] [--format FORMAT]
pangram history search QUERY [--status STATUS] [--check CHECK] [--limit N]
pangram history delete ID [--yes]
pangram history clear [--yes]
pangram history export [--format jsonl|markdown] [--redact-content]
pangram history rerun ID [--format FORMAT] [--progress ...]
```

Export writes stdout. There is no general output-path flag.

### 14.6 Authentication

```text
pangram auth
pangram auth set --api-key VALUE
pangram auth set --api-key-stdin
pangram auth status
pangram auth logout [--yes]
```

`auth status` is local and non-billable. It reports credential source and a
short masked suffix.

Interactive `pangram auth` reads a masked value from the controlling terminal.
`--api-key-stdin` reads exactly one UTF-8 line and is the preferred persistent
setup path for agents that cannot use `PANGRAM_API_KEY`. `--api-key VALUE`
remains supported but help and documentation warn that argv may be visible in
process listings and shell history.

### 14.7 MCP

```text
pangram mcp [--history] [--allow-history-mutations]
  [--allow-config-mutations] [--allow-public-links]
  [--allow-file-root PATH]...
pangram mcp install [--target CLIENT]... [--all] [--server-name NAME] [--dry-run]
pangram mcp uninstall [--target CLIENT]... [--all] [--server-name NAME] [--dry-run]
pangram mcp status [--format json|pretty]
```

Clients are `claude-code`, `claude-desktop`, `codex`, `cursor`, `vscode`,
`windsurf`, `gemini`, `opencode`, `cline`, `roo-code`, `droid`, and
`antigravity`.

Default server name is `pangram`.

### 14.8 Skills and agents

```text
pangram agent
pangram skills list
pangram skills get pangram [--full]
pangram skills path [pangram]
```

`agent` and `skills get` emit Markdown only. `skills path` returns an
`embedded://` locator.

### 14.9 Configuration and diagnostics

```text
pangram config list
pangram config get KEY
pangram config set KEY VALUE
pangram config path
pangram doctor [--format json|pretty]
```

Credential keys are rejected by `config`.

### 14.10 Completions and update

```text
pangram completions bash|zsh|fish|powershell|elvish
pangram update --check
pangram update
pangram update --yes
```

Completions emit only the completion script.

## 15. Local setup contract

The configuration, credential, and diagnostics contract is normative in
[local-setup-contract.md](local-setup-contract.md).

## 16. Update state and receipt

The state and installation receipt are normative in
[update-contract.md](update-contract.md).

## 17. SQLite history schema

The versioned database contract lives in
[history-contract.md](history-contract.md). `HistoryStore` is its sole write
owner.

## 18. Module contracts

- The MCP interface is normative in [mcp-contract.md](mcp-contract.md).
- The signed updater interface is normative in
  [update-contract.md](update-contract.md).
- The local history interface is normative in
  [history-contract.md](history-contract.md).
- The local setup interface is normative in
  [local-setup-contract.md](local-setup-contract.md).

## 19. Shell contracts

- stdout contains only primary output.
- stderr contains progress, warnings, diagnostics, and errors.
- JSON and JSONL never contain ANSI sequences.
- Markdown never contains ANSI sequences.
- `--no-color` and `NO_COLOR` disable color.
- noninteractive commands do not prompt.
- prompts use `/dev/tty` or platform equivalent only when explicitly in an
  interactive workflow.
- interruption exits 130 after terminal restoration and final identifier
  reporting where safe.

## 20. Drift enforcement

The contract generator produces committed schemas and reference inputs from
Rust-owned types. CI runs:

```text
generate contracts
verify working tree unchanged
build Fumadocs
run compiled contract tests
run MCP conformance
```

The generated artifacts MUST point back to their owner and MUST NOT be edited
by hand.
