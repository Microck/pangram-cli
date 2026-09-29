---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Fixed

Installing `@microck/pangram-cli` from npm on Apple Silicon Macs includes the native binary again. The 1.2.0 macOS arm64 package could not be published because of an npm registry fault, so npm installs of 1.2.0 on those machines had no `pangram` executable.
