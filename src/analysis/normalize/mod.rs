//! Upstream document normalization: Pangram 4 JSON -> canonical domain.
//!
//! One pass validation. Unknown required values (stage, classification,
//! confidence), wrong or missing versions, missing humanizer evidence, and
//! out-of-range scores all map to `upstream_contract_changed` with a
//! sanitized `(field, token, shape)` detail set. Optional additive upstream
//! fields are ignored until adopted, and response text never crosses into
//! error values.
//!
//! `CanonicalError` is 224 bytes wide because it is the adapter-facing
//! canonical object: normalization is a cold path (at most once per poll),
//! so boxing every intermediate would add churn for no measurable gain.
#![allow(clippy::result_large_err)]

use crate::domain::{
    AiClassification, AiDetectionResult, Confidence, Fraction, NonEmptyString, Percentage,
    PlagiarismMatch, PlagiarismResult, Segment,
};
use crate::output::{CanonicalError, ErrorCode};

pub(in crate::analysis) mod bulk;

/// The accepted terminal success version. Pangram 4 returns exactly `4.0`.
const REQUIRED_VERSION: &str = "4.0";

/// The provider's documented in-progress stage tokens.
const IN_PROGRESS_STAGES: &[&str] = &["STAGE_PREPROCESSING", "STAGE_INFERENCE"];
const TERMINAL_SUCCESS_STAGE: &str = "STAGE_SUCCESS";
const TERMINAL_FAILURE_STAGE: &str = "STAGE_FAILED";

/// The retained length ceiling for an upstream failure message. Provider
/// text is untrusted: it may echo submitted content or carry terminal
/// control sequences, so it is reduced to a short ASCII-printable prefix
/// before it can cross into canonical error details.
pub(crate) const MAX_UPSTREAM_MESSAGE_CHARS: usize = 200;

/// Reduces an upstream failure message to a safe, bounded, printable form.
///
/// Upstream text is untrusted: it may echo the submitted content (including
/// material that looks like an API key) and may embed terminal control
/// sequences. Before it can appear in a canonical error `details` map we
///
/// 1. replace tab/line-feed/carriage-return with spaces (keeping single-line
///    structure but dropping hard control effects),
/// 2. drop every other control character (including the CSI/OSC/Oscescape
///    introducers, C0/C1 ranges, and DEL) and every non-ASCII scalar, and
/// 3. truncate the result to [`MAX_UPSTREAM_MESSAGE_CHARS`] characters.
///
/// The result is never the raw provider text. Empty reductions fall back to
/// a fixed placeholder so the field stays informative without content.
pub(crate) fn sanitize_upstream_message(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .filter_map(|ch| match ch {
            '\t' | '\n' | '\r' => Some(' '),
            ch if ch.is_ascii() && !ch.is_ascii_control() => Some(ch),
            _ => None,
        })
        .take(MAX_UPSTREAM_MESSAGE_CHARS)
        .collect();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "Pangram reported a task failure without readable detail".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// A sanitized contract violation. Details carry only the field path, the
/// offending scalar token or structural shape; never text content. The token
/// can be provider-controlled (`stage`, `version`, `confidence`, `label`), so
/// it is reduced through the same bounded sanitizer as any other upstream
/// string before it crosses into the canonical error, covering every caller
/// in one place.
pub(super) fn contract_changed(field: &'static str, token: impl Into<String>) -> CanonicalError {
    let mut details = std::collections::BTreeMap::new();
    details.insert("field".to_owned(), serde_json::Value::from(field));
    details.insert(
        "token".to_owned(),
        serde_json::Value::from(sanitize_upstream_message(&token.into())),
    );
    CanonicalError::new(
        ErrorCode::UpstreamContractChanged,
        "Pangram returned a document outside the pinned Pangram 4 contract.",
    )
    .and_then(|error| error.with_details(details))
    .expect("the upstream-contract template is statically valid")
}

pub(super) fn missing(field: &'static str) -> CanonicalError {
    contract_changed(field, "missing")
}

pub(super) fn out_of_range(field: &'static str, token: impl Into<String>) -> CanonicalError {
    contract_changed(field, format!("out of range: {}", token.into()))
}

fn parse_fraction(
    field: &'static str,
    value: Option<&serde_json::Value>,
) -> Result<Fraction, CanonicalError> {
    let Some(value) = value else {
        return Err(missing(field));
    };
    let raw = value
        .as_f64()
        .ok_or_else(|| contract_changed(field, shape_of(value)))?;
    Fraction::new(raw).map_err(|_| out_of_range(field, raw.to_string()))
}

fn parse_percentage(
    field: &'static str,
    value: Option<&serde_json::Value>,
) -> Result<Percentage, CanonicalError> {
    let Some(value) = value else {
        return Err(missing(field));
    };
    let raw = value
        .as_f64()
        .ok_or_else(|| contract_changed(field, shape_of(value)))?;
    Percentage::new(raw).map_err(|_| out_of_range(field, raw.to_string()))
}

fn parse_bool(
    field: &'static str,
    value: Option<&serde_json::Value>,
) -> Result<bool, CanonicalError> {
    let Some(value) = value else {
        return Err(missing(field));
    };
    value
        .as_bool()
        .ok_or_else(|| contract_changed(field, shape_of(value)))
}

pub(super) fn parse_u64(
    field: &'static str,
    value: Option<&serde_json::Value>,
) -> Result<u64, CanonicalError> {
    let Some(value) = value else {
        return Err(missing(field));
    };
    value
        .as_u64()
        .ok_or_else(|| contract_changed(field, shape_of(value)))
}

pub(super) fn parse_string(
    field: &'static str,
    value: Option<&serde_json::Value>,
) -> Result<String, CanonicalError> {
    let Some(value) = value else {
        return Err(missing(field));
    };
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| contract_changed(field, shape_of(value)))
}

