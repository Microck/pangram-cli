---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Changed

AI detection results in pretty, Markdown, and TUI views now caution that scans below 150 words are less certain, following Pangram 4.1's published interpretation guidance. Scores and machine-readable schemas are unchanged; undocumented dashboard uncertainty ranges are not fabricated.
