//! Documented Pangram 4 bulk wire types and the local submission plan.
//!
//! These types mirror the official Bulk API source `eb214f4` (re-verified the
//! latest commit on `api-reference/bulk-api.mdx` on 2026-08-01) and contracts
//! section 9.1. They are the typed fixture spines the loopback server plays
//! and the shapes the future production client consumes. They are
//! deserialization-only fixtures: unknown upstream fields are ignored
//! (architecture section 6.2 reserves strict rejection for required-state
//! mismatches, which the analysis normalizer owns), and required state values
//! stay as raw strings here so the protocol tests can drive the normalizer's
//! `upstream_contract_changed` path.
//!
//! Cross-field rules JSON cannot express (exactly one of `items`/`text`,
//! unique caller IDs, the ceiling math) are locked in Rust constructors, not
//! in schemas.

use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Value;

use super::{DomainError, NonEmptyString};

/// The exact production bulk selector and the only model value the client
/// sends. Pangram accepts one job-wide `model`; per-item selectors are not
/// supported.
pub const BULK_MODEL: &str = "pangram-4";

/// Pangram's documented maximum billable units per bulk request.
pub const BULK_BILLABLE_UNIT_LIMIT: u64 = 1000;

/// The documented maximum `limit` on item and result pages.
pub const BULK_PAGE_LIMIT_MAX: u64 = 1000;

/// The fixed production bulk base URL. All bulk routes join beneath it.
pub const PRODUCTION_BULK_URL: &str = "https://text.external-api.pangram.com/bulk";

/// Sums the Pangram 4 per-item units with a checked accumulation.
///
/// Each item contributes [`super::text_billable_units`] (one unit per started
/// 100-word block, minimum one). The sum uses `u64::checked_add` so a
/// pathological item set saturates into a [`DomainError::OutOfRange`] instead
/// of wrapping. Item word counts come from `split_whitespace` over real
/// UTF-8 input, so the sum cannot approach `u64::MAX` in practice; the check
/// exists so the invariant is constructor-owned rather than assumed.
pub fn bulk_estimated_billable_units(
    item_word_counts: impl IntoIterator<Item = u64>,
) -> Result<u64, DomainError> {
    let mut total = 0_u64;
    for words in item_word_counts {
        total = total
            .checked_add(super::text_billable_units(words))
            .ok_or(DomainError::OutOfRange("bulk billable estimate"))?;
    }
    Ok(total)
}

/// One ordered caller-supplied bulk input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BulkSubmissionItem {
    caller_id: Option<NonEmptyString>,
    text: String,
    word_count: u64,
    billable_units: u64,
}

impl BulkSubmissionItem {
    /// Builds one validated item. `word_count` is the adapter-computed
    /// `split_whitespace` count also shown in input summaries; the per-item
    /// billable units derive from it once here and are never recomputed.
    pub fn new(
        caller_id: Option<NonEmptyString>,
        text: String,
        word_count: u64,
    ) -> Result<Self, DomainError> {
        if text.is_empty() {
            return Err(DomainError::EmptyValue("bulk item text"));
        }
        Ok(Self {
            caller_id,
            text,
            word_count,
            billable_units: super::text_billable_units(word_count),
        })
    }

    #[must_use]
    pub fn caller_id(&self) -> Option<&NonEmptyString> {
        self.caller_id.as_ref()
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn word_count(&self) -> u64 {
        self.word_count
    }

    #[must_use]
    pub const fn billable_units(&self) -> u64 {
        self.billable_units
    }
}

/// The validated local bulk submission: ordered items plus the mandatory
/// ceiling. Construction owns every preflight invariant the JSONL validator
/// and the MCP/CLI adapter rely on before credential or network work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BulkSubmissionPlan {
    items: Vec<BulkSubmissionItem>,
    caller_ids_present: bool,
    max_billable_units: u64,
    estimated_billable_units: u64,
}

