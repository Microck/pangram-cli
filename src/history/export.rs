//! Consistent, side-effect-free history export reads.

use super::{HistoryError, HistoryErrorCode, HistoryStore};

impl HistoryStore {
    /// Reads every complete analysis newest-first from one SQLite snapshot.
    /// The deferred transaction holds its snapshot through whole-store
    /// certification and export projection.
    pub fn export_analyses(
        &self,
        redact_content: bool,
    ) -> Result<Vec<serde_json::Value>, HistoryError> {
        self.with_read_snapshot(|transaction| {
            let mut analyses = super::read_validation::certify_analysis_batch(transaction, true)?;
            analyses.sort_by(|left, right| {
                right
                    .created_at
                    .cmp(&left.created_at)
                    .then_with(|| left.id.cmp(&right.id))
            });

            let mut values = Vec::with_capacity(analyses.len());
            for analysis in analyses {
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
