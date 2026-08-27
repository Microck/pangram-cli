## microck-pangram-cli@0.1.0

### Added

- Analyze text, files, and bulk jobs with Pangram 4 from a JSON-first CLI.
- Check plagiarism alone or alongside AI detection while preserving partial results.
- Use the interactive Ratatui interface with mouse, keyboard, Vim bindings, history, settings, and the approved terminal fox intro.
- Connect agents through a typed stdio MCP server with explicit file, history, mutation, and billing gates.
- Store optional local history in a protected SQLite database with search, export, rerun, and deletion controls.
- Install signed native builds on Linux x64 and ARM64, macOS x64 and ARM64, and Windows x64 through direct installers, npm, Homebrew, or Scoop.
- Generate shell completions for Bash, Zsh, Fish, PowerShell, and Elvish.

### Changed

- Require Pangram 4 for text analysis and estimate usage in started 100-word units before billable bulk work.
- Keep history, public dashboard links, telemetry, and update-network access disabled by default.
- Report ambiguous submissions and partial combined analyses without automatic billable retries.

### Fixed

- Bound synchronous file and plagiarism requests with the configured timeout.
- Preserve the prior executable and install receipt when a signed replacement fails validation or smoke testing.
- Reject unsupported hosts and malformed release archives before installation.

### Security

- Store API credentials with owner-only filesystem protections and keep secrets and submitted content out of logs and diagnostics.
- Verify signed update metadata, archive shape, checksums, target identity, and executable bytes before installation or replacement.
- Sanitize untrusted terminal and Markdown content before human-readable rendering.
