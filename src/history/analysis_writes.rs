//! Single-analysis mutations and their atomic observation writes.
//!
//! Every write runs inside one transaction, and the FTS row moves with the
//! typed row so a rollback can never leave the search index desynchronized.
//! A fresh save (the row, its FTS payload, and its current observation rows)
//! commits together or not at all, so a half-committed analysis can never
//! persist.

use rusqlite::params;

use crate::domain::{AnalysisId, AnalysisStatus, SubmissionOutcome, UtcTimestamp};

use super::records::{StoredAnalysis, StoredCheck, StoredUpstreamTask};
use super::store::HistoryStore;
use super::wire::{wire_check_kind, wire_check_status, wire_outcome, wire_status};
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
            let checks = legacy_checks_for_reconcile(record, &[])?;
            validate_legacy_save(record, &checks)?;
            insert_check_rows(transaction, &checks)?;
            set_check_count(transaction, record.id, checks.len())?;
            certify_new_analysis_tx(transaction, record)?;
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
            let checks = legacy_checks_for_reconcile(record, observations)?;
            validate_legacy_save(record, &checks)?;
            insert_analysis_row(transaction, record)?;
            insert_search_row(transaction, record)?;
            insert_check_rows(transaction, &checks)?;
            set_check_count(transaction, record.id, checks.len())?;
            for task in observations {
                upsert_observation_row(transaction, task)?;
            }
            certify_new_analysis_tx(transaction, record)?;
            Ok(())
        })
    }

    /// Inserts one complete analysis with its authoritative ordered check
    /// payloads and optional upstream task evidence in one transaction.
    pub fn save_analysis_complete(
        &mut self,
        record: &StoredAnalysis,
        checks: &[StoredCheck],
        observations: &[StoredUpstreamTask],
    ) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            validate_check_rows(record.id, checks)?;
            insert_analysis_row(transaction, record)?;
            insert_search_row(transaction, record)?;
            insert_check_rows(transaction, checks)?;
            set_check_count(transaction, record.id, checks.len())?;
            for task in observations {
                upsert_observation_row(transaction, task)?;
            }
            certify_new_analysis_tx(transaction, record)?;
            Ok(())
        })
    }

    /// Upserts the current remote observation of one check. The key is
    /// `(analysis_id, check_kind)` per the locked schema.
    pub fn record_observation(&mut self, task: &StoredUpstreamTask) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            // Preserve the existing orphan-observation write-failure
            // contract while ensuring a real owner cannot be repaired by
            // replacing corrupt task evidence.
            let owner_exists = transaction
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM analyses WHERE id = ?1)",
                    [task.analysis_id.to_string()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "read observation owner",
                    )
                })?;
            if owner_exists {
                super::read_validation::certify_analysis_aggregate(transaction, &task.analysis_id)?;
            }
            upsert_observation_row(transaction, task)?;
            super::read_validation::certify_analysis_aggregate(transaction, &task.analysis_id)
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
        let checks = legacy_checks_for_terminal(*id, snapshot);
        self.update_terminal_result_complete(id, snapshot, &checks)
    }

    /// Writes a terminal snapshot and its complete authoritative ordered
    /// check payload atomically.
    pub fn update_terminal_result_complete(
        &mut self,
        id: &AnalysisId,
        snapshot: &TerminalResult,
        checks: &[StoredCheck],
    ) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            // Replacing any part of the aggregate must never silently repair
            // corruption in another part.
            super::read_validation::certify_analysis_aggregate(transaction, id)?;
            let rebound_checks = checks
                .iter()
                .cloned()
                .map(|check| StoredCheck {
                    analysis_id: *id,
                    ..check
                })
                .collect::<Vec<_>>();
            validate_check_rows(*id, &rebound_checks)?;
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
            replace_check_rows(transaction, *id, &rebound_checks)?;
            set_check_count(transaction, *id, rebound_checks.len())?;
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
            super::read_validation::certify_analysis_aggregate(transaction, id)
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
        let checks = legacy_checks_for_snapshot(*id, snapshot);
        self.in_immediate_transaction(|transaction| {
            super::read_validation::certify_analysis_aggregate(transaction, id)?;
            super::reconcile::update_observation_snapshot_tx(
                transaction,
                id,
                observed_at,
                snapshot,
                &checks,
                false,
            )?;
            super::read_validation::certify_analysis_aggregate(transaction, id)
        })
    }
}

