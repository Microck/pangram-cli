//! Pangram 4 text protocol integration tests against a real loopback
//! Axum fixture. No mocks, no live Pangram, no credentials.
//!
//! Decomposition (N1): the suite was split into cohesive modules under
//! `tests/analysis-protocol/` so no test file crosses the repository's
//! 800-line hygiene threshold (AGENTS.md). The grouping follows the protocol
//! seams: `submission` (POST grammar, terminal mapping, and the billable
//! send/cancellation boundary), `observation` (wait timeouts, bounded
//! retries, `Retry-After`, cancellation), and `contract_matrix` (HTTP status
//! and document-validation matrices), over one shared `support` module. The
//! tests, assertions, flavors, and timing evidence are unchanged by the
//! split; this root file only declares the modules.

#![cfg(feature = "dev-tools")]

#[path = "analysis-protocol/mod.rs"]
mod analysis_protocol;
