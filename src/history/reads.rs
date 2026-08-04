//! Reads over the history tables and the destructive clear/delete writes.
//!
//! Every read surfaces not-found distinctly from storage unavailability, and
//! a structurally missing FTS row fails closed as `history_corrupt` rather
//! than silently `None`. Deletes and clears synchronize FTS inside the same
//! transaction and end with `wal_checkpoint(TRUNCATE)`.

use rusqlite::{Connection, params};

use crate::domain::{AnalysisId, CheckKind};

use super::records::{StoredAnalysis, StoredSearchHit};
use super::store::HistoryStore;
use super::wire::{row_to_analysis, row_to_summary, wire_check_kind};
use super::{HistoryError, HistoryErrorCode};

impl HistoryStore {
    /// Resolves the locally saved analysis observing one upstream task, if
    /// any. Task reads generate their local `anl_` identity fresh per read
    /// (contracts.md 4.6), so without this reconciliation every repeated
    /// observation of the same remote task would insert a duplicate row;
    /// the upsert key for resume continuity is the observed upstream task
    /// identity itself. SQL absence (`Ok(None)`) stays distinct from a
    /// storage failure (`Err`).
    ///
    /// Determinism invariant: the schema enforces one stored analysis per
    /// `(check_kind, upstream_task_id)`, so the lookup resolves at most one
    /// row. The `ORDER BY analysis_id` clause remains as the belt-and-braces
    /// tiebreaker for a legacy database written before the constraint
    /// existed, never as an arbitrary SQLite scan result.
    pub fn find_analysis_by_task(
        &self,
        check_kind: CheckKind,
        upstream_task_id: &str,
    ) -> Result<Option<StoredAnalysis>, HistoryError> {
        self.with_connection_result(|connection| {
            stored_analysis_by_task(connection, check_kind, upstream_task_id)
        })
    }

    /// Fetches one full analysis row by its canonical identity.
    ///
    /// A record owning an `analyses` row without the synchronized FTS row is
    /// structurally inconsistent: the store always writes them together.
    /// Absence therefore fails as `history_corrupt` with sanitized recovery
    /// guidance, never a silent `None` payload.
    pub fn get_analysis(&self, id: &AnalysisId) -> Result<StoredAnalysis, HistoryError> {
        self.with_connection_result(|connection| stored_analysis_on(connection, id))
    }

    /// Most recent analyses first, for `history list`.
    pub fn list(&self, limit: u32, offset: u32) -> Result<Vec<StoredSearchHit>, HistoryError> {
        self.with_connection_result(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, status, save_state, input_type, display_name, created_at
                     FROM analyses ORDER BY created_at DESC, id LIMIT ?1 OFFSET ?2",
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "list analyses")
                })?;
            let rows = statement
                .query_map([limit, offset], row_to_summary)
                .map_err(|_| {
                    HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "list analyses")
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| {
                    HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "list analyses")
                })?;
            Ok(rows)
        })
    }

    /// FTS5 query over the search index. The query is bound as a MATCH
    /// parameter so no FTS syntax is interpreted from the caller; a query
    /// that matches nothing returns an empty page.
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<StoredSearchHit>, HistoryError> {
        self.with_connection_result(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT a.id, a.status, a.save_state, a.input_type, a.display_name, a.created_at
                     FROM analysis_search s JOIN analyses a ON a.id = s.analysis_id
                     WHERE analysis_search MATCH ?1
                     ORDER BY a.created_at DESC, a.id LIMIT ?2",
                )
                .map_err(|_| HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "search analyses"))?;
            let rows = statement
                .query_map(params![query, limit], row_to_summary)
                .map_err(|_| HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "search analyses"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "search analyses"))?;
            Ok(rows)
        })
    }

    /// Logical delete of one analysis: its upstream task rows cascade, and
    /// the FTS entry is removed inside the same transaction. The caller
    /// checkpoints the WAL before reporting success.
    pub fn delete_analysis(&mut self, id: &AnalysisId) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            let written = transaction
                .execute(
                    "DELETE FROM analysis_search WHERE analysis_id = ?1",
                    [id.to_string()],
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryWriteFailed,
                        "delete search entry",
                    )
                })?;
            if written == 0 {
                return Err(HistoryError::new(
                    HistoryErrorCode::NotFound,
                    "no analysis with that identity is recorded",
                ));
            }
            transaction
                .execute("DELETE FROM analyses WHERE id = ?1", [id.to_string()])
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryWriteFailed,
                        "delete analysis",
                    )
                })?;
            Ok(())
        })?;
        // wal_checkpoint(TRUNCATE) after the commit: a truncate failure is
        // reported, but the logical deletion above stays committed.
        self.checkpoint_truncate()
    }

    /// Logical clear of every history table. FTS rows move in the same
    /// transaction; the WAL is truncated before success is reported.
    pub fn clear(&mut self) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            transaction
                .execute_batch(
                    "DELETE FROM analysis_search;
                     DELETE FROM upstream_tasks;
                     DELETE FROM analyses;
                     DELETE FROM bulk_collections;",
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "clear history")
                })?;
            Ok(())
        })?;
        self.checkpoint_truncate()
    }

    /// Runs `operation` with the store's connection when the operation can
    /// fail. Read helpers route every fallible raw query through this seam so
    /// `with_connection` stays infallible for assertion-style access.
    pub(super) fn with_connection_result<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, HistoryError>,
    ) -> Result<T, HistoryError> {
        let connection: &Connection = self.connection_ref();
        operation(connection)
    }
}

