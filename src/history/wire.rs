//! Wire spellings and row mappings shared by the history mutation and read
//! modules.
//!
//! These spellings live beside the SQL, so the domain crate stays free of
//! persistence concerns and an accidental enum rename fails the history
//! contract tests, not every domain test at once.

use rusqlite::Connection;

use crate::domain::{AnalysisStatus, CheckKind, CheckStatus, SaveState, SubmissionOutcome};

use super::records::{InputKind, StoredAnalysis, StoredBulkCollection, StoredSearchHit};
use super::{HistoryError, HistoryErrorCode};

pub(super) const fn wire_status(status: AnalysisStatus) -> &'static str {
    match status {
        AnalysisStatus::Queued => "queued",
        AnalysisStatus::Running => "running",
        AnalysisStatus::Succeeded => "succeeded",
        AnalysisStatus::Failed => "failed",
        AnalysisStatus::Partial => "partial",
    }
}

pub(super) const fn wire_outcome(outcome: SubmissionOutcome) -> &'static str {
    match outcome {
        SubmissionOutcome::NotSubmitted => "not_submitted",
        SubmissionOutcome::Accepted => "accepted",
        SubmissionOutcome::Terminal => "terminal",
        SubmissionOutcome::AcceptanceUnknown => "acceptance_unknown",
    }
}

pub(super) const fn wire_save_state(state: SaveState) -> &'static str {
    match state {
        SaveState::Ephemeral => "ephemeral",
        SaveState::SavedManual => "saved_manual",
        SaveState::SavedHistory => "saved_history",
    }
}

pub(super) const fn wire_check_kind(kind: CheckKind) -> &'static str {
    match kind {
        CheckKind::AiDetection => "ai_detection",
        CheckKind::Plagiarism => "plagiarism",
    }
}

pub(super) const fn wire_check_status(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Queued => "queued",
        CheckStatus::Running => "running",
        CheckStatus::Succeeded => "succeeded",
        CheckStatus::Failed => "failed",
    }
}

pub(super) fn unwire_check_status(value: &str) -> Result<CheckStatus, HistoryError> {
    match value {
        "queued" => Ok(CheckStatus::Queued),
        "running" => Ok(CheckStatus::Running),
        "succeeded" => Ok(CheckStatus::Succeeded),
        "failed" => Ok(CheckStatus::Failed),
        _ => Err(HistoryError::from_sqlite(
            HistoryErrorCode::HistoryCorrupt,
            "read check status",
        )),
    }
}

pub(super) fn unwire_status(value: &str) -> Result<AnalysisStatus, HistoryError> {
    match value {
        "queued" => Ok(AnalysisStatus::Queued),
        "running" => Ok(AnalysisStatus::Running),
        "succeeded" => Ok(AnalysisStatus::Succeeded),
        "failed" => Ok(AnalysisStatus::Failed),
        "partial" => Ok(AnalysisStatus::Partial),
        _ => Err(HistoryError::from_sqlite(
            HistoryErrorCode::HistoryCorrupt,
            "read row",
        )),
    }
}

pub(super) fn unwire_outcome(value: &str) -> Result<SubmissionOutcome, HistoryError> {
    match value {
        "not_submitted" => Ok(SubmissionOutcome::NotSubmitted),
        "accepted" => Ok(SubmissionOutcome::Accepted),
        "terminal" => Ok(SubmissionOutcome::Terminal),
        "acceptance_unknown" => Ok(SubmissionOutcome::AcceptanceUnknown),
        _ => Err(HistoryError::from_sqlite(
            HistoryErrorCode::HistoryCorrupt,
            "read row",
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
            "read row",
        )),
    }
}

pub(super) fn unwire_check_kind(value: &str) -> Result<CheckKind, HistoryError> {
    match value {
        "ai_detection" => Ok(CheckKind::AiDetection),
        "plagiarism" => Ok(CheckKind::Plagiarism),
        _ => Err(HistoryError::from_sqlite(
            HistoryErrorCode::HistoryCorrupt,
            "read check kind",
        )),
    }
}

/// The owned typed `analyses` row columns in statement order. Binding them
/// into one struct keeps the fresh-row insert and the reconciliation upsert
/// from drifting on column order, and owning the strings lets the caller
/// borrow them across the `params!` statement without temporary-lifetime
/// traps.
pub(super) struct AnalysisRow {
    pub id: String,
    pub bulk_id: Option<String>,
    pub bulk_index: Option<i64>,
    pub caller_id: Option<String>,
    pub status: &'static str,
    pub submission_outcome: &'static str,
    pub save_state: &'static str,
    pub input_type: &'static str,
    pub input_sha256: String,
    pub display_name: Option<String>,
    pub input_json: String,
    pub result_json: Option<String>,
    pub error_json: Option<String>,
    pub upstream_version: Option<String>,
    pub retry_of: Option<String>,
    pub rerun_of: Option<String>,
    pub submitted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

impl AnalysisRow {
    pub fn of(record: &StoredAnalysis) -> Self {
        Self {
            id: record.id.to_string(),
            bulk_id: record.bulk.map(|(bulk, _)| bulk.to_string()),
            bulk_index: record.bulk.map(|(_, index)| index),
            caller_id: record.caller_id.clone(),
            status: wire_status(record.status),
            submission_outcome: wire_outcome(record.submission_outcome),
            save_state: wire_save_state(record.save_state),
            input_type: record.input_kind.as_str(),
            input_sha256: record.input_sha256.to_string(),
            display_name: record.display_name.clone(),
            input_json: record.input_json.clone(),
            result_json: record.result_json.clone(),
            error_json: record.error_json.clone(),
            upstream_version: record.upstream_version.clone(),
            retry_of: record.retry_of.map(|id| id.to_string()),
            rerun_of: record.rerun_of.map(|id| id.to_string()),
            submitted_at: record.submitted_at.map(|instant| instant.to_string()),
            created_at: record.created_at.to_string(),
            updated_at: record.updated_at.to_string(),
            completed_at: record.completed_at.map(|instant| instant.to_string()),
        }
    }