impl BulkSubmissionPlan {
    /// Validates the ordered item set, derives the estimate, and enforces the
    /// smaller of the caller ceiling and Pangram's 1,000-unit request limit.
    pub fn new(
        items: Vec<BulkSubmissionItem>,
        max_billable_units: u64,
    ) -> Result<Self, DomainError> {
        if items.is_empty() {
            return Err(DomainError::EmptyValue("bulk items"));
        }
        if max_billable_units == 0 {
            return Err(DomainError::OutOfRange("max billable units"));
        }
        // Caller IDs must be unique when provided. A linear scan is fine:
        // the item count is bounded by the request-body limit and the
        // 1,000-unit ceiling, never by an unbounded stream.
        let mut seen: Vec<&NonEmptyString> = Vec::new();
        let mut caller_ids_present = false;
        for item in &items {
            if let Some(id) = item.caller_id() {
                caller_ids_present = true;
                if seen.contains(&id) {
                    return Err(DomainError::DuplicateBulkCallerId);
                }
                seen.push(id);
            }
        }
        let estimated = bulk_estimated_billable_units(items.iter().map(|item| item.word_count()))?;
        let ceiling = max_billable_units.min(BULK_BILLABLE_UNIT_LIMIT);
        if estimated > ceiling {
            return Err(DomainError::BulkLimitExceeded);
        }
        Ok(Self {
            items,
            caller_ids_present,
            max_billable_units,
            estimated_billable_units: estimated,
        })
    }

    #[must_use]
    pub fn items(&self) -> &[BulkSubmissionItem] {
        &self.items
    }

    /// Whether any item carries a caller ID, which selects the wire shape:
    /// caller IDs require the `items` object list; otherwise the plain
    /// `text` list is used. The two shapes are never mixed on one request.
    #[must_use]
    pub const fn caller_ids_present(&self) -> bool {
        self.caller_ids_present
    }

    #[must_use]
    pub const fn max_billable_units(&self) -> u64 {
        self.max_billable_units
    }

    #[must_use]
    pub const fn estimated_billable_units(&self) -> u64 {
        self.estimated_billable_units
    }

    /// The exact documented submit body. One job-wide `model`, exactly one of
    /// `items` or `text`, no per-item selector, and no public-link field.
    #[must_use]
    pub fn submit_body(&self) -> Value {
        let model = Value::String(BULK_MODEL.to_owned());
        if self.caller_ids_present {
            let items: Vec<Value> = self
                .items
                .iter()
                .map(|item| {
                    let mut entry = serde_json::Map::new();
                    if let Some(id) = item.caller_id() {
                        entry.insert("id".into(), Value::String(id.as_str().to_owned()));
                    }
                    entry.insert("text".into(), Value::String(item.text().to_owned()));
                    Value::Object(entry)
                })
                .collect();
            serde_json::json!({ "items": items, "model": model })
        } else {
            let text: Vec<Value> = self
                .items
                .iter()
                .map(|item| Value::String(item.text().to_owned()))
                .collect();
            serde_json::json!({ "text": text, "model": model })
        }
    }
}

/// One item object decoded from the local JSONL submission file.
/// Deserialization rejects unknown fields, null `id`, and empty `text`; the
/// adapter then converts it into a [`BulkSubmissionItem`].
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BulkJsonlItem {
    #[serde(default, deserialize_with = "super::deserialize_missing_only")]
    pub id: Option<NonEmptyString>,
    pub text: String,
}

