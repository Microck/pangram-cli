---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Fixed

Longer AI detections no longer fail with `upstream_contract_changed` when Pangram reports its new `STAGE_POSTPROCESSING` stage. Any well-formed intermediate stage now keeps polling until Pangram returns a terminal result.

`video`, `audio`, and `youtube` failures now include the underlying tool's error (for example a yt-dlp HTTP 403) in `details.tool_stderr` instead of only "yt-dlp failed."
