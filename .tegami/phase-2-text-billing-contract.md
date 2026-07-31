---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Changed

Pangram CLI now pins Pangram 4 as the only production text model through the
documented request selector `model` set to `pangram-4`, so text analysis never
falls back to Pangram's temporary Pangram 3 default routing (retired
2026-09-30). Pangram 4 text analysis is estimated at one billable unit per
started 100-word block with a minimum of one, and that canonical estimate is
the single rule later used by CLI preflight and the analysis module.

Bulk submission remains blocked until Pangram documents its Pangram 4 bulk
selection, billing, and request ceiling.

This fragment uses `patch` for both packages, matching every existing
`.tegami/` fragment in this repository: `minor` is reserved for a release
that turns the Pangram 4 text detection workflow on, and this packet lands
contract and domain foundation without enabling any new CLI capability.
