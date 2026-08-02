//! Core mutations and reads of the history tables.
//!
//! Every write runs inside one transaction, and the FTS row moves with the
//! typed row so a rollback can never leave the search index desynchronized.
//! Destructive operations truncate the WAL before the caller reports
//! success, per docs/history-contract.md.

use rusqlite::{Connection, params};

use crate::domain::{
    AnalysisId, AnalysisStatus, BulkId, CheckKind, SaveState, SubmissionOutcome, UtcTimestamp,
};

use super::records::{
    InputKind, StoredAnalysis, StoredBulkCollection, StoredSearchHit, StoredUpstreamTask,
};
use super::store::HistoryStore;
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
    pub completed_at: UtcTimestamp,
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
            transaction
                .execute(
                    "INSERT INTO analyses (
                        id, bulk_id, bulk_index, caller_id, status, submission_outcome,
                        save_state, input_type, input_sha256, display_name, input_json,
                        result_json, error_json, retry_of, rerun_of, created_at, updated_at,
                        completed_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
                    params![
                        record.id.to_string(),
                        record.bulk.map(|(bulk, _)| bulk.to_string()),
                        record.bulk.map(|(_, index)| index),
                        record.caller_id,
                        wire_status(record.status),
                        wire_outcome(record.submission_outcome),
                        wire_save_state(record.save_state),
                        record.input_kind.as_str(),
                        record.input_sha256.to_string(),
                        record.display_name,
                        record.input_json,
                        record.result_json,
                        record.error_json,
                        record.retry_of.map(|id| id.to_string()),
                        record.rerun_of.map(|id| id.to_string()),
                        record.created_at.to_string(),
                        record.updated_at.to_string(),
                        record.completed_at.map(|instant| instant.to_string()),
                    ],
                )
                .map_err(|_| HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "save analysis"))?;

            transaction
                .execute(
                    "INSERT INTO analysis_search (analysis_id, input_text, filename, headline, source_urls)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        record.id.to_string(),
                        record.search_input_text,
                        record.search_filename,
                        record.search_headline,
                        record.search_source_urls,
                    ],
                )
                .map_err(|_| HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "index analysis"))?;
            Ok(())
        })
    }

    /// Inserts one bulk collection row. Same transaction contract as
    /// `save_analysis`.
    pub fn save_bulk_collection(
        &mut self,
        record: &StoredBulkCollection,
    ) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            transaction
                .execute(
                    "INSERT INTO bulk_collections (
                        id, upstream_bulk_id, status, submission_outcome, total_items,
                        accepted, succeeded, failed, estimated_billable_units, created_at,
                        updated_at, completed_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        record.id.to_string(),
                        record.upstream_bulk_id,
                        wire_status(record.status),
                        wire_outcome(record.submission_outcome),
                        record.counters.total_items() as i64,
                        record.counters.accepted() as i64,
                        record.counters.succeeded() as i64,
                        record.counters.failed() as i64,
                        record.estimated_billable_units.unwrap_or(0) as i64,
                        record.created_at.to_string(),
                        record.updated_at.to_string(),
                        record.completed_at.map(|instant| instant.to_string()),
                    ],
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryWriteFailed,
                        "save bulk collection",
                    )
                })?;
            Ok(())
        })
    }

    /// Upserts the current remote observation of one check. The key is
    /// `(analysis_id, check_kind)` per the locked schema.
    pub fn record_observation(&mut self, task: &StoredUpstreamTask) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
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
                .map_err(|_| HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "record observation"))?;
            Ok(())
        })
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
            let written = transaction
                .execute(
                    "UPDATE analyses SET
                        status = ?2,
                        submission_outcome = ?3,
                        result_json = ?4,
                        error_json = ?5,
                        updated_at = ?6,
                        completed_at = ?6
                     WHERE id = ?1",
                    params![
                        id.to_string(),
                        wire_status(snapshot.status),
                        wire_outcome(snapshot.submission_outcome),
                        snapshot.result_json,
                        snapshot.error_json,
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
            transaction
                .execute(
                    "DELETE FROM analysis_search WHERE analysis_id = ?1",
                    [id.to_string()],
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryWriteFailed,
                        "replace search entry",
                    )
                })?;
            transaction
                .execute(
                    "INSERT INTO analysis_search (analysis_id, input_text, filename, headline, source_urls)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        id.to_string(),
                        snapshot.search_input_text,
                        snapshot.search_filename,
                        snapshot.search_headline,
                        snapshot.search_source_urls,
                    ],
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryWriteFailed,
                        "replace search entry",
                    )
                })?;
            Ok(())
        })
    }

    /// Fetches one full analysis row by its canonical identity.
    ///
    /// A record owning an `analyses` row without the synchronized FTS row is
    /// structurally inconsistent: the store always writes them together.
    /// Absence therefore fails as `history_corrupt` with sanitized recovery
    /// guidance, never a silent `None` payload.
    pub fn get_analysis(&self, id: &AnalysisId) -> Result<StoredAnalysis, HistoryError> {
        self.with_connection_result(|connection| {
            connection
                .query_row(
                    "SELECT id, bulk_id, bulk_index, caller_id, status, submission_outcome,
                            save_state, input_type, input_sha256, display_name, input_json,
                            result_json, error_json, retry_of, rerun_of, created_at, updated_at,
                            completed_at
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
                    _ => HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "read analysis",
                    ),
                })
        })
    }

    /// Fetches one full bulk collection row by its canonical identity.
    pub fn get_bulk_collection(&self, id: &BulkId) -> Result<StoredBulkCollection, HistoryError> {
        self.with_connection_result(|connection| {
            connection
                .query_row(
                    "SELECT id, upstream_bulk_id, status, submission_outcome, total_items,
                            accepted, succeeded, failed, estimated_billable_units, created_at,
                            updated_at, completed_at
                     FROM bulk_collections WHERE id = ?1",
                    [id.to_string()],
                    row_to_bulk,
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => HistoryError::new(
                        HistoryErrorCode::NotFound,
                        "no bulk collection with that identity is recorded",
                    ),
                    _ => HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "read bulk collection",
                    ),
                })
        })
    }

    /// Members of one bulk collection in index order.
    ///
    /// Same structural FTS rule as `get_analysis`: a member row missing its
    /// synchronized search entry is corruption, not an absent payload.
    pub fn list_bulk_analyses(&self, bulk: &BulkId) -> Result<Vec<StoredAnalysis>, HistoryError> {
        self.with_connection_result(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, bulk_id, bulk_index, caller_id, status, submission_outcome,
                            save_state, input_type, input_sha256, display_name, input_json,
                            result_json, error_json, retry_of, rerun_of, created_at, updated_at,
                            completed_at
                     FROM analyses WHERE bulk_id = ?1 ORDER BY bulk_index",
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "list bulk analyses",
                    )
                })?;
            let rows = statement
                .query_map([bulk.to_string()], |row| row_to_analysis(row, connection))
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "list bulk analyses",
                    )
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| match error {
                    rusqlite::Error::InvalidQuery => HistoryError::new(
                        HistoryErrorCode::HistoryCorrupt,
                        "the history database is structurally inconsistent. \
                         Move the history directory aside and rerun the command; \
                         the original file is preserved.",
                    ),
                    _ => HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "list bulk analyses",
                    ),
                })?;
            Ok(rows)
        })
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
    fn with_connection_result<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, HistoryError>,
    ) -> Result<T, HistoryError> {
        let connection: &Connection = self.connection_ref();
        operation(connection)
    }
}

