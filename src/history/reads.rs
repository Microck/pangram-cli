//! Reads over the history tables and the destructive clear/delete writes.
//!
//! Every read surfaces not-found distinctly from storage unavailability, and
//! a structurally missing FTS row fails closed as `history_corrupt` rather
//! than silently `None`. Deletes and clears synchronize FTS inside the same
//! transaction and end with `wal_checkpoint(TRUNCATE)`.

use rusqlite::{Connection, params};

use crate::domain::{AnalysisId, CheckKind};

use super::records::StoredAnalysis;
use super::store::HistoryStore;
use super::wire::{row_to_analysis, wire_check_kind};
use super::{HistoryError, HistoryErrorCode};

impl HistoryStore {
    /// Resolves one saved analysis to the single upstream AI-detection task
    /// that the complete canonical record attests. The parent, input, checks,
    /// memberships, FTS row, and every task-evidence row are validated inside
    /// one read snapshot before the identity is returned. Zero task rows,
    /// multiple task rows, or a lone non-AI task are valid history shapes but
    /// are not resolvable for the text-task CLI surface.
    pub fn resolve_analysis_task(
        &self,
        id: &AnalysisId,
    ) -> Result<Option<crate::domain::UpstreamTaskId>, HistoryError> {
        self.with_read_snapshot(|connection| {
            super::read_validation::certified_analysis_on(connection, id, false)?;
            let mut statement = connection
                .prepare(
                    "SELECT check_kind, upstream_task_id
                     FROM upstream_tasks
                     WHERE analysis_id = ?1
                     ORDER BY check_kind",
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "resolve saved task",
                    )
                })?;
            let rows = statement
                .query_map([id.to_string()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "resolve saved task",
                    )
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "resolve saved task",
                    )
                })?;
            let ai_tasks = rows
                .iter()
                .filter(|(kind, _)| kind == wire_check_kind(CheckKind::AiDetection))
                .collect::<Vec<_>>();
            match ai_tasks.as_slice() {
                [(_, task_id)] => task_id
                    .parse()
                    .map(Some)
                    .map_err(|_| corrupt_stored_value("resolve saved task")),
                _ => Ok(None),
            }
        })
    }

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
        self.with_read_snapshot(|connection| {
            let found = stored_analysis_by_task(connection, check_kind, upstream_task_id)?;
            if let Some(record) = &found {
                super::read_validation::certified_analysis_on(connection, &record.id, false)?;
            }
            Ok(found)
        })
    }

    /// Fetches one full analysis row by its canonical identity.
    ///
    /// A record owning an `analyses` row without the synchronized FTS row is
    /// structurally inconsistent: the store always writes them together.
    /// Absence therefore fails as `history_corrupt` with sanitized recovery
    /// guidance, never a silent `None` payload.
    pub fn get_analysis(&self, id: &AnalysisId) -> Result<StoredAnalysis, HistoryError> {
        self.with_read_snapshot(|connection| {
            super::read_validation::certified_analysis_on(connection, id, true)?;
            stored_analysis_on(connection, id)
        })
    }

    /// Reconstructs and validates the canonical analysis stored for `show`.
    pub fn canonical_analysis(
        &self,
        id: &AnalysisId,
        include_input: bool,
    ) -> Result<crate::domain::Analysis<crate::output::CanonicalError>, HistoryError> {
        self.with_read_snapshot(|connection| {
            super::read_validation::certified_analysis_on(connection, id, include_input)
        })
    }

    /// Logical delete of one analysis: its upstream task rows cascade, and
    /// the FTS entry is removed inside the same transaction. The caller
    /// checkpoints the WAL before reporting success.
    pub fn delete_analysis(&mut self, id: &AnalysisId) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            // A destructive command is never a corruption-repair mechanism.
            // Certify the complete logical store in this transaction's
            // snapshot before its first mutation.
            super::read_validation::certify_store_integrity(transaction)?;
            let exists = match transaction.query_row(
                "SELECT 1 FROM analyses WHERE id = ?1",
                [id.to_string()],
                |row| row.get::<_, i64>(0),
            ) {
                Ok(_) => true,
                Err(rusqlite::Error::QueryReturnedNoRows) => false,
                Err(_) => {
                    return Err(HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryWriteFailed,
                        "delete analysis",
                    ));
                }
            };
            if !exists {
                return Err(HistoryError::new(
                    HistoryErrorCode::NotFound,
                    "no analysis with that identity is recorded",
                ));
            }
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
                    HistoryErrorCode::HistoryCorrupt,
                    "the recorded analysis has no synchronized search entry",
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
            super::read_validation::certify_store_integrity(transaction)
        })?;
        // wal_checkpoint(TRUNCATE) after the commit: a truncate failure is
        // reported, but the logical deletion above stays committed.
        self.checkpoint_truncate()
    }

    /// Logical clear of every history table. FTS rows move in the same
    /// transaction; the WAL is truncated before success is reported.
    pub fn clear(&mut self) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            super::read_validation::certify_store_integrity(transaction)?;
            transaction
                .execute_batch(
                    "DELETE FROM analysis_search;
                     DELETE FROM upstream_tasks;
                     DELETE FROM analysis_checks;
                     DELETE FROM analyses;
                     DELETE FROM bulk_collections;",
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "clear history")
                })?;
            super::read_validation::certify_store_integrity(transaction)
        })?;
        self.checkpoint_truncate()
    }

    /// Runs a complete logical read inside one deferred SQLite transaction.
    /// The first statement fixes a WAL snapshot while ordinary writers remain
    /// free to commit. Every dependent parent/FTS/check/task query therefore
    /// observes one database state.
    pub(super) fn with_read_snapshot<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, HistoryError>,
    ) -> Result<T, HistoryError> {
        let transaction = self.connection_ref().unchecked_transaction().map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "begin read snapshot")
        })?;
        super::read_validation::certify_foreign_keys(&transaction)?;
        let outcome = operation(&transaction)?;
        transaction.commit().map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "commit read snapshot")
        })?;
        Ok(outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CanonicalCheckRow {
    pub index: i64,
    pub kind: String,
    pub status: String,
    pub result_json: Option<String>,
    pub error_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CanonicalTaskRow {
    pub kind: String,
    pub upstream_task_id: String,
    pub last_stage: Option<String>,
    pub observed_at: String,
}

/// Reconstructs one canonical analysis from an already fetched parent and
/// its ordered child rows. Batch certification uses this pure owner after
/// loading the complete table surfaces with three set queries.
pub(super) fn canonical_analysis_from_rows(
    record: &StoredAnalysis,
    expected_check_count: i64,
    observations: &[CanonicalCheckRow],
    tasks: &[CanonicalTaskRow],
    upstream_bulk_id: Option<&str>,
    include_input: bool,
) -> Result<crate::domain::Analysis<crate::output::CanonicalError>, HistoryError> {
    let mut input: serde_json::Value = serde_json::from_str(&record.input_json)
        .map_err(|_| corrupt_stored_value("read stored input"))?;
    super::read_validation::validate_stored_input(record, &input)?;
    if !include_input && let Some(object) = input.as_object_mut() {
        object.remove("text");
        object.remove("path");
        object.remove("extracted_text");
    }

    if observations.is_empty() || observations.len() > 2 {
        return Err(corrupt_stored_value("read stored checks"));
    }
    if expected_check_count != i64::try_from(observations.len()).unwrap_or(-1) {
        return Err(corrupt_stored_value("read stored check count"));
    }
    let mut kinds = Vec::with_capacity(observations.len());
    let mut checks = Vec::with_capacity(observations.len());
    for (expected_index, observation) in observations.iter().enumerate() {
        if observation.index != i64::try_from(expected_index).unwrap_or(-1) {
            return Err(corrupt_stored_value("read stored check order"));
        }
        let parsed_kind = super::wire::unwire_check_kind(&observation.kind)?;
        kinds.push(parsed_kind);
        let parsed_status = super::wire::unwire_check_status(&observation.status)?;
        let bodies_ok = match parsed_status {
            crate::domain::CheckStatus::Succeeded => {
                observation.result_json.is_some() && observation.error_json.is_none()
            }
            crate::domain::CheckStatus::Failed => {
                observation.result_json.is_none() && observation.error_json.is_some()
            }
            crate::domain::CheckStatus::Queued | crate::domain::CheckStatus::Running => {
                observation.result_json.is_none() && observation.error_json.is_none()
            }
        };
        if !bodies_ok {
            return Err(corrupt_stored_value("read stored check payload"));
        }
        let mut check = serde_json::json!({"kind": observation.kind, "status": observation.status});
        let matching_tasks = tasks
            .iter()
            .filter(|task| task.kind == observation.kind)
            .collect::<Vec<_>>();
        match matching_tasks.as_slice() {
            [task] => {
                task.observed_at
                    .parse::<crate::domain::UtcTimestamp>()
                    .map_err(|_| corrupt_stored_value("read stored task evidence"))?;
                task.upstream_task_id
                    .parse::<crate::domain::UpstreamTaskId>()
                    .map_err(|_| corrupt_stored_value("read stored task evidence"))?;
                let mut upstream = serde_json::json!({"task_id": task.upstream_task_id});
                if let Some(stage) = &task.last_stage {
                    stage
                        .parse::<crate::domain::NonEmptyString>()
                        .map_err(|_| corrupt_stored_value("read stored task evidence"))?;
                    upstream["last_stage"] = serde_json::Value::String(stage.clone());
                }
                check["upstream"] = upstream;
            }
            [] => {}
            _ => return Err(corrupt_stored_value("read stored task evidence")),
        }
        if let Some(result) = &observation.result_json {
            check["result"] = serde_json::from_str(result)
                .map_err(|_| corrupt_stored_value("read stored check result"))?;
        }
        if let Some(error) = &observation.error_json {
            check["error"] = serde_json::from_str(error)
                .map_err(|_| corrupt_stored_value("read stored check error"))?;
        }
        checks.push(check);
    }
    if tasks
        .iter()
        .any(|task| !observations.iter().any(|check| check.kind == task.kind))
    {
        return Err(corrupt_stored_value("read stored task evidence"));
    }
    crate::domain::OrderedChecks::new(kinds)
        .map_err(|_| corrupt_stored_value("read stored check order"))?;

    let task_ids = tasks
        .iter()
        .map(|task| task.upstream_task_id.as_str())
        .collect::<Vec<_>>();
    let mut provenance = serde_json::json!({"provider": "pangram"});
    if let Some(version) = &record.upstream_version {
        provenance["upstream_version"] = serde_json::Value::String(version.clone());
    }
    if !task_ids.is_empty() {
        provenance["upstream_task_ids"] = serde_json::json!(task_ids);
    }
    if let Some(submitted_at) = record.submitted_at {
        provenance["submitted_at"] = serde_json::json!(submitted_at);
    }
    if let Some(upstream_bulk_id) = upstream_bulk_id {
        provenance["upstream_bulk_id"] = serde_json::Value::String(upstream_bulk_id.to_owned());
    }
    if let Some(completed) = record.completed_at {
        provenance["completed_at"] = serde_json::json!(completed);
    }

    let mut value = serde_json::json!({
        "id": record.id,
        "status": super::wire::wire_status(record.status),
        "submission_outcome": super::wire::wire_outcome(record.submission_outcome),
        "checks": checks,
        "save_state": super::wire::wire_save_state(record.save_state),
        "provenance": provenance,
        "created_at": record.created_at,
        "updated_at": record.updated_at,
    });
    if !input.is_null() {
        value["input"] = input;
    }
    if let Some(retry_of) = record.retry_of {
        value["retry_of"] = serde_json::json!(retry_of);
    }
    if let Some(rerun_of) = record.rerun_of {
        value["rerun_of"] = serde_json::json!(rerun_of);
    }
    if let Some(completed_at) = record.completed_at {
        value["completed_at"] = serde_json::json!(completed_at);
    }
    serde_json::from_value(value).map_err(|_| corrupt_stored_value("read stored analysis"))
}

fn corrupt_stored_value(operation: &'static str) -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::HistoryCorrupt,
        format!(
            "{operation}: the history database contains an invalid canonical value. \
             Move the history directory aside and rerun the command; the original file is preserved."
        ),
    )
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
pub(super) fn stored_analysis_on(
    connection: &Connection,
    id: &AnalysisId,
) -> Result<StoredAnalysis, HistoryError> {
    connection
        .query_row(
            "SELECT id, bulk_id, bulk_index, caller_id, status, submission_outcome,
                    save_state, input_type, input_sha256, display_name, input_json,
                    result_json, error_json, upstream_version, retry_of, rerun_of,
                    submitted_at, created_at, updated_at, completed_at
             FROM analyses WHERE id = ?1",
            [id.to_string()],
            |row| row_to_analysis(row, connection),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => HistoryError::new(
                HistoryErrorCode::NotFound,
                "no analysis with that identity is recorded",
            ),
            rusqlite::Error::InvalidQuery
            | rusqlite::Error::InvalidColumnType(..)
            | rusqlite::Error::FromSqlConversionFailure(..)
            | rusqlite::Error::IntegralValueOutOfRange(..)
            | rusqlite::Error::Utf8Error(..) => HistoryError::new(
                HistoryErrorCode::HistoryCorrupt,
                "the history database is structurally inconsistent. \
                 Move the history directory aside and rerun the command; \
                 the original file is preserved.",
            ),
            _ => HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "read analysis"),
        })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn deferred_read_snapshot_keeps_parent_and_children_consistent_without_blocking_wal_writer() {
        let root = tempfile::tempdir().expect("temporary history root");
        let reader = HistoryStore::open(root.path()).expect("reader");
        reader
            .with_connection(|connection| {
                connection.execute_batch(
                    "INSERT INTO analyses (
                       id, status, submission_outcome, save_state, input_type,
                       input_sha256, input_json, check_count, created_at, updated_at
                     ) VALUES (
                       'anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a70',
                       'running', 'accepted', 'saved_history', 'text',
                       '0000000000000000000000000000000000000000000000000000000000000000',
                       '{}', 1, '2026-08-01T10:00:00Z', '2026-08-01T10:00:00Z'
                     );
                     INSERT INTO analysis_checks
                       (analysis_id, check_index, check_kind, status)
                     VALUES
                       ('anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a70', 0, 'ai_detection', 'running');
                     INSERT INTO analysis_search
                       (analysis_id, input_text, filename, headline, source_urls)
                     VALUES
                       ('anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a70', NULL, NULL, NULL, NULL);",
                )
            })
            .expect("borrow reader")
            .expect("seed rows");

        let writer = HistoryStore::open(root.path()).expect("writer");
        let (start_tx, start_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            start_rx.recv().expect("reader established snapshot");
            writer
                .with_connection(|connection| {
                    let transaction = connection.unchecked_transaction()?;
                    transaction.execute(
                        "UPDATE analyses SET status = 'succeeded'
                         WHERE id = 'anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a70'",
                        [],
                    )?;
                    transaction.execute(
                        "UPDATE analysis_checks SET status = 'succeeded',
                           result_json = '{}'
                         WHERE analysis_id = 'anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a70'",
                        [],
                    )?;
                    transaction.commit()
                })
                .expect("borrow writer")
                .expect("writer commits while snapshot is open");
            done_tx.send(()).expect("writer completion");
        });

        reader
            .with_read_snapshot(|connection| {
                let parent: String = connection
                    .query_row(
                        "SELECT status FROM analyses
                         WHERE id = 'anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a70'",
                        [],
                        |row| row.get(0),
                    )
                    .expect("read parent");
                start_tx.send(()).expect("release writer");
                done_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("WAL writer is not blocked by deferred reader");
                let child: String = connection
                    .query_row(
                        "SELECT status FROM analysis_checks
                         WHERE analysis_id = 'anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a70'",
                        [],
                        |row| row.get(0),
                    )
                    .expect("read child");
                assert_eq!((parent.as_str(), child.as_str()), ("running", "running"));
                Ok(())
            })
            .expect("consistent read snapshot");
        handle.join().expect("writer thread");
    }
}
