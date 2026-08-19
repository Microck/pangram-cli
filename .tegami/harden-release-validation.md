---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Fixed

Timeouts now cover binary file detection, and combined cancellation retains an
ambiguous plagiarism submission for safe reconciliation. The POSIX installer
rejects non-glibc Linux hosts before downloading a GNU build. Draft releases
pin their tag to the built commit and use the exact Tegami changelog section.
