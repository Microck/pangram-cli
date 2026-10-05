---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Fixed

YouTube transcription now explicitly downloads the best audio-only format instead of relying on yt-dlp's default format selection.
