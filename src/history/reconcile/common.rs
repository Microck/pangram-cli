//! Shared reconciliation primitives for terminal-body and search-payload
//! refreshes.

use rusqlite::params;

use crate::domain::AnalysisId;

use super::super::analysis_writes::replace_search_row;
use super::super::wire::{wire_outcome, wire_status};
use super::super::{HistoryError, HistoryErrorCode, ObservationSnapshot};

#[derive(Clone, Copy)]
pub(super) enum IncomingBody {
    Empty,
    Result,
    Error,
}

pub(super) fn incoming_body(
    result_json: &Option<String>,
    error_json: &Option<String>,
) -> Result<IncomingBody, HistoryError> {
    match (result_json.is_some(), error_json.is_some()) {
        (false, false) => Ok(IncomingBody::Empty),
        (true, false) => Ok(IncomingBody::Result),
        (false, true) => Ok(IncomingBody::Error),
        (true, true) => Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "an observation cannot carry both result and error bodies",
        )),
    }
}

/// The observation-refresh write shared by the store-facing method and the
/// atomic reconciliation transaction: identical semantics and SQL, so the
/// resume path and the concurrent reconcile path can never drift.
pub(in crate::history) fn update_observation_snapshot_tx(
    transaction: &rusqlite::Transaction<'_>,
    id: &AnalysisId,
    observed_at: crate::domain::UtcTimestamp,
    snapshot: &ObservationSnapshot,
) -> Result<(), HistoryError> {
    let prior_search = child_search_by_id_tx(transaction, id)?;
    let (body_result_sql, body_error_sql) =
        match incoming_body(&snapshot.result_json, &snapshot.error_json)? {
            IncomingBody::Empty => ("COALESCE(?4, result_json)", "COALESCE(?5, error_json)"),
            IncomingBody::Result => ("?4", "NULL"),
            IncomingBody::Error => ("NULL", "?5"),
        };
    let statement = format!(
        "UPDATE analyses SET
            status = ?2,
            submission_outcome = ?3,
            result_json = {body_result_sql},
            error_json = {body_error_sql},
            upstream_version = COALESCE(?6, upstream_version),
            updated_at = ?7,
            completed_at = COALESCE(?8, completed_at)
         WHERE id = ?1"
    );
    let written = transaction
        .execute(
            &statement,
            params![
                id.to_string(),
                wire_status(snapshot.status),
                wire_outcome(snapshot.submission_outcome),
                snapshot.result_json,
                snapshot.error_json,
                snapshot.upstream_version,
                observed_at.to_string(),
                snapshot.completed_at.map(|instant| instant.to_string()),
            ],
        )
        .map_err(|_| {
            HistoryError::from_sqlite(
                HistoryErrorCode::HistoryWriteFailed,
                "refresh observation snapshot",
            )
        })?;
    if written == 0 {
        return Err(HistoryError::new(
            HistoryErrorCode::NotFound,
            "the recorded analysis no longer exists",
        ));
    }
    let search = (
        prior_search
            .0
            .or_else(|| snapshot.search_input_text.clone()),
        prior_search.1.or_else(|| snapshot.search_filename.clone()),
        snapshot.search_headline.clone().or(prior_search.2),
        snapshot.search_source_urls.clone().or(prior_search.3),
    );
    replace_search_row(
        transaction,
        &id.to_string(),
        &search.0,
        &search.1,
        &search.2,
        &search.3,
    )
}

/// The stored search payload columns synchronized with one analysis row.
pub(super) type SearchColumns = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// The search payload of a refreshed or adopted child row.
pub(in crate::history) fn child_search_by_id_tx(
    transaction: &rusqlite::Transaction<'_>,
    id: &AnalysisId,
) -> Result<SearchColumns, HistoryError> {
    let mut statement = transaction
        .prepare(
            "SELECT input_text, filename, headline, source_urls
             FROM analysis_search WHERE analysis_id = ?1",
        )
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "read search payload")
        })?;
    let mut rows = statement.query([id.to_string()]).map_err(|_| {
        HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "read search payload")
    })?;
    let row = rows
        .next()
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "read search payload")
        })
        .and_then(|row| {
            row.ok_or_else(|| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "read search payload")
            })
        })?;
    let search = (
        row.get::<_, Option<String>>(0),
        row.get::<_, Option<String>>(1),
        row.get::<_, Option<String>>(2),
        row.get::<_, Option<String>>(3),
    );
    let search = match search {
        (Ok(input_text), Ok(filename), Ok(headline), Ok(source_urls)) => {
            (input_text, filename, headline, source_urls)
        }
        _ => {
            return Err(HistoryError::from_sqlite(
                HistoryErrorCode::HistoryCorrupt,
                "read search payload",
            ));
        }
    };
    if rows
        .next()
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "read search payload")
        })?
        .is_some()
    {
        return Err(HistoryError::from_sqlite(
            HistoryErrorCode::HistoryCorrupt,
            "read search payload",
        ));
    }
    Ok(search)
}
