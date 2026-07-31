---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Changed

Pangram CLI now targets Pangram 4 as its only text model. Detection results
include Pangram 4 humanizer evidence, use its three document classifications,
and estimate text billing in started 100-word units.

Text and bulk submission remain blocked until Pangram documents the Pangram 4
request selector and current bulk billing contract.
