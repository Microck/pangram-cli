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

For every repeated-file run the analyses are one canonical ordered series.
Single-document success formats (JSON, TOON, Markdown, and pretty) place the
whole series inside one success envelope whose `data` is the ordered analysis
array, so an explicit non-JSONL format never performs billable work and then
fails to render. JSONL is the only repeated-file streaming projection: it
emits one ordered success envelope per analyzed file, one per line. The
ordered-series rule is stable for schema major `"1"` (section 1).

JSON envelope commands and their `data` roots are closed:

| Commands | `data` root |
| --- | --- |
| `detect`, `plagiarism`, `analyze` | one analysis, or an ordered analysis array for repeated files |
| `task_status`, `task_wait`, `history_show`, `history_rerun` | one analysis |
| `bulk_submit` | one bulk collection, or the canonical bulk dry-run shape (section 9.2) |
| `bulk_status`, `bulk_wait` | one bulk collection |
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

Partial combined, bulk, and repeated-file output uses the success envelope.
Successful data remains present, failed checks or items contain canonical
errors, and the process exits 3. For a single analysis or bulk collection the
`data` object carries `status: partial`. A repeated-file run renders `data` as
the canonical ordered analysis *array* (section 3.1): an untagged JSON array
carries no envelope-level `status`, so the run's partial nature is conveyed by
exit 3 and by each member analysis's own `status` field.

Repeated single-document files form one ordered series, each member one
analysis. Only an *ambiguous* mid-run submission preserves the run as a
partial series: a billable submission POST that was issued but whose
acceptance became uncertain (for example a dropped connection after the
send). Three other failure classes behave differently and abort the run with
their canonical top-level failure envelope: a deterministic pre-billing
rejection (authentication, payment, usage, or a provably unreached send), an
accepted task's local observation failure (wait timeout, contract drift, or
transport), and a genuine SIGINT interruption, which always exits 130. An
ambiguous submission never discards the analyses already completed; the run
continues with the remaining files, and the ambiguous-submission file is
represented in the ordered series by one synthesized analysis member whose:

- `id` is the local `AnalysisId` generated for that file's request, so the
  member is reconcilable to the exact request that failed
- `input` carries the file's real `TextInput` (`origin`, `name`, `sha256`,
  `byte_count`, `word_count`, and `text` only when `--include-input`), so
  source identity and order metadata are preserved without fabrication
- single `ai_detection` check is `failed` with the canonical
  `submission_outcome_unknown` error
- `save_state` is `ephemeral`
- `submission_outcome` is `acceptance_unknown`, with
  `submission_outcome_unknown` reconciliation details (request `sha256`, and
  the `analysis_id`); the run never replays the ambiguous billable POST and
  reports the ambiguous outcome so the operator reconciles it
- `provenance` carries only upstream identity facts that actually exist; the
  synthesized member never fabricates `result`, upstream identity, or other
  remote detail, and carries no task id because acceptance was never reached
- `completed_at` and `provenance.completed_at` are absent because no remote
  terminal observation occurred; the local `failed` status records the
  ambiguous submission outcome rather than claiming remote completion

The envelope's parent `status` is `partial` (mixed succeeded and failed
members; section 4.1), and the process exits 3. JSONL preserves the ordered
series as one envelope per line, with the failed member emitted as its own
line in submission order; single-document formats (JSON, TOON, Markdown,
pretty) emit the whole ordered series inside one success envelope. Because
the ambiguous member's outcome is uncertain, reconciliation guidance is
emitted on stderr with the local reconciliation identity and the fixed
duplicate-billing recovery reminder.

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
- `unknown`

`unknown` marks the descriptor of a remotely authored operation the caller
observes by explicit upstream ID (section 4.6): the input was submitted by
another actor or process, so no local submission category applies. It is
valid only on that resumed-observation path; locally submitted commands
never emit it.

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

### 4.6 Resumed observation authorship

A command that observes a Pangram operation by its explicit upstream ID
(`task status`, `task wait`, and every bulk status, wait, items, and results
read of a job not submitted in the same invocation) reconciles a remote
record it did not author. `task status` and `task wait` also accept a saved
local `anl_` ID. Only a complete canonical local UUIDv7 ID selects this path;
every other non-empty spelling is an opaque upstream ID and bypasses local
lookup unchanged. Before credential resolution or network work, a canonical
local ID opens only an existing history database, validates the complete
canonical record and task evidence, and resolves exactly one AI-detection
upstream task ID. An absent canonical local ID, a record with no applicable
task, or ambiguous applicable task evidence is `local_task_unresolvable`;
malformed stored canonical evidence retains its specific local-history error.
Resolution never creates a missing history database and does not consult
`history.enabled`, so disabling future automatic writes does not hide a saved
task.

With local history disabled, a process observing an explicit upstream ID
retains no authorship record of earlier submissions, so the canonical analysis
or bulk collection it emits honestly records only the observed remote state
and never fabricates a local authorship fact:

- the local `anl_` or `bulk_` identity is generated fresh for the read, so
  the envelope stays self-describing; it makes no claim about the original
  submission's local identity
- `submission_outcome` is `accepted` whenever an upstream identity was
  observed (the evidence that a remote operation exists), never `terminal`;
  `terminal` is reserved for an operation the caller itself submitted, so a
  resumed read never claims it. This rule binds every emitted analysis,
  including the per-item child analyses inside a resumed bulk items or
  results page: an observed bulk child analysis (succeeded or failed) is
  `accepted`, never `terminal`, even after the observed child has reached
  its terminal state. A same-process read of a job this process submitted
  builds its child analyses from the validated local plan and claims
  `terminal`, exactly as before
- `provenance.submitted_at` is omitted because the caller did not submit the
  operation; `provenance.completed_at` is present only when observation
  reached a terminal state, and preserves that observation's time
- `provenance` carries the observed upstream identities
  (`upstream_task_ids`, `upstream_bulk_id`, and `upstream_version` from a
  terminal document) and nothing inferred
