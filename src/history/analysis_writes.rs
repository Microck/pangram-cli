//! Single-analysis mutations and their atomic observation writes.
//!
//! Every write runs inside one transaction, and the FTS row moves with the
//! typed row so a rollback can never leave the search index desynchronized.
//! A fresh save (the row, its FTS payload, and its current observation rows)
//! commits together or not at all, so a half-committed analysis can never
//! persist.

use rusqlite::params;

use crate::domain::{AnalysisId, AnalysisStatus, SubmissionOutcome, UtcTimestamp};

use super::records::StoredAnalysis;
use super::records::StoredUpstreamTask;
use super::store::HistoryStore;
use super::wire::{wire_check_kind, wire_outcome, wire_status};
use super::{HistoryError, HistoryErrorCode};

/// The immutable terminal snapshot of one analysis, written together with
/// its refreshed search payload in one transaction.
///
/// The typed `search_*` fields are the complete replacement for the
/// `analysis_search` row of this analysis. Carrying them as one value keeps
/// the terminal write and its search projection indivisible: a commit
/// advances both, and a rollback undoes both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalResult {
    pub status: AnalysisStatus,
    pub submission_outcome: SubmissionOutcome,
    pub result_json: Option<String>,
    pub error_json: Option<String>,
    pub upstream_version: Option<String>,
    pub completed_at: UtcTimestamp,
    pub search_input_text: Option<String>,
    pub search_filename: Option<String>,
    pub search_headline: Option<String>,
    pub search_source_urls: Option<String>,
}

/// One observation refresh of an already-recorded analysis (the resume
/// path). Unlike [`TerminalResult`], `completed_at` stays optional: a
/// running observation moves `updated_at` only and must never fabricate a
/// terminal stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationSnapshot {
    pub status: AnalysisStatus,
    pub submission_outcome: SubmissionOutcome,
    pub result_json: Option<String>,
    pub error_json: Option<String>,
    pub upstream_version: Option<String>,
    pub completed_at: Option<UtcTimestamp>,
    pub search_input_text: Option<String>,
    pub search_filename: Option<String>,
    pub search_headline: Option<String>,
    pub search_source_urls: Option<String>,
}

impl HistoryStore {
    /// Inserts one analysis and its FTS row in one transaction. A duplicate
    /// row or a violated scoped uniqueness is a write failure; the caller
    /// maps it onto the adapter's retry policy.
    pub fn save_analysis(&mut self, record: &StoredAnalysis) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            insert_analysis_row(transaction, record)?;
            insert_search_row(transaction, record)?;
            Ok(())
        })
    }

    /// Inserts one analysis atomically with its current observation rows:
    /// the typed row, its FTS payload, and one `upstream_tasks` row per
    /// observed check, all committed in one transaction. A mid-write failure
    /// rolls the whole batch back, so a half-committed analysis (a typed row
    /// without its observation identity, or vice versa) can never persist.
    /// Observation rows upsert on their `(analysis_id, check_kind)` key, so
    /// a retry of an interrupted resume never duplicates them.
    pub fn save_analysis_atomic(
        &mut self,
        record: &StoredAnalysis,
        observations: &[StoredUpstreamTask],
    ) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            insert_analysis_row(transaction, record)?;
            insert_search_row(transaction, record)?;
            for task in observations {
                upsert_observation_row(transaction, task)?;
            }
            Ok(())
        })
    }

    /// Upserts the current remote observation of one check. The key is
    /// `(analysis_id, check_kind)` per the locked schema.
    pub fn record_observation(&mut self, task: &StoredUpstreamTask) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| upsert_observation_row(transaction, task))
    }

    /// Writes the immutable terminal snapshot of one analysis: status,
    /// outcome, JSON bodies, lifecycle stamps, and the refreshed search
    /// payload, in one transaction. A commit advances both the typed row
    /// and its FTS projection; a rollback undoes both.
    pub fn update_terminal_result(
        &mut self,
        id: &AnalysisId,
        snapshot: &TerminalResult,
    ) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            // Certify the exact synchronized FTS row before the first typed
            // mutation. Replacing it must never silently repair a missing,
            // duplicate, or malformed payload and thereby hide corruption.
            super::reconcile::child_search_by_id_tx(transaction, id)?;
            let written = transaction
                .execute(
                    "UPDATE analyses SET
                        status = ?2,
                        submission_outcome = ?3,
                        result_json = ?4,
                        error_json = ?5,
                        upstream_version = COALESCE(?6, upstream_version),
                        updated_at = ?7,
                        completed_at = ?7
                     WHERE id = ?1",
                    params![
                        id.to_string(),
                        wire_status(snapshot.status),
                        wire_outcome(snapshot.submission_outcome),
                        snapshot.result_json,
                        snapshot.error_json,
                        snapshot.upstream_version,
                        snapshot.completed_at.to_string(),
                    ],
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryWriteFailed,
                        "record terminal result",
                    )
                })?;
            if written == 0 {
                return Err(HistoryError::new(
                    HistoryErrorCode::NotFound,
                    "the recorded analysis no longer exists",
                ));
            }
            // Replace the FTS row inside the same transaction so a search
            // never sees the pre-terminal payload alongside the terminal
            // typed columns.
            replace_search_row(
                transaction,
                &id.to_string(),
                &snapshot.search_input_text,
                &snapshot.search_filename,
                &snapshot.search_headline,
                &snapshot.search_source_urls,
            )?;
            Ok(())
        })
    }

    /// Refreshes one saved analysis row with a newer observation of the same
    /// remote task: status and any terminal snapshot move together in one
    /// transaction; identity columns (id, bulk link, save_state, lineage,
    /// created_at) are immutable, and the snapshot carries the already-merged
    /// `submission_outcome` (the merge preserves the stored row's original
    /// outcome). This is the resume-observation write path; it never inserts
    /// a duplicate row for a task it already tracks. `observed_at` stamps
    /// `updated_at`; `completed_at` moves only when the observation reached
    /// a terminal state.
    ///
    /// Durable authorship invariance (contracts.md 14.2 note,
    /// docs/history-contract.md): the JSON bodies are terminal snapshots, so
    /// a refresh never erases what the store already attested. A terminal
    /// observation (which attests its `completed_at`) replaces both body
    /// columns with its own exactly-one-of pair; a non-terminal observation
    /// carries no body, and the `COALESCE` keeps the stored result or error
    /// intact, so a `running` read can never blank a recorded terminal body.
    pub fn update_observation_snapshot(
        &mut self,
        id: &AnalysisId,
        observed_at: UtcTimestamp,
        snapshot: &ObservationSnapshot,
    ) -> Result<(), HistoryError> {
        self.in_immediate_transaction(|transaction| {
            super::reconcile::update_observation_snapshot_tx(transaction, id, observed_at, snapshot)
        })
    }
}

