---
packages:
  "cargo:microck-pangram-cli": minor
  "npm:@microck/pangram-cli": minor
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
a terminal `partial` collection or analysis reported by `bulk status`,
`bulk wait`, or `task status`/`task wait` exits 3, a local observation
failure after acceptance exits 1 through an accepted status-changed
envelope, and SIGINT during a wait exits 130 with the identity tuple on
stderr. A successfully normalized mixed-acceptance `bulk submit` and a
successful `bulk results` page or fetch-all read exit 0 regardless of
failed children (contracts.md 12/14.3); exit 3 applies only to the terminal
observation outcome. Progress on `bulk wait` and `task wait` follows the
shared `auto|never|jsonl` policy. Bulk submission carries no `--public-link`
option, matching the documented Bulk API.

## Fixed

A `bulk submit` accepted without `--wait` now projects the truthful HTTP 202
acceptance snapshot: the validated accepted and immediately failed counters
and the derived collection status (a `queued` collection while accepted work
remains, or the terminal `failed` collection when the 202 rejected every
submitted item). Previously it fabricated an all-queued-zero state over an
acceptance that could already report immediate failures.

A resumed or observed `bulk items`/`bulk results` read of a job this process
did not submit now emits every child analysis with `submission_outcome:
accepted` (never `terminal`), matching the task observed-read contract:
`terminal` is reserved for an operation the caller itself submitted. A
succeeded child whose terminal document carries no normalized text, and any
failed child, correctly normalize as valid accepted children instead of
failing as `upstream_contract_changed`.

A successful `bulk results` page or fetch-all read exits 0 regardless of
failed children on the returned window, preserving them as failed children:
one page is not authoritative for whole-job terminal state, and `bulk
status`/`bulk wait` own the job-outcome exit. A fetch-all read reports one
canonical aggregate window (`offset: 0`, `limit: max(1, total_items)` bounded
by 1,000, no `next_offset`) representing the complete reassembled set rather
than the 100-item walk granularity.
