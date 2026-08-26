---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Fixed

`--timeout` now bounds synchronous plagiarism checks and both members of a
combined analysis. A timed-out request that may have reached Pangram remains
an explicit unknown submission outcome and is never retried automatically.