/// The column list shared by the `analyses` INSERT statement.
pub(super) const ANALYSES_INSERT: &str = "INSERT INTO analyses (
    id, bulk_id, bulk_index, caller_id, status, submission_outcome,
    save_state, input_type, input_sha256, display_name, input_json,
    result_json, error_json, upstream_version, retry_of, rerun_of, submitted_at,
    created_at, updated_at, completed_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)";

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

pub(super) fn insert_check_rows(
    transaction: &rusqlite::Transaction<'_>,
    checks: &[StoredCheck],
) -> Result<(), HistoryError> {
    for check in checks {
        transaction
            .execute(
                "INSERT INTO analysis_checks
                 (analysis_id, check_index, check_kind, status, result_json, error_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    check.analysis_id.to_string(),
                    i64::from(check.check_index),
                    wire_check_kind(check.check_kind),
                    wire_check_status(check.status),
                    check.result_json,
                    check.error_json,
                ],
            )
            .map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "save checks")
            })?;
    }
    Ok(())
}

pub(super) fn replace_check_rows(
    transaction: &rusqlite::Transaction<'_>,
    analysis_id: AnalysisId,
    checks: &[StoredCheck],
) -> Result<(), HistoryError> {
    validate_check_rows(analysis_id, checks)?;
    transaction
        .execute(
            "DELETE FROM analysis_checks WHERE analysis_id = ?1",
            [analysis_id.to_string()],
        )
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "replace checks")
        })?;
    insert_check_rows(transaction, checks)
}

pub(super) fn set_check_count(
    transaction: &rusqlite::Transaction<'_>,
    analysis_id: AnalysisId,
    count: usize,
) -> Result<(), HistoryError> {
    let count = i64::try_from(count).map_err(|_| {
        HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "analysis check count is invalid",
        )
    })?;
    transaction
        .execute(
            "UPDATE analyses SET check_count = ?2 WHERE id = ?1",
            params![analysis_id.to_string(), count],
        )
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "save check count")
        })?;
    Ok(())
}

pub(super) fn validate_check_rows(
    analysis_id: AnalysisId,
    checks: &[StoredCheck],
) -> Result<(), HistoryError> {
    let kinds = checks
        .iter()
        .map(|check| check.check_kind)
        .collect::<Vec<_>>();
    crate::domain::OrderedChecks::new(kinds).map_err(|_| {
        HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "analysis checks are missing, duplicated, or out of order",
        )
    })?;
    for (index, check) in checks.iter().enumerate() {
        let bodies_ok = match check.status {
            crate::domain::CheckStatus::Succeeded => {
                check.result_json.is_some() && check.error_json.is_none()
            }
            crate::domain::CheckStatus::Failed => {
                check.result_json.is_none() && check.error_json.is_some()
            }
            crate::domain::CheckStatus::Queued | crate::domain::CheckStatus::Running => {
                check.result_json.is_none() && check.error_json.is_none()
            }
        };
        if check.analysis_id != analysis_id || usize::from(check.check_index) != index || !bodies_ok
        {
            return Err(HistoryError::new(
                HistoryErrorCode::HistoryWriteFailed,
                "analysis check payload is inconsistent",
            ));
        }
        if check
            .result_json
            .as_deref()
            .is_some_and(|json| !json_object_valid(json))
            || check
                .error_json
                .as_deref()
                .is_some_and(|json| !json_object_valid(json))
        {
            return Err(HistoryError::new(
                HistoryErrorCode::HistoryWriteFailed,
                "analysis check payload is malformed",
            ));
        }
    }
    Ok(())
}

/// Certifies the complete authoritative check set already owned by one
/// parent before an update performs its first write. Corruption is never
/// repaired by replacement: missing rows, cardinality/order/kind drift, and
/// malformed typed result/error JSON all fail closed and roll back.
pub(super) fn certify_stored_check_rows(
    transaction: &rusqlite::Transaction<'_>,
    analysis_id: &AnalysisId,
) -> Result<(), HistoryError> {
    load_stored_check_rows(transaction, analysis_id).map(drop)
}

