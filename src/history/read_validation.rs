//! Cross-column validation shared by full and summary history reads.

use rusqlite::Connection;

use super::reads::{CanonicalCheckRow, CanonicalTaskRow};
use super::records::{InputKind, StoredAnalysis};
use super::{HistoryError, HistoryErrorCode};

struct BatchParentRow {
    id: String,
    bulk_id: Option<String>,
    bulk_index: Option<i64>,
    caller_id: Option<String>,
    status: String,
    outcome: String,
    save_state: String,
    input_kind: String,
    input_sha256: String,
    display_name: Option<String>,
    input_json: String,
    result_json: Option<String>,
    error_json: Option<String>,
    upstream_version: Option<String>,
    retry_of: Option<String>,
    rerun_of: Option<String>,
    submitted_at: Option<String>,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
    check_count: i64,
    search_rowid: Option<i64>,
    search_analysis_id: Option<String>,
    search_input_text: Option<String>,
    search_filename: Option<String>,
    search_headline: Option<String>,
    search_source_urls: Option<String>,
    upstream_bulk_id: Option<String>,
    bulk_total_items: Option<i64>,
}

struct CertifiedParent {
    record: StoredAnalysis,
    check_count: i64,
    upstream_bulk_id: Option<String>,
}

struct BatchCheckRow {
    analysis_id: String,
    row: CanonicalCheckRow,
}

struct BatchTaskRow {
    analysis_id: String,
    row: CanonicalTaskRow,
}

struct CertifiedBatch {
    canonical: Vec<crate::domain::Analysis<crate::output::CanonicalError>>,
    stored: Vec<StoredAnalysis>,
}

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
    certify_analysis_projection(connection, id)
}

/// Reconstructs and certifies only the requested analysis in the caller's
/// snapshot after the global FK and native FTS prerequisites pass.
pub(super) fn certified_analysis_on(
    connection: &Connection,
    id: &crate::domain::AnalysisId,
    include_input: bool,
) -> Result<crate::domain::Analysis<crate::output::CanonicalError>, HistoryError> {
    certify_analysis_target_inner(connection, id, include_input)?.ok_or_else(target_not_found)
}

/// Certifies only one analysis projection. Mutation paths use this bounded
/// check for their target; destructive operations certify the whole store.
pub(super) fn certify_analysis_projection(
    connection: &Connection,
    id: &crate::domain::AnalysisId,
) -> Result<(), HistoryError> {
    certify_analysis_target_inner(connection, id, true)?
        .map(drop)
        .ok_or_else(target_not_found)
}

fn certify_analysis_target_inner(
    connection: &Connection,
    id: &crate::domain::AnalysisId,
    include_input: bool,
) -> Result<Option<crate::domain::Analysis<crate::output::CanonicalError>>, HistoryError> {
    let mut certified =
        certify_analysis_batch_inner(connection, include_input, true, false, None, Some(id))?;
    Ok(certified.canonical.pop())
}

fn target_not_found() -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::NotFound,
        "no analysis with that identity is recorded",
    )
}

/// Certifies and reconstructs every analysis in identity order with exactly
/// one parent/FTS query, one checks query, and one task-evidence query.
pub(super) fn certify_analysis_batch(
    connection: &Connection,
    include_input: bool,
) -> Result<Vec<crate::domain::Analysis<crate::output::CanonicalError>>, HistoryError> {
    certify_analysis_batch_inner(connection, include_input, true, false, None, None)
        .map(|batch| batch.canonical)
}

/// Certifies every analysis without retaining reconstructed return values.
/// Destructive mutation preflights use this exact whole-store projection.
pub(super) fn certify_analysis_store(connection: &Connection) -> Result<(), HistoryError> {
    certify_analysis_batch_inner(connection, true, false, false, None, None).map(drop)
}