// Wire spellings of the closed domain enums. These live here, beside the
// SQL, so the domain crate stays free of persistence concerns and an
// accidental enum rename fails the history contract tests, not every domain
// test at once.
const fn wire_status(status: AnalysisStatus) -> &'static str {
    match status {
        AnalysisStatus::Queued => "queued",
        AnalysisStatus::Running => "running",
        AnalysisStatus::Succeeded => "succeeded",
        AnalysisStatus::Failed => "failed",
        AnalysisStatus::Partial => "partial",
    }
}

const fn wire_outcome(outcome: SubmissionOutcome) -> &'static str {
    match outcome {
        SubmissionOutcome::NotSubmitted => "not_submitted",
        SubmissionOutcome::Accepted => "accepted",
        SubmissionOutcome::Terminal => "terminal",
        SubmissionOutcome::AcceptanceUnknown => "acceptance_unknown",
    }
}

const fn wire_save_state(state: SaveState) -> &'static str {
    match state {
        SaveState::Ephemeral => "ephemeral",
        SaveState::SavedManual => "saved_manual",
        SaveState::SavedHistory => "saved_history",
    }
}

const fn wire_check_kind(kind: CheckKind) -> &'static str {
    match kind {
        CheckKind::AiDetection => "ai_detection",
        CheckKind::Plagiarism => "plagiarism",
    }
}

