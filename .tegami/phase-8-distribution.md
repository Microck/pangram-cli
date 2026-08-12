---
packages:
  "cargo:microck-pangram-cli": minor
  "npm:@microck/pangram-cli": minor
---

## Added

Defines the Phase 8 signed-update and distribution contract, including an
explicit nonbillable MCP update check, direct-install ownership receipts, and
interactive or `--yes` update confirmation.

Private `0.x` builds keep update networking disabled with an empty production
key ring. Direct installers stay blocked until their exact-byte Ed25519
verification works on every supported clean-machine baseline.
