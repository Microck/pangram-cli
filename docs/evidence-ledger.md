# Pangram CLI external evidence ledger

Status: non-normative research record

This ledger records mutable external facts that influence implementation.
Product contracts state required behavior; this file records why a current
external assumption was considered credible and when it must be rechecked.

| Evidence | Source | Checked | Expires or recheck gate |
| --- | --- | --- | --- |
| Pangram documented text, file, plagiarism, task, and bulk interfaces | [Pangram SDK](https://github.com/pangramlabs/pangram-sdk) | 2026-07-27 | Before each protocol phase and live conformance |
| Bulk limit is 1,000 billable units; one unit is each started 1,000-word block per valid item, minimum one | Pangram SDK `docs/api/rest.rst` | 2026-07-27 | Before Phase 3 bulk implementation |
| Bulk metadata and results are documented as retained for 48 hours after terminal state | Pangram SDK `docs/api/rest.rst` | 2026-07-27 | Before Phase 3 and public docs |
| Rust target runtime baselines | [Rust platform support](https://doc.rust-lang.org/rustc/platform-support.html) | 2026-07-27 | On MSRV selection and before each public release |
| MCP Task states and fields | [MCP 2025-11-25 Tasks](https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/tasks) | 2026-07-27 | Before Phase 6 MCP implementation |
| Terminal Control package version and native-platform support | Package registry and upstream repository | 2026-07-23 | Immediately before Phase 5 scaffolding |
| Rust stable toolchain | [Rust stable channel manifest](https://static.rust-lang.org/dist/channel-rust-stable.toml) | 2026-07-28: 1.97.1 | Before each implementation phase and public release |
| Phase 0 dependency versions and declared MSRV compatibility | crates.io API metadata and deps.dev | 2026-07-28 | On direct dependency changes; prove the selected lockfile on Rust 1.85 before accepting the manifest |
| Fumadocs and frontend version compatibility | Fumadocs and framework release documentation | Not yet revalidated | Immediately before Phase 8 scaffolding |
| Registry names, package-manager formulas, and client configuration formats | Each registry or client owner | Not yet revalidated | Before implementing or publishing that channel |
| Search-positioning volumes | Semrush US database | 2026-07-23 research snapshot only | Before public landing-page copy; advisory only |

An expired or unverified row cannot justify a public compatibility, price,
retention, platform, or availability claim. Revalidation updates this ledger,
then changes a normative contract only when external behavior actually changed.