pub(super) fn shape_of(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_owned(),
        serde_json::Value::Bool(_) => "a boolean".to_owned(),
        serde_json::Value::Number(_) => "a number".to_owned(),
        serde_json::Value::String(_) => "a string".to_owned(),
        serde_json::Value::Array(array) => format!("an array of {} items", array.len()),
        serde_json::Value::Object(object) => format!("an object of {} keys", object.len()),
    }
}

/// The normalized classification of one observed task document.
#[derive(Debug, Clone)]
pub enum TaskState {
    /// The provider is still working; `last_stage` preserves its token.
    /// The poll layer reports identical stage provenance through
    /// `TaskPoll::InProgress`, so this field is read only by future
    /// synchronous-terminal flows.
    InProgress {
        #[allow(dead_code)]
        last_stage: String,
    },
    /// A terminal success document normalized against the Pangram 4 shape.
    Success(Box<NormalizedTask>),
    /// A terminal provider failure with a sanitized (never raw) message.
    Failed { message: String, stage: String },
}

/// The one owner of the provider failure-message reduction shared by every
/// stage classifier: walk the documented detail fields in priority order,
/// take the first string, and reduce it through the sanitizer so the exact
/// failure contract holds in exactly one place. Returns `None` when no
/// detail field carries a string, letting the caller supply its fallback.
pub(crate) fn failure_message(body: &serde_json::Value) -> Option<String> {
    body.get("error_message")
        .or_else(|| body.get("headline"))
        .or_else(|| body.get("detail"))
        .and_then(serde_json::Value::as_str)
        .map(sanitize_upstream_message)
}

/// A fully normalized Pangram 4 success document, before assembly into the
/// canonical check state. Carries provenance and result-side content.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedTask {
    pub last_stage: String,
    pub version: String,
    pub result: AiDetectionResult,
    /// The provider's normalized text when present; segments' offsets refer
    /// to it. Not echoed into any error.
    pub normalized_text: Option<String>,
}

/// One synchronous file result after validating the live rich response shape.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedFile {
    pub(crate) filename: String,
    pub(crate) version: String,
    pub(crate) extracted_text: String,
    pub(crate) result: AiDetectionResult,
}

impl NormalizedFile {
    #[must_use]
    pub fn filename(&self) -> &str {
        &self.filename
    }
}

/// Normalizes the verified file endpoint array and binds every item to the
/// request-order filename. Pangram returns no task identity for this
/// synchronous endpoint.
pub fn normalize_file_results(
    body: &serde_json::Value,
    expected_filenames: &[String],
) -> Result<Vec<NormalizedFile>, CanonicalError> {
    let items = body
        .as_array()
        .ok_or_else(|| contract_changed("file_response", shape_of(body)))?;
    if items.len() != expected_filenames.len() {
        return Err(contract_changed(
            "file_response",
            format!(
                "expected {} items, received {}",
                expected_filenames.len(),
                items.len()
            ),
        ));
    }

    items
        .iter()
        .zip(expected_filenames)
        .map(|(item, expected_filename)| normalize_file_item(item, expected_filename))
        .collect()
}