/// Resolves the stored analysis owning one upstream task identity,
/// returning the full stored row the reconciliation merge needs. This is the
/// store-facing half of `find_analysis_by_task` (same deterministic
/// `ORDER BY analysis_id` lookup): the public method keeps its call sites,
/// while the reconciliation transaction reads through this one inside its
/// own connection so the merge sees the same committed row the unique
/// constraint serializes.
pub(crate) fn stored_analysis_by_task(
    connection: &Connection,
    check_kind: CheckKind,
    upstream_task_id: &str,
) -> Result<Option<StoredAnalysis>, HistoryError> {
    let found: Option<String> = connection
        .query_row(
            "SELECT analysis_id FROM upstream_tasks
             WHERE check_kind = ?1 AND upstream_task_id = ?2
             ORDER BY analysis_id",
            params![wire_check_kind(check_kind), upstream_task_id],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            _ => Err(HistoryError::from_sqlite(
                HistoryErrorCode::HistoryUnavailable,
                "find analysis",
            )),
        })?;
    match found {
        None => Ok(None),
        Some(id) => {
            let parsed = id.parse().map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "read analysis")
            })?;
            stored_analysis_on(connection, &parsed).map(Some)
        }
    }
}

/// The full stored `analyses` row by identity, when it exists, for the
/// in-transaction child reconciliation lookup (`None` on absence; the
/// structurally-missing-FTS row stays `history_corrupt`, never `None`).
pub(crate) fn stored_analysis_opt_on(
    connection: &Connection,
    id: &AnalysisId,
) -> Result<Option<StoredAnalysis>, HistoryError> {
    match stored_analysis_on(connection, id) {
        Ok(record) => Ok(Some(record)),
        Err(error) if error.code() == HistoryErrorCode::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The full stored `analyses` row (typed columns plus its synchronized FTS
/// payload) read through one borrowed connection, so the in-transaction
/// reconciliation merge sees the real stored content. Shared by
/// `get_analysis` and the atomic reconciliation reads; a structurally
/// missing FTS row is `history_corrupt`, never `None`.
fn stored_analysis_on(
    connection: &Connection,
    id: &AnalysisId,
) -> Result<StoredAnalysis, HistoryError> {
    connection
        .query_row(
            "SELECT id, bulk_id, bulk_index, caller_id, status, submission_outcome,
                    save_state, input_type, input_sha256, display_name, input_json,
                    result_json, error_json, upstream_version, retry_of, rerun_of,
                    created_at, updated_at, completed_at
             FROM analyses WHERE id = ?1",
            [id.to_string()],
            |row| row_to_analysis(row, connection),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => HistoryError::new(
                HistoryErrorCode::NotFound,
                "no analysis with that identity is recorded",
            ),
            rusqlite::Error::InvalidQuery => HistoryError::new(
                HistoryErrorCode::HistoryCorrupt,
                "the history database is structurally inconsistent. \
                 Move the history directory aside and rerun the command; \
                 the original file is preserved.",
            ),
            _ => HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "read analysis"),
        })
}
