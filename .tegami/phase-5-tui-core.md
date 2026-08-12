---
packages:
  "cargo:microck-pangram-cli": minor
  "npm:@microck/pangram-cli": minor
---

## Added

Added the first full-screen terminal interface for interactive Pangram 4 text
analysis. The TUI includes regular and Vim navigation, responsive wide and
narrow layouts, first-use credential and update settings, active analysis
progress, sanitized results, persistent settings, and reliable terminal
restoration after normal exit, interruption, handled I/O failure, or panic.

Added a local History route backed by the certified SQLite store. It supports
literal search, closed status and check filters, redacted detail, cancel-safe
delete and export confirmations, streamed JSONL or Markdown export after the
terminal is restored, and focused billable reruns through the shared Analyzer.

The intro control plane implements frequency, motion, timing, skip, and atomic
seen-state rules. Artwork remains suppressed until approved source geometry
and logo rights are recorded, so no unapproved Pangram or Droid material ships.

## Fixed

Prevented repeated TUI activation from starting duplicate billable analysis or
rerun work. History reloads can no longer consume a confirmed delete or export,
erase a cleanup warning, or combine a fresh summary with stale record detail.

Kept the focused composer edit position visible while long or multiline text
scrolls. History display names now clip on whole Unicode graphemes by terminal
cell width, so wide names cannot wrap into another record.

Kept bracketed paste as one literal composer edit, so pasted tabs, line breaks,
and navigation-like characters cannot activate controls or submit work. Long
result evidence now pages by terminal row without clipping, and completed
results show canonical provenance and upstream task identities.