fn normalize_file_item(
    item: &serde_json::Value,
    expected_filename: &str,
) -> Result<NormalizedFile, CanonicalError> {
    let filename = parse_string("filename", item.get("filename"))?;
    if filename != expected_filename {
        return Err(contract_changed("filename", "response order mismatch"));
    }
    if item
        .get("dashboard_link")
        .or_else(|| item.get("public_dashboard_link"))
        .is_some_and(|value| !value.is_null())
    {
        return Err(contract_changed(
            "dashboard_link",
            "unexpected private link",
        ));
    }

    let version = parse_string("version", item.get("version"))?;
    NonEmptyString::new(version.clone()).map_err(|_| contract_changed("version", "empty"))?;
    let prediction_short = parse_string("prediction_short", item.get("prediction_short"))?;
    let classification = normalize_classification(&prediction_short)?;
    let windows_value = item.get("windows").ok_or_else(|| missing("windows"))?;
    let windows = windows_value
        .as_array()
        .ok_or_else(|| contract_changed("windows", shape_of(windows_value)))?;
    let segments = windows
        .iter()
        .map(normalize_file_window)
        .collect::<Result<Vec<_>, _>>()?;
    let extracted_text = parse_string("text", item.get("text"))?;

    Ok(NormalizedFile {
        filename,
        version,
        extracted_text,
        result: AiDetectionResult {
            classification,
            headline: parse_string("headline", item.get("headline"))?,
            prediction: parse_string("prediction", item.get("prediction"))?,
            fraction_ai: parse_fraction("fraction_ai", item.get("fraction_ai"))?,
            fraction_ai_assisted: parse_fraction(
                "fraction_ai_assisted",
                item.get("fraction_ai_assisted"),
            )?,
            fraction_human: parse_fraction("fraction_human", item.get("fraction_human"))?,
            num_ai_segments: parse_u64("num_ai_segments", item.get("num_ai_segments"))?,
            num_ai_assisted_segments: parse_u64(
                "num_ai_assisted_segments",
                item.get("num_ai_assisted_segments"),
            )?,
            num_human_segments: parse_u64("num_human_segments", item.get("num_human_segments"))?,
            segments,
            dashboard_link: None,
        },
    })
}

fn normalize_file_window(window: &serde_json::Value) -> Result<Segment, CanonicalError> {
    let label = NonEmptyString::new(parse_string("label", window.get("label"))?)
        .map_err(|_| contract_changed("label", "empty"))?;
    let confidence = normalize_confidence(&parse_string("confidence", window.get("confidence"))?)?;
    Ok(Segment {
        text: parse_string("text", window.get("text"))?,
        label,
        ai_assistance_score: parse_fraction(
            "ai_assistance_score",
            window.get("ai_assistance_score"),
        )?,
        confidence,
        start_index: parse_u64("start_index", window.get("start_index"))?,
        end_index: parse_u64("end_index", window.get("end_index"))?,
        word_count: parse_u64("word_count", window.get("word_count"))?,
        token_length: parse_u64("token_length", window.get("token_length"))?,
        humanizer_score: None,
        is_humanized: None,
    })
}

/// Normalizes the synchronous plagiarism document. Additive provider metadata
/// stays upstream; canonical schema major 1 owns only the documented fields.
pub fn normalize_plagiarism(body: &serde_json::Value) -> Result<PlagiarismResult, CanonicalError> {
    let content_value = body
        .get("plagiarized_content")
        .ok_or_else(|| missing("plagiarized_content"))?;
    let content = content_value
        .as_array()
        .ok_or_else(|| contract_changed("plagiarized_content", shape_of(content_value)))?;
    let matches = content
        .iter()
        .map(|item| {
            Ok(PlagiarismMatch {
                source_url: parse_string("source_url", item.get("source_url"))?,
                matched_text: parse_string("matched_text", item.get("matched_text"))?,
                similarity_score: parse_fraction("similarity_score", item.get("similarity_score"))?,
            })
        })
        .collect::<Result<Vec<_>, CanonicalError>>()?;

    Ok(PlagiarismResult {
        plagiarism_detected: parse_bool("plagiarism_detected", body.get("plagiarism_detected"))?,
        total_sentences: parse_u64("total_sentences", body.get("total_sentences"))?,
        plagiarized_sentence_count: parse_u64(
            "plagiarized_sentences",
            body.get("plagiarized_sentences"),
        )?,
        percent_plagiarized: parse_percentage(
            "percent_plagiarized",
            body.get("percent_plagiarized"),
        )?,
        matches,
    })
}

