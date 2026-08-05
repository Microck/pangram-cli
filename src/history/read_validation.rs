//! Cross-column validation shared by full and summary history reads.

use rusqlite::Connection;

use super::records::{InputKind, StoredAnalysis};
use super::{HistoryError, HistoryErrorCode};

type SummaryCheckRow = (
    String,
    String,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub(super) fn validate_stored_input(
    record: &StoredAnalysis,
    value: &serde_json::Value,
) -> Result<(), HistoryError> {
    if value.is_null() {
        let absent_is_canonical = record.input_kind == InputKind::Text
            && record.input_sha256 == crate::domain::Sha256Hash::from_bytes([0; 32])
            && record.display_name.is_none()
            && record.search_input_text.is_none()
            && record.search_filename.is_none();
        return absent_is_canonical
            .then_some(())
            .ok_or_else(|| corrupt("read absent stored input"));
    }

    let input: crate::domain::AnalysisInput =
        serde_json::from_value(value.clone()).map_err(|_| corrupt("read stored input"))?;
    match input {
        crate::domain::AnalysisInput::Text(input) => {
            if record.input_kind != InputKind::Text || record.input_sha256 != input.sha256 {
                return Err(corrupt("read stored input columns"));
            }
            if let Some(text) = input.text.as_deref() {
                let byte_count = u64::try_from(text.len()).unwrap_or(u64::MAX);
                let word_count = crate::analysis::canonical_text_word_count(text);
                if input.sha256 != crate::domain::Sha256Hash::digest(text.as_bytes())
                    || input.byte_count != byte_count
                    || input.word_count != word_count
                {
                    return Err(corrupt("verify retained stored input"));
                }
            }
        }
        crate::domain::AnalysisInput::File(input) => {
            if record.input_kind != InputKind::File || record.input_sha256 != input.sha256 {
                return Err(corrupt("read stored input columns"));
            }
        }
    }
    Ok(())
}

/// Certifies one complete stored analysis through the same canonical
/// reconstruction and projection owners used by history reads and writes.
///
/// Re-projecting the reconstructed value is what proves the FTS payload and
/// legacy aggregate body columns contain the canonical content, rather than
/// merely proving that one row exists with SQLite-compatible types.
pub(super) fn certify_analysis_aggregate(
    connection: &Connection,
    id: &crate::domain::AnalysisId,
) -> Result<(), HistoryError> {
    let record = super::reads::stored_analysis_on(connection, id)?;
    let analysis = super::reads::canonical_analysis_on(connection, &record, true)?;
    let input: serde_json::Value =
        serde_json::from_str(&record.input_json).map_err(|_| corrupt("certify analysis input"))?;
    let retained_text = input.get("text").and_then(serde_json::Value::as_str);
    let projected = super::save::stored_analysis_with_retained_text(
        &analysis,
        record.save_state,
        retained_text,
    )
    .map_err(|_| corrupt("certify analysis projection"))?;
    let lifecycle_valid = record.completed_at.is_some()
        == !matches!(
            record.status,
            crate::domain::AnalysisStatus::Queued | crate::domain::AnalysisStatus::Running
        );
    // Input-derived FTS fields have one deterministic owner. Result-derived
    // headline/source fields may intentionally retain a prior terminal
    // snapshot across a body-less refresh, so canonical reconstruction
    // validates their types/cardinality while the write transaction proves
    // their exact supplied bytes.
    let canonical_content = record.input_kind == projected.input_kind
        && record.input_sha256 == projected.input_sha256
        && record.display_name == projected.display_name
        && record.search_input_text == projected.search_input_text
        && record.search_filename == projected.search_filename;
    if !lifecycle_valid || !canonical_content {
        return Err(corrupt("certify analysis aggregate"));
    }
    Ok(())
}

/// Certifies one bulk row and every analysis currently attached to it.
pub(super) fn certify_bulk_aggregate(
    connection: &Connection,
    id: &crate::domain::BulkId,
) -> Result<(), HistoryError> {
    let raw = connection
        .query_row(
            "SELECT id, upstream_bulk_id, status, submission_outcome, total_items,
                    accepted, succeeded, failed, estimated_billable_units, created_at,
                    updated_at, completed_at
             FROM bulk_collections WHERE id = ?1",
            [id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            },
        )
        .map_err(|_| corrupt("certify bulk collection"))?;
    if [raw.4, raw.5, raw.6, raw.7, raw.8]
        .into_iter()
        .any(|value| value < 0)
    {
        return Err(corrupt("certify bulk counters"));
    }
    let bulk_id = raw
        .0
        .parse::<crate::domain::BulkId>()
        .map_err(|_| corrupt("certify bulk identity"))?;
    if bulk_id != *id {
        return Err(corrupt("certify bulk identity"));
    }
    let upstream = raw
        .1
        .map(|value| value.parse::<crate::domain::UpstreamBulkId>())
        .transpose()
        .map_err(|_| corrupt("certify bulk identity"))?;
    let status = super::wire::unwire_status(&raw.2)
        .map_err(|_| corrupt("certify bulk collection status"))?;
    let outcome = super::wire::unwire_outcome(&raw.3)
        .map_err(|_| corrupt("certify bulk collection outcome"))?;
    let counters =
        crate::domain::BulkCounters::new(raw.4 as u64, raw.5 as u64, raw.6 as u64, raw.7 as u64)
            .map_err(|_| corrupt("certify bulk counters"))?;
    let created_at = raw
        .9
        .parse::<crate::domain::UtcTimestamp>()
        .map_err(|_| corrupt("certify bulk timestamps"))?;
    let updated_at = raw
        .10
        .parse::<crate::domain::UtcTimestamp>()
        .map_err(|_| corrupt("certify bulk timestamps"))?;
    let completed_at = raw
        .11
        .map(|value| value.parse::<crate::domain::UtcTimestamp>())
        .transpose()
        .map_err(|_| corrupt("certify bulk timestamps"))?;
    let collection = crate::domain::BulkCollection::new(
        bulk_id,
        upstream,
        status,
        outcome,
        counters,
        (raw.8 != 0).then_some(raw.8 as u64),
        created_at,
        updated_at,
        completed_at,
    )
    .map_err(|_| corrupt("certify bulk collection"))?;
    let lifecycle_valid = completed_at.is_some()
        == !matches!(
            collection.status(),
            crate::domain::AnalysisStatus::Queued | crate::domain::AnalysisStatus::Running
        );
    if !lifecycle_valid {
        return Err(corrupt("certify bulk timestamps"));
    }

    let mut statement = connection
        .prepare("SELECT id FROM analyses WHERE bulk_id = ?1 ORDER BY bulk_index")
        .map_err(|_| unavailable("certify bulk members"))?;
    let members = statement
        .query_map([id.to_string()], |row| row.get::<_, String>(0))
        .map_err(|_| unavailable("certify bulk members"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| corrupt("certify bulk members"))?;
    for member in members {
        let member = member
            .parse::<crate::domain::AnalysisId>()
            .map_err(|_| corrupt("certify bulk members"))?;
        certify_analysis_aggregate(connection, &member)?;
    }
    Ok(())
}

/// Certifies every logical history aggregate in the caller's transaction.
/// Destructive mutations call this before their first write.
pub(super) fn certify_store_integrity(connection: &Connection) -> Result<(), HistoryError> {
    super::search::certify_search_index(connection)?;
    let mut analyses = connection
        .prepare("SELECT id FROM analyses ORDER BY id")
        .map_err(|_| unavailable("certify history"))?;
    let analysis_ids = analyses
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| unavailable("certify history"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| corrupt("certify history"))?;
    for id in analysis_ids {
        let id = id
            .parse::<crate::domain::AnalysisId>()
            .map_err(|_| corrupt("certify history"))?;
        certify_analysis_aggregate(connection, &id)?;
    }
    let mut bulks = connection
        .prepare("SELECT id FROM bulk_collections ORDER BY id")
        .map_err(|_| unavailable("certify history"))?;
    let bulk_ids = bulks
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| unavailable("certify history"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| corrupt("certify history"))?;
    for id in bulk_ids {
        let id = id
            .parse::<crate::domain::BulkId>()
            .map_err(|_| corrupt("certify history"))?;
        certify_bulk_aggregate(connection, &id)?;
    }
    Ok(())
}

/// Validates every nullable bulk-membership pair and its parent range.
pub(super) fn certify_analysis_memberships(connection: &Connection) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT a.bulk_id, a.bulk_index, b.total_items
             FROM analyses a
             LEFT JOIN bulk_collections b ON b.id = a.bulk_id
             WHERE a.bulk_id IS NOT NULL OR a.bulk_index IS NOT NULL",
        )
        .map_err(|_| unavailable("validate bulk membership"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })
        .map_err(|_| unavailable("validate bulk membership"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| corrupt("validate bulk membership"))?;
    for (bulk_id, index, total_items) in rows {
        let valid = match (bulk_id, index, total_items) {
            (Some(id), Some(index), Some(total)) => {
                id.parse::<crate::domain::BulkId>().is_ok()
                    && index >= 0
                    && total >= 0
                    && index < total
            }
            _ => false,
        };
        if !valid {
            return Err(corrupt("validate bulk membership"));
        }
    }
    Ok(())
}

/// Validates the complete authoritative check surface used by list/search.
///
/// One ordered set query covers every parent and check row in the caller's
/// read snapshot. This deliberately avoids reconstructing summaries through
/// an N+1 query pattern while still refusing to summarize corrupt evidence.
pub(super) fn certify_summary_checks(connection: &Connection) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT a.id, a.status, a.check_count,
                    c.check_index, c.check_kind, c.status,
                    c.result_json, c.error_json
             FROM analyses a
             LEFT JOIN analysis_checks c ON c.analysis_id = a.id
             ORDER BY a.id, c.check_index",
        )
        .map_err(|_| unavailable("validate summary checks"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })
        .map_err(|_| unavailable("validate summary checks"))?
        .collect::<Result<Vec<SummaryCheckRow>, _>>()
        .map_err(|_| corrupt("validate summary checks"))?;

    let mut start = 0;
    while start < rows.len() {
        let id = rows[start].0.as_str();
        let mut end = start + 1;
        while end < rows.len() && rows[end].0 == id {
            end += 1;
        }
        validate_summary_group(&rows[start..end])?;
        start = end;
    }
    Ok(())
}

