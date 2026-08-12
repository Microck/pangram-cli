//! Validated history summaries and literal FTS5 search.

use rusqlite::{Connection, params};

use crate::domain::{AnalysisStatus, CheckKind};

use super::records::StoredSearchHit;
use super::store::HistoryStore;
use super::wire::{row_to_summary, wire_check_kind, wire_status};
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

impl HistoryStore {
    /// Most recent analyses first, for `history list`.
    pub fn list(&self, limit: u32, offset: u32) -> Result<Vec<StoredSearchHit>, HistoryError> {
        self.list_filtered(None, None, limit, offset)
    }

    /// Every saved queued or running analysis, independent of any paginated
    /// or filtered history view.
    pub fn list_unfinished(&self) -> Result<Vec<StoredSearchHit>, HistoryError> {
        self.with_read_snapshot(|connection| {
            certify_list_search_store(connection)?;
            let mut statement = connection
                .prepare(
                    "SELECT a.id, a.status,
                            COALESCE((
                              SELECT group_concat(k.check_kind, ',') FROM (
                                SELECT check_kind FROM analysis_checks
                                WHERE analysis_id = a.id
                                ORDER BY check_index
                              ) k
                            ), ''),
                            a.check_count, a.save_state, a.input_type, a.display_name, a.created_at
                     FROM analyses a
                     WHERE a.status IN (?1, ?2)
                     ORDER BY a.created_at DESC, a.id",
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "list unfinished analyses",
                    )
                })?;
            statement
                .query_map(
                    params![
                        wire_status(AnalysisStatus::Queued),
                        wire_status(AnalysisStatus::Running)
                    ],
                    row_to_summary,
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "list unfinished analyses",
                    )
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryCorrupt,
                        "list unfinished analyses",
                    )
                })
        })
    }

    /// Most recent analyses first with closed parent/check filters.
    pub fn list_filtered(
        &self,
        status: Option<AnalysisStatus>,
        check: Option<CheckKind>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<StoredSearchHit>, HistoryError> {
        self.with_read_snapshot(|connection| {
            certify_list_search_store(connection)?;
            let mut statement = connection
                .prepare(
                    "SELECT a.id, a.status,
                            COALESCE((
                              SELECT group_concat(k.check_kind, ',') FROM (
                                SELECT check_kind FROM analysis_checks
                                WHERE analysis_id = a.id
                                ORDER BY check_index
                              ) k
                            ), ''),
                            a.check_count, a.save_state, a.input_type, a.display_name, a.created_at
                     FROM analyses a
                     WHERE (?1 IS NULL OR a.status = ?1)
                       AND (?2 IS NULL OR EXISTS (
                              SELECT 1 FROM analysis_checks t
                              WHERE t.analysis_id = a.id AND t.check_kind = ?2
                            ))
                     ORDER BY a.created_at DESC, a.id
                     LIMIT ?3 OFFSET ?4",
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "list analyses")
                })?;
            statement
                .query_map(
                    params![
                        status.map(wire_status),
                        check.map(wire_check_kind),
                        limit,
                        offset
                    ],
                    row_to_summary,
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "list analyses")
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| {
                    HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "list analyses")
                })
        })
    }

    /// FTS5 query over the search index. The query is bound as a MATCH
    /// parameter so no FTS syntax is interpreted from the caller; a query
    /// that matches nothing returns an empty page.
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<StoredSearchHit>, HistoryError> {
        self.search_filtered(query, None, None, limit)
    }

    /// Literal-text FTS search with the same closed filters as list.
    pub fn search_filtered(
        &self,
        query: &str,
        status: Option<AnalysisStatus>,
        check: Option<CheckKind>,
        limit: u32,
    ) -> Result<Vec<StoredSearchHit>, HistoryError> {
        let query = literal_fts_query(query)?;
        self.with_read_snapshot(|connection| {
            certify_list_search_store(connection)?;
            let Some(query) = query else {
                return Ok(Vec::new());
            };
            let mut statement = connection
                .prepare(
                    "SELECT a.id, a.status,
                            COALESCE((
                              SELECT group_concat(k.check_kind, ',') FROM (
                                SELECT check_kind FROM analysis_checks
                                WHERE analysis_id = a.id
                                ORDER BY check_index
                              ) k
                            ), ''),
                            a.check_count, a.save_state, a.input_type, a.display_name, a.created_at
                     FROM analysis_search s JOIN analyses a ON a.id = s.analysis_id
                     WHERE analysis_search MATCH ?1
                       AND (?2 IS NULL OR a.status = ?2)
                       AND (?3 IS NULL OR EXISTS (
                              SELECT 1 FROM analysis_checks t
                              WHERE t.analysis_id = a.id AND t.check_kind = ?3
                            ))
                     ORDER BY a.created_at DESC, a.id LIMIT ?4",
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "search analyses",
                    )
                })?;
            statement
                .query_map(
                    params![
                        query,
                        status.map(super::wire::wire_status),
                        check.map(wire_check_kind),
                        limit
                    ],
                    row_to_summary,
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "search analyses",
                    )
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| {
                    HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "search analyses")
                })
        })
    }
}