fn normalize_classification(raw: &str) -> Result<AiClassification, CanonicalError> {
    match raw {
        "AI" => Ok(AiClassification::Ai),
        "Human" => Ok(AiClassification::Human),
        "Mixed" => Ok(AiClassification::Mixed),
        other => Err(contract_changed("prediction_short", other)),
    }
}

fn normalize_confidence(raw: &str) -> Result<Confidence, CanonicalError> {
    match raw {
        "High" => Ok(Confidence::High),
        "Medium" => Ok(Confidence::Medium),
        "Low" => Ok(Confidence::Low),
        other => Err(contract_changed("confidence", other)),
    }
}

/// Classifies one raw task document (from a poll or a synchronous terminal
/// submit). This is the first normalization seam; it peeks at `stage` only.
pub fn normalize_task_state(body: &serde_json::Value) -> Result<TaskState, CanonicalError> {
    let stage = match body.get("stage") {
        Some(value) => value
            .as_str()
            .ok_or_else(|| contract_changed("stage", shape_of(value)))?,
        None => return Err(missing("stage")),
    };

    if IN_PROGRESS_STAGES.contains(&stage) {
        return Ok(TaskState::InProgress {
            last_stage: stage.to_owned(),
        });
    }
    match stage {
        TERMINAL_SUCCESS_STAGE => Ok(TaskState::Success(Box::new(normalize_success_task(body)?))),
        TERMINAL_FAILURE_STAGE => Ok(TaskState::Failed {
            message: failure_message(body)
                .unwrap_or_else(|| "Pangram reported a task failure without detail".to_owned()),
            stage: stage.to_owned(),
        }),
        other => Err(contract_changed("stage", other)),
    }
}

/// Full Pangram 4 success validation. Any deviation is a contract change.
pub fn normalize_success_task(body: &serde_json::Value) -> Result<NormalizedTask, CanonicalError> {
    let last_stage = parse_string("stage", body.get("stage"))?;
    let version = parse_string("version", body.get("version"))?;
    if version != REQUIRED_VERSION {
        return Err(contract_changed("version", version));
    }

    let prediction_short = parse_string("prediction_short", body.get("prediction_short"))?;
    let classification = normalize_classification(&prediction_short)?;

    let windows_value = body.get("windows").ok_or_else(|| missing("windows"))?;
    let windows = windows_value
        .as_array()
        .ok_or_else(|| contract_changed("windows", shape_of(windows_value)))?;
    let mut segments = Vec::with_capacity(windows.len());
    for (index, window) in windows.iter().enumerate() {
        segments.push(normalize_window(window, index)?);
    }

    let result = AiDetectionResult {
        classification,
        headline: parse_string("headline", body.get("headline"))?,
        prediction: parse_string("prediction", body.get("prediction"))?,
        fraction_ai: parse_fraction("fraction_ai", body.get("fraction_ai"))?,
        fraction_ai_assisted: parse_fraction(
            "fraction_ai_assisted",
            body.get("fraction_ai_assisted"),
        )?,
        fraction_human: parse_fraction("fraction_human", body.get("fraction_human"))?,
        num_ai_segments: parse_u64("num_ai_segments", body.get("num_ai_segments"))?,
        num_ai_assisted_segments: parse_u64(
            "num_ai_assisted_segments",
            body.get("num_ai_assisted_segments"),
        )?,
        num_human_segments: parse_u64("num_human_segments", body.get("num_human_segments"))?,
        segments,
        dashboard_link: match body.get("dashboard_link") {
            Some(value) if !value.is_null() => Some(
                value
                    .as_str()
                    .ok_or_else(|| contract_changed("dashboard_link", shape_of(value)))?
                    .to_owned(),
            ),
            _ => None,
        },
    };

    let normalized_text = match body.get("text") {
        Some(value) if !value.is_null() => Some(
            value
                .as_str()
                .ok_or_else(|| contract_changed("text", shape_of(value)))?
                .to_owned(),
        ),
        _ => None,
    };

    Ok(NormalizedTask {
        last_stage,
        version,
        result,
        normalized_text,
    })
}