fn validate_summary_group(rows: &[SummaryCheckRow]) -> Result<(), HistoryError> {
    let (id, parent_status, check_count, ..) = rows
        .first()
        .ok_or_else(|| corrupt("validate summary checks"))?;
    id.parse::<crate::domain::AnalysisId>()
        .map_err(|_| corrupt("validate summary checks"))?;
    let parent_status: crate::domain::AnalysisStatus =
        serde_json::from_value(serde_json::Value::String(parent_status.clone()))
            .map_err(|_| corrupt("validate summary checks"))?;
    let expected = usize::try_from(*check_count)
        .ok()
        .filter(|count| (1..=2).contains(count))
        .ok_or_else(|| corrupt("validate summary checks"))?;
    if rows.len() != expected {
        return Err(corrupt("validate summary checks"));
    }

    let mut checks = Vec::with_capacity(expected);
    for (expected_index, row) in rows.iter().enumerate() {
        if row.0.as_str() != id
            || row.1 != rows[0].1
            || row.2 != *check_count
            || row.3 != i64::try_from(expected_index).ok()
        {
            return Err(corrupt("validate summary checks"));
        }
        let kind = row
            .4
            .as_deref()
            .ok_or_else(|| corrupt("validate summary checks"))?;
        let status = row
            .5
            .as_deref()
            .ok_or_else(|| corrupt("validate summary checks"))?;
        let mut value = serde_json::json!({"kind": kind, "status": status});
        if let Some(result) = &row.6 {
            value["result"] =
                serde_json::from_str(result).map_err(|_| corrupt("validate summary checks"))?;
        }
        if let Some(error) = &row.7 {
            value["error"] =
                serde_json::from_str(error).map_err(|_| corrupt("validate summary checks"))?;
        }
        let check: crate::domain::Check<crate::output::CanonicalError> =
            serde_json::from_value(value).map_err(|_| corrupt("validate summary checks"))?;
        checks.push(check);
    }
    let checks = crate::domain::OrderedChecks::new(checks)
        .map_err(|_| corrupt("validate summary checks"))?;
    let statuses = checks
        .iter()
        .map(crate::domain::Check::status)
        .collect::<Vec<_>>();
    let derived = crate::domain::derive_parent_status(&statuses)
        .map_err(|_| corrupt("validate summary checks"))?;
    if derived != parent_status {
        return Err(corrupt("validate summary checks"));
    }
    Ok(())
}