/// The validated result of parsing one whole local JSONL submission file.
/// Parsing is whole-file (contracts section 14.3): every line must decode as
/// a [`BulkJsonlItem`] with no unknown fields before any item is accepted.
/// Caller-supplied word counts come from the adapter's canonical
/// `split_whitespace` count; the JSONL file carries text only.
pub fn parse_bulk_jsonl(
    input: &str,
    word_count: impl Fn(&str) -> u64,
) -> Result<Vec<BulkSubmissionItem>, BulkJsonlError> {
    let mut items = Vec::new();
    for (index, line) in input.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() {
            // A blank line is not a decodable item object; treat it as a
            // whole-file validation failure so an accidental trailing newline
            // inside the payload cannot silently drop an item.
            return Err(BulkJsonlError::InvalidLine {
                line: line_number,
                reason: "an empty line is not a bulk item object".to_owned(),
            });
        }
        let decoded: BulkJsonlItem =
            serde_json::from_str(line).map_err(|error| BulkJsonlError::InvalidLine {
                line: line_number,
                reason: error.to_string(),
            })?;
        items.push((line_number, decoded));
    }
    if items.is_empty() {
        return Err(BulkJsonlError::EmptyFile);
    }
    let mut built = Vec::with_capacity(items.len());
    for (line_number, decoded) in items {
        let text = decoded.text;
        let words = word_count(&text);
        let caller_id = decoded.id;
        let item = BulkSubmissionItem::new(caller_id, text, words).map_err(|error| {
            BulkJsonlError::InvalidLine {
                line: line_number,
                reason: error.to_string(),
            }
        })?;
        built.push(item);
    }
    Ok(built)
}

/// A whole-file JSONL validation failure. The message never echoes item text
/// or caller IDs; it carries only the line number and a structural reason.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BulkJsonlError {
    #[error("the bulk JSONL file contains no items")]
    EmptyFile,
    #[error("invalid bulk JSONL item on line {line}: {reason}")]
    InvalidLine { line: usize, reason: String },
}

/// One accepted item in the submit response.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct BulkAcceptedItem {
    pub index: u64,
    #[serde(default, deserialize_with = "super::deserialize_missing_only")]
    pub id: Option<NonEmptyString>,
    pub task_id: NonEmptyString,
}

/// One item that failed immediate validation in the submit response. Its
/// `task_id` is documented as null and is preserved as such.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct BulkFailedItem {
    pub index: u64,
    #[serde(default, deserialize_with = "super::deserialize_missing_only")]
    pub id: Option<NonEmptyString>,
    pub task_id: Option<NonEmptyString>,
    #[serde(default, deserialize_with = "super::deserialize_missing_only")]
    pub stage: Option<NonEmptyString>,
    #[serde(default, deserialize_with = "super::deserialize_missing_only")]
    pub error: Option<String>,
}

/// The documented `202 Accepted` submit response.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct BulkSubmitResponse {
    pub bulk_id: NonEmptyString,
    pub status: NonEmptyString,
    pub total_items: u64,
    #[serde(default)]
    pub accepted_items: Vec<BulkAcceptedItem>,
    #[serde(default)]
    pub failed_items: Vec<BulkFailedItem>,
}

/// The documented `GET /bulk/{bulk_id}` status response. Timestamps stay in
/// their raw epoch-second string form; the analysis normalizer owns the
/// conversion to RFC 3339 UTC and the retention/terminal semantics.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct BulkStatusResponse {
    pub bulk_id: NonEmptyString,
    pub status: NonEmptyString,
    pub total_items: u64,
    pub accepted: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub created_at: NonEmptyString,
    /// Documented as null while the job is not terminal.
    #[serde(default)]
    pub completed_at: Option<NonEmptyString>,
}

/// One item in the `GET /bulk/{bulk_id}/items` metadata page.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct BulkItemMetadata {
    pub index: u64,
    #[serde(default, deserialize_with = "super::deserialize_missing_only")]
    pub id: Option<NonEmptyString>,
    /// Documented as null for items that failed immediate validation.
    #[serde(default)]
    pub task_id: Option<NonEmptyString>,
    #[serde(default, deserialize_with = "super::deserialize_missing_only")]
    pub stage: Option<NonEmptyString>,
    #[serde(default)]
    pub error: Option<String>,
}