/// The column list shared by the `analyses` INSERT statement.
pub(super) const ANALYSES_INSERT: &str = "INSERT INTO analyses (
    id, bulk_id, bulk_index, caller_id, status, submission_outcome,
    save_state, input_type, input_sha256, display_name, input_json,
    result_json, error_json, upstream_version, retry_of, rerun_of, created_at,
    updated_at, completed_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)";

/// Inserts the typed `analyses` row. Shared by `save_analysis` and the
/// atomic whole-batch write so the fresh-row statement stays in one place.
pub(super) fn insert_analysis_row(
    transaction: &rusqlite::Transaction<'_>,
    record: &StoredAnalysis,
) -> Result<(), HistoryError> {
    let row = super::wire::AnalysisRow::of(record);
    transaction
        .execute(ANALYSES_INSERT, row.as_params())
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "save analysis")
        })?;
    Ok(())
}

/// Inserts the `analysis_search` FTS payload of a fresh row.
pub(super) fn insert_search_row(
    transaction: &rusqlite::Transaction<'_>,
    record: &StoredAnalysis,
) -> Result<(), HistoryError> {
    insert_search_columns(
        transaction,
        &record.id.to_string(),
        &record.search_input_text,
        &record.search_filename,
        &record.search_headline,
        &record.search_source_urls,
        "index analysis",
    )
}

/// Replaces the `analysis_search` row of an existing analysis inside the
/// caller's transaction, so a search never sees a stale pre-write payload.
pub(super) fn replace_search_row(
    transaction: &rusqlite::Transaction<'_>,
    analysis_id: &str,
    input_text: &Option<String>,
    filename: &Option<String>,
    headline: &Option<String>,
    source_urls: &Option<String>,
) -> Result<(), HistoryError> {
    transaction
        .execute(
            "DELETE FROM analysis_search WHERE analysis_id = ?1",
            [analysis_id],
        )
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "replace search entry")
        })?;
    insert_search_columns(
        transaction,
        analysis_id,
        input_text,
        filename,
        headline,
        source_urls,
        "replace search entry",
    )
}

/// The shared `analysis_search` insert, parameterized by operation name.
fn insert_search_columns(
    transaction: &rusqlite::Transaction<'_>,
    analysis_id: &str,
    input_text: &Option<String>,
    filename: &Option<String>,
    headline: &Option<String>,
    source_urls: &Option<String>,
    operation: &'static str,
) -> Result<(), HistoryError> {
    transaction
        .execute(
            "INSERT INTO analysis_search (analysis_id, input_text, filename, headline, source_urls)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![analysis_id, input_text, filename, headline, source_urls,],
        )
        .map_err(|_| HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, operation))?;
    Ok(())
}

/// Upserts one `upstream_tasks` observation row on its
/// `(analysis_id, check_kind)` key.
pub(super) fn upsert_observation_row(
    transaction: &rusqlite::Transaction<'_>,
    task: &StoredUpstreamTask,
) -> Result<(), HistoryError> {
    transaction
        .execute(
            "INSERT INTO upstream_tasks (analysis_id, check_kind, upstream_task_id, last_stage, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (analysis_id, check_kind) DO UPDATE SET
                upstream_task_id = excluded.upstream_task_id,
                last_stage = excluded.last_stage,
                observed_at = excluded.observed_at",
            params![
                task.analysis_id.to_string(),
                wire_check_kind(task.check_kind),
                task.upstream_task_id,
                task.last_stage,
                task.observed_at.to_string(),
            ],
        )
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "record observation")
        })?;
    Ok(())
}