    /// The params slice, borrowing this struct's owned fields.
    pub fn as_params(&self) -> [&dyn rusqlite::ToSql; 20] {
        [
            &self.id,
            &self.bulk_id,
            &self.bulk_index,
            &self.caller_id,
            &self.status,
            &self.submission_outcome,
            &self.save_state,
            &self.input_type,
            &self.input_sha256,
            &self.display_name,
            &self.input_json,
            &self.result_json,
            &self.error_json,
            &self.upstream_version,
            &self.retry_of,
            &self.rerun_of,
            &self.submitted_at,
            &self.created_at,
            &self.updated_at,
            &self.completed_at,
        ]
    }
}

/// The owned typed `bulk_collections` row columns in statement order.
pub(super) struct BulkRow {
    pub id: String,
    pub upstream_bulk_id: Option<String>,
    pub status: &'static str,
    pub submission_outcome: &'static str,
    pub total_items: i64,
    pub accepted: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub estimated_billable_units: i64,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

impl BulkRow {
    pub fn of(record: &StoredBulkCollection) -> Self {
        Self {
            id: record.id.to_string(),
            upstream_bulk_id: record.upstream_bulk_id.clone(),
            status: wire_status(record.status),
            submission_outcome: wire_outcome(record.submission_outcome),
            total_items: record.counters.total_items() as i64,
            accepted: record.counters.accepted() as i64,
            succeeded: record.counters.succeeded() as i64,
            failed: record.counters.failed() as i64,
            estimated_billable_units: record.estimated_billable_units.unwrap_or(0) as i64,
            created_at: record.created_at.to_string(),
            updated_at: record.updated_at.to_string(),
            completed_at: record.completed_at.map(|instant| instant.to_string()),
        }
    }

    pub fn as_params(&self) -> [&dyn rusqlite::ToSql; 12] {
        [
            &self.id,
            &self.upstream_bulk_id,
            &self.status,
            &self.submission_outcome,
            &self.total_items,
            &self.accepted,
            &self.succeeded,
            &self.failed,
            &self.estimated_billable_units,
            &self.created_at,
            &self.updated_at,
            &self.completed_at,
        ]
    }
}

pub(super) fn row_to_analysis(
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
    let upstream_version: Option<String> = row.get(13)?;
    let retry_of: Option<String> = row.get(14)?;
    let rerun_of: Option<String> = row.get(15)?;
    let submitted_at: Option<String> = row.get(16)?;
    let created_at: String = row.get(17)?;
    let updated_at: String = row.get(18)?;
    let completed_at: Option<String> = row.get(19)?;

    // The FTS payload lives beside the typed row. A missing row is a
    // structural inconsistency: the store always writes them together, so
    // absence means the database is corrupt, never a silent `None` mapping.
    let mut statement = connection.prepare(
        "SELECT input_text, filename, headline, source_urls
         FROM analysis_search WHERE analysis_id = ?1",
    )?;
    let mut rows = statement.query([id.clone()])?;
    let search = rows.next()?.ok_or(rusqlite::Error::InvalidQuery)?;
    let fts: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = (
        search.get(0)?,
        search.get(1)?,
        search.get(2)?,
        search.get(3)?,
    );
    if rows.next()?.is_some() {
        return Err(rusqlite::Error::InvalidQuery);
    }

    Ok(StoredAnalysis {
        id: id.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
        bulk: super::read_validation::decode_analysis_membership(connection, bulk_id, bulk_index)?,
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
        upstream_version,
        retry_of: retry_of
            .map(|value| value.parse())
            .transpose()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        rerun_of: rerun_of
            .map(|value| value.parse())
            .transpose()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        submitted_at: submitted_at
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

pub(super) fn row_to_bulk(
    row: &rusqlite::Row<'_>,
) -> Result<StoredBulkCollection, rusqlite::Error> {
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

pub(super) fn row_to_summary(row: &rusqlite::Row<'_>) -> Result<StoredSearchHit, rusqlite::Error> {
    let id: String = row.get(0)?;
    let status: String = row.get(1)?;
    let checks: String = row.get(2)?;
    let check_count: i64 = row.get(3)?;
    let save_state: String = row.get(4)?;
    let input_type: String = row.get(5)?;
    let display_name: Option<String> = row.get(6)?;
    let created_at: String = row.get(7)?;
    let checks = checks
        .split(',')
        .map(|kind| unwire_check_kind(kind).map_err(|_| rusqlite::Error::InvalidQuery))
        .collect::<Result<Vec<_>, _>>()?;
    if check_count != i64::try_from(checks.len()).unwrap_or(-1)
        || crate::domain::OrderedChecks::new(checks.clone()).is_err()
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(StoredSearchHit {
        analysis_id: id.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
        status: unwire_status(&status).map_err(|_| rusqlite::Error::InvalidQuery)?,
        checks,
        save_state: unwire_save_state(&save_state).map_err(|_| rusqlite::Error::InvalidQuery)?,
        input_kind: InputKind::parse(&input_type).map_err(|_| rusqlite::Error::InvalidQuery)?,
        display_name,
        created_at: created_at
            .parse()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}
