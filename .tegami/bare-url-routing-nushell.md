---
packages:
  "cargo:microck-pangram-cli": minor
  "npm:@microck/pangram-cli": minor
---

## Added

`pangram URL` now routes a bare GitHub pull request, GitHub issue, or YouTube URL to `pangram pr`, `pangram issue`, or `pangram youtube`. Flags after the URL go to that command, so `pangram https://github.com/owner/repo/pull/123 --max-billable-units 5` works. Any other bare URL fails with `unsupported_input` instead of being analyzed as text; use `pangram detect` to analyze a URL as text.

`pangram completions nushell` generates Nushell completions, and native release archives now include `completions/pangram.nu`.