fn unwire_status(value: &str) -> Result<AnalysisStatus, HistoryError> {
    match value {
        "queued" => Ok(AnalysisStatus::Queued),
        "running" => Ok(AnalysisStatus::Running),
        "succeeded" => Ok(AnalysisStatus::Succeeded),
        "failed" => Ok(AnalysisStatus::Failed),
        "partial" => Ok(AnalysisStatus::Partial),
        _ => Err(HistoryError::from_sqlite(
            HistoryErrorCode::HistoryCorrupt,
            "read analysis",
        )),
    }
}

fn unwire_outcome(value: &str) -> Result<SubmissionOutcome, HistoryError> {
    match value {
        "not_submitted" => Ok(SubmissionOutcome::NotSubmitted),
        "accepted" => Ok(SubmissionOutcome::Accepted),
        "terminal" => Ok(SubmissionOutcome::Terminal),
        "acceptance_unknown" => Ok(SubmissionOutcome::AcceptanceUnknown),
        _ => Err(HistoryError::from_sqlite(
            HistoryErrorCode::HistoryCorrupt,
            "read analysis",
        )),
    }
}

fn unwire_save_state(value: &str) -> Result<SaveState, HistoryError> {
    match value {
        "ephemeral" => Ok(SaveState::Ephemeral),
        "saved_manual" => Ok(SaveState::SavedManual),
        "saved_history" => Ok(SaveState::SavedHistory),
        _ => Err(HistoryError::from_sqlite(
            HistoryErrorCode::HistoryCorrupt,
            "read analysis",
        )),
    }
}

