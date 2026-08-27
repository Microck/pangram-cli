---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Changed

Platform support now matches the native release evidence: glibc 2.17 on Linux,
macOS 15 on both architectures, and Windows Server 2025 on x86-64.
