---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Fixed

The Windows direct installer now completes receipt-owned replacement before
returning instead of racing a detached self-replacement helper.
