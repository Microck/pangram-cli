//! Validated history summaries and literal FTS5 search.

use rusqlite::{Connection, params};

use crate::domain::{AnalysisStatus, CheckKind};

use super::records::StoredSearchHit;
use super::store::HistoryStore;
use super::wire::{row_to_summary, wire_check_kind};
use super::{HistoryError, HistoryErrorCode};

impl HistoryStore {
    /// Most recent analyses first, for `history list`.
    pub fn list(&self, limit: u32, offset: u32) -> Result<Vec<StoredSearchHit>, HistoryError> {
        self.list_filtered(None, None, limit, offset)
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
            certify_search_index(connection)?;
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
                        status.map(super::wire::wire_status),
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
            certify_search_index(connection)?;
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

/// Proves the one-to-one typed relationship between `analyses` and its FTS5
/// projection before list/search returns any page. This runs in the caller's
/// read snapshot and performs no repair.
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
    super::read_validation::certify_analysis_memberships(connection).map_err(|_| {
        HistoryError::new(
            HistoryErrorCode::HistoryCorrupt,
            "the recorded analyses contain invalid bulk membership",
        )
    })?;
    super::read_validation::certify_summary_checks(connection)
}
