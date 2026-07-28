# Pangram CLI documentation plan

Status: approved outline
Framework: Diataxis
Public destination: `https://pangram.micr.dev`

## 1. Documentation goals

The documentation must help:

- a new terminal user complete an AI detection check
- a shell user obtain and consume canonical JSON
- an agent operator install and safely gate the MCP server
- an experienced user solve file, bulk, plagiarism, history, and update tasks
- an integrator inspect exact commands, schemas, errors, and tool contracts
- a reviewer understand evidence, privacy, billing, parity, and architecture

Every page has one Diataxis job. Tutorials, how-to guides, reference, and
explanation MUST NOT be mixed into a single manual page.

## 2. Audience

Primary audience:

- people using the CLI or TUI

First-class secondary audience:

- developers writing shell automation
- AI-agent and MCP operators

Contributor documentation is separate from user documentation and lives in
the repository.

The home page provides two equally visible starting paths:

```text
Use the CLI or TUI        Connect an AI agent
```

## 3. Scope

Included:

- installation and authentication
- text AI detection
- plagiarism and combined analysis
- supported files
- bulk analysis
- local history
- structured output and errors
- MCP setup and security gates
- updater behavior
- evidence interpretation
- billing estimates
- privacy and retention
- documented parity and limitations
- release verification

Excluded:

- Pangram account, key, billing, and credit administration
- unsupported dashboard history workflows
- Playback, Feed Scanner, extension, and LMS administration
- generic essays about academic integrity
- undocumented interfaces
- a public Rust SDK guide

## 4. Information architecture

```text
Home
|-- Tutorials
|   |-- Your first TUI analysis
|   |-- Your first JSON pipeline
|   `-- Connect your first MCP client
|-- How-to guides
|   |-- Install Pangram CLI
|   |-- Configure an API key
|   |-- Detect AI-written text
|   |-- Check plagiarism
|   |-- Run a combined analysis
|   |-- Analyze PDF, DOCX, and RTF files
|   |-- Submit and retrieve bulk work
|   |-- Enable and search local history
|   |-- Export and rerun results
|   |-- Use JSON, JSONL, TOON, and Markdown
|   |-- Configure MCP clients
|   |-- Gate MCP capabilities and file access
|   |-- Generate shell completions
|   |-- Check and install updates
|   `-- Uninstall Pangram MCP entries
|-- Reference
|   |-- Command index
|   |-- Authentication
|   |-- Configuration
|   |-- Environment variables
|   |-- Output schema
|   |-- Error catalog
|   |-- Exit codes
|   |-- Progress events
|   |-- MCP tools
|   |-- MCP resources and capabilities
|   |-- TUI shortcuts
|   |-- History storage
|   |-- Update manifest
|   `-- Supported platforms and packages
|-- Explanation
|   |-- How to interpret AI-detection evidence
|   |-- AI detection and plagiarism
|   |-- Privacy and retention boundaries
|   |-- Billing and billable-unit estimates
|   |-- Pangram web parity and API limitations
|   |-- Why one core serves CLI, TUI, and MCP
|   `-- How signed releases and updates work
`-- Changelog
```

## 5. Tutorials

Tutorials are short, linear lessons with a guaranteed successful outcome.
They use synthetic content and avoid optional branches.

### 5.1 Your first TUI analysis

Audience: new terminal user
Goal: see and interpret one AI detection result
Scope: install, guided auth, launch TUI, enter text, submit, inspect overall and
segment evidence
Exclude: plagiarism, history, bulk, MCP, configuration reference

### 5.2 Your first JSON pipeline

Audience: shell user
Goal: pipe text into `pangram` and select stable JSON fields
Scope: stdin, default JSON envelope, exit code, stderr separation
Exclude: complete schema reference and alternate formats

### 5.3 Connect your first MCP client

Audience: agent operator
Goal: install the default server and complete one `detect_text` call
Scope: persistent auth, installer dry run, one supported client, billing
annotation, result identity
Exclude: mutations, history, file-root configuration, files, bulk, all-client
matrix

## 6. How-to guides

How-to pages start with the user's desired outcome and include prerequisites,
steps, verification, and relevant recovery links.

Each page MUST:

- show the smallest command that solves the task
- state whether the operation is billable
- state whether content is retained or made public
- show structured automation where relevant
- link to exact reference instead of repeating every flag

File and plagiarism pages remain visibly marked blocked until live conformance
passes. The docs build MUST derive that state from the parity contract.

## 7. Reference

Reference pages describe exact machinery and are generated where possible.

Generated:

- command tree, arguments, defaults, and incompatibilities
- output and progress JSON Schemas
- errors and exit codes
- MCP protocol version, tools, inputs, annotations, cache metadata, and
  capability gates
- configuration fields and environment precedence
- TUI keymaps
- update manifest

Handwritten reference:

- history storage location and retention
- supported distribution channels and targets
- auth acquisition instructions

Generated pages MUST identify the Pangram CLI version and schema major that
produced them.

## 8. Explanation

Explanation pages provide context without becoming procedures.

### Evidence interpretation

Explain:

- AI, AI-assisted, human, and mixed classifications
- fractions versus certainty
- segment scores and confidence
- why a result is one signal among many
- why the CLI preserves Pangram wording

Do not invent thresholds, accuracy claims, or disciplinary guidance.

### Privacy and retention

Contrast:

- content submitted to Pangram
- Pangram dashboard retention
- bulk 48-hour upstream retention
- optional local plaintext history
- opt-in public links
- normal output redaction

### Parity

Use one generated matrix with:

- Supported
- Local substitution
- Blocked by upstream contract
- Unavailable through documented APIs

### Architecture

Explain the single-core model at a user-integrator level. Detailed module
ownership stays in the repository architecture specification.

## 9. Home page

Title:

```text
Pangram CLI - AI detection and plagiarism from the terminal
```

Home-page sequence:

1. one-sentence product statement
2. two starting-path cards for humans and agents
3. terminal example with canonical JSON
4. TUI preview
5. capability summary
6. privacy and billing note
7. parity limitation link
8. install call to action

The docs site footer carries a concise unofficial-project disclosure and links
to the full README disclaimer. Package metadata carries the same concise
disclosure. The home-page body does not need a second long disclaimer.

## 10. Search positioning

Use branded and task-specific phrasing naturally:

- Pangram AI detector
- AI detection from the terminal
- Pangram API for scripts and agents
- plagiarism checking from the CLI

Do not chase broad generic search volume with inaccurate landing pages or
keyword repetition.

Suggested page titles:

- Detect AI writing with Pangram CLI
- Check plagiarism from the terminal
- Use Pangram AI detection from scripts
- Connect Pangram MCP to an AI agent
- Pangram CLI JSON and error reference

## 11. README

The repository README follows the Kagi CLI reader journey:

1. product statement and status
2. why
3. quickstart
4. authentication
5. command surface
6. agent-native behavior
7. privacy
8. architecture and documentation links
9. disclaimer
10. license

After implementation:

- replace the specification status callout with release badges
- add one TUI GIF showing intro through result
- add one compact shell-to-JSON demonstration
- add final installation channels
- keep deep reference on the docs site

Do not copy Kagi wording.

## 12. LLM-readable output

Publish:

- `/llms.txt`: concise navigation and product contract
- `/llms-full.txt`: combined public documentation
- clean Markdown for each page

LLM output MUST:

- preserve headings and code fences
- omit navigation chrome
- identify generated reference versions
- retain billing and privacy warnings
- link to schemas

## 13. Generated content pipeline

```text
Rust types and Clap definitions
            |
            v