/// The documented items-metadata page.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct BulkItemsPage {
    pub bulk_id: NonEmptyString,
    pub offset: u64,
    pub limit: u64,
    pub total_items: u64,
    #[serde(default)]
    pub items: Vec<BulkItemMetadata>,
}

/// One in-progress or succeeded entry in the results page. `result` is null
/// for in-progress work and carries the raw Pangram 4 task document for a
/// completed success. The `result` payload is intentionally the uninterpreted
/// upstream document: the analysis normalizer owns its validation.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BulkResultItem {
    pub index: u64,
    #[serde(default, deserialize_with = "super::deserialize_missing_only")]
    pub id: Option<NonEmptyString>,
    #[serde(default)]
    pub task_id: Option<NonEmptyString>,
    #[serde(default, deserialize_with = "super::deserialize_missing_only")]
    pub stage: Option<NonEmptyString>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub result: Option<Value>,
}

/// The documented results page: an `items` list plus a separate
/// `failed_items` metadata list for the same page window.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BulkResultsPage {
    pub bulk_id: NonEmptyString,
    pub offset: u64,
    pub limit: u64,
    pub total_items: u64,
    #[serde(default)]
    pub items: Vec<BulkResultItem>,
    #[serde(default)]
    pub failed_items: Vec<BulkFailedItem>,
}