- a resumed analysis or collection carries a descriptor only for what the
  local caller actually holds. A same-process `bulk wait` or `bulk results`
  after `bulk submit` builds each item's input descriptor from the validated
  local plan (section 9.1). A `task` read holds no local submission plan, so
  its item descriptor derives only from the terminal document Pangram
  attested: when a terminal success document carries the normalized text,
  the descriptor computes SHA-256, byte count, word count, and the
  `unknown` origin over that attested text, and it never echoes the text
  itself. A task read that has not yet reached a terminal document carries
  no input descriptor (`input` is omitted) rather than inventing one

Workflow caveat: none of this exposes content. A terminal task read surfaces
hashes and counts, not the submitted text, unless the caller separately
holds and supplies it locally.

A resumed bulk child analysis whose remote item has not attested terminal
content carries no input descriptor (`input` is omitted) rather than a
fabricated or placeholder one. This binds both directions of terminal
content: a failed child never carries an input descriptor, and a succeeded
child whose terminal document carries no normalized text also omits `input`
rather than inventing one. A same-process read of a locally submitted job
still builds every succeeded or failed child's input descriptor from the
validated local plan and claims `terminal`.

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
normalizer accepts the numeric upstream `plagiarized_sentences` documented by
the current official API reference. A list or missing value triggers
`upstream_contract_changed`. Live conformance must confirm the numeric shape
before public support.

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
and `succeeded + failed <= total_items`. For an accepted submission
(`submission_outcome: "accepted"`), `total_items` is the validated input count,
so the submit accepted list plus the submit failed list MUST cover positions
`0..total_items` in ascending order exactly once; a gap, a duplicate, or a
position at or above `total_items` fails upstream contract validation. At
terminal state:

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

`updated_at` is required and changes whenever counters or state change. For an
accepted submission, when some but not all accepted items have finished, the
collection is `running` (not `partial`): `partial` is terminal only, matching
a mixed-terminal analysis. For an accepted submission the upstream status is
poll-driven during observation, so it always agrees with the exact counters;
a collection status/counter combination outside the accepted-submission
precedence (`running` while any items remain unfinished, else the terminal
equation) fails upstream contract validation. `estimated_billable_units`
records the estimate calculated under the documented billing rule for the
selected Pangram operation. Pangram 4 bulk
selection uses one job-wide JSON `model` field with the exact value
`pangram-4`; per-item model selectors are not supported. Each valid item costs
one unit per started 100-word block, with a minimum of one unit per item. The
job estimate is the sum of those per-item units. Pangram accepts at most 1,000
billable units in one bulk request and documents no separate item-count limit.
Normal request-body limits still apply.

The effective local ceiling is the smaller of the required caller-supplied
`max_billable_units` and Pangram's 1,000-unit request limit. An estimate above
that ceiling fails with `bulk_limit_exceeded` before credential or network
work. An unexpected upstream `413 Payload Too Large` for a submitted bulk
request maps to the same code and retains sanitized `http_status: 413` detail.
Estimates MUST NOT be presented as exact charges.

A submitted (non-dry-run) bulk run is envelope-only output: as with the dry
run, `--format` with a non-JSON value is rejected as a usage error before any
bulk preparation, credential resolution, or network access, so no
billable-unit estimate, plan validation, or submission work is performed for
a projection the submitted run cannot express.

### 9.2 Bulk dry run

`bulk submit --dry-run` reports the validated plan without credentials,
network, or any remote identity. It is a local preflight: it runs after
whole-file JSONL validation and the billable-unit ceiling check, and returns
exit 0 with the canonical `bulk_submit` success envelope. The `bulk_submit`
data root is therefore the closed union of `BulkCollection` (a submitted run)
and exactly one `BulkDryRun` shape (a dry run):

```json
{
  "id": "bulk_01983c20-0180-7a80-a001-000000000001",
  "status": "queued",
  "submission_outcome": "not_submitted",
  "plan_sha256": "2f77668a9dfbf8d5848b9eeb4a7145ca94c6ed9236e4a773f6dcafa5132b2f91",
  "estimated_billable_units": 3,
  "item_count": 3
}
```

- `id` is the freshly generated local bulk ID. No upstream identity exists,
  so `upstream_bulk_id` is absent.
- `status` is always the closed value `queued` (nothing has run).
- `submission_outcome` is always the closed value `not_submitted`.
- `plan_sha256` is the request-document SHA-256 the submitted run would send,
  for reconciliation.
- `estimated_billable_units` and `item_count` come from the validated plan.

A dry run is JSON-only: the machine reconciliation shape has no TOON,
Markdown, or pretty projection, so `--format` with a non-JSON value is
rejected as a usage error before credentials or network. The dry run carries
no analyzed content and reserves an additional dry-run marker (`dry.noop`,
`dry.observed` false) under its data root so machine consumers can tell a
preflight from a real queued collection without relying on
`submission_outcome` alone.

Bulk metadata and results expire 48 hours after the job reaches a terminal
status. Bulk timestamps from Pangram are Unix epoch seconds encoded as
strings (for example `"1760000000.0"`). The normalizer converts them to RFC
3339 UTC.

### 9.1 Bulk wire contract

The loopback fixture and the future production client assert these documented
shapes exactly (official Bulk API source `eb214f4`, verified current on
2026-08-01). Unknown upstream values on a required state or shape fail
upstream contract validation per section 6.2 of the architecture; the fixture
plays them to prove it.

Submit is `POST {text-base}/bulk`. The request body carries exactly one of
`items` or `text` plus one job-wide `model`:

- `items` is the ordered list `{"id": optional-caller-id, "text": "..."}`.
  Caller-supplied `id` values are optional and MUST be unique within one
  request when provided; the local JSONL validator rejects duplicates before
  submission (section 14.3).
- `text` is the ordered list of plain strings used when no caller IDs are
  needed.
- `model` is exactly `pangram-4` for the whole job. No per-item selector
  exists, and there is no public-dashboard-link request field.

