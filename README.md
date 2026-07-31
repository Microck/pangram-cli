# Pangram CLI

`pangram` is a planned terminal client for Pangram AI detection and plagiarism
checking. It is designed for interactive TUI use, shell pipelines, and AI
agents that need stable JSON and MCP contracts.

> [!NOTE]
> This repository has a Phase 0 executable contract scaffold. The compiled
> binary currently exposes only `--help` and `--version`. The analysis, TUI, and
> MCP commands below remain planned and are not available yet.

The project will use Pangram's documented API-key-authenticated REST endpoints.
It will not use browser sessions, private dashboard routes, or scraping.

## Why

Pangram's web application is useful for interactive checks, but terminal and
agent workflows need a different interface:

- one command surface for AI detection, plagiarism, files, and bulk work
- JSON-first output with stable errors and exit codes
- an interactive TUI with optional local history
- typed MCP tools with billing, filesystem, and mutation safeguards
- signed native releases with package-manager-aware updates

AI detection is the primary planned workflow. Plagiarism is planned as an
independent check and as an explicit combined analysis.

## Planned quickstart

Install a release, configure a Pangram API key, and open the TUI:

```bash
pangram auth
pangram
```

Analyze text in a shell pipeline:

```bash
printf 'Text to analyze' | pangram
```

Request human-readable output:

```bash
pangram detect --format pretty 'Text to analyze'
```

The default noninteractive output is JSON. Repeated files default to JSONL so
each result remains independently streamable. `--format toon`, `markdown`, and
`jsonl` provide alternate projections of the same canonical result.

## Authentication

The guided setup command will be:

```bash
pangram auth
```

Persistent noninteractive setup will also be supported:

```bash
pangram auth set --api-key VALUE
```

Agents can avoid argv exposure while keeping persistent setup:

```bash
printf '%s\n' "$PANGRAM_API_KEY" | pangram auth set --api-key-stdin
```

Create a Pangram API key and add prepaid credits:

https://www.pangram.com/apikey

`PANGRAM_API_KEY` will override the stored key for ephemeral CI and agent
environments. Passing a key on the command line can expose it through shell
history and process listings.

## Planned command surface

| Command | Purpose |
| --- | --- |
| `pangram` | Launch the TUI when all standard streams are TTYs, or detect piped/provided text |
| `pangram detect` | Run AI detection |
| `pangram plagiarism` | Run plagiarism checking |
| `pangram analyze` | Run AI detection and plagiarism together |
| `pangram bulk` | Submit and inspect asynchronous bulk detection |
| `pangram task` | Inspect or wait for a Pangram text task |
| `pangram history` | Manage optional local analysis history |
| `pangram auth` | Configure and inspect API-key authentication |
| `pangram mcp` | Run or install the typed stdio MCP server |
| `pangram agent` | Print the embedded agent usage guide |
| `pangram skills` | List and load version-matched embedded skills |
| `pangram config` | Inspect and update non-secret configuration |
| `pangram doctor` | Run local, non-billable diagnostics |
| `pangram completions` | Generate shell completions |
| `pangram update` | Check for or install an eligible direct update |

See [the CLI contract](docs/contracts.md) for the approved grammar, formats,
errors, and exit codes.

## Agent-native behavior

The CLI is being designed so agents do not need to scrape help text or parse
terminal decoration:

- JSON is canonical and remains the default outside the TUI.
- stdout contains primary results; stderr contains progress and diagnostics.
- errors include stable codes, categories, retryability, and recovery actions.
- the stdio MCP server targets protocol version `2026-07-28`.
- MCP tools map directly to analysis operations rather than spawning the CLI.
- billable MCP tools are marked non-idempotent and require explicit bulk limits.
- file tools are absent by default and use only directories approved through
  repeated `--allow-file-root PATH` startup options.
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

The shipped runtime will use Rust. A single deep analysis module will own
Pangram HTTP behavior, polling, normalization, retries, and task state. The CLI,
TUI, and MCP server will be adapters over that module.

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
