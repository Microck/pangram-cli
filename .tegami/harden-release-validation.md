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
Interrupted combined history reruns now retain that reconciliation error while
returning the documented exit 130. Fresh combined analysis now does the same.
Documentation CI runs the complete validator, and regeneration removes stale
public Markdown when a source page moves or disappears.
