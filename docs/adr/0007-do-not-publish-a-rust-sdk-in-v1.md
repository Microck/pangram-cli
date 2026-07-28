# ADR 0007: Do not publish a Rust SDK in v1

Status: accepted
Date: 2026-07-27

## Context

The runtime uses one Rust package with a library target and the `pangram`
binary. The library is an internal seam shared by the binary and tests. A
crates.io release would make its public Rust items depend-able even though the
product does not promise a stable Rust SDK.

Publishing a library while disclaiming its interface would create an accidental
compatibility surface. Stabilizing a Rust SDK would also constrain the internal
module design before CLI, TUI, and MCP behavior has implementation evidence.

## Decision

Do not publish the Rust package to crates.io in v1.

Distribute native binaries through GitHub Releases, direct installers,
Homebrew, Scoop, and npm platform packages. Keep the library target internal.

Reconsider crates.io only with a separate ADR that defines a supported Rust
interface, SemVer policy, documentation, and conformance tests.

## Consequences

- The internal library can change with the binary while observable contracts
  remain stable.
- `cargo install` is not a supported public installation path in v1.
- Tegami still coordinates the Cargo package version with npm, schemas, docs,
  and release artifacts but does not publish it.
