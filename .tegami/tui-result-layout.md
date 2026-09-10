---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Added

`tui.highlight` (default `false`) paints AI-detection segment text in its evidence tone in the TUI result. A `Highlight` toggle in the Analyze inspector changes it while a result is shown.

## Changed

The TUI result view groups each check under a heading, gives every AI segment its own label, text, and measurement rows, and mutes identities and timestamps so evidence is readable instead of one long line per segment.

AI, AI-assisted, and human evidence use the same red, amber, and green tones as the Pangram dashboard, and a word-proportional bar shows where each kind of text sits in the document.
