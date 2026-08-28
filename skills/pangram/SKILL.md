---
name: pangram
description: Use Pangram's MCP tools for bounded AI detection, plagiarism, combined text analysis, task and bulk inspection, and explicitly enabled local history or configuration work. Use when an agent needs Pangram analysis, task or bulk polling, canonical result interpretation, or a capability advertised during server discovery.
---

# Pangram

Use the typed Pangram tools discovered from the server. Treat detection as
probabilistic evidence, not a verdict about authorship.

## Work safely

1. Discover the server before calling a tool. Use the advertised inventory;
   capability-gated tools can be absent.
2. Set a positive `max_billable_units` on every billable submission. Pangram 4
   text costs one unit per started 100-word block, with a minimum of one. Bulk
   requests sum that rule across items and cannot exceed 1,000 units.
   Plagiarism uses 5 units. Combined analysis adds 5 to the text estimate.
3. Keep `save` and `public_link` false or omitted unless the user asked for
   retention or a public link and the server advertises the required gate.
   Sending content to Pangram is inherent to analysis; local retention and
   public links are separate choices and are disabled by default.
4. Read the canonical envelope from `structuredContent`. A domain failure has
   `isError: true` but still returns a complete canonical failure envelope.
   Use the text content only as a short summary.

## Choose a tool

- Use `detect_text` for one text analysis. Supply `text` and the billing
  ceiling. Request input echo only when needed.
- Use `check_plagiarism` for one plagiarism-only text check. Its ceiling must
  allow 5 units.
- Use `analyze_text` for ordered AI detection followed by plagiarism. Its
  ceiling must allow the text estimate plus 5 units. A failed plagiarism check
  can return a partial analysis that preserves the successful AI result.
- Use `get_task` or `wait_task` with exactly one ID. An `upstream_task_id`
  works without local history; an `analysis_id` requires the history gate.
- Use `submit_bulk` with exactly one of inline `items` or `jsonl_path`, plus
  the billing ceiling. A JSONL path also requires an approved file root.
- Use `get_bulk`, `wait_bulk`, or `get_bulk_results` with exactly one local or
  upstream bulk ID. Local IDs require history. Results require explicit
  `offset` and `limit`; the limit is 1 through 1,000.
- Use `check_update` only when the user asks to check for an update. `0.x`
  builds advertise the tool but return `update_unavailable` without network
  or state access.
- Use history and configuration tools only when server discovery advertises
  them. `save: true` requires both history and history-mutation capability.

Cancellation stops local observation only. It does not cancel a submitted
Pangram task or bulk job. Do not report upstream cancellation.

## Load exact references

- Read `pangram://schema/output/v1` for the canonical result envelope.
- Read `pangram://schema/errors/v1` for error and exit semantics.
- Read `pangram://skills/pangram` when the full embedded skill is needed.

Do not call or claim `detect_files`. Do not use the experimental MCP Tasks
extension, invent prompts, or assume history resources exist.