/// Loads the complete authoritative check set after proving that its stored
/// cardinality, order, kinds, statuses, and bodies are internally valid.
/// Reconciliation uses the returned rows as the merge base, so omitted check
/// kinds are preserved byte-for-byte instead of inferred from parent fields.
pub(super) fn load_stored_check_rows(
    transaction: &rusqlite::Transaction<'_>,
    analysis_id: &AnalysisId,
) -> Result<Vec<StoredCheck>, HistoryError> {
    let expected: i64 = transaction
        .query_row(
            "SELECT check_count FROM analyses WHERE id = ?1",
            [analysis_id.to_string()],
            |row| row.get(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => HistoryError::new(
                HistoryErrorCode::NotFound,
                "the recorded analysis no longer exists",
            ),
            _ => HistoryError::from_sqlite(
                HistoryErrorCode::HistoryCorrupt,
                "validate stored checks",
            ),
        })?;
    let mut statement = transaction
        .prepare(
            "SELECT check_index, check_kind, status, result_json, error_json
             FROM analysis_checks WHERE analysis_id = ?1 ORDER BY check_index",
        )
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "validate stored checks")
        })?;
    let rows = statement
        .query_map([analysis_id.to_string()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "validate stored checks")
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "validate stored checks")
        })?;
    if expected != i64::try_from(rows.len()).unwrap_or(-1) {
        return Err(stored_checks_corrupt());
    }
    let mut checks = Vec::with_capacity(rows.len());
    for (expected_index, (index, kind, status, result, error)) in rows.into_iter().enumerate() {
        if index != i64::try_from(expected_index).unwrap_or(-1) {
            return Err(stored_checks_corrupt());
        }
        let kind = super::wire::unwire_check_kind(&kind).map_err(|_| stored_checks_corrupt())?;
        let status =
            super::wire::unwire_check_status(&status).map_err(|_| stored_checks_corrupt())?;
        let payload_valid = match status {
            crate::domain::CheckStatus::Succeeded => {
                result.as_deref().is_some_and(json_object_valid) && error.is_none()
            }
            crate::domain::CheckStatus::Failed => {
                error.as_deref().is_some_and(json_object_valid) && result.is_none()
            }
            crate::domain::CheckStatus::Queued | crate::domain::CheckStatus::Running => {
                result.is_none() && error.is_none()
            }
        };
        if !payload_valid {
            return Err(stored_checks_corrupt());
        }
        checks.push(StoredCheck {
            analysis_id: *analysis_id,
            check_index: u8::try_from(expected_index).map_err(|_| stored_checks_corrupt())?,
            check_kind: kind,
            status,
            result_json: result,
            error_json: error,
        });
    }
    crate::domain::OrderedChecks::<crate::domain::CheckKind>::new(
        checks.iter().map(|check| check.check_kind),
    )
    .map_err(|_| stored_checks_corrupt())?;
    Ok(checks)
}

fn json_object_valid(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json).is_ok_and(|value| value.is_object())
}

fn stored_checks_corrupt() -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::HistoryCorrupt,
        "the authoritative stored checks are missing, duplicated, or malformed",
    )
}

pub(super) fn legacy_checks_for_reconcile(
    record: &StoredAnalysis,
    observations: &[StoredUpstreamTask],
) -> Result<Vec<StoredCheck>, HistoryError> {
    if observations
        .iter()
        .any(|task| task.analysis_id != record.id)
    {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "an observation does not belong to the analysis being saved",
        ));
    }
    let kinds = if observations.is_empty() {
        vec![crate::domain::CheckKind::AiDetection]
    } else {
        observations
            .iter()
            .map(|task| task.check_kind)
            .collect::<Vec<_>>()
    };
    crate::domain::OrderedChecks::new(kinds.clone()).map_err(|_| {
        HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "analysis observations are duplicated or out of canonical order",
        )
    })?;
    let status = match (&record.result_json, &record.error_json) {
        (Some(_), None) => crate::domain::CheckStatus::Succeeded,
        (None, Some(_)) => crate::domain::CheckStatus::Failed,
        (Some(_), Some(_)) => {
            return Err(HistoryError::new(
                HistoryErrorCode::HistoryWriteFailed,
                "an analysis cannot carry both result and error bodies",
            ));
        }
        _ => match record.status {
            AnalysisStatus::Queued => crate::domain::CheckStatus::Queued,
            AnalysisStatus::Running => crate::domain::CheckStatus::Running,
            AnalysisStatus::Succeeded => crate::domain::CheckStatus::Succeeded,
            AnalysisStatus::Failed | AnalysisStatus::Partial => crate::domain::CheckStatus::Failed,
        },
    };
    let checks = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| StoredCheck {
            analysis_id: record.id,
            check_index: u8::try_from(index).expect("ordered checks contain at most two entries"),
            check_kind: kind,
            status,
            result_json: record.result_json.clone(),
            error_json: record.error_json.clone(),
        })
        .collect::<Vec<_>>();
    Ok(checks)
}