generate-contracts development binary
            |
            +--> contracts/*.json
            +--> generated command reference
            +--> generated MCP reference
            +--> generated keymap reference
            +--> embedded skill inputs
            |
            v
Fumadocs MDX source
```

CI regenerates and fails on a dirty tree. Fumadocs does not execute Rust at
runtime.

## 14. Fumadocs implementation

Workspace: `docs-app/`

Exact framework versions are selected and recorded in the external evidence
ledger immediately before Phase 8 scaffolding. The workspace uses current
compatible Next.js, React, Fumadocs Core, Fumadocs MDX, and TypeScript releases
verified by a production build.

Requirements:

- typed navigation
- local full-text search
- generated schema rendering
- code block copy controls
- responsive navigation
- accessible focus and contrast
- no analytics or telemetry in v1
- no version switcher before a second supported docs major exists

## 15. Hosting

During private development:

- CI builds documentation
- no public preview deployment
- local development uses a fixed documented URL

After public-release gates:

- deploy through Vercel
- attach `pangram.micr.dev`
- make production deployment part of release readiness

## 16. Documentation verification

CI MUST verify:

- links resolve
- added Markdown URLs return expected content
- generated reference matches Rust contracts
- examples validate against JSON Schemas
- example commands exist in Clap
- no secrets or real submitted content appear
- no blocked feature is documented as available
- `llms.txt`, `llms-full.txt`, and page Markdown build
- Fumadocs typecheck and production build pass
- ASCII punctuation policy passes for edited source and docs

## 17. Writing style

- Lead with the outcome.
- Use short paragraphs and concrete examples.
- Prefer common words.
- Use sentence case.
- Use `pangram` for commands and Pangram for the service.
- Use `analysis`, `check`, `segment`, and `local history` consistently.
- Say "AI-assisted", not alternative spellings.
- Say "public dashboard link", not "share link".
- Say "stop waiting", not "cancel", unless discussing unsupported remote
  cancellation.
- Avoid unsupported accuracy, security, cost, or retention claims.
- Use ASCII punctuation.