pub(super) fn decode_analysis_membership(
    connection: &Connection,
    bulk_id: Option<String>,
    index: Option<i64>,
) -> Result<Option<(crate::domain::BulkId, i64)>, rusqlite::Error> {
    match (bulk_id, index) {
        (None, None) => Ok(None),
        (Some(id), Some(index)) if index >= 0 => {
            let parsed = id.parse().map_err(|_| rusqlite::Error::InvalidQuery)?;
            let total = connection
                .query_row(
                    "SELECT total_items FROM bulk_collections WHERE id = ?1",
                    [id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            if total < 0 || index >= total {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok(Some((parsed, index)))
        }
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

pub(super) fn validated_task_evidence_count(
    connection: &Connection,
    analysis_id: &crate::domain::AnalysisId,
) -> Result<usize, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT t.check_kind, t.upstream_task_id, t.last_stage, t.observed_at,
                    EXISTS (
                        SELECT 1 FROM analysis_checks c
                        WHERE c.analysis_id = t.analysis_id
                          AND c.check_kind = t.check_kind
                    )
             FROM upstream_tasks t
             WHERE t.analysis_id = ?1
             ORDER BY t.check_kind",
        )
        .map_err(|_| unavailable("read stored task evidence"))?;
    let rows = statement
        .query_map([analysis_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })
        .map_err(|_| unavailable("read stored task evidence"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| corrupt("read stored task evidence"))?;
    for (kind, task_id, last_stage, observed_at, has_check) in &rows {
        let valid = super::wire::unwire_check_kind(kind).is_ok()
            && task_id.parse::<crate::domain::UpstreamTaskId>().is_ok()
            && last_stage
                .as_deref()
                .is_none_or(|value| value.parse::<crate::domain::NonEmptyString>().is_ok())
            && observed_at.parse::<crate::domain::UtcTimestamp>().is_ok()
            && *has_check;
        if !valid {
            return Err(corrupt("read stored task evidence"));
        }
    }
    Ok(rows.len())
}

fn corrupt(operation: &'static str) -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::HistoryCorrupt,
        format!(
            "{operation}: the history database contains an invalid canonical value. \
             Move the history directory aside and rerun the command; the original file is preserved."
        ),
    )
}

fn unavailable(operation: &'static str) -> HistoryError {
    HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, operation)
}
