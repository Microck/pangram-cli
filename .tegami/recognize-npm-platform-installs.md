---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Fixed

The updater now recognizes npm's platform-package executable paths and returns
the correct npm update command instead of treating those installs as unowned.
