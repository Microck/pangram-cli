# Pangram CLI architecture dependency baseline

This document is the normative dependency-baseline extension of the
[architecture specification](architecture-spec.md). The architecture
specification remains the owner of all other module, seam, and dependency
rules.

Phase 0 revalidates and pins exact versions only for the roles used by the
network-free scaffold: Clap, Serde and JSON, Schemars, Jiff, thiserror, UUIDv7,
SHA-256, JSON Schema validation, property tests, and temporary directories.
Each later phase revalidates and pins its newly introduced roles before first
use. Phase-owned roles do not become dependencies before their owning phase.
Ed25519 remains a later role; Tokio, Reqwest, RMCP, Ratatui, and SQLite entered
only when their owning phases implemented them.

Phase 6 pins stable RMCP exactly to 3.1.2 at upstream commit
`02c62aef2e331e5cf79c06c744eb1eb052cc8ebd`. Its crate archive SHA-256 is
`c8dddc5b1924b9a59fba420166160ca2c4663a4e01803e52eda33070f56d63c8`, its
license is Apache-2.0 with the upstream transition notice retained, and its
minimum Rust version is 1.88. The dependency-driven package `rust-version`
therefore rose from 1.87 to 1.88 when RMCP entered the lockfile.

Phase 2 applies the same dependency-driven `rust-version` rule to the locked v1
TOON projection. `toon-format 0.5.0` requires Rust 1.87: its decode parser uses
`unsigned_is_multiple_of` (stabilized in Rust 1.87.0) and fails to compile on
Rust 1.85 with `E0658`. Because TOON is part of the locked projection contract
and the lowest selected direct-dependency-compatible toolchain becomes the
package `rust-version`, Phase 2 raises `rust-version` from 1.85 to 1.87 (not to
the RMCP 1.88 prerelease floor, which was not yet a selected dependency in
Phase 2).

Later tests add real Axum loopback servers, snapshots, PTYs, and the pinned
Terminal Control harness when the corresponding behavior exists. Exact
research snapshots live in [evidence-ledger.md](evidence-ledger.md), not in
this normative architecture.

The workspace pins current stable Rust for development and records the lowest
dependency-compatible Rust 2024 toolchain as `rust-version`. Each phase MUST
prove both the current stable toolchain and the pinned `rust-version`
toolchain before its manifest is accepted. Direct dependency upgrades are
intentional changes.