fn certify_analysis_batch_inner(
    connection: &Connection,
    include_input: bool,
    retain_analyses: bool,
    retain_stored: bool,
    bulk: Option<&crate::domain::BulkId>,
    analysis: Option<&crate::domain::AnalysisId>,
) -> Result<CertifiedBatch, HistoryError> {
    let parents = load_batch_parents(connection, bulk, analysis)?;
    let checks = load_batch_checks(connection, bulk, analysis)?;
    let tasks = load_batch_tasks(connection, bulk, analysis)?;
    let mut analyses = if retain_analyses {
        Vec::with_capacity(parents.len())
    } else {
        Vec::new()
    };
    let mut stored = if retain_stored {
        Vec::with_capacity(parents.len())
    } else {
        Vec::new()
    };
    let mut check_start = 0;
    let mut task_start = 0;

    for parent in parents {
        let id = parent.record.id.to_string();
        if checks
            .get(check_start)
            .is_some_and(|row| row.analysis_id < id)
            || tasks
                .get(task_start)
                .is_some_and(|row| row.analysis_id < id)
        {
            return Err(corrupt("certify analysis children"));
        }
        let check_end = advance_analysis_rows(&checks, check_start, &id, |row| &row.analysis_id);
        let task_end = advance_analysis_rows(&tasks, task_start, &id, |row| &row.analysis_id);
        let check_rows = checks[check_start..check_end]
            .iter()
            .map(|row| row.row.clone())
            .collect::<Vec<_>>();
        let task_rows = tasks[task_start..task_end]
            .iter()
            .map(|row| row.row.clone())
            .collect::<Vec<_>>();
        let canonical = super::reads::canonical_analysis_from_rows(
            &parent.record,
            parent.check_count,
            &check_rows,
            &task_rows,
            parent.upstream_bulk_id.as_deref(),
            true,
        )?;
        certify_canonical_projection(&parent.record, &canonical, &check_rows, &task_rows)?;
        if retain_analyses {
            analyses.push(if include_input {
                canonical
            } else {
                super::reads::canonical_analysis_from_rows(
                    &parent.record,
                    parent.check_count,
                    &check_rows,
                    &task_rows,
                    parent.upstream_bulk_id.as_deref(),
                    false,
                )?
            });
        }
        if retain_stored {
            stored.push(parent.record);
        }
        check_start = check_end;
        task_start = task_end;
    }
    if check_start != checks.len() || task_start != tasks.len() {
        return Err(corrupt("certify analysis children"));
    }
    Ok(CertifiedBatch {
        canonical: analyses,
        stored,
    })
}

fn advance_analysis_rows<T>(
    rows: &[T],
    start: usize,
    id: &str,
    analysis_id: impl Fn(&T) -> &String,
) -> usize {
    let mut end = start;
    while end < rows.len() && analysis_id(&rows[end]) == id {
        end += 1;
    }
    end
}

fn certify_canonical_projection(
    record: &StoredAnalysis,
    analysis: &crate::domain::Analysis<crate::output::CanonicalError>,
    checks: &[CanonicalCheckRow],
    tasks: &[CanonicalTaskRow],
) -> Result<(), HistoryError> {
    let retained_text = analysis.input().and_then(|input| match input {
        crate::domain::AnalysisInput::Text(input) => input.text.as_deref(),
        crate::domain::AnalysisInput::File(_) => None,
    });
    let projected =
        super::save::stored_analysis_with_retained_text(analysis, record.save_state, retained_text)
            .map_err(|_| corrupt("certify analysis projection"))?;
    let projected_checks =
        super::save::stored_checks(analysis).map_err(|_| corrupt("certify check projection"))?;
    let projected_tasks = super::save::stored_observations(analysis);
    let lifecycle_valid = record.completed_at.is_some()
        == !matches!(
            record.status,
            crate::domain::AnalysisStatus::Queued | crate::domain::AnalysisStatus::Running
        );
    let canonical_parent = record.input_kind == projected.input_kind
        && record.input_sha256 == projected.input_sha256
        && record.display_name == projected.display_name
        && record.input_json == projected.input_json
        && record.status == projected.status
        && record.submission_outcome == projected.submission_outcome
        && record.save_state == projected.save_state
        && record.result_json == projected.result_json
        && record.error_json == projected.error_json
        && record.upstream_version == projected.upstream_version
        && record.retry_of == projected.retry_of
        && record.rerun_of == projected.rerun_of
        && record.submitted_at == projected.submitted_at
        && record.created_at == projected.created_at
        && record.updated_at == projected.updated_at
        && record.completed_at == projected.completed_at
        && record.search_input_text == projected.search_input_text
        && record.search_filename == projected.search_filename
        && record.search_headline == projected.search_headline
        && record.search_source_urls == projected.search_source_urls;
    let canonical_checks = checks.len() == projected_checks.len()
        && checks
            .iter()
            .zip(&projected_checks)
            .all(|(stored, projected)| {
                stored.index == i64::from(projected.check_index)
                    && stored.kind == super::wire::wire_check_kind(projected.check_kind)
                    && stored.status == super::wire::wire_check_status(projected.status)
                    && stored.result_json == projected.result_json
                    && stored.error_json == projected.error_json
            });
    let canonical_tasks = tasks.len() == projected_tasks.len()
        && tasks
            .iter()
            .zip(&projected_tasks)
            .all(|(stored, projected)| {
                stored.kind == super::wire::wire_check_kind(projected.check_kind)
                    && stored.upstream_task_id == projected.upstream_task_id
                    && stored.last_stage == projected.last_stage
                    && stored
                        .observed_at
                        .parse::<crate::domain::UtcTimestamp>()
                        .is_ok_and(|stamp| stored.observed_at == stamp.to_string())
            });
    if lifecycle_valid && canonical_parent && canonical_checks && canonical_tasks {
        Ok(())
    } else {
        Err(corrupt("certify analysis aggregate"))
    }
}

