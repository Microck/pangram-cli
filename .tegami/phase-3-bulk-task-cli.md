---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Added

The Pangram CLI now activates the contracted bulk and task command surfaces.
`pangram bulk submit [JSONL_PATH|-] --max-billable-units N [--dry-run]
[--wait] [--format FORMAT] [--progress MODE]` validates the whole JSONL file
before any credential or network work, enforces the billable-unit ceiling,
reports the canonical dry-run reconciliation tuple (local bulk ID, plan
SHA-256, estimate) at exit 0 without credentials or network, and submits
through the shared analyzer. `pangram bulk status ID`, `pangram bulk wait ID
[--timeout DURATION] [--progress MODE]`, and `pangram bulk results ID
[--offset N] [--limit N] [--format FORMAT]` read the canonical collection
and page shapes, with fetch-all paging as the default full read.
`pangram task status ID` and `pangram task wait ID [--timeout DURATION]
[--progress MODE]` observe one Pangram 4 task, reconciling a remotely
authored record honestly: the analysis marks `submission_outcome:
accepted`, omits `provenance.submitted_at`, and derives its input
descriptor only from the terminal document Pangram attested.

Every envelope routes through the canonical projection and error owners:
a partial collection or analysis exits 3, a local observation failure
after acceptance exits 1 through an accepted status-changed envelope, and
SIGINT during a wait exits 130 with the identity tuple on stderr. Progress
on `bulk wait` and `task wait` follows the shared `auto|never|jsonl`
policy. Bulk submission carries no `--public-link` option, matching the
documented Bulk API.
