//! Focused Rust-owned domain schema contracts.

#![forbid(unsafe_code)]

use microck_pangram_cli::domain::{
    AiDetectionResult, BulkCollection, BulkCounters, BulkPage, Provenance, UpstreamTaskIds,
};
use schemars::schema_for;
use serde_json::{Value, json};

#[test]
fn bulk_json_schemas_expose_runtime_numeric_bounds() {
    let counters = serde_json::to_value(schema_for!(BulkCounters)).unwrap();
    assert_eq!(counters["properties"]["total_items"]["minimum"], json!(1));

    let collection = serde_json::to_value(schema_for!(BulkCollection)).unwrap();
    assert_eq!(
        collection["properties"]["estimated_billable_units"]["minimum"],
        json!(1)
    );

    let page = serde_json::to_value(schema_for!(BulkPage<Value>)).unwrap();
    assert_eq!(page["properties"]["limit"]["minimum"], json!(1));
    assert_eq!(page["properties"]["limit"]["maximum"], json!(1000));
}

#[test]
fn pangram_4_detection_schema_exposes_current_result_fields() {
    let schema = serde_json::to_value(schema_for!(AiDetectionResult)).unwrap();

    assert_eq!(
        schema["$defs"]["AiClassification"]["enum"],
        json!(["ai", "human", "mixed"])
    );
    assert_eq!(
        schema["$defs"]["Segment"]["required"],
        json!([
            "text",
            "label",
            "ai_assistance_score",
            "confidence",
            "start_index",
            "end_index",
            "word_count",
            "token_length",
            "humanizer_score",
            "is_humanized"
        ])
    );
}

#[test]
fn provenance_uses_a_neutral_upstream_version_name() {
    let schema = serde_json::to_value(schema_for!(Provenance)).unwrap();

    assert!(schema["properties"].get("upstream_version").is_some());
    assert!(schema["properties"].get("api_version").is_none());
    assert!(schema["properties"].get("model_version").is_none());
}

#[test]
fn upstream_task_ids_schema_requires_unique_items() {
    let schema = serde_json::to_value(schema_for!(UpstreamTaskIds)).unwrap();

    assert_eq!(schema["uniqueItems"], json!(true));
}