impl<'de> Deserialize<'de> for BulkSubmissionItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            caller_id: Option<NonEmptyString>,
            text: String,
            word_count: u64,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.caller_id, wire.text, wire.word_count).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::text_billable_units;
    use serde_json::json;

    fn item(id: Option<&str>, text: &str, words: u64) -> BulkSubmissionItem {
        BulkSubmissionItem::new(
            id.map(|value| NonEmptyString::new(value).unwrap()),
            text.to_owned(),
            words,
        )
        .unwrap()
    }

    #[test]
    fn per_item_units_follow_started_100_word_blocks_with_a_minimum_of_one() {
        for (words, expected) in [(0, 1), (1, 1), (99, 1), (100, 1), (101, 2), (1000, 10)] {
            assert_eq!(text_billable_units(words), expected, "words={words}");
        }
    }

    #[test]
    fn estimate_sums_per_item_units() {
        let words = [0_u64, 1, 100, 101, 1000];
        assert_eq!(
            bulk_estimated_billable_units(words).unwrap(),
            1 + 1 + 1 + 2 + 10
        );
    }

    #[test]
    fn estimate_saturates_at_the_reachable_top_without_overflowing() {
        // Each per-item unit is `text_billable_units(words) <= ceil(u64::MAX /
        // 100)`, so a sum of u64-typed item word counts can reach ~1/100 of
        // the u64 space per item and cannot wrap through the public iterator
        // type. The checked accumulation still owns the invariant; this pins
        // the largest reachable single- and two-item values exactly and proves
        // they neither wrap nor error.
        let huge = text_billable_units(u64::MAX);
        assert_eq!(bulk_estimated_billable_units([u64::MAX]).unwrap(), huge);
        assert_eq!(
            bulk_estimated_billable_units([u64::MAX, u64::MAX]).unwrap(),
            2 * huge
        );
    }

    #[test]
    fn plan_rejects_an_empty_item_set() {
        assert_eq!(
            BulkSubmissionPlan::new(Vec::new(), 10),
            Err(DomainError::EmptyValue("bulk items"))
        );
    }

    #[test]
    fn plan_rejects_a_zero_caller_ceiling() {
        let items = vec![item(None, "hello", 3)];
        assert_eq!(
            BulkSubmissionPlan::new(items, 0),
            Err(DomainError::OutOfRange("max billable units"))
        );
    }

    #[test]
    fn plan_enforces_the_caller_ceiling() {
        let items = vec![item(None, "a", 100), item(None, "b", 101)]; // 1 + 2 = 3
        assert_eq!(
            BulkSubmissionPlan::new(items, 2),
            Err(DomainError::BulkLimitExceeded)
        );
    }

    #[test]
    fn plan_enforces_the_upstream_1000_unit_cap_above_the_caller_ceiling() {
        // A large caller ceiling cannot raise Pangram's documented cap.
        let items = vec![item(None, "x", 1001)]; // 1001 words -> 11 units
        let plan = BulkSubmissionPlan::new(items, 2000).unwrap();
        assert!(plan.estimated_billable_units() <= BULK_BILLABLE_UNIT_LIMIT);

        // 100 items of 10 units each = 1000 fits; 1010 units rejects.
        let fits: Vec<_> = (0..100).map(|_| item(None, "x", 1000)).collect();
        assert!(BulkSubmissionPlan::new(fits, 2000).is_ok());
        let over: Vec<_> = (0..101).map(|_| item(None, "x", 1000)).collect();
        assert_eq!(
            BulkSubmissionPlan::new(over, 2000),
            Err(DomainError::BulkLimitExceeded)
        );
    }

    #[test]
    fn plan_rejects_duplicate_caller_ids() {
        let items = vec![item(Some("row-1"), "a", 1), item(Some("row-1"), "b", 1)];
        assert_eq!(
            BulkSubmissionPlan::new(items, 10),
            Err(DomainError::DuplicateBulkCallerId)
        );
    }

    #[test]
    fn plan_preserves_order_and_partial_caller_id_presence() {
        let items = vec![item(Some("row-1"), "first", 50), item(None, "second", 150)];
        let plan = BulkSubmissionPlan::new(items, 10).unwrap();
        assert_eq!(plan.items().len(), 2);
        assert!(plan.caller_ids_present());
        assert_eq!(plan.estimated_billable_units(), 1 + 2);
    }

    #[test]
    fn submit_body_uses_the_items_shape_with_one_job_wide_model_when_ids_exist() {
        let items = vec![item(Some("row-001"), "First text", 1)];
        let plan = BulkSubmissionPlan::new(items, 10).unwrap();
        let body = plan.submit_body();
        assert_eq!(body["model"], "pangram-4");
        assert_eq!(
            body["items"],
            json!([{ "id": "row-001", "text": "First text" }])
        );
        assert!(body.get("text").is_none());
        assert!(body.get("public_dashboard_link").is_none());
        assert_eq!(body.as_object().unwrap().len(), 2);
    }

    #[test]
    fn submit_body_uses_the_text_shape_when_no_ids_exist() {
        let items = vec![item(None, "First text", 1), item(None, "Second text", 1)];
        let plan = BulkSubmissionPlan::new(items, 10).unwrap();
        let body = plan.submit_body();
        assert_eq!(body["model"], "pangram-4");
        assert_eq!(body["text"], json!(["First text", "Second text"]));
        assert!(body.get("items").is_none());
        assert_eq!(body.as_object().unwrap().len(), 2);
    }

    #[test]
    fn jsonl_item_rejects_unknown_fields_and_null_id() {
        assert!(serde_json::from_str::<BulkJsonlItem>(r#"{"text":"a","extra":1}"#).is_err());
        assert!(serde_json::from_str::<BulkJsonlItem>(r#"{"text":"a","id":null}"#).is_err());
        let ok: BulkJsonlItem = serde_json::from_str(r#"{"id":"row-1","text":"a"}"#).unwrap();
        assert_eq!(ok.id.unwrap().as_str(), "row-1");
    }

    #[test]
    fn status_response_decodes_the_documented_shape() {
        let decoded: BulkStatusResponse = serde_json::from_value(json!({
            "bulk_id": "blk_123",
            "status": "partial",
            "total_items": 3,
            "accepted": 2,
            "succeeded": 2,
            "failed": 1,
            "created_at": "1760000000.0",
            "completed_at": "1760000030.0"
        }))
        .unwrap();
        assert_eq!(decoded.bulk_id.as_str(), "blk_123");
        assert_eq!(decoded.completed_at.unwrap().as_str(), "1760000030.0");

        // A non-terminal job documents a null completed_at.
        let running: BulkStatusResponse = serde_json::from_value(json!({
            "bulk_id": "blk_123",
            "status": "running",
            "total_items": 3,
            "accepted": 2,
            "succeeded": 0,
            "failed": 0,
            "created_at": "1760000000.0",
            "completed_at": null
        }))
        .unwrap();
        assert!(running.completed_at.is_none());
    }

    #[test]
    fn results_page_decodes_success_and_failed_lists() {
        let page: BulkResultsPage = serde_json::from_value(json!({
            "bulk_id": "blk_123",
            "offset": 0,
            "limit": 100,
            "total_items": 2,
            "items": [{
                "index": 0,
                "id": "row-001",
                "task_id": "task-1",
                "stage": "STAGE_SUCCESS",
                "error": null,
                "result": {"version": "4.0"}
            }],
            "failed_items": [{
                "index": 1,
                "id": "row-002",
                "task_id": null,
                "stage": "STAGE_FAILED",
                "error": "Text must contain at least one valid token"
            }]
        }))
        .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].result.as_ref().unwrap()["version"], "4.0");
        assert_eq!(page.failed_items.len(), 1);
    }

    fn words(text: &str) -> u64 {
        text.split_whitespace().count() as u64
    }

    #[test]
    fn jsonl_parses_a_whole_valid_file_preserving_order_and_ids() {
        let input = concat!(
            "{\"id\":\"row-001\",\"text\":\"First text to analyze\"}\n",
            "{\"text\":\"Second text\"}\n",
            "{\"id\":\"row-003\",\"text\":\"one two three\"}"
        );
        let items = parse_bulk_jsonl(input, words).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].caller_id().unwrap().as_str(), "row-001");
        assert!(items[1].caller_id().is_none());
        assert_eq!(items[2].word_count(), 3);
    }

    #[test]
    fn jsonl_rejects_an_empty_file_and_a_blank_line() {
        assert_eq!(parse_bulk_jsonl("", words), Err(BulkJsonlError::EmptyFile));
        // A wholly-whitespace file is a blank line, not an empty file: the
        // whole-file rule never silently drops it.
        assert!(matches!(
            parse_bulk_jsonl("   \n  ", words),
            Err(BulkJsonlError::InvalidLine { line: 1, .. })
        ));

        let with_blank = "{\"text\":\"a\"}\n\n{\"text\":\"b\"}";
        assert_eq!(
            parse_bulk_jsonl(with_blank, words),
            Err(BulkJsonlError::InvalidLine {
                line: 2,
                reason: "an empty line is not a bulk item object".to_owned()
            })
        );
    }

    #[test]
    fn jsonl_rejects_unknown_fields_duplicate_ids_and_bad_lines_whole_file() {
        // Unknown field anywhere => whole-file failure (never partial).
        let unknown = "{\"text\":\"a\"}\n{\"text\":\"b\",\"extra\":1}";
        assert!(matches!(
            parse_bulk_jsonl(unknown, words),
            Err(BulkJsonlError::InvalidLine { line: 2, .. })
        ));

        // A malformed line fails the whole file even after valid items.
        let malformed = "{\"text\":\"a\"}\nnot-json";
        assert!(matches!(
            parse_bulk_jsonl(malformed, words),
            Err(BulkJsonlError::InvalidLine { line: 2, .. })
        ));

        // Duplicate caller IDs are caught at plan construction (the adapter
        // feeds parsed items straight into the plan).
        let duplicate = "{\"id\":\"row-1\",\"text\":\"a\"}\n{\"id\":\"row-1\",\"text\":\"b\"}";
        let items = parse_bulk_jsonl(duplicate, words).unwrap();
        assert_eq!(
            BulkSubmissionPlan::new(items, 10),
            Err(DomainError::DuplicateBulkCallerId)
        );
    }
}
