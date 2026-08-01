---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Added

Added the canonical output-projection layer with a single owner: every adapter
renders JSON, JSONL, TOON, Markdown, and pretty output from the same typed
canonical envelope. JSON, JSONL, and TOON are machine projections of the
canonical JSON value and never sanitize, truncate, or reorder content; JSONL
writes one complete envelope per line in input order for repeated-file work.
Markdown and pretty render the same typed data for humans, escape Markdown
structure, and replace terminal control characters (C0, DEL, and the C1 range)
with U+FFFD so untrusted upstream text, URLs, and payload fields cannot inject
terminal sequences or forge report structure. Pretty color is opt-in through a
caller-resolved policy, wraps only trusted enum/status markers, and never
colors payload text. Every projection streams into the caller's writer and
propagates write and flush failures instead of claiming success.

## Security

Machine projections carry present privacy fields byte-exactly and never invent
content, while human projections surface present input content only after
control-character sanitization, so the output layer cannot leak omitted input
text or let hostile content masquerade as terminal control or markup.

The minimum supported Rust version rose from 1.85 to the exact minimum 1.87.0
because the pinned `toon-format 0.5.0` codec (MIT, `default-features = false`)
requires `unsigned_is_multiple_of`, stabilized in Rust 1.87.0.

This fragment uses `patch` for both packages, matching every existing
`.tegami/` fragment: the projection layer is internal infrastructure consumed
by adapters that are not yet enabled as a user-facing workflow.
