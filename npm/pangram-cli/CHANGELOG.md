## @microck/pangram-cli@0.1.1

### Fixed

Corrected command examples and generated reference pages so they match the shipped CLI and MCP contracts.

### Added

Public landing page for the Pangram CLI.

### Changed

Aligned the README with the current release and installation paths.

### Security

Updated the documentation site to Next.js 16.3.4, which carries the fixes for GHSA-p293-qw3h-jr36 and GHSA-2xp9-vwfh-vxw4.

### Added

`tui.highlight` (default `false`) paints AI-detection segment text in its evidence tone in the TUI result. A `Highlight` toggle in the Analyze inspector changes it while a result is shown.

### Changed

The TUI result view groups each check under a heading, gives every AI segment its own label, text, and measurement rows, and mutes identities and timestamps so evidence is readable instead of one long line per segment.

AI, AI-assisted, and human evidence use the same red, amber, and green tones as the Pangram dashboard, and a word-proportional bar shows where each kind of text sits in the document.

## @microck/pangram-cli@0.1.0

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