fn row_to_analysis(
    row: &rusqlite::Row<'_>,
    connection: &Connection,
) -> Result<StoredAnalysis, rusqlite::Error> {
    let id: String = row.get(0)?;
    let bulk_id: Option<String> = row.get(1)?;
    let bulk_index: Option<i64> = row.get(2)?;
    let caller_id: Option<String> = row.get(3)?;
    let status: String = row.get(4)?;
    let outcome: String = row.get(5)?;
    let save_state: String = row.get(6)?;
    let input_type: String = row.get(7)?;
    let input_sha256: String = row.get(8)?;
    let display_name: Option<String> = row.get(9)?;
    let input_json: String = row.get(10)?;
    let result_json: Option<String> = row.get(11)?;
    let error_json: Option<String> = row.get(12)?;
    let retry_of: Option<String> = row.get(13)?;
    let rerun_of: Option<String> = row.get(14)?;
    let created_at: String = row.get(15)?;
    let updated_at: String = row.get(16)?;
    let completed_at: Option<String> = row.get(17)?;

    // The FTS payload lives beside the typed row. A missing row is a
    // structural inconsistency: the store always writes them together, so
    // absence means the database is corrupt, never a silent `None` mapping.
    let fts: (Option<String>, Option<String>, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT input_text, filename, headline, source_urls FROM analysis_search WHERE analysis_id = ?1",
            [id.clone()],
            |search| Ok((search.get(0)?, search.get(1)?, search.get(2)?, search.get(3)?)),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => rusqlite::Error::InvalidQuery,
            other => other,
        })?;

    Ok(StoredAnalysis {
        id: id.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
        bulk: match (bulk_id, bulk_index) {
            (Some(bulk), Some(index)) => Some((
                bulk.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
                index,
            )),
            _ => None,
        },
        caller_id,
        status: unwire_status(&status).map_err(|_| rusqlite::Error::InvalidQuery)?,
        submission_outcome: unwire_outcome(&outcome).map_err(|_| rusqlite::Error::InvalidQuery)?,
        save_state: unwire_save_state(&save_state).map_err(|_| rusqlite::Error::InvalidQuery)?,
        input_kind: InputKind::parse(&input_type).map_err(|_| rusqlite::Error::InvalidQuery)?,
        input_sha256: input_sha256
            .parse()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        display_name,
        input_json,
        result_json,
        error_json,
        retry_of: retry_of
            .map(|value| value.parse())
            .transpose()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        rerun_of: rerun_of
            .map(|value| value.parse())
            .transpose()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        created_at: created_at
            .parse()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        updated_at: updated_at
            .parse()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        completed_at: completed_at
            .map(|value| value.parse())
            .transpose()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        search_input_text: fts.0,
        search_filename: fts.1,
        search_headline: fts.2,
        search_source_urls: fts.3,
    })
}

fn row_to_bulk(row: &rusqlite::Row<'_>) -> Result<StoredBulkCollection, rusqlite::Error> {
    let id: String = row.get(0)?;
    let upstream_bulk_id: Option<String> = row.get(1)?;
    let status: String = row.get(2)?;
    let outcome: String = row.get(3)?;
    let total_items: i64 = row.get(4)?;
    let accepted: i64 = row.get(5)?;
    let succeeded: i64 = row.get(6)?;
    let failed: i64 = row.get(7)?;
    let estimated: i64 = row.get(8)?;
    let created_at: String = row.get(9)?;
    let updated_at: String = row.get(10)?;
    let completed_at: Option<String> = row.get(11)?;

    Ok(StoredBulkCollection {
        id: id.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
        upstream_bulk_id,
        status: unwire_status(&status).map_err(|_| rusqlite::Error::InvalidQuery)?,
        submission_outcome: unwire_outcome(&outcome).map_err(|_| rusqlite::Error::InvalidQuery)?,
        counters: crate::domain::BulkCounters::new(
            total_items as u64,
            accepted as u64,
            succeeded as u64,
            failed as u64,
        )
        .map_err(|_| rusqlite::Error::InvalidQuery)?,
        estimated_billable_units: if estimated == 0 {
            None
        } else {
            Some(estimated as u64)
        },
        created_at: created_at
            .parse()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        updated_at: updated_at
            .parse()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        completed_at: completed_at
            .map(|value| value.parse())
            .transpose()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}

fn row_to_summary(row: &rusqlite::Row<'_>) -> Result<StoredSearchHit, rusqlite::Error> {
    let id: String = row.get(0)?;
    let status: String = row.get(1)?;
    let save_state: String = row.get(2)?;
    let input_type: String = row.get(3)?;
    let display_name: Option<String> = row.get(4)?;
    let created_at: String = row.get(5)?;
    Ok(StoredSearchHit {
        analysis_id: id.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
        status: unwire_status(&status).map_err(|_| rusqlite::Error::InvalidQuery)?,
        save_state: unwire_save_state(&save_state).map_err(|_| rusqlite::Error::InvalidQuery)?,
        input_kind: InputKind::parse(&input_type).map_err(|_| rusqlite::Error::InvalidQuery)?,
        display_name,
        created_at: created_at
            .parse()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}
