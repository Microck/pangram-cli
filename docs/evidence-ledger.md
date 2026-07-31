# Pangram CLI external evidence ledger

Status: non-normative research record

This ledger records mutable external facts that influence implementation.
Product contracts state required behavior; this file records why a current
external assumption was considered credible and when it must be rechecked.

| Evidence | Source | Checked | Expires or recheck gate |
| --- | --- | --- | --- |
| Pangram 4 is the current text model; unspecified requests temporarily route to Pangram 3; Pangram 3 is scheduled for deprecation on 2026-09-30; Pangram 4 API text billing is USD 0.05 per started 100 words | [Introducing Pangram 4](https://www.pangram.com/blog/introducing-pangram-4) | 2026-07-29 | Before Phase 2, public pricing docs, and each public release |
| Pangram 4 segment output adds `humanizer_score` and `is_humanized`; offsets are zero-based and half-open; document classification is Human, Mixed, or AI; intended input is at least 50 words | [Pangram 4 model card](https://www.pangram.com/research/model-card/pangram-4) | 2026-07-29 | Before Phase 2 normalization and live conformance |
| Pangram's public REST reference and SDK still omit a Pangram 4 selector and humanizer fields, and still describe the Pangram 3-era 1,000-word bulk unit and 1,000-unit maximum | [REST reference](https://pangram.readthedocs.io/en/latest/api/rest.html) and [Pangram SDK](https://github.com/pangramlabs/pangram-sdk) | 2026-07-29 | Blocking Phase 2 text submission and Phase 3 bulk submission until updated |
| Pangram Image is available in the dashboard, but Image API access is invitation-only during the research preview | [Introducing Pangram Image Detection](https://www.pangram.com/blog/introducing-pangram-image-detection) | 2026-07-29 | Image detection stays out of scope until the API is documented and generally available |
| Bulk metadata and results are documented as retained for 48 hours after terminal state | Pangram SDK `docs/api/rest.rst` | 2026-07-27 | Before Phase 3 and public docs |
| Rust target runtime baselines | [Rust platform support](https://doc.rust-lang.org/rustc/platform-support.html) | 2026-07-27 | On MSRV selection and before each public release |
| MCP stateless lifecycle, deprecated Roots, result metadata, and extension model | [MCP 2026-07-28 changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog) | 2026-07-28 | Before Phase 6 MCP implementation and public docs |
| The Tasks extension repository remains experimental and has no release | [MCP Tasks extension](https://github.com/modelcontextprotocol/ext-tasks) | 2026-07-28 | Before considering the extension for a later product revision |
| RMCP 3.0.0-beta.4 supports MCP 2026-07-28 and requires Rust 1.88 | [RMCP 3.0.0-beta.4](https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.0.0-beta.4) | 2026-07-28 | Before Phase 6 dependency selection; require a stable release |
| Terminal Control package version and native-platform support | Package registry and upstream repository | 2026-07-23 | Immediately before Phase 5 scaffolding |
| Rust stable toolchain | [Rust stable channel manifest](https://static.rust-lang.org/dist/channel-rust-stable.toml) | 2026-07-28: 1.97.1 | Before each implementation phase and public release |
| Phase 0 dependency versions and declared MSRV compatibility | crates.io API metadata and deps.dev | 2026-07-28 | On direct dependency changes; prove the selected lockfile on Rust 1.85 before accepting the manifest |
| Phase 1 dependency versions and declared MSRV compatibility: `directories 6.0.0`, `rpassword 7.5.4`, `secrecy 0.10.3`, `toml 1.0.7`, `zeroize 1.8.2`, `windows-sys 0.61.2` (Windows-only), and dev dependency `tempfile 3.27.0` | crates.io API metadata and deps.dev | 2026-07-30 | On direct dependency changes; prove the selected lockfile on Rust 1.85 before accepting the manifest |
| `option-ext 0.2.0` is licensed MPL-2.0 (Mozilla Public License 2.0). It is a transitive, non-Windows build dependency reached through `microck-pangram-cli 0.1.0 -> directories 6.0.0 -> dirs-sys 0.5.0 -> option-ext 0.2.0`. MPL-2.0 is OSI-approved, FSF Free/Libre, file-level weak copyleft, and compatible with this project's MIT outbound license. The operator decision to allow MPL-2.0 was recorded 2026-07-30; the authoritative allow-list is the cargo-deny `[licenses] allow` entry in `.github/workflows/ci.yml` | `option-ext 0.2.0` Cargo.toml metadata and cargo-deny rejection output | 2026-07-30 | On `directories` or `dirs-sys` version changes; on any `option-ext` version change; before each public release |
| Fumadocs and frontend version compatibility | Fumadocs and framework release documentation | Not yet revalidated | Immediately before Phase 8 scaffolding |
| Registry names, package-manager formulas, and client configuration formats | Each registry or client owner | Not yet revalidated | Before implementing or publishing that channel |
| Search-positioning volumes | Semrush US database | 2026-07-23 research snapshot only | Before public landing-page copy; advisory only |

An expired or unverified row cannot justify a public compatibility, price,
retention, platform, or availability claim. Revalidation updates this ledger,
then changes a normative contract only when external behavior actually changed.
