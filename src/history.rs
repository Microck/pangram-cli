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
pub use export::{HistoryExportError, HistoryExportFormat, export_history};
pub use reconcile::{ReconciledAnalysis, ReconciledBulk};
pub use records::{
    InputKind, StoredAnalysis, StoredBulkCollection, StoredCheck, StoredSearchHit,
    StoredUpstreamTask,
};
pub use store::{DATABASE_DIRECTORY_NAME, DATABASE_FILE_NAME, HistoryStore, SCHEMA_VERSION};

/// Projects storage rows onto the privacy-bounded summary type shared by
/// CLI, TUI, and MCP adapters.
pub(crate) fn summary_page(
    hits: Vec<StoredSearchHit>,
) -> Result<crate::domain::AnalysisSummaryPage, HistoryError> {
    use crate::domain::{AnalysisInputKind, AnalysisSummary, OrderedChecks};

    let items = hits
        .into_iter()
        .map(|hit| {
            let checks = OrderedChecks::new(hit.checks).map_err(|_| {
                HistoryError::new(
                    HistoryErrorCode::HistoryCorrupt,
                    "a stored history summary has invalid check ordering",
                )
            })?;
            Ok(AnalysisSummary {
                id: hit.analysis_id,
                status: hit.status,
                checks,
                save_state: hit.save_state,
                input_kind: match hit.input_kind {
                    InputKind::Text => AnalysisInputKind::Text,
                    InputKind::File => AnalysisInputKind::File,
                },
                display_name: hit.display_name,
                created_at: hit.created_at,
            })
        })
        .collect::<Result<Vec<_>, HistoryError>>()?;
    Ok(crate::domain::AnalysisSummaryPage { items })
}

/// Persists one canonical analysis with its ordered checks and observation
/// evidence in the store's single atomic write. Adapters decide the retention
/// policy; this history-owned seam decides the exact durable projection.
pub(crate) enum RetainedInput {
    Text(String),
    File {
        path: String,
        extracted_text: Option<String>,
    },
}

pub(crate) fn save_complete_analysis(
    store: &mut HistoryStore,
    analysis: &crate::domain::Analysis<crate::output::CanonicalError>,
    save_state: crate::domain::SaveState,
    retained_input: Option<&RetainedInput>,
) -> Result<(), HistoryError> {
    let record = save::stored_analysis_with_retained_input(analysis, save_state, retained_input)?;
    store.save_analysis_complete(
        &record,
        &save::stored_checks(analysis)?,
        &save::stored_observations(analysis),
    )
}

/// Persists one canonical bulk snapshot and its ordered child membership in
/// exactly one store-owned reconciliation transaction. Adapters own policy
/// (automatic versus explicit) and store opening; this history seam owns the
/// complete durable projection. Projection finishes before SQLite mutates,
/// so a malformed child cannot leave a parent-only or partial snapshot.
pub(crate) fn save_bulk_snapshot(
    store: &mut HistoryStore,
    collection: &crate::domain::BulkCollection,
    children: &[(
        crate::domain::Analysis<crate::output::CanonicalError>,
        Option<String>,
    )],
) -> Result<(), HistoryError> {
    let provisional_id = collection.id();
    let prepared = children
        .iter()
        .enumerate()
        .map(|(index, (child, caller_id))| {
            let bulk_index =
                i64::try_from(index).expect("a validated bulk plan has at most 1,000 children");
            let mut record = save::stored_analysis(child, crate::domain::SaveState::SavedHistory)?;
            record.bulk = Some((provisional_id, bulk_index));
            record.caller_id = caller_id.clone();
            let checks = save::stored_checks(child)?;
            let observations = save::stored_observations(child);
            Ok((record, checks, observations))
        })
        .collect::<Result<Vec<_>, HistoryError>>()?;
    let row = save::stored_bulk_collection(collection);
    store
        .reconcile_bulk_collection_complete(&row, &prepared)
        .map(|_| ())
}

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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::domain::{
        Analysis, AnalysisStatus, BulkCollection, BulkCounters, BulkId, Check, CheckState,
        OrderedChecks, Provenance, Provider, SaveState, SubmissionOutcome, UpstreamBulkId,
        UpstreamIdentity, UpstreamTaskId, UpstreamTaskIds, UtcTimestamp,
    };

    use super::{HistoryStore, save_bulk_snapshot};

    fn accepted_child(
        upstream_bulk_id: &UpstreamBulkId,
        task: &str,
        observed_at: UtcTimestamp,
    ) -> Analysis<crate::output::CanonicalError> {
        let task_id = UpstreamTaskId::from_str(task).unwrap();
        let checks = OrderedChecks::new([Check::AiDetection(CheckState::Queued {
            upstream: Some(UpstreamIdentity {
                task_id: Some(task_id.clone()),
                last_stage: None,
            }),
        })])
        .unwrap();
        Analysis::with_optional_input(
            crate::domain::AnalysisId::new(),
            SubmissionOutcome::Accepted,
            None,
            checks,
            SaveState::Ephemeral,
            Provenance {
                provider: Provider::Pangram,
                upstream_version: None,
                upstream_task_ids: Some(UpstreamTaskIds::new(vec![task_id]).unwrap()),
                upstream_bulk_id: Some(upstream_bulk_id.clone()),
                submitted_at: Some(observed_at),
                completed_at: None,
            },
            None,
            None,
            observed_at,
            observed_at,
            None,
        )
        .unwrap()
    }

    #[test]
    fn save_bulk_snapshot_commits_parent_children_and_memberships_together() {
        let root = tempfile::tempdir().unwrap();
        let observed_at = UtcTimestamp::from_str("2026-08-12T20:00:00Z").unwrap();
        let upstream_bulk_id = UpstreamBulkId::from_str("bulk-atomic-snapshot").unwrap();
        let collection = BulkCollection::new(
            BulkId::new(),
            Some(upstream_bulk_id.clone()),
            AnalysisStatus::Queued,
            SubmissionOutcome::Accepted,
            BulkCounters::new(2, 2, 0, 0).unwrap(),
            Some(2),
            observed_at,
            observed_at,
            None,
        )
        .unwrap();
        let children = vec![
            (
                accepted_child(&upstream_bulk_id, "task-atomic-0", observed_at),
                Some("caller-0".to_owned()),
            ),
            (
                accepted_child(&upstream_bulk_id, "task-atomic-1", observed_at),
                Some("caller-1".to_owned()),
            ),
        ];
        let mut store = HistoryStore::open(root.path()).unwrap();

        save_bulk_snapshot(&mut store, &collection, &children).unwrap();

        let (collection_count, memberships): (i64, Vec<(i64, String)>) = store
            .with_connection(|connection| {
                let collection_count =
                    connection.query_row("SELECT COUNT(*) FROM bulk_collections", [], |row| {
                        row.get(0)
                    })?;
                let mut statement = connection
                    .prepare("SELECT bulk_index, caller_id FROM analyses ORDER BY bulk_index")?;
                let memberships = statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok::<_, rusqlite::Error>((collection_count, memberships))
            })
            .unwrap()
            .unwrap();
        assert_eq!(collection_count, 1);
        assert_eq!(
            memberships,
            vec![(0, "caller-0".to_owned()), (1, "caller-1".to_owned())]
        );
    }
}
