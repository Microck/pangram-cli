---
packages:
  "cargo:microck-pangram-cli": minor
  "npm:@microck/pangram-cli": minor
---

## Added

Pangram CLI now performs Pangram 4 AI text detection end to end. The
`pangram detect` command and bare input both work: pass literal text
(`pangram 'some text'`), read stdin explicitly (`pangram -`), or pipe content
(`printf 'some text' | pangram`). Literal text, piped input, and repeated
UTF-8 text files are analyzed through the documented Pangram 4 request
selector, and results print as canonical JSON by default, with JSONL, TOON,
Markdown, and pretty projections available through `--format`.

## Changed

Repeated `--file` runs now keep one consistent shape in every output format:
JSONL stays one envelope per file, while an explicitly requested
single-document format (JSON, TOON, Markdown, pretty) wraps the ordered
results in one envelope instead of failing after the analysis completed.

Cancelling a submission is now reported honestly. Interrupting before the
request is sent completes no remote action; interrupting after the request
is sent reports the canonical "acceptance unknown" outcome with
reconciliation guidance, because Pangram may have received it, and the
request is never silently resent.
