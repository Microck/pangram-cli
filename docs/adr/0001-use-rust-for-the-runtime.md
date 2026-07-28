# ADR 0001: Use Rust for the runtime

Status: accepted
Date: 2026-07-23

## Context

Pangram CLI needs one distributable binary that serves four callers:

- humans using commands
- humans using a terminal UI
- agents using stable structured output
- MCP clients using stdio

The runtime must handle HTTP polling, multipart uploads, SQLite, terminal
restoration, self-update verification, and cross-platform release artifacts.
It should install without requiring a language runtime.

TypeScript would reduce the language count with the documentation site, but
it would make the installed runtime and self-updater depend on a JavaScript
distribution choice. Go would also fit the single-binary requirement, but its
TUI and type-level contract ecosystem is a weaker match for the chosen design.

## Decision

Implement the runtime as one Rust 2024 package with:

- an internal library target
- one `pangram` binary
- a feature-gated contract generator used only during development

Use TypeScript only for the separate Fumadocs application.

Do not create a multi-crate Rust workspace in v1. Do not advertise the library
target as a stable public Rust SDK.

## Consequences

- Users receive one native executable.
- CLI, TUI, MCP, persistence, and update logic share Rust domain types.
- The project carries both Rust and TypeScript toolchains.
- Contributors must understand async Rust and terminal lifecycle handling.
- A future public SDK requires a separate compatibility decision.

## Enforcement

- `Cargo.toml` declares edition 2024 and one package.
- `cargo metadata` must report one workspace member.
- Release tests execute the built binary rather than an alternate wrapper.
- Documentation generation consumes committed schemas produced from the Rust
  contract owner.
