//! The one concrete SQLite history store (architecture-spec 11).
//!
//! `HistoryStore` owns the Pangram CLI history database: open, schema
//! validation, filesystem protection, transactions, FTS synchronization, and
//! every mutation of the `bulk_collections`, `analyses`, `upstream_tasks`,
//! and `analysis_search` tables locked by docs/history-contract.md.
//!
//! Contract rules implemented here:
//! - one schema version (`SCHEMA_VERSION = 1`) recorded in `user_version`;
//!   an unknown or newer value is `history_corrupt` with recovery guidance,
//!   never a silent migration or replacement
//! - every connection enables WAL, `foreign_keys = ON`, and
//!   `secure_delete = ON` before touching application tables
//! - the history directory, database, and WAL/shared-memory sidecars are
//!   owner-only (`0700`/`0600` on Unix; the Phase 1 owner-only ACL policy on
//!   Windows); protection is established before SQLite is ever opened and
//!   verified on every open, failing closed as `insecure_history_permissions`
//! - deletes and clears synchronize FTS inside the same transaction and end
//!   with `wal_checkpoint(TRUNCATE)`; a truncate failure is reported but the
//!   logical deletion stays committed
//! - `input_json`/`result_json`/`error_json` carry canonical schema-major-1
//!   JSON exactly as produced by the output projection; this module never
//!   invents, normalizes, or logs their content
//!
//! No repository trait, ORM, async layer, daemon, or second backend exists:
//! this is the only history module. Automatic retention remains disabled by
//! default; the CLI list/show/search/delete/clear adapter calls this store.

mod analysis_writes;
mod collections;
mod export;
mod read_validation;
mod reads;
mod reconcile;
mod records;
pub(crate) mod save;
mod schema_v1;
mod search;
mod sidecars;
mod store;
mod wire;

pub use analysis_writes::{ObservationSnapshot, TerminalResult};
pub use reconcile::{ReconciledAnalysis, ReconciledBulk};
pub use records::{
    InputKind, StoredAnalysis, StoredBulkCollection, StoredCheck, StoredSearchHit,
    StoredUpstreamTask,
};
pub use store::{DATABASE_DIRECTORY_NAME, DATABASE_FILE_NAME, HistoryStore, SCHEMA_VERSION};

use std::fmt;

impl HistoryError {
    /// The adapter-facing canonical error for one history failure. Messages
    /// are already sanitized by construction (operation and platform state
    /// only), so they copy through unchanged.
    #[must_use]
    pub fn into_canonical(&self) -> crate::output::CanonicalError {
        crate::output::CanonicalError::new(self.code().canonical(), self.message().to_owned())
            .unwrap_or_else(|_| {
                crate::output::CanonicalError::new(
                    crate::output::ErrorCode::HistoryUnavailable,
                    "history is unavailable",
                )
                .expect("static fallback")
            })
    }
}

use thiserror::Error;

use crate::output::ErrorCode;

/// The adapter-facing failure classification of a history operation.
///
/// These map one-to-one onto the closed `local_history` error codes of the
/// output contract; `NotFound` exists inside the store so an explicit fetch
/// of an absent row stays distinguishable from a storage failure, and
/// adapters decide whether absence becomes `history_unavailable` or a
/// usage-level answer in their own contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryErrorCode {
    /// The platform protection could not be established or verified.
    InsecureHistoryPermissions,
    /// Schema version unknown or newer, or the file failed SQLite integrity
    /// probing. Original bytes are preserved; writes stay blocked.
    HistoryCorrupt,
    /// A requested row does not exist.
    NotFound,
    /// A mutation failed inside or around its transaction.
    HistoryWriteFailed,
    /// The database could not be opened or read at all.
    HistoryUnavailable,
}

impl HistoryErrorCode {
    /// The canonical wire code adapters map this failure onto.
    #[must_use]
    pub const fn canonical(self) -> ErrorCode {
        match self {
            Self::InsecureHistoryPermissions => ErrorCode::InsecureHistoryPermissions,
            Self::HistoryCorrupt => ErrorCode::HistoryCorrupt,
            Self::NotFound => ErrorCode::HistoryUnavailable,
            Self::HistoryWriteFailed => ErrorCode::HistoryWriteFailed,
            Self::HistoryUnavailable => ErrorCode::HistoryUnavailable,
        }
    }
}

impl fmt::Display for HistoryErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.canonical().as_str())
    }
}

/// A sanitized history failure. Messages name the operation and platform
/// state only; they never include submitted content, SQL text, JSON bodies,
/// upstream IDs, or file contents.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code}: {message}")]
pub struct HistoryError {
    code: HistoryErrorCode,
    message: String,
}

impl HistoryError {
    #[must_use]
    pub fn new(code: HistoryErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn code(&self) -> HistoryErrorCode {
        self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Sanitizes a rusqlite failure into the adapter-facing classification.
    /// The raw error string is intentionally discarded: SQLite messages can
    /// embed SQL text and binding values (submitted content).
    pub(crate) fn from_sqlite(code: HistoryErrorCode, operation: &'static str) -> Self {
        Self::new(code, format!("{operation}: the database reported an error"))
    }
}
