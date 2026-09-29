# ADR 0010: Source ingest shortcuts

Status: accepted
Date: 2026-09-28

## Context

Users want `pangram video`, `pangram youtube`, and GitHub `pr` / `issue` /
`comments` shortcuts. Pangram's documented file API accepts PDF, DOCX, and RTF
only. Speech and GitHub prose have to become UTF-8 text before detection.

Schema major `"1"` forbids adding `origin` values. Calling source-derived text
`literal` contradicts the argv meaning of that origin.

## Decision

Bump the output envelope to schema major `"2"`.

- `transcript` origin: local media or YouTube audio after local ASR
- `github` origin: GitHub REST title, body, and comment `body` fields
- Envelope `command` remains `detect`
- Ingest never calls Pangram HTTP
- whisper.cpp is a sidecar plus downloaded ggml weights, not linked into
  `pangram`
- `ffmpeg` and `yt-dlp` are user-installed PATH tools
- Review comments include `body` and omit `diff_hunk`

There is no major-`"1"` compatibility path.

## Consequences

- Existing major `"1"` envelopes are not valid major `"2"` documents
- CLI, TUI, and MCP analysis semantics stay in the shared analysis module
- TUI and MCP do not grow ingest tools in this change