pub(super) fn validate_legacy_save(
    record: &StoredAnalysis,
    checks: &[StoredCheck],
) -> Result<(), HistoryError> {
    if checks.len() > 1 && (record.result_json.is_some() || record.error_json.is_some()) {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "the legacy save API cannot assign one terminal body to multiple checks",
        ));
    }
    validate_check_rows(record.id, checks)?;
    let statuses = checks.iter().map(|check| check.status).collect::<Vec<_>>();
    let parent = crate::domain::derive_parent_status(&statuses).map_err(|_| {
        HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "analysis checks cannot derive a parent status",
        )
    })?;
    if parent != record.status {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "analysis status does not match its authoritative checks",
        ));
    }
    Ok(())
}

/// Proves a fresh legacy save can be reconstructed through the same canonical
/// read used by `history show` before its transaction is allowed to commit.
pub(super) fn certify_new_analysis_tx(
    transaction: &rusqlite::Transaction<'_>,
    record: &StoredAnalysis,
) -> Result<(), HistoryError> {
    let stored = super::reads::stored_analysis_on(transaction, &record.id)?;
    if stored != *record {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryCorrupt,
            "the inserted analysis aggregate does not match its canonical write",
        ));
    }
    super::read_validation::certify_analysis_aggregate(transaction, &record.id)
}

fn legacy_checks_for_snapshot(
    analysis_id: AnalysisId,
    snapshot: &ObservationSnapshot,
) -> Vec<StoredCheck> {
    let status = match (&snapshot.result_json, &snapshot.error_json) {
        (Some(_), None) => crate::domain::CheckStatus::Succeeded,
        (None, Some(_)) => crate::domain::CheckStatus::Failed,
        _ => match snapshot.status {
            AnalysisStatus::Queued => crate::domain::CheckStatus::Queued,
            AnalysisStatus::Running => crate::domain::CheckStatus::Running,
            AnalysisStatus::Succeeded => crate::domain::CheckStatus::Succeeded,
            AnalysisStatus::Failed | AnalysisStatus::Partial => crate::domain::CheckStatus::Failed,
        },
    };
    vec![StoredCheck {
        analysis_id,
        check_index: 0,
        check_kind: crate::domain::CheckKind::AiDetection,
        status,
        result_json: snapshot.result_json.clone(),
        error_json: snapshot.error_json.clone(),
    }]
}

fn legacy_checks_for_terminal(
    analysis_id: AnalysisId,
    snapshot: &TerminalResult,
) -> Vec<StoredCheck> {
    let status = match (&snapshot.result_json, &snapshot.error_json) {
        (Some(_), None) => crate::domain::CheckStatus::Succeeded,
        (None, Some(_)) => crate::domain::CheckStatus::Failed,
        _ => match snapshot.status {
            AnalysisStatus::Succeeded => crate::domain::CheckStatus::Succeeded,
            AnalysisStatus::Failed | AnalysisStatus::Partial => crate::domain::CheckStatus::Failed,
            AnalysisStatus::Queued => crate::domain::CheckStatus::Queued,
            AnalysisStatus::Running => crate::domain::CheckStatus::Running,
        },
    };
    vec![StoredCheck {
        analysis_id,
        check_index: 0,
        check_kind: crate::domain::CheckKind::AiDetection,
        status,
        result_json: snapshot.result_json.clone(),
        error_json: snapshot.error_json.clone(),
    }]
}