fn load_batch_parents(
    connection: &Connection,
    bulk: Option<&crate::domain::BulkId>,
    analysis: Option<&crate::domain::AnalysisId>,
) -> Result<Vec<CertifiedParent>, HistoryError> {
    let scope = match (bulk, analysis) {
        (None, Some(_)) => "a.id = ?2 AND ?1 IS NULL",
        (Some(_), None) => "a.bulk_id = ?1 AND ?2 IS NULL",
        (None, None) => "?1 IS NULL AND ?2 IS NULL",
        (Some(_), Some(_)) => unreachable!("analysis certification scopes cannot overlap"),
    };
    let mut statement = connection
        .prepare(&format!(
            "SELECT a.id, a.bulk_id, a.bulk_index, a.caller_id, a.status,
                    a.submission_outcome, a.save_state, a.input_type,
                    a.input_sha256, a.display_name, a.input_json, a.result_json,
                    a.error_json, a.upstream_version, a.retry_of, a.rerun_of,
                    a.submitted_at, a.created_at, a.updated_at, a.completed_at,
                    a.check_count, s.rowid, s.analysis_id, s.input_text,
                    s.filename, s.headline, s.source_urls, b.upstream_bulk_id,
                    b.total_items
             FROM analyses a
             FULL OUTER JOIN analysis_search s ON s.analysis_id = a.id
             LEFT JOIN bulk_collections b ON b.id = a.bulk_id
             WHERE {scope}
             ORDER BY a.id, s.rowid"
        ))
        .map_err(|_| unavailable("certify analysis parents"))?;
    let bulk_id = bulk.map(ToString::to_string);
    let analysis_id = analysis.map(ToString::to_string);
    let rows = statement
        .query_map(rusqlite::params![bulk_id, analysis_id], |row| {
            Ok(BatchParentRow {
                id: row.get(0)?,
                bulk_id: row.get(1)?,
                bulk_index: row.get(2)?,
                caller_id: row.get(3)?,
                status: row.get(4)?,
                outcome: row.get(5)?,
                save_state: row.get(6)?,
                input_kind: row.get(7)?,
                input_sha256: row.get(8)?,
                display_name: row.get(9)?,
                input_json: row.get(10)?,
                result_json: row.get(11)?,
                error_json: row.get(12)?,
                upstream_version: row.get(13)?,
                retry_of: row.get(14)?,
                rerun_of: row.get(15)?,
                submitted_at: row.get(16)?,
                created_at: row.get(17)?,
                updated_at: row.get(18)?,
                completed_at: row.get(19)?,
                check_count: row.get(20)?,
                search_rowid: row.get(21)?,
                search_analysis_id: row.get(22)?,
                search_input_text: row.get(23)?,
                search_filename: row.get(24)?,
                search_headline: row.get(25)?,
                search_source_urls: row.get(26)?,
                upstream_bulk_id: row.get(27)?,
                bulk_total_items: row.get(28)?,
            })
        })
        .map_err(|_| unavailable("certify analysis parents"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| corrupt("certify analysis parents"))?;
    let mut parents = Vec::with_capacity(rows.len());
    for row in rows {
        if parents
            .last()
            .is_some_and(|prior: &CertifiedParent| prior.record.id.to_string() == row.id)
            || row.search_rowid.is_none()
            || row.search_analysis_id.as_deref() != Some(row.id.as_str())
        {
            return Err(corrupt("certify analysis search row"));
        }
        let id = row
            .id
            .parse::<crate::domain::AnalysisId>()
            .map_err(|_| corrupt("certify analysis identity"))?;
        let bulk = match (row.bulk_id.as_deref(), row.bulk_index, row.bulk_total_items) {
            (None, None, None) => None,
            (Some(bulk_id), Some(index), Some(total)) if index >= 0 && total > index => Some((
                bulk_id
                    .parse::<crate::domain::BulkId>()
                    .map_err(|_| corrupt("certify analysis membership"))?,
                index,
            )),
            _ => return Err(corrupt("certify analysis membership")),
        };
        parents.push(CertifiedParent {
            record: StoredAnalysis {
                id,
                bulk,
                caller_id: row.caller_id,
                status: super::wire::unwire_status(&row.status)
                    .map_err(|_| corrupt("certify analysis status"))?,
                submission_outcome: super::wire::unwire_outcome(&row.outcome)
                    .map_err(|_| corrupt("certify analysis outcome"))?,
                save_state: super::wire::unwire_save_state(&row.save_state)
                    .map_err(|_| corrupt("certify analysis save state"))?,
                input_kind: InputKind::parse(&row.input_kind)
                    .map_err(|_| corrupt("certify analysis input"))?,
                input_sha256: row
                    .input_sha256
                    .parse()
                    .map_err(|_| corrupt("certify analysis input"))?,
                display_name: row.display_name,
                input_json: row.input_json,
                result_json: row.result_json,
                error_json: row.error_json,
                upstream_version: row.upstream_version,
                retry_of: row
                    .retry_of
                    .map(|value| value.parse())
                    .transpose()
                    .map_err(|_| corrupt("certify analysis retry identity"))?,
                rerun_of: row
                    .rerun_of
                    .map(|value| value.parse())
                    .transpose()
                    .map_err(|_| corrupt("certify analysis rerun identity"))?,
                submitted_at: row
                    .submitted_at
                    .map(|value| value.parse())
                    .transpose()
                    .map_err(|_| corrupt("certify analysis timestamps"))?,
                created_at: row
                    .created_at
                    .parse()
                    .map_err(|_| corrupt("certify analysis timestamps"))?,
                updated_at: row
                    .updated_at
                    .parse()
                    .map_err(|_| corrupt("certify analysis timestamps"))?,
                completed_at: row
                    .completed_at
                    .map(|value| value.parse())
                    .transpose()
                    .map_err(|_| corrupt("certify analysis timestamps"))?,
                search_input_text: row.search_input_text,
                search_filename: row.search_filename,
                search_headline: row.search_headline,
                search_source_urls: row.search_source_urls,
            },
            check_count: row.check_count,
            upstream_bulk_id: row.upstream_bulk_id,
        });
    }
    Ok(parents)
}

fn load_batch_checks(
    connection: &Connection,
    bulk: Option<&crate::domain::BulkId>,
    analysis: Option<&crate::domain::AnalysisId>,
) -> Result<Vec<BatchCheckRow>, HistoryError> {
    let scope = match (bulk, analysis) {
        (None, Some(_)) => "analysis_id = ?2 AND ?1 IS NULL",
        (Some(_), None) => {
            "analysis_id IN (SELECT id FROM analyses WHERE bulk_id = ?1) AND ?2 IS NULL"
        }
        (None, None) => "?1 IS NULL AND ?2 IS NULL",
        (Some(_), Some(_)) => unreachable!("analysis certification scopes cannot overlap"),
    };
    let mut statement = connection
        .prepare(&format!(
            "SELECT analysis_id, check_index, check_kind, status, result_json, error_json
             FROM analysis_checks
             WHERE {scope}
             ORDER BY analysis_id, check_index"
        ))
        .map_err(|_| unavailable("certify analysis checks"))?;
    let bulk_id = bulk.map(ToString::to_string);
    let analysis_id = analysis.map(ToString::to_string);
    statement
        .query_map(rusqlite::params![bulk_id, analysis_id], |row| {
            Ok(BatchCheckRow {
                analysis_id: row.get(0)?,
                row: CanonicalCheckRow {
                    index: row.get(1)?,
                    kind: row.get(2)?,
                    status: row.get(3)?,
                    result_json: row.get(4)?,
                    error_json: row.get(5)?,
                },
            })
        })
        .map_err(|_| unavailable("certify analysis checks"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| corrupt("certify analysis checks"))
}

fn load_batch_tasks(
    connection: &Connection,
    bulk: Option<&crate::domain::BulkId>,
    analysis: Option<&crate::domain::AnalysisId>,
) -> Result<Vec<BatchTaskRow>, HistoryError> {
    let scope = match (bulk, analysis) {
        (None, Some(_)) => "analysis_id = ?2 AND ?1 IS NULL",
        (Some(_), None) => {
            "analysis_id IN (SELECT id FROM analyses WHERE bulk_id = ?1) AND ?2 IS NULL"
        }
        (None, None) => "?1 IS NULL AND ?2 IS NULL",
        (Some(_), Some(_)) => unreachable!("analysis certification scopes cannot overlap"),
    };
    let mut statement = connection
        .prepare(&format!(
            "SELECT analysis_id, check_kind, upstream_task_id, last_stage, observed_at
             FROM upstream_tasks
             WHERE {scope}
             ORDER BY analysis_id, check_kind"
        ))
        .map_err(|_| unavailable("certify analysis tasks"))?;
    let bulk_id = bulk.map(ToString::to_string);
    let analysis_id = analysis.map(ToString::to_string);
    statement
        .query_map(rusqlite::params![bulk_id, analysis_id], |row| {
            Ok(BatchTaskRow {
                analysis_id: row.get(0)?,
                row: CanonicalTaskRow {
                    kind: row.get(1)?,
                    upstream_task_id: row.get(2)?,
                    last_stage: row.get(3)?,
                    observed_at: row.get(4)?,
                },
            })
        })
        .map_err(|_| unavailable("certify analysis tasks"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| corrupt("certify analysis tasks"))
}

/// Certifies one bulk row and every analysis currently attached to it.
pub(super) fn certify_bulk_aggregate(
    connection: &Connection,
    id: &crate::domain::BulkId,
) -> Result<(), HistoryError> {
    certify_bulk_analysis_batch(connection, id, false).map(drop)
}

/// Certifies one bulk row and every attached analysis, returning the already
/// loaded stored rows in membership order without rereading their parent,
/// FTS, or bulk-membership data.
pub(super) fn certify_bulk_analyses(
    connection: &Connection,
    id: &crate::domain::BulkId,
) -> Result<Vec<StoredAnalysis>, HistoryError> {
    let mut stored = certify_bulk_analysis_batch(connection, id, true)?;
    stored.sort_by_key(|record| record.bulk.map(|(_, index)| index));
    Ok(stored)
}

fn certify_bulk_analysis_batch(
    connection: &Connection,
    id: &crate::domain::BulkId,
    retain_stored: bool,
) -> Result<Vec<StoredAnalysis>, HistoryError> {
    let exists = connection
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM bulk_collections WHERE id = ?1)",
            [id.to_string()],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| unavailable("certify bulk identity"))?;
    exists.then_some(()).ok_or_else(|| {
        HistoryError::new(
            HistoryErrorCode::NotFound,
            "no bulk collection with that identity is recorded",
        )
    })?;
    certify_bulk_aggregate_rows(connection, id)?;
    certify_analysis_batch_inner(connection, true, false, retain_stored, Some(id), None)
        .map(|batch| batch.stored)
}

fn certify_bulk_aggregate_rows(
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

    Ok(())
}

/// Certifies every logical history aggregate in the caller's transaction.
/// Destructive mutations call this before their first write.
pub(super) fn certify_store_integrity(connection: &Connection) -> Result<(), HistoryError> {
    super::search::certify_search_index_for_write(connection)?;
    certify_foreign_keys(connection)?;
    certify_analysis_store(connection)?;
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
        certify_bulk_aggregate_rows(connection, &id)?;
    }
    Ok(())
}

/// Fully consumes SQLite's authoritative foreign-key violation report.
///
/// Foreign-key enforcement prevents application writes from creating these
/// rows, but an external connection can disable enforcement. Certification
/// therefore checks the persisted relationships themselves inside the
/// caller's snapshot before any mutation can erase or repair the evidence.
pub(super) fn certify_foreign_keys(connection: &Connection) -> Result<(), HistoryError> {
    type ForeignKeyViolation = (String, Option<i64>, String, i64);

    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|_| unavailable("certify foreign keys"))?;
    let violations = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| unavailable("certify foreign keys"))?
        .collect::<Result<Vec<ForeignKeyViolation>, _>>()
        .map_err(|_| corrupt("certify foreign keys"))?;
    if !violations.is_empty() {
        return Err(corrupt("certify foreign keys"));
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