Exactly an HTTP `202 Accepted` is the submit success signal. Any other
status, including another `2xx`, is not an acceptance: a non-`202` `2xx`
falls into the never-replayed ambiguous class (section 12.1) because the job
may exist remotely, and a non-`2xx` status maps through the error matrix
below. The 202 response carries the upstream `bulk_id`, an initial `status`,
the `total_items` count, an ordered `accepted_items` list (`index`, optional
`id`, `task_id`), and an ordered `failed_items` list for items rejected by
immediate validation (`index`, optional `id`, null `task_id`, `stage`,
`error`). The 202 `status` token is the closed start-of-life marker
`queued`: the client has no network authority between the acceptance and the
first status read that could set a later value, so any other token fails
upstream contract validation. Per-item `stage` fields are provider
diagnostics, not protocol state: the item's own shape decides its class
(a `result` document is a success, `result: null` is in-progress, a
`failed_items` entry is failed), per-item `stage` never carries or gates
termination, and it is preserved only as sanitized provenance evidence
(bounded, ASCII-printable, control sequences stripped). A failed entry
accepts null or any string `stage`; a missing `stage` on a failed entry is
not contract drift.

Status polling is `GET /bulk/{bulk_id}` and returns the job `bulk_id`, one of
the closed statuses `queued`/`running`/`succeeded`/`failed`/`partial`, the
`total_items`/`accepted`/`succeeded`/`failed` counters, and the
`created_at`/`completed_at` epoch-second strings (`completed_at` is null
while the job is not terminal). Observation enforces the section 9 precedence
everywhere: when some but not all accepted items have finished the normalized
collection status is `running`, so an upstream `partial` token on a
non-terminal counter set is contract drift, and a terminal counter set with a
disagreeing status is contract drift. Normalized timestamps preserve the
upstream epoch-second strings converted to RFC 3339 UTC; a malformed or
out-of-range upstream timestamp is contract drift.

The analysis core builds each canonical bulk item's input descriptor from the
validated local submission plan (origin, SHA-256, byte and word counts), never
from untrusted upstream result text. An upstream result document that echoes a
`text` field is result-side evidence and is not reparsed as the item's input.

