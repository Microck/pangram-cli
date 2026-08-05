//! Consistent, side-effect-free history export reads.

use crate::domain::Analysis;
use crate::output::CanonicalError;

use super::reads::canonical_analysis_on;
use super::{HistoryError, HistoryErrorCode, HistoryStore};

impl HistoryStore {
    /// Reads every complete analysis newest-first from one SQLite snapshot.
    /// The deferred transaction acquires its snapshot on the ID query and
    /// holds it through every canonical reconstruction.
    pub fn export_analyses(
        &self,
        redact_content: bool,
    ) -> Result<Vec<serde_json::Value>, HistoryError> {
        self.with_read_snapshot(|transaction| {
            super::search::certify_search_index(transaction)?;
            let mut statement = transaction
                .prepare("SELECT id FROM analyses ORDER BY created_at DESC, id")
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "export history",
                    )
                })?;
            let ids = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "export history",
                    )
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| {
                    HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "export history")
                })?;
            drop(statement);

            let mut values = Vec::with_capacity(ids.len());
            for id in ids {
                let id = id.parse().map_err(|_| {
                    HistoryError::new(
                        HistoryErrorCode::HistoryCorrupt,
                        "export history: a stored analysis identity is invalid",
                    )
                })?;
                let record = super::reads::stored_analysis_on(transaction, &id)?;
                let analysis: Analysis<CanonicalError> =
                    canonical_analysis_on(transaction, &record, true)?;
                let mut value = serde_json::to_value(analysis).map_err(|_| {
                    HistoryError::new(
                        HistoryErrorCode::HistoryCorrupt,
                        "export history: a canonical analysis could not be encoded",
                    )
                })?;
                if redact_content {
                    redact(&mut value);
                }
                values.push(value);
            }
            Ok(values)
        })
    }
}

fn redact(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(input) = object
        .get_mut("input")
        .and_then(serde_json::Value::as_object_mut)
    {
        input.remove("text");
        input.remove("path");
        input.remove("extracted_text");
    }
    if let Some(checks) = object
        .get_mut("checks")
        .and_then(serde_json::Value::as_array_mut)
    {
        for check in checks {
            let Some(check) = check.as_object_mut() else {
                continue;
            };
            let kind = check
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let Some(result) = check
                .get_mut("result")
                .and_then(serde_json::Value::as_object_mut)
            else {
                continue;
            };
            result.remove("dashboard_link");
            if kind == "ai_detection" {
                if let Some(segments) = result
                    .get_mut("segments")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    for segment in segments {
                        if let Some(segment) = segment.as_object_mut() {
                            segment.insert(
                                "text".to_owned(),
                                serde_json::Value::String(String::new()),
                            );
                        }
                    }
                }
            } else if kind == "plagiarism" {
                if let Some(matches) = result
                    .get_mut("matches")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    matches.retain_mut(redact_plagiarism_match);
                }
            }
        }
    }
}

fn redact_plagiarism_match(value: &mut serde_json::Value) -> bool {
    let Some(matched) = value.as_object_mut() else {
        return false;
    };
    let Some(source) = matched
        .get("source_url")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    let Ok(parsed) = url::Url::parse(source) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(hostname) = parsed.host_str() else {
        return false;
    };
    let hostname = hostname.to_owned();
    matched.insert(
        "matched_text".to_owned(),
        serde_json::Value::String(String::new()),
    );
    matched.insert("source_url".to_owned(), serde_json::Value::String(hostname));
    true
}