/// Converts arbitrary user text into an FTS5 expression containing quoted
/// literal tokens only. Punctuation and operator spellings never retain raw
/// query syntax.
fn literal_fts_query(query: &str) -> Result<Option<String>, HistoryError> {
    let tokenizer = Connection::open_in_memory().map_err(|_| {
        HistoryError::from_sqlite(
            HistoryErrorCode::HistoryUnavailable,
            "tokenize search query",
        )
    })?;
    tokenizer
        .execute_batch(
            "CREATE VIRTUAL TABLE literal_input USING fts5(value, tokenize = 'unicode61');
             CREATE VIRTUAL TABLE literal_terms USING fts5vocab(literal_input, 'row');",
        )
        .map_err(|_| {
            HistoryError::from_sqlite(
                HistoryErrorCode::HistoryUnavailable,
                "tokenize search query",
            )
        })?;
    tokenizer
        .execute("INSERT INTO literal_input(value) VALUES (?1)", [query])
        .map_err(|_| {
            HistoryError::from_sqlite(
                HistoryErrorCode::HistoryUnavailable,
                "tokenize search query",
            )
        })?;
    let mut statement = tokenizer
        .prepare("SELECT term FROM literal_terms ORDER BY term")
        .map_err(|_| {
            HistoryError::from_sqlite(
                HistoryErrorCode::HistoryUnavailable,
                "tokenize search query",
            )
        })?;
    let tokens = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| {
            HistoryError::from_sqlite(
                HistoryErrorCode::HistoryUnavailable,
                "tokenize search query",
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            HistoryError::from_sqlite(
                HistoryErrorCode::HistoryUnavailable,
                "tokenize search query",
            )
        })?;
    Ok((!tokens.is_empty()).then(|| {
        tokens
            .into_iter()
            .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ")
    }))
}

/// Certifies the summary contract in bounded set queries before list/search
/// returns any page. Full canonical reconstruction belongs to show/export and
/// destructive mutations, not the performance-sensitive summary surface.
fn certify_list_search_store(connection: &Connection) -> Result<(), HistoryError> {
    certify_search_index(connection)?;
    certify_analysis_memberships(connection).map_err(|_| {
        HistoryError::new(
            HistoryErrorCode::HistoryCorrupt,
            "the recorded analyses contain invalid bulk membership",
        )
    })?;
    certify_summary_checks(connection)
}

/// Proves the one-to-one typed relationship between analyses and their FTS5
/// rows before list/search returns a page. Exact content projection is owned
/// by full reads and destructive certification.
pub(super) fn certify_search_index(connection: &Connection) -> Result<(), HistoryError> {
    let corrupt: bool = connection
        .query_row(
            "SELECT EXISTS (
               SELECT 1 WHERE
                 (SELECT COUNT(*) FROM analyses) <>
                 (SELECT COUNT(*) FROM analysis_search)
                 OR (SELECT COUNT(*) FROM analyses) <>
                    (SELECT COUNT(DISTINCT analysis_id) FROM analysis_search)
               UNION ALL
               SELECT 1 FROM analysis_search s
               WHERE NOT EXISTS (
                         SELECT 1 FROM analyses a WHERE a.id = s.analysis_id
                     )
                  OR typeof(s.analysis_id) <> 'text'
                  OR typeof(s.input_text) NOT IN ('null', 'text')
                  OR typeof(s.filename) NOT IN ('null', 'text')
                  OR typeof(s.headline) NOT IN ('null', 'text')
                  OR typeof(s.source_urls) NOT IN ('null', 'text')
             )",
            [],
            |row| row.get(0),
        )
        .map_err(|_| {
            HistoryError::from_sqlite(
                HistoryErrorCode::HistoryUnavailable,
                "validate search index",
            )
        })?;
    if corrupt {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryCorrupt,
            "the recorded analyses and search index are not synchronized",
        ));
    }
    Ok(())
}

/// Validates every nullable bulk-membership pair and its parent range.
fn certify_analysis_memberships(connection: &Connection) -> Result<(), HistoryError> {
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
/// read snapshot. Summary reads do not need full input, provenance, or task
/// reconstruction, and the contract forbids an N+1 validation pattern.
fn certify_summary_checks(connection: &Connection) -> Result<(), HistoryError> {
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

/// Runs FTS5's dedicated integrity command inside a mutation transaction.
///
/// The read-oriented SQLite integrity pragma is not safe to run before a
/// later FTS write in the same transaction when another WAL reader holds an
/// older snapshot. FTS5's control command is the writer-side equivalent: it
/// checks the same index/content relationship without inserting a document.
pub(super) fn certify_search_index_for_write(connection: &Connection) -> Result<(), HistoryError> {
    connection
        .execute(
            "INSERT INTO analysis_search(analysis_search) VALUES ('integrity-check')",
            [],
        )
        .map_err(search_integrity_error)?;
    Ok(())
}

fn search_integrity_error(error: rusqlite::Error) -> HistoryError {
    let corrupt = match error {
        rusqlite::Error::SqliteFailure(inner, _) => matches!(
            inner.code,
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
        ),
        rusqlite::Error::SqlInputError { error: inner, .. } => matches!(
            inner.code,
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
        ),
        _ => false,
    };
    HistoryError::from_sqlite(
        if corrupt {
            HistoryErrorCode::HistoryCorrupt
        } else {
            HistoryErrorCode::HistoryUnavailable
        },
        "validate search index",
    )
}
