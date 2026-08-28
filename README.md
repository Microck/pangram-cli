<p align="center">
  <img src="docs-app/public/landing/images/pangram-cli-logo.svg" width="120" alt="Pangram CLI logo">
</p>

<h1 align="center">pangram</h1>

<p align="center">
  <a href="https://github.com/Microck/pangram-cli/releases"><img src="https://img.shields.io/github/v/release/Microck/pangram-cli?display_name=tag&style=flat-square&label=release&color=000000" alt="release badge"></a>
  <a href="https://registry.npmjs.org/@microck/pangram-cli"><img src="https://img.shields.io/npm/dt/@microck/pangram-cli?style=flat-square&label=downloads&color=000000" alt="npm downloads"></a>
  <a href="https://github.com/Microck/pangram-cli/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/Microck/pangram-cli/ci.yml?branch=main&style=flat-square&label=ci&color=000000" alt="ci badge"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-mit-000000?style=flat-square" alt="license badge"></a>
</p>

---

`pangram` is an unofficial terminal client for Pangram AI detection and plagiarism checking. it serves three interaction modes through one behavioral core: a JSON-first command-line interface for scripts and shell pipelines, an interactive terminal user interface for people, and a typed stdio MCP server for AI agents.

[documentation](https://pangram.micr.dev/docs) | [npm](https://registry.npmjs.org/@microck/pangram-cli) | [github](https://github.com/Microck/pangram-cli) | [contracts](docs/contracts.md)

## why

Pangram's web application is useful for interactive checks, but terminal and agent workflows need a different interface:

- one command surface for AI detection, plagiarism, files, and bulk work
- JSON-first output with stable errors and exit codes
- an interactive TUI with optional local history
- typed MCP tools with billing, filesystem, and mutation safeguards
- signed native releases with package-manager-aware updates

## quickstart

### Linux or macOS

Install the signed native binary:

```bash
curl -fsSL https://github.com/Microck/pangram-cli/releases/latest/download/pangram-installer.sh | sh
pangram --help
```

The installer places `pangram` in `~/.local/bin`. Add that directory to `PATH` when the installer asks you to.

### Windows

Run the signed PowerShell installer:

```powershell
irm https://github.com/Microck/pangram-cli/releases/latest/download/pangram-installer.ps1 | iex
pangram --help
```

### Using a package manager

```bash
npm install -g @microck/pangram-cli

# homebrew
brew install --formula https://github.com/Microck/pangram-cli/releases/latest/download/pangram.rb
```

```powershell
# scoop
scoop install https://github.com/Microck/pangram-cli/releases/latest/download/pangram-scoop.json
```

Native archives for Linux, macOS, and Windows are also available from [GitHub Releases](https://github.com/Microck/pangram-cli/releases). See the [installation guide](https://pangram.micr.dev/docs/how-to/install) for platform details.

### Authentication

Set `PANGRAM_API_KEY` before the first billable command:

```bash
export PANGRAM_API_KEY='...'
pangram auth status
```

Store the key in the protected local credential file without putting it in shell history or process arguments:

```bash
printf '%s\n' "$PANGRAM_API_KEY" | pangram auth set --api-key-stdin
pangram auth status
```

Interactive terminals can run `pangram auth` for masked setup. Authentication commands do not submit content or consume credits.

### First analysis

Run AI detection on literal text:

```bash
pangram detect "text to analyze" --max-billable-units 1
```

Read text from a shell pipeline:

```bash
printf '%s\n' 'text to analyze' | pangram detect - --max-billable-units 1
```

Run AI detection and plagiarism checks together:

```bash
pangram analyze "text to analyze" --max-billable-units 6
```

Use terminal-readable output instead of the default JSON envelope:

```bash
pangram detect --format pretty "text to analyze" --max-billable-units 1
```

Open the interactive terminal interface:

```bash
pangram
```

Every analysis submits content to Pangram. Use `--max-billable-units` to reject requests above a local spending ceiling. Local history and public dashboard links stay disabled by default.

### Files and bulk jobs

Analyze supported documents directly:

```bash
pangram detect --file ./paper.pdf --max-billable-units 10
pangram detect --file ./notes.txt --format pretty --max-billable-units 1
```

Preview a bulk JSONL request without credentials or network access:

```bash
pangram bulk submit items.jsonl --max-billable-units 20 --dry-run
```

Submit a bulk request and wait for its terminal result:

```bash
pangram bulk submit items.jsonl --max-billable-units 20 --wait
```

### MCP client

Preview the exact configuration edit before writing it:

```bash
pangram mcp install --target cursor --dry-run
pangram mcp install --target cursor
```

Restart the client, then call its Pangram detection tool with a positive billable-unit ceiling. MCP startup exposes no history mutations, public links, or file roots unless you enable those capabilities explicitly.

### Shell completion

Generate completion for your shell:

```bash
pangram completions bash > ~/.local/share/bash-completion/completions/pangram
pangram completions zsh > ~/.zsh/completions/_pangram
pangram completions fish > ~/.config/fish/completions/pangram.fish
```

For the full authentication, MCP, history, and platform guides, see the [documentation](https://pangram.micr.dev/docs).

## command surface

available today:

| command | purpose |
| --- | --- |
| `pangram` | bare literal text or piped stdin runs AI detection; an all-TTY launch opens the TUI |
| `pangram detect` | run Pangram 4 AI text detection |
| `pangram plagiarism` | run plagiarism checking on text |
| `pangram analyze` | run AI detection and plagiarism together on text |
| `pangram auth` | configure and inspect API-key authentication |
| `pangram config` | inspect and update non-secret configuration |
| `pangram doctor` | run local, non-billable diagnostics |
| `pangram bulk` | submit and inspect asynchronous bulk detection |
| `pangram task` | inspect or wait for a Pangram text task |
| `pangram history` | list, show, search, delete, clear, export, and rerun optional local history |
| `pangram mcp` | run or install the typed stdio MCP server |
| `pangram agent` | print the embedded agent usage guide |
| `pangram skills` | list and load version-matched embedded skills |
| `pangram completions` | generate a completion script |
| `pangram update` | inspect signed update status |

## privacy and agent safety

local history is disabled by default. when enabled, it stores canonical inputs, results, and task identity in a local SQLite database until you delete them. public Pangram dashboard links are also disabled by default and must be requested explicitly for each submission.

for AI agents, the MCP server enforces strict safeguards. it requires explicit startup capabilities for history mutation, configuration changes, public links, and filesystem access. file inputs are restricted to approved root directories, and billable MCP tools require explicit ceilings to prevent uncontrolled costs.

## architecture summary

the runtime uses Rust. a single deep analysis module owns Pangram HTTP behavior, polling, normalization, retries, and task state. the CLI, TUI, and MCP server act as adapters over that module to ensure consistent semantics. documentation lives in a separate TypeScript workspace and consumes generated schemas to stay aligned with the runtime contract.

## development status

version `0.1.0` is available through npm and GitHub Releases. observable behavior is defined in `docs/contracts.md` and enforced through compiled-binary, loopback HTTP, SQLite, PTY, MCP, update, and generated-contract tests.

## documentation

- [product specification](docs/product-spec.md)
- [architecture specification](docs/architecture-spec.md)
- [observable contracts](docs/contracts.md)
- [history contract](docs/history-contract.md)
- [mcp contract](docs/mcp-contract.md)
- [update contract](docs/update-contract.md)

## disclaimer

this project is unofficial and is not affiliated with, endorsed by, or connected to Pangram Labs, Inc. it is an independent client for documented Pangram APIs.

use of Pangram services remains subject to Pangram's terms, billing, retention, and acceptable-use policies. detection results are probabilistic evidence and should be one signal among many, not a verdict.

## license

[mit license](LICENSE)
