---
packages:
  "cargo:microck-pangram-cli": minor
  "npm:@microck/pangram-cli": minor
---

## Added

Expose the final update command grammar in `0.x` builds with a typed no-network
response until signed self-update begins at `1.0.0`.

Ship direct POSIX and PowerShell installers that verify the signed release,
install atomically, and record ownership without editing PATH.
