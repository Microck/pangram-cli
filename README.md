# Pangram CLI

`pangram` is an unofficial terminal client for Pangram AI detection and
plagiarism checking. It is designed for interactive TUI use, shell pipelines,
and AI agents that need stable JSON and MCP contracts.

> [!NOTE]
> The runtime is mid-build: the compiled binary currently ships working
> `auth` (persistent API-key setup and status), `config` (list/get/set/path),
> `doctor` (local diagnostics), `detect` (Pangram 4 text AI detection,
> including bare literal text, `-`, and piped stdin), `bulk` (submit, status,
> wait, and results for asynchronous bulk detection), `task`
> (status and wait for one Pangram text task), and `history` (list, show,
> literal-text search, delete, clear, export, and rerun) are compiled and available.
> Plagiarism and combined analysis remain planned and are not available yet.
> The TUI and typed stdio MCP server are compiled and available. Official MCP
> conformance and live Pangram conformance are pending, and public release support
> (including live file and plagiarism
> conformance) is still gated; compiled contract and loopback protocol tests
> are the current correctness gates.

The project uses Pangram's documented API-key-authenticated REST endpoints. It
does not use browser sessions, private dashboard routes, or scraping.

## Why

Pangram's web application is useful for interactive checks, but terminal and
agent workflows need a different interface:

- one command surface for AI detection, plagiarism, files, and bulk work
- JSON-first output with stable errors and exit codes
- an interactive TUI with optional local history
- typed MCP tools with billing, filesystem, and mutation safeguards
- signed native releases with package-manager-aware updates

AI detection is the primary workflow. Plagiarism remains planned as an
independent check and as an explicit combined analysis.

## Quickstart

Install a release, configure a Pangram API key, and run your first detection:

```bash
pangram auth
pangram detect 'Text to analyze'
```

Detect text in a shell pipeline (a bare launch reads piped stdin):

```bash
printf 'Text to analyze' | pangram
```

Open the interactive terminal interface with an all-TTY bare launch:

```bash
pangram
```

Request human-readable output:

```bash
pangram detect --format pretty 'Text to analyze'
```

The default noninteractive output is JSON. Repeated files default to JSONL so
each result remains independently streamable; passing an explicit single-
document format (`--format json`, `toon`, `markdown`, or `pretty`) wraps the
repeated results in one ordered array instead.

## Authentication

Guided setup stores a Pangram API key in the protected local credential file:

```bash
pangram auth
```

Persistent noninteractive setup is also available:

```bash
pangram auth set --api-key VALUE
```

Agents can avoid argv exposure while keeping persistent setup:

```bash
printf '%s\n' "$PANGRAM_API_KEY" | pangram auth set --api-key-stdin
```

Create a Pangram API key and add prepaid credits:

https://www.pangram.com/apikey

`PANGRAM_API_KEY` overrides the stored key for ephemeral CI and agent
environments. Passing a key on the command line can expose it through shell
history and process listings.

## Command surface

Available today:

| Command | Purpose |
| --- | --- |
| `pangram` | Bare literal text or piped stdin runs AI detection; an all-TTY launch opens the TUI |
| `pangram detect` | Run Pangram 4 AI text detection |
| `pangram auth` | Configure and inspect API-key authentication |
| `pangram config` | Inspect and update non-secret configuration |
| `pangram doctor` | Run local, non-billable diagnostics |
| `pangram bulk` | Submit and inspect asynchronous bulk detection |
| `pangram task` | Inspect or wait for a Pangram text task |
| `pangram history` | List, show, search, delete, clear, export, and rerun optional local history |
| `pangram mcp` | Run or install the typed stdio MCP server |
| `pangram agent` | Print the embedded agent usage guide |
| `pangram skills` | List and load version-matched embedded skills |
| `pangram completions` | Generate shell completion scripts |

Planned:

| Command | Purpose |
| --- | --- |
| `pangram plagiarism` | Run plagiarism checking |
| `pangram analyze` | Run AI detection and plagiarism together |
| `pangram update` | Check for or install an eligible direct update |

See [the CLI contract](docs/contracts.md) for the approved grammar, formats,
errors, and exit codes.

## Agent-native behavior

The CLI gives agents structured surfaces so they do not need to scrape help
text or parse terminal decoration:

- JSON is canonical and remains the default outside the TUI.
- stdout contains primary results; stderr contains progress and diagnostics.
- errors include stable codes, categories, retryability, and recovery actions.
- the stdio MCP server targets protocol version `2026-07-28`.
- MCP tools map directly to analysis operations rather than spawning the CLI.
- billable MCP tools are marked non-idempotent and require explicit bulk limits.
- filesystem-path inputs use only directories approved through repeated
  `--allow-file-root PATH` startup options.
- history, public links, configuration changes, and destructive operations are
  separately gated.
- long-running Pangram work uses ordinary typed tools, not the experimental MCP
  Tasks extension.
- an embedded, version-matched skill explains safe command and tool selection.

## Local history and privacy

Local history is disabled by default. When enabled, it stores canonical inputs,
results, task identity, and searchable text in a local SQLite database until
the user deletes them.

Public Pangram dashboard links are also disabled by default and must be
requested for each submission.

See [the product specification](docs/product-spec.md) for the complete privacy,
retention, and unsupported-parity boundaries.

## Architecture

The runtime uses Rust. A single deep analysis module owns Pangram HTTP
behavior, polling, normalization, retries, and task state. The CLI, TUI, and
MCP server are adapters over that module.

Fumadocs will live in a separate TypeScript workspace and consume generated
schemas and reference material rather than duplicating runtime contracts.

See [the architecture specification](docs/architecture-spec.md).

## Documentation

- [Product specification](docs/product-spec.md)
- [Architecture specification](docs/architecture-spec.md)
- [Observable contracts](docs/contracts.md)
- [History contract](docs/history-contract.md)
- [MCP contract](docs/mcp-contract.md)
- [Update contract](docs/update-contract.md)
- [Intro art contract](docs/intro-art-contract.md)
- [External evidence ledger](docs/evidence-ledger.md)
- [Documentation plan](docs/documentation-plan.md)
- [Testing and release plan](docs/testing-release-plan.md)
- [Implementation roadmap](docs/implementation-roadmap.md)

Accepted decisions are recorded under [`docs/adr/`](docs/adr/).

The public documentation destination is `https://pangram.micr.dev`.

## Disclaimer

This project is unofficial and is not affiliated with, endorsed by, or
connected to Pangram Labs, Inc. It is an independent client for documented
Pangram APIs.

Use of Pangram services remains subject to Pangram's terms, billing, retention,
and acceptable-use policies. Detection results are probabilistic evidence and
should be one signal among many, not a verdict.

Public distribution is blocked until Pangram confirms in writing that a
third-party CLI and MCP server may use its documented APIs.

## License

[MIT](LICENSE)
