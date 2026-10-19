# Pangram CLI documentation

Pangram CLI runs Pangram AI detection and plagiarism checks from a terminal.
It is an unofficial MIT-licensed client.

## Start here

- [Use the TUI](./tutorials/first-tui-analysis)
- [Build a JSON pipeline](./tutorials/first-json-pipeline)
- [Connect an MCP client](./tutorials/first-mcp-client)

Content sent for analysis goes to Pangram. Local history is off by default.
Every billable MCP submission requires a caller-supplied cost ceiling.

## Pangram 4.1 interpretation guidance

Human-readable AI results warn when the returned segments total fewer than 150
words. This reflects the new short-scan guidance; it does not change the model's
score or imply that longer scans are proof of authorship. The announced dashboard
AI-percentage range is not yet part of the documented public API contract, so the
CLI does not manufacture one from segment confidence.