/// One window. Pangram 4 requires humanizer evidence on every window; a
/// missing or non-(0..=1) value is a contract violation, never a default.
fn normalize_window(window: &serde_json::Value, index: usize) -> Result<Segment, CanonicalError> {
    let field = |name: &'static str| -> &'static str {
        // Borrowed constant paths keep details stable; index is provenance
        // carried by the caller in `details.token` only when needed.
        let _ = index;
        name
    };

    let label = parse_string("label", window.get(field("label")))?;
    let confidence_raw = parse_string("confidence", window.get(field("confidence")))?;
    let confidence = normalize_confidence(&confidence_raw)?;

    let humanizer_score = parse_fraction("humanizer_score", window.get("humanizer_score"))?;
    let is_humanized = parse_bool("is_humanized", window.get("is_humanized"))?;

    let label_value = crate::domain::NonEmptyString::new(label)
        .map_err(|_| contract_changed("label", "empty"))?;

    Ok(Segment {
        text: parse_string("text", window.get(field("text")))?,
        label: label_value,
        ai_assistance_score: parse_fraction(
            "ai_assistance_score",
            window.get("ai_assistance_score"),
        )?,
        confidence,
        start_index: parse_u64("start_index", window.get("start_index"))?,
        end_index: parse_u64("end_index", window.get("end_index"))?,
        word_count: parse_u64("word_count", window.get("word_count"))?,
        token_length: parse_u64("token_length", window.get("token_length"))?,
        humanizer_score: Some(humanizer_score),
        is_humanized: Some(is_humanized),
    })
}

#[cfg(test)]
mod phase7_tests {
    use serde_json::json;

    use super::{normalize_file_results, normalize_plagiarism};

    fn file_result() -> serde_json::Value {
        json!({
            "filename": "minimal.rtf",
            "text": "Synthetic file text.",
            "version": "3.3",
            "headline": "Human-written",
            "prediction": "The document appears human-written.",
            "prediction_short": "Human",
            "fraction_ai": 0.0,
            "fraction_ai_assisted": 0.0,
            "fraction_human": 1.0,
            "num_ai_segments": 0,
            "num_ai_assisted_segments": 0,
            "num_human_segments": 1,
            "windows": [{
                "text": "Synthetic file text.",
                "label": "Human Written",
                "ai_assistance_score": 0.0,
                "confidence": "High",
                "start_index": 0,
                "end_index": 20,
                "word_count": 3,
                "token_length": 4
            }]
        })
    }

    #[test]
    fn verified_file_shape_normalizes_without_inventing_humanizer_evidence() {
        let response = json!([file_result()]);
        let normalized = normalize_file_results(&response, &["minimal.rtf".to_owned()]).unwrap();

        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].filename, "minimal.rtf");
        assert_eq!(normalized[0].version, "3.3");
        assert_eq!(normalized[0].extracted_text, "Synthetic file text.");
        assert_eq!(normalized[0].result.segments[0].humanizer_score, None);
        assert_eq!(normalized[0].result.segments[0].is_humanized, None);
    }

    #[test]
    fn dashboard_link_only_file_shape_is_rejected() {
        let response = json!([{"public_dashboard_link": "https://example.invalid"}]);
        let error = normalize_file_results(&response, &["minimal.rtf".to_owned()]).unwrap_err();

        assert_eq!(
            error.code(),
            crate::output::ErrorCode::UpstreamContractChanged
        );
    }

    #[test]
    fn numeric_plagiarized_sentence_count_normalizes() {
        let response = json!({
            "text": "Synthetic plagiarism text.",
            "plagiarism_detected": false,
            "plagiarized_content": [],
            "total_sentences": 1,
            "plagiarized_sentences": 0,
            "percent_plagiarized": 0.0,
            "metadata": {"ignored_additive_field": true}
        });
        let result = normalize_plagiarism(&response).unwrap();

        assert_eq!(result.total_sentences, 1);
        assert_eq!(result.plagiarized_sentence_count, 0);
        assert!(result.matches.is_empty());
    }

    #[test]
    fn plagiarism_sentence_list_is_contract_drift() {
        let response = json!({
            "text": "Synthetic plagiarism text.",
            "plagiarism_detected": true,
            "plagiarized_content": [],
            "total_sentences": 1,
            "plagiarized_sentences": ["Synthetic plagiarism text."],
            "percent_plagiarized": 100.0
        });
        let error = normalize_plagiarism(&response).unwrap_err();

        assert_eq!(
            error.code(),
            crate::output::ErrorCode::UpstreamContractChanged
        );
    }
}