Item metadata paging is `GET /bulk/{bulk_id}/items?offset=N&limit=M`; result
paging is `GET /bulk/{bulk_id}/results?offset=N&limit=M`. Both accept a
zero-based `offset` and a `limit` in `1..=1,000`, and return the job
`bulk_id`, the echoed page `offset` and `limit`, `total_items`, and page
lists. The analysis core revalidates every page: the echoed `bulk_id` MUST
equal the queried job, the echoed `offset`/`limit` MUST echo the request,
`total_items` MUST agree across pages, page entries MUST be strictly ascending
by source `index`, and a fetch-all walk covers each item position in
`0..total_items` exactly once with a strictly advancing `next_offset` until it
exhausts the set (the final page's `next_offset` is the end-of-set marker).
A duplicate, out-of-order, counter-mismatched, identity-mismatched, or
non-advancing page fails upstream contract validation. An empty page while
positions remain uncovered is non-advancing drift; completion requires exact
coverage of `0..total_items`. The client supplies its own constant page
`limit` for a fetch-all walk; there is no aggregate endpoint. The fetch-all
walk uses the conservative bounded page size of 100 rather than the 1,000
maximum, because every received response body is bounded in memory right up
to the client's 16 MiB hard response cap and a 1,000-item page is the worst
case. Explicit one-page `bulk items`/`bulk results` requests may still use
any `limit` in `1..=1,000`; only the internal fetch-all walk is capped to
100. Because there is no aggregate endpoint, a fetch-all read reassembles
the strictly ordered union of every walked page into one canonical page
whose synthetic window metadata reports the whole aggregate: `offset` is
`0`, `limit` is `max(1, total_items)` (still bounded by the documented
1,000 cap), and `next_offset` is absent (the end-of-set marker). That
synthetic `limit` names the complete aggregate window the caller received;
it does not echo any single request page size (the walker requested pages
of 100), so consumers can tell "one complete aggregate" from "one bounded
upstream page" without hidden state. The exact-coverage and no-advance
safeguards above bind the walk before this reassembly, so a synthesized
aggregate exists only after every position in `0..total_items` was covered
exactly once. A submitted session's response `total_items` MUST equal the validated
local plan count and is checked before any client-side allocation; a
mismatch is contract drift. For a status/page read of a job without a local
plan (a resumed remote handle), the client validates the upstream-reported
`total_items` against the documented hard bound before any allocation.
Because a job bills each valid item at a minimum of one unit and a request
is capped at 1,000 billable units, no valid job exceeds 1,000 items; a
larger reported `total_items` fails upstream contract validation and the
client never allocates from an unchecked count.

- the items page returns one `items` list of metadata (`index`, optional
  `id`, `task_id`, `stage`, optional `error`).
- the results page returns an `items` list for successful or in-progress
  work (completed items add a `result`; in-progress items carry
  `result: null`) plus a separate `failed_items` metadata list for the same
  page. A `result: null` entry normalizes to the canonical bulk item status
  `running`. Each of the two lists is strictly ascending by source `index`
  on its own; cross-list integrity is disjointness plus coverage of the
  requested window, not a chained ordering across the lists.

The documented bulk error matrix is: `401` missing or invalid key, `402`
insufficient credits, `403` model not enabled or job not owned, `404` unknown
job, `413` over the billable-unit limit, `422` empty/duplicate-ID/both-shapes/
invalid-model validation, `500` processing error, and `503` model temporarily
unavailable. `413` is documented on the submit route; the other statuses bind
every `/bulk` route that the matrix indicates for them, most importantly the
safe GET routes whose key, ownership, unknown-job, validation, and server
failures are exercised by the loopback protocol suite. Only the safe GET routes (`GET /bulk/{bulk_id}`, `/items`,
`/results`) are eligible for the bounded transient-failure retry policy; the
billable `POST /bulk` is never replayed after an ambiguous send (section
12.1).

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
| 3 | Partial combined, bulk, or repeated-file result |
| 4 | Authentication or permission failure |
| 5 | Payment, quota, or rate-limit failure |
| 6 | Network or upstream failure |
| 7 | Local configuration, history, or update-state failure |
| 130 | Interrupted by the user |

An accepted detached or bulk submission exits 0. This binds every
successfully parsed HTTP 202 bulk acceptance, including one whose
`failed_items` list rejects some or even every submitted item through
immediate upstream validation: the acceptance itself is the authority for
exit 0, and the envelope MUST report the truthful validated counters and
derived status from that 202 response (the accepted and immediately failed
counts, a `queued` collection while any accepted work remains, or the
terminal `failed`/`partial`/`succeeded` collection when the 202 rejected
every item), preserving every accepted caller ID and the observed upstream
identity. A `bulk submit` without `--wait` never fabricates all-queued-zero
counters over an acceptance that already reports immediate failures.

A successful `bulk results` page or fetch-all read exits 0 regardless of the
child outcomes on the returned window. One results page is a successful
retrieval of a window and a page is not authoritative for the whole-job
terminal state: failed children on the returned page are preserved as failed
children (`status: failed` with their sanitized `error`) inside the
successful command envelope. Callers that need the job-outcome exit use
`bulk status` or `bulk wait`, whose observed terminal collection owns the
section 12 outcome mapping (a terminal `failed` collection exits 6, a
`partial` exits 3, a `succeeded` exits 0).

A failure exit derives from the canonical error object's category everywhere
it surfaces: as a top-level command failure envelope, and as the terminal
check `error` carried inside a canonical failed analysis. The category-to-exit
mapping is identical in both positions. In particular:

- an upstream terminal task failure (`STAGE_FAILED` on the observe stream)
  is an upstream analysis failure: the check carries
  `upstream_analysis_failed` (category `upstream`), and the command exits 6
  (network or upstream failure), never general exit 1
- the per-task status snapshot (`task status` and `task wait`, reading a
  text task by its upstream ID or resolving it from one saved local `anl_`
  ID) follows the identical observation contract:
  an upstream terminal `STAGE_FAILED` on the poll stream is normalized to
  the same upstream terminal failure (`upstream_analysis_failed`, category
  `upstream`, exit 6), and any non-terminal stage reads as still `running`
  until a later poll reports a terminal state
- a failed analysis whose check error is a local usage or authentication
  failure exits per that error's category instead
- a bulk collection that reaches the terminal `failed` state failed every
  item through an upstream terminal analysis failure (immediate upstream
  rejection or `STAGE_FAILED`); that terminal bulk failure is an upstream
  failure and exits 6, never general exit 1
- process interruption stays exit 130 per section 19

For a repeated-file ordered series the run's exit follows the parent-status
derivation of section 4.1 applied across the members: a `partial` run (mixed
member outcomes, or any individually `partial` member) exits 3, while a run
whose members are ALL `failed` exits per the first failed member's
check-error category under the identical mapping above. This precedence is one
shared rule used by every repeated-output projection.

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

- no command, no input, and stdin, stdout, and stderr all TTYs: enter the TUI
  alternate-screen Analyze route
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
piped stdin never prints help. A bare all-TTY process launch bypasses the help
surface and enters the TUI.

An all-TTY bare launch enters the alternate-screen Analyze route. The initial
interactive surface has these observable boundaries:

- `Analyze`, `Active`, `History`, and `Settings` are reachable through the
  regular keymap with `Analyze` selected first. The Vim keymap adds its
  documented navigation keys without stealing printable input from the text
  composer.
- text AI detection is the first positive analysis capability and must enter
  the shared `Analyzer`. File input, plagiarism, and combined analysis remain
  visible but unavailable until their owning implementation phases; activating
  an unavailable control makes no request and spends no Pangram credit.
- While the text composer owns focus and no overlay covers it, the terminal
  cursor is visible at the canonical edit position. The composer derives
  horizontal and vertical scroll to keep that position on-screen. Leaving the
  composer focus hides the cursor.
- The full-screen session enables bracketed paste and disables it through the
  same idempotent restoration path as raw mode. One paste is one literal
  composer edit, including tabs and line breaks; its bytes never become
  navigation, toggle, help, or submit commands. A covered composer, another
  focus, or the resize-required surface ignores the paste.
- terminals at least 120 columns wide render route rail, center workspace, and
  inspector as three stable areas. Widths 80 through 119 render top tabs and
  place inspector content below the center content. Any viewport below 80x24
  renders a resize overlay without changing application state.
- normal keyboard exit is the focusable `Quit` command-bar action. Ctrl+C is
  always an interruption. No unlisted single-key quit shortcut is implied by
  this contract.
- Settings changes commit through the same typed configuration service used by
  the CLI and become visible only after persistence succeeds. Enabling local
  history requires an explicit plaintext-retention warning and confirmation;
  cancelling that overlay leaves the disabled default unchanged.
- eligible intro playback remains unconsumed while the viewport is below
  80x24. Missing approved source geometry or logo rights suppresses generated
  intro frames but does not block the reducer, Analyze workflow, layout, or
  terminal restoration.

The TUI Active route is an ID-keyed union of in-session operations and saved
unfinished analyses discovered through certified History pages. Queued and
running summaries merge into Active without duplicating an in-session entry.
A newly accepted in-session entry precedes saved entries and becomes the
ID-owned selection. Up, Down, Home, End, and Vim `j`/`k` traverse every entry;
the derived six-row window follows that selection without a separate mutable
scroll position.
A matching progress event advances only that analysis to running. A returned
terminal summary, successful deletion, or matching terminal analysis event
removes only that exact ID. Omission from a later History page does not remove
an Active entry because search, filters, and the 50-record page limit can omit
an otherwise unfinished saved analysis.

The TUI History route uses the same certified SQLite records and closed filters
as the noninteractive history commands. Disabling automatic history does not
hide records already stored. It shows at most the newest 50 matching records,
labels that value as `Showing N` rather than a total count, and includes each
record's status, ordered checks, save state, display name, and timestamp.
Each summary occupies exactly one terminal row. The display name is clipped at
an extended-grapheme boundary to the remaining terminal-cell width, so wide
Unicode cannot wrap into another record.
Status cycles through all, queued, running, succeeded, failed, and partial;
check cycles through all, AI detection, and plagiarism. Filter changes reload
immediately. Search applies the literal query only when Enter is pressed. One
history operation may be pending at a time, so repeated activation cannot
duplicate reads, mutations, exports, or billable reruns. Selection is owned by
analysis ID and survives a reload when that ID remains present.

Opening a selected record loads `canonical_analysis(ID, false)`. Retained text,
original paths, and extracted text never enter TUI state or rendered cells.
Detail and every diagnostic still pass through terminal sanitization.
Completed Analyze results and loaded History detail expose every ordered AI
segment and plagiarism match through one ID-owned result viewport; projection
never truncates evidence. Result paging budgets rendered terminal rows at the
active content width. Provider-authored text is split at extended-grapheme
boundaries without clipping, and continuation rows remain navigable even when
one evidence value is taller than the viewport. Up and Down move one result
row, PageUp and PageDown move six rows, and Home and End move to the first and
last row. Vim adds
`k`/`j`, Ctrl+u/Ctrl+d, and `gg`/`G`. Loading a different analysis starts at
its first result row. Tab and Shift+Tab leave or enter the viewport.
After ordered evidence, the shared projection shows canonical provider,
version, aggregate task IDs, bulk ID, submission and completion times, then
per-check task IDs in canonical check order. It omits absent identities and
sanitizes every upstream value before rendering. Save state and actions follow
that provenance block.

Rerun is a focused action labeled as billable; activating it is the explicit
request and does not add a second confirmation. It reconstructs and validates
the retained text inside the shared history/analysis module before credential
resolution, creates a fresh analysis with `rerun_of`, always requests no public
dashboard link, and never implies manual save. The current automatic-history setting
still determines whether a successful rerun is saved automatically.

Delete exists only in the selected record's contextual action menu. Enter on
Delete opens a confirmation whose default is Cancel, a second Enter therefore
cancels, `Y` confirms, and `N` or Escape cancels. A bare `d` never mutates
history. The selected row remains visible until the real deletion returns.
After either a clean deletion or a deletion that committed before a WAL
checkpoint warning, History reloads the current criteria so the screen matches
the committed database. A pre-commit failure preserves the row and shows one
sanitized notice.

Export means all certified history records. Its overlay defaults to JSONL,
redacted content, and Cancel; Markdown is the other format. Full content
requires a separate explicit confirmation. Confirming an export restores the
terminal and leaves the interactive loop before writing any stdout byte, then
uses the same certified streaming exporter as `history export`. A history
failure before output exits 7 with no export prefix. An output or flush failure
exits 1 and emits no secondary stdout document. Cancelling stays in History and
writes nothing.

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

`--save` persists one analysis locally even while automatic history remains
disabled (product-spec section 10, docs/history-contract.md, and ADR 0004).
Its observable semantics are locked:

- Persists the completed local envelope: the input as submitted (durable
  `input_json` retains the plaintext authorized by manual save or enabled
  automatic history even when the primary canonical output omits it; history
  show redacts it again unless `--include-input` is given), the terminal
  result or the canonical check error, typed lifecycle
  state, and current observation identity. Persistence runs after the
  projection content is fixed and before the primary output render; the
  projection then renders the honest `save_state` the store committed.
- `--save` and `--detach` are mutually exclusive. A detached detection has
  only an accepted queued/running snapshot, not the completed envelope that
  `--save` promises, so Clap rejects the combination before credentials,
  network, or history work (usage error, exit 2).
- A saved analysis reports `saved_manual`; one written under the contracted
  automatic gate (`history.enabled = true`) reports `saved_history`; every
  unsaved analysis stays `ephemeral`. The save-state field is already part
  of the canonical analysis schema, so no schema-major change occurs.
- The write is durable per invocation: one run inserts each newly analyzed
  input exactly once (its local ID is generated for that invocation, so an
  argument listed twice analyzes twice and rows twice), in invocation order,
  and the process exits only after the store handle is released, leaving no
  WAL or shared-memory sidecar behind a closed store.
- A repeated-file run never drops an envelope: every completed member
  (succeeded or billable-failed) renders exactly once, in invocation order,
  with only its own `save_state` changed by the save outcome. A member whose
  save failed stays in the rendered series with `ephemeral`; a member that
  saved reports its committed state. One member's store failure changes no
  other member's envelope content, and every completed member persists to
  the store before any primary output is written (an early store failure
  never suppresses the persisted row of a later member that did save).
- A manual save failure is a canonical `local_history` error: the explicit
  request could not be honored, so the command fails after the remote result
  with the exact store error code (for example `insecure_history_permissions`
  or `history_write_failed`), category-derived exit 7, and every member's
  envelope keeping the honest save state it actually committed (a member
  that saved stays `saved_manual`; the unwritten members stay `ephemeral`).
  An upstream result is never turned into a failure and never dropped
  silently. When the primary render itself fails (for example a closed
  stdout), the primary render failure wins: the process reports the render
  failure (exit 1), never masking it behind the save-failure exit, and this
  precedence binds inside one outcome chain: after a primary render has
  already failed, attaching the post-primary save failure can never
  overwrite the general render-failure exit 1 with the save failure's
  category-derived exit 7 on any error surface (JSON or text); the exit 7
  applies only when the primary output honestly rendered (or was written
  successfully) first.
- Automatic history write failures surface exactly once per invocation: one
  sanitized warning on stderr covers the whole automatic save flow for that
  command (one detect run, one bulk submission or read, one task read),
  regardless of how many analyses or bulk children were being saved, and
  never turns a successful remote analysis into a failure; the affected
  analyses keep `ephemeral`. An automatic save under a data directory whose
  owner-only protection cannot be established or verified follows the same
  one-warning rule. Explicit `--save` in that situation fails with
  `insecure_history_permissions` and the process does not open or create the
  database. Existing or raced terminal symbolic-link/reparse aliases at the
  database WAL and shared-memory sidecar names inside that protected history
  directory fail closed without mutating their targets. On Windows, pinned
  no-follow sidecar handles exclude rename and deletion for the SQLite
  connection lifetime and release immediately before SQLite closes. SQLite's
  already-open sidecar handles also exclude delete sharing, preserving object
  identity through that sequential handoff and final cleanup. This boundary
  assumes the already-required owner-only history directory; it does not claim
  protection from arbitrary operations by the same owner or from pre-existing
  hard links (docs/history-contract.md).
- When automatic history is disabled and no explicit `--save` is given, the
  process does not open or create the history directory or database at all.
- When automatic history is enabled, `detect --detach` likewise leaves its
  accepted queued/running snapshot ephemeral and does not open or create the
  history directory or database. It emits no history warning because no
  persistence was attempted. The same rule applies to resumed task reads:
  automatic history persists `task status` and `task wait` observations only
  after the canonical analysis is terminal (`succeeded`, `failed`, or
  `partial`). A queued or running `task status` snapshot remains ephemeral,
  does not open history, and emits no history warning. `task wait` persists
  only the terminal observation it returns. Manual `detect --save` semantics
  are unchanged.

`--save` exists only where the normative grammar lists it: the analysis
commands (`detect`, and the planned `plagiarism`/`analyze` on their phase).
The bulk and task surfaces carry no `--save`; their completed work persists
only under the `history.enabled = true` automatic gate. There is no
`history save` command: the section 14.5 grammar is closed, so that spelling
is rejected as an unknown argument (Clap usage error, exit 2) before any
billable work, exactly as any other unknown flag is.

Retried and resumed work never duplicates rows: persistence keys on the local
`anl_` identity generated for one invocation, a bulk child keys on its
`(bulk_id, bulk_index)` pair, and observation updates for one check upsert
the single `upstream_tasks` row keyed by `(analysis_id, check_kind)`
(docs/history-contract.md schema v1). No API keys, auth headers, raw response
bodies, or extra diagnostics are persisted; the stored JSON columns carry
exactly the canonical schema-major-1 envelopes plus the contracted search
payload (input text when included or available locally, filename, result
headline, source URLs), nothing else.

The durable row the store keeps and the envelope output of any one read are
distinct layers, and their identities are never conflated:

- Output identity. Every emission of one analysis or bulk collection keeps
  the section 4.6 fresh-read identity: a read generates its envelope `anl_`
  or `bulk_` identity fresh for that invocation, because with no retained
  authorship record the read cannot claim the original submission's local
  identity, and `submission_outcome` on a resumed emission stays `accepted`
  exactly as section 4.6 locks.
- Stored-row identity. One remote operation owns at most one stored row,
  reconciled by the observed upstream identity: repeated `task status` or
  `task wait` reads of one upstream task refresh the one analysis row keyed
  by that task identity, and every `bulk submit`, `bulk submit --wait`,
  `bulk status`, or `bulk wait` of one upstream bulk job refreshes the one
  `bulk_collections` row keyed by `upstream_bulk_id` together with its
  children (keyed by their `(bulk_id, bulk_index)` membership) and
  observation rows, atomically and without duplicates. A first observation
  inserts the row with the fresh `anl_` or `bulk_` identity minted for that
  read. Output renders the fresh-read identity per section 4.6; persistence
  reconciles onto the stored row without fabricating outbound save state.
  The fresh `task status`/`task wait` emission itself carries the read's own
  fresh-read identity and the read's own save outcome: it reports
  `saved_history` when this observation persisted, and `ephemeral` when it
  did not (automatic history disabled, or a warned automatic failure). It
  never claims the prior row's `saved_manual` state, because this read did
  not perform a manual save.
- Atomic, database-enforced uniqueness. The reconciliation above is one
  `HistoryStore`-owned atomic unit (docs/history-contract.md): the
  prior-row lookup, the merge, and the insert-or-refresh commit inside one
  immediate SQLite write transaction, and the schema v1 uniqueness
  (`bulk_collections.upstream_bulk_id UNIQUE`, and `upstream_tasks`
  `UNIQUE (check_kind, upstream_task_id)`) enforces the one-row rule at
  the database. Two Pangram CLI processes observing the same upstream
  task or bulk job concurrently therefore serialize on the write lock
  and converge on exactly one stored row with its children and
  observation rows exactly once; a conflicting write rolls its whole
  batch back rather than duplicating a durable row. The automatic-history
  one-warning rule binds a whole invocation across its phases (section
  14.2 below): one bulk command in which both the observed-children read
  and the store write fail still emits exactly one `warning:` line.
  Exact-v1 validation includes the complete SQLite index semantics for
  every primary-key, unique-identity, and named index: expected
  uniqueness and origin, `partial = 0`, and only the contracted ordered
  `key = 1` columns with `BINARY` collation and the contracted sort
  direction. Partial, expression, extra-key, wrong-origin, wrong-direction,
  `NOCASE`, or custom-collation near misses fail before mutation as
  `history_corrupt`, preserving the database bytes.
  The probe also compares the complete deterministic `sqlite_master.sql`
  catalog with the catalog produced by executing the compiled schema-v1 body
  in an isolated in-memory connection to the same bundled SQLite engine.
  Comparison normalizes only SQL keyword case, insignificant whitespace and
  comments, optional trailing semicolons, and equivalent quoting of the exact
  contracted identifiers. It does not erase semantic tokens. Hidden
  `MATCH`, `DEFERRABLE` / `INITIALLY`, primary-key or unique-constraint
  `ON CONFLICT` policies, additional or changed FTS5 options, and any extra,
  missing, or altered table, index, virtual-table, or SQLite-owned FTS object
  therefore fail before mutation as `history_corrupt`.
- Serialized first open. Before classifying or initializing the history
  schema, every opener takes an `IMMEDIATE` SQLite transaction. From an
  absent path, one process atomically commits the exact schema v1 together
  with `user_version = 1`; concurrent processes wait and then validate that
  committed schema instead of misclassifying the transient zero version as
  corruption. A protected zero-byte file left before schema creation is
  initialized through the same path. A `user_version = 0` database that
  already contains any schema object is incompatible and fails closed before
  mutation. Interrupted initialization rolls back the whole schema unit and
  never exposes a partial v1 catalog.
- Durable authorship invariance. An observation-based reconciliation
  refresh moves only `status`, `updated_at`, terminal JSON bodies, and a
  terminal-observed `completed_at`. It never overwrites the stored row's
  original `submission_outcome` (a locally authored `terminal` row stays
  `terminal`; it is never rewritten to the observation's `accepted`), never
  rewrites `save_state`, and never discards existing locally stored content
  (`input_json`, input text, filename, or search payload) when the
  observation carries less local information than the stored row already
  holds. A non-terminal refresh also never erases an attested terminal
  body. A refresh carrying neither `result_json` nor `error_json` preserves
  both stored body columns even when it carries `completed_at`; a result
  replaces the stored result and clears a stale error, while an error
  replaces the stored error and clears a stale result. A refresh carrying
  terminal result metadata replaces an older search headline and source URL
  payload when that newer observation supplies those fields; an absent
  incoming field keeps the durable value. Input text and filename remain
  durable-authorship fields: an observation may fill them only when the row
  has none and can never erase or replace locally held input provenance.
  Every typed terminal or observation update first verifies that the target
  owns exactly one synchronized, well-typed FTS payload row. A missing,
  duplicate, or malformed payload fails closed as `history_corrupt` before
  the typed row changes, and the transaction rolls back without mutation.
  A refresh carrying
  both body branches is invalid and the atomic reconciliation fails closed
  as `history_write_failed`. `completed_at` coalesces independently and can
  never, by itself, erase a terminal result or check error. These same rules
  bind standalone refresh, membership refresh, and task-key adoption.
  A standalone task refresh that selects a row through one incoming task
  key must also compare every other incoming key with all evidence already
  owned by that row before mutation: a missing check kind may be added, the
  same kind and ID may refresh, but a different ID for an already-owned
  check kind fails closed and rolls back. Omitted existing kinds remain
  untouched. The same invariant holds under concurrent refreshes because
  lookup, comparison, and mutation share the immediate transaction.
- Accepted bulk children are honest at submission (section 9): an item the
  HTTP 202 acceptance attests with an upstream `task_id` persists as
  `accepted` through the same fresh-read-identity rule of section 4.6 (the
  task ID is real remote identity), with both the accepted task identity
  and the observed upstream bulk identity recorded in evidence and one
  `upstream_tasks` observation row per attested check; an item the
  acceptance reports as failed persists as a terminal-failed child with
  its canonical check error and the accepted upstream bulk identity (so
  the child stays input-carrying and honestly outcome-bearing without
  fabricating a task identity); an item the acceptance attests with no
  task identity at all stays `not_submitted` and queued rather than
  fabricating an identity. A `bulk submit` without `--wait` persists
  exactly this acceptance snapshot (it never fabricates all-queued or
  all-`not_submitted` children over an acceptance that already attests
  task IDs or immediate failures). A later `bulk submit --wait`,
  `bulk status`, or `bulk wait` observation refreshes those same stored
  children in place: the read fetches the documented results window,
  rebuilds each child from its observed state without discarding the
  result document's attested input descriptor, upstream version, task
  identity, or last stage. Running and failed result-page items may expose the
  provider's diagnostic `stage` as canonical `last_stage`, but only after it
  has been reduced to a bounded, single-line, printable ASCII, non-empty
  value. Queued and succeeded items always omit `last_stage`. Domain
  construction, JSON deserialization, and the generated output schema reject
  an empty or unsanitized stage and reject `last_stage` on either inapplicable
  state. Persistence copies a valid value into the child's
  observation evidence when a task identity exists. The read reconciles onto the
  stored `(bulk_id, bulk_index)` membership (the collection deduped by
  `upstream_bulk_id`) so repeated reads never duplicate a collection or a
  child and locally held metadata (identity, `submission_outcome`,
  `save_state`, caller ID, input payload, and the original creation time)
  is preserved exactly as first recorded; only observation fields move. A
  held local JSONL plan may add its truthful `file` origin, basename, and
  locally held plaintext to the child, but never replaces the result
  document's attested hash/count evidence or upstream provenance.
- Task/bulk reconciliation is evidence-driven (docs/history-contract.md
  task-first/bulk-second rule). A standalone `task status`/`task wait`
  read may already have reconciled the one stored row for a task identity
  a later bulk read also attests for one of its children. The reverse
  correlation exists only after a bulk observation has attached that same
  attested task key to the membership child: a standalone task read has no
  bulk membership or other documented correlation evidence, so it remains
  a separate row while the child has no attested task key and must never
  fabricate a match from status, order, content, or timing. After a later
  bulk refresh attaches the task key, subsequent standalone reads reconcile
  onto that membership row. Inside the one write transaction the store resolves each
  bulk child by its membership AND by every attested
  `(check_kind, upstream_task_id)` key together, then: **reuses** the one
  existing durable row when the membership and the task keys agree (or the
  task keys resolve one row and the membership is unoccupied), keeping its
  first-recorded identity, authorship, `save_state`, local input and FTS
  payload, and creation time, adding the `(bulk_id, bulk_index)`
  membership to a previously standalone row, and moving only observation
  fields; leaves a task-less membership child and an uncorrelated direct
  task read separate; and **fails closed** with `history_corrupt` and a full
  atomic batch rollback when a later bulk observation would assign the
  direct row's task identity to that distinct task-less membership. This
  ambiguous provenance must never transfer, delete, merge, or rekey either
  row's evidence. Other candidate conflicts fail closed with
  `history_write_failed` and a full atomic batch rollback (the task keys resolve more than
  one distinct row, the task-key row differs from the membership row, or
  the membership row already attests a different overlapping set of task
  keys, including another task ID for the same check kind, or one candidate
  itself attests two IDs for one check kind). The store never deletes,
  merges, or rekeys an unrelated row to force a fit, and no duplicate
  analysis or observation row ever persists.

First enablement of durable plaintext storage is always acknowledged
(ADR 0004). The transition of `history.enabled` from unset or `false` to
`true` prints exactly one direct plaintext warning on stderr: that history
stores submitted content and results unencrypted in the local data
directory. The warning names the storage location class and the plaintext
fact only; it never echoes submitted content, results, or credentials. It
is an advisory stderr note, so the enabling command still exits 0 and
stdout stays the canonical machine-readable envelope (noninteractive
automation is never broken or prompted). Re-confirming an already-enabled
`true` (idempotent re-set) prints nothing; disabling (`false`) prints nothing.

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
Pangram's Bulk API does not document a public-dashboard-link request or
response field, so bulk submission has no `--public-link` option.

`bulk results` exit semantics follow section 12: a successful page read
(one explicit `--offset`/`--limit` window, or the offset-0 no-limit
fetch-all) exits 0 regardless of failed children on the returned window,
because it is a successful retrieval and one page is not authoritative for
whole-job terminal state; `bulk status` and `bulk wait` own the job-outcome
exit mapping. A fetch-all read emits one canonical aggregate page whose
window metadata reports `offset: 0` and `limit: max(1, total_items)`
(section 9.1): the complete set, not the 100-item walk granularity.

### 14.4 Task

```text
pangram task status ID
pangram task wait ID [--timeout DURATION] [--progress ...]
```

`ID` is either an opaque non-empty upstream Pangram task ID or a canonical
saved local `anl_` UUIDv7 ID resolved under section 4.6.

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

`history list` and `history search` return an ordered
`AnalysisSummaryPage`, not fabricated `Analysis` values. Each summary contains
only the stored local analysis ID, parent status, ordered check kinds,
save-state, input kind, optional display name, and creation timestamp.
`history show` is the only stage-1 read that returns one complete canonical
analysis.

Before list or search returns summaries, the store validates every
authoritative check row in the same SQLite snapshot as the summary query. The
check indexes and kinds must form the complete canonical order declared by
the parent `check_count`; every status and kind must be known; queued/running
rows have no body, succeeded rows have exactly one kind-correct result, and
failed rows have exactly one canonical error. The parent status must equal the
status derived from those complete rows. Missing, extra, malformed,
misordered, or inconsistent rows fail closed as `history_corrupt`. Validation
uses a bounded set query over the snapshot, not one query per summary.

History reads do not consult `history.enabled`: disabling automatic writes
stops future automatic retention but never hides records already stored.
When the history database does not exist, list and search return an empty
summary page and clear succeeds as an empty mutation without creating a
database. Show and delete of an absent record fail with
`history_unavailable`, the existing canonical mapping for a missing local
history row. Reads never create a missing database.

`--limit` is a strict positive ASCII decimal in `1..=1000`, defaults to 50,
and is validated before configuration or database access. `STATUS` is one of
`queued|running|succeeded|failed|partial`; `CHECK` is one of
`ai_detection|plagiarism`. Results are ordered by `created_at` descending and
then local analysis ID ascending, so timestamp ties are deterministic.

`history search QUERY` treats QUERY as literal user text, tokenizes it with
the FTS5 `unicode61` tokenizer used by the index itself, quotes and
escapes every token, and joins the resulting terms with `AND`. Raw FTS5
operators, column selectors, quotes, parentheses, prefix markers, and other
syntax supplied by the caller never acquire query semantics. A query with no
searchable token returns an empty page.

`history show` redacts retained plaintext by default. `--include-input`
restores the stored `text`, `path`, and `extracted_text` input fields; all
other canonical fields and every ordered check remain present. Stored JSON is
validated once on read; malformed input, check, result, or error values fail
closed as `history_corrupt`. The parent row retains the expected check
cardinality, so deleting one row from a two-check analysis is corruption
rather than a valid one-check projection.
An absent optional input descriptor is omitted from canonical output; it is
never encoded as JSON `null`. The typed input discriminator and hash columns
must agree with the canonical input JSON, and retained text must reproduce its
recorded SHA-256, UTF-8 byte count, and Unicode-whitespace word count.
Every task-evidence row must match exactly one authoritative check of the same
kind; unmatched or malformed task evidence is `history_corrupt`. Bulk
membership is valid only when both the collection ID and a nonnegative index
within that collection's declared item range are present. A partial or
out-of-range membership fails closed on full and summary reads.
The complete reconstruction uses one deferred SQLite read snapshot for the
parent, its one synchronized FTS row, ordered checks, task evidence, and bulk
provenance. Export uses the same snapshot rule across its complete result.
Normal WAL writers remain unblocked. The canonical
`provenance.submitted_at` value is read from its independent nullable stored
column exactly; it is never synthesized from `created_at`, and resumed
observations without local authorship keep it absent.

Before list or search returns any page, the same read snapshot proves that
every analysis has exactly one well-typed FTS row and that no orphan FTS row
exists. Missing, duplicate, or malformed search state fails closed as
`history_corrupt` without mutation.

Delete and clear require `--yes` unless stdin, stdout, and stderr are all
TTYs. A noninteractive or CI invocation without `--yes` fails as a usage
error before configuration resolution or database access. An interactive
decline leaves the database unchanged and exits 130. A confirmed mutation
first certifies every collection, analysis, authoritative check, task,
membership, lineage link, input/provenance value, lifecycle timestamp, and
exact FTS projection in the mutation transaction's snapshot. Any logical
corruption fails closed as `history_corrupt`: no row changes and no
post-commit checkpoint runs. A valid store then updates analyses, check rows,
task evidence, bulk relationships, and FTS in one transaction and follows the
WAL-truncation rule in the history contract.

Deleting an analysis clears any dependent `retry_of` or `rerun_of` links
atomically while preserving those dependent analyses, their authoritative
checks, task evidence, bulk membership, and synchronized FTS rows. Schema v1
therefore declares both lineage foreign keys `ON DELETE SET NULL`; there is no
migration because Phase 4 has not shipped.

Stage 2 export redaction removes submitted/extracted text, original paths,
segment text, plagiarism matched text, and public dashboard links while
retaining hashes, byte/word counts, filenames, segment classifications,
numeric scores, offsets/counts and humanizer evidence, states, timestamps,
and lineage. Each valid HTTP(S) plagiarism source URL is reduced to only its
normalized hostname: userinfo, port, path, query, and fragment are discarded.
Invalid URLs, non-HTTP(S) schemes, and hostless values are omitted rather than
copied or partially sanitized. Stage 2 rerun
requires retained plaintext, creates a fresh analysis with `rerun_of`, resets
public-link creation, and follows the effective automatic-history policy:
it is saved automatically only when `history.enabled = true`; an explicit
rerun does not imply manual `--save`.
Before credential resolution or any network operation, rerun reconstructs the
retained text through the canonical text-input validator and verifies its
input kind, SHA-256, UTF-8 byte count, and Unicode-whitespace word count.
Corrupt or no-longer-resolvable local input fails locally and sends no POST.

Export is a raw primary stdout surface. A stdout write or flush failure
(including a closed pipe or full device) is therefore a primary output
failure and exits 1. It is never classified as `history_write_failed` (exit
7), and after any raw export byte may have been written the process emits no
secondary JSON or text error envelope that could corrupt or misdescribe the
stream.

`history rerun` uses the shared SIGINT bridge and reset discipline used by
`detect` and task waits. An interrupt before a billable send reports the
canonical local interruption. Once the POST may have been issued, interruption
exits 130 and reports the canonical ambiguous/reconciliation identity without
replaying the POST. Primary-output write failure still takes precedence over
the interruption exit.

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
