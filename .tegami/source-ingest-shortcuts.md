---
packages:
  "cargo:microck-pangram-cli": minor
  "npm:@microck/pangram-cli": minor
---

## Added

`pangram video`, `audio`, and `youtube` transcribe media locally with whisper.cpp and run AI detection on the transcript. `pangram pr`, `issue`, and `comments` detect AI writing in GitHub titles, descriptions, and comment bodies.

## Changed

The JSON output schema is now major `"2"`. Text input `origin` adds `transcript` and `github`, and new error codes cover missing tools, model downloads, transcription, and GitHub access.
