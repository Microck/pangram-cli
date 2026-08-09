//! Shared reconciliation primitives for terminal-body and search-payload
//! refreshes.

use rusqlite::params;

use crate::domain::AnalysisId;

use super::super::analysis_writes::{
    load_stored_check_rows, replace_check_rows, replace_search_row, set_check_count,
    validate_check_rows,
};
use super::super::records::StoredCheck;
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
    authoritative_checks: &[StoredCheck],
    replace_authoritative_checks: bool,
) -> Result<(), HistoryError> {
    let prior_search = child_search_by_id_tx(transaction, id)?;
    let prior_terminal = stored_terminal_snapshot_tx(transaction, id)?;
    let stored_checks = load_stored_check_rows(transaction, id)?;
    let incoming_checks = authoritative_checks
        .iter()
        .cloned()
        .map(|check| StoredCheck {
            analysis_id: *id,
            ..check
        })
        .collect::<Vec<_>>();
    let incoming_body = incoming_body(&snapshot.result_json, &snapshot.error_json)?;
    let terminal_dominates = prior_terminal && matches!(incoming_body, IncomingBody::Empty);
    let replace_checks = !terminal_dominates
        && (replace_authoritative_checks || !matches!(incoming_body, IncomingBody::Empty));
    let checks = if replace_checks && !replace_authoritative_checks {
        validate_check_rows(*id, &incoming_checks)?;
        let mut merged = stored_checks;
        for incoming in incoming_checks {
            let stored = merged
                .iter_mut()
                .find(|stored| stored.check_kind == incoming.check_kind)
                .ok_or_else(|| {
                    HistoryError::new(
                        HistoryErrorCode::HistoryWriteFailed,
                        "an observation cannot add a check kind not owned by the analysis",
                    )
                })?;
            let check_index = stored.check_index;
            *stored = StoredCheck {
                check_index,
                ..incoming
            };
        }
        merged
    } else {
        incoming_checks
    };
    if replace_checks {
        validate_check_rows(*id, &checks)?;
    }
    let (body_result_sql, body_error_sql) = match incoming_body {
        IncomingBody::Empty => ("COALESCE(?4, result_json)", "COALESCE(?5, error_json)"),
        IncomingBody::Result => ("?4", "NULL"),
        IncomingBody::Error => ("NULL", "?5"),
    };
    let statement = format!(
        "UPDATE analyses SET
            status = CASE WHEN ?9 THEN status ELSE ?2 END,
            submission_outcome = CASE WHEN ?9 THEN submission_outcome ELSE ?3 END,
            result_json = {body_result_sql},
            error_json = {body_error_sql},
            upstream_version = CASE
                WHEN ?9 THEN upstream_version
                ELSE COALESCE(?6, upstream_version)
            END,
            updated_at = ?7,
            completed_at = CASE
                WHEN ?9 THEN completed_at
                ELSE COALESCE(?8, completed_at)
            END
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
                terminal_dominates,
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
    if replace_checks {
        replace_check_rows(transaction, *id, &checks)?;
        set_check_count(transaction, *id, checks.len())?;
    }
    let search = if terminal_dominates {
        prior_search
    } else {
        (
            prior_search
                .0
                .or_else(|| snapshot.search_input_text.clone()),
            prior_search.1.or_else(|| snapshot.search_filename.clone()),
            snapshot.search_headline.clone(),
            snapshot.search_source_urls.clone(),
        )
    };
    replace_search_row(
        transaction,
        &id.to_string(),
        &search.0,
        &search.1,
        &search.2,
        &search.3,
    )
}

pub(super) fn stored_terminal_snapshot_tx(
    transaction: &rusqlite::Transaction<'_>,
    id: &AnalysisId,
) -> Result<bool, HistoryError> {
    transaction
        .query_row(
            "SELECT result_json IS NOT NULL OR error_json IS NOT NULL
             FROM analyses WHERE id = ?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .map_err(|_| {
            HistoryError::from_sqlite(
                HistoryErrorCode::HistoryCorrupt,
                "read terminal observation snapshot",
            )
        })
}

/// Validates one existing durable analysis through the exact canonical
/// reconstruction used by `history show`, while borrowing the caller's
/// transaction. Reconciliation calls this before its first mutation so
/// corrupt input/provenance, lineage, membership, checks, task evidence, or
/// FTS state can never be silently repaired by an upsert.
pub(super) fn certify_existing_analysis_tx(
    transaction: &rusqlite::Transaction<'_>,
    id: &AnalysisId,
) -> Result<(), HistoryError> {
    super::super::read_validation::certify_analysis_projection(transaction, id)
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
