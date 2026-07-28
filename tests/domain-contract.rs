use std::str::FromStr;

use microck_pangram_cli::domain::{
    AiDetectionResult, Analysis, AnalysisId, AnalysisInput, AnalysisStatus, BulkCollection,
    BulkCounters, BulkId, BulkItem, BulkItemState, BulkPage, Check, CheckKind, CheckState,
    CheckStatus, FileInput, LocalOperationId, OrderedChecks, Provenance, Sha256Hash,
    SubmissionOutcome, SubmissionOutcomeUnknownDetails, UpstreamBulkId, UpstreamIdentity,
    UpstreamTaskIds, UtcTimestamp, derive_parent_status,
};
use proptest::prelude::*;
use schemars::schema_for;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use uuid::Version;

#[test]
fn local_ids_are_prefixed_uuid_v7_values() {
    let analysis_id = AnalysisId::new();
    let bulk_id = BulkId::new();

    assert!(analysis_id.to_string().starts_with("anl_"));
    assert!(bulk_id.to_string().starts_with("bulk_"));
    assert_eq!(analysis_id.uuid().get_version(), Some(Version::SortRand));
    assert_eq!(bulk_id.uuid().get_version(), Some(Version::SortRand));
}

#[test]
fn accepted_local_ids_round_trip_through_their_public_form() {
    let analysis_id = AnalysisId::new();
    let bulk_id = BulkId::new();

    assert_eq!(
        AnalysisId::from_str(&analysis_id.to_string()).unwrap(),
        analysis_id
    );
    assert_eq!(BulkId::from_str(&bulk_id.to_string()).unwrap(), bulk_id);
}

#[test]
fn local_ids_reject_wrong_prefix_case_and_uuid_version() {
    let valid = AnalysisId::new().to_string();
    let uuid = valid.strip_prefix("anl_").unwrap();

    assert!(AnalysisId::from_str(uuid).is_err());
    assert!(AnalysisId::from_str(&format!("ANL_{uuid}")).is_err());
    assert!(AnalysisId::from_str("anl_550e8400-e29b-41d4-a716-446655440000").is_err());
}

proptest! {
    #[test]
    fn arbitrary_strings_never_parse_unless_they_match_the_local_id_contract(value in "\\PC{0,100}") {
        if let Ok(id) = AnalysisId::from_str(&value) {
            prop_assert_eq!(id.to_string(), value.as_str());
            prop_assert!(value.starts_with("anl_"));
            prop_assert_eq!(id.uuid().get_version(), Some(Version::SortRand));
        }
    }

    #[test]
    fn parent_status_follows_running_then_queued_then_terminal_precedence(
        statuses in prop::collection::vec(
            prop_oneof![
                Just(CheckStatus::Queued),
                Just(CheckStatus::Running),
                Just(CheckStatus::Succeeded),
                Just(CheckStatus::Failed),
            ],
            1..=2,
        )
    ) {
        let expected = if statuses.contains(&CheckStatus::Running) {
            AnalysisStatus::Running
        } else if statuses.contains(&CheckStatus::Queued) {
            AnalysisStatus::Queued
        } else if statuses.iter().all(|status| *status == CheckStatus::Succeeded) {
            AnalysisStatus::Succeeded
        } else if statuses.iter().all(|status| *status == CheckStatus::Failed) {
            AnalysisStatus::Failed
        } else {
            AnalysisStatus::Partial
        };

        prop_assert_eq!(derive_parent_status(&statuses).unwrap(), expected);
    }

    #[test]
    fn bulk_counters_accept_exactly_the_contract_range(
        total in 0_u64..8,
        accepted in 0_u64..8,
        succeeded in 0_u64..8,
        failed in 0_u64..8,
    ) {
        let expected = total > 0
            && accepted <= total
            && succeeded <= accepted
            && succeeded.checked_add(failed).is_some_and(|finished| finished <= total);

        prop_assert_eq!(
            BulkCounters::new(total, accepted, succeeded, failed).is_ok(),
            expected
        );
    }

    #[test]
    fn unknown_submission_details_accept_exactly_one_local_identity(
        analysis_identity in any::<bool>(),
        bulk_identity in any::<bool>(),
    ) {
        let mut value = json!({
            "request_sha256": "0".repeat(64),
            "last_status": "sending"
        });
        if analysis_identity {
            value["analysis_id"] = json!(AnalysisId::new());
        }
        if bulk_identity {
            value["bulk_id"] = json!(BulkId::new());
        }

        prop_assert_eq!(
            serde_json::from_value::<SubmissionOutcomeUnknownDetails>(value).is_ok(),
            analysis_identity ^ bulk_identity
        );
    }
}

#[test]
fn ordered_checks_require_ai_detection_first_and_no_duplicates() {
    assert!(OrderedChecks::new([CheckKind::AiDetection]).is_ok());
    assert!(OrderedChecks::new([CheckKind::Plagiarism]).is_ok());
    assert!(OrderedChecks::new([CheckKind::AiDetection, CheckKind::Plagiarism]).is_ok());

    assert!(OrderedChecks::new([CheckKind::Plagiarism, CheckKind::AiDetection]).is_err());
    assert!(OrderedChecks::new([CheckKind::AiDetection, CheckKind::AiDetection]).is_err());
}

fn ai_result() -> Value {
    json!({
        "classification": "human",
        "headline": "Human-written",
        "prediction": "The document appears to be human-written.",
        "fraction_ai": 0.0,
        "fraction_ai_assisted": 0.0,
        "fraction_human": 1.0,
        "num_ai_segments": 0,
        "num_ai_assisted_segments": 0,
        "num_human_segments": 1,
        "segments": [{
            "text": "The text to analyze",
            "label": "Human Written",
            "ai_assistance_score": 0.0,
            "confidence": "high",
            "start_index": 0,
            "end_index": 19,
            "word_count": 4,
            "token_length": 4,
            "humanizer_score": 0.0,
            "is_humanized": false
        }]
    })
}

fn analysis_value(
    status: &str,
    submission_outcome: &str,
    check_status: &str,
    with_upstream_id: bool,
) -> Value {
    let mut check = json!({
        "kind": "ai_detection",
        "status": check_status
    });
    if with_upstream_id {
        check["upstream"] = json!({"task_id": "task-123"});
    }
    match check_status {
        "succeeded" => check["result"] = ai_result(),
        "failed" => check["error"] = json!({"message": "failed"}),
        _ => {}
    }

    json!({
        "id": AnalysisId::new(),
        "status": status,
        "submission_outcome": submission_outcome,
        "input": {
            "type": "text",
            "origin": "literal",
            "sha256": "0".repeat(64),
            "byte_count": 4,
            "word_count": 1
        },
        "checks": [check],
        "save_state": "ephemeral",
        "provenance": {"provider": "pangram"},
        "created_at": "2026-07-23T12:00:00Z",
        "updated_at": "2026-07-23T12:00:01Z"
    })
}

#[test]
fn check_states_reject_inapplicable_payload_fields_even_when_null() {
    for invalid in [
        json!({"status": "queued", "result": null}),
        json!({"status": "running", "error": {"message": "no"}}),
        json!({"status": "succeeded"}),
        json!({"status": "succeeded", "result": {}, "error": null}),
        json!({"status": "failed"}),
        json!({"status": "failed", "result": null, "error": {}}),
    ] {
        assert!(serde_json::from_value::<CheckState<Value, Value>>(invalid).is_err());
    }

    assert!(
        serde_json::from_value::<CheckState<Value, Value>>(json!({
            "status": "queued",
            "future_field": true
        }))
        .is_ok(),
        "unknown future fields remain forward-compatible"
    );
}

#[test]
fn check_state_serialization_keeps_kind_and_state_in_one_object() {
    let check: Check<Value> = Check::AiDetection(CheckState::Failed {
        upstream: None,
        error: json!({"message": "failed"}),
    });

    assert_eq!(
        serde_json::to_value(check).unwrap(),
        json!({
            "kind": "ai_detection",
            "status": "failed",
            "error": {"message": "failed"}
        })
    );
}

#[test]
fn bulk_item_states_reject_inapplicable_payload_fields_even_when_null() {
    let analysis = analysis_value("queued", "not_submitted", "queued", false);
    for invalid in [
        json!({"index": 0, "status": "queued", "analysis": null}),
        json!({"index": 0, "status": "running", "error": null}),
        json!({"index": 0, "status": "succeeded"}),
        json!({
            "index": 0,
            "status": "succeeded",
            "analysis": analysis.clone(),
            "error": null
        }),
        json!({"index": 0, "status": "failed"}),
        json!({
            "index": 0,
            "status": "failed",
            "analysis": analysis.clone(),
            "error": {"message": "failed"}
        }),
    ] {
        assert!(serde_json::from_value::<BulkItem<Value>>(invalid).is_err());
    }

    assert!(
        serde_json::from_value::<BulkItem<Value>>(json!({
            "index": 0,
            "status": "running",
            "future_field": true
        }))
        .is_ok(),
        "unknown future fields remain forward-compatible"
    );
}

#[test]
fn bulk_item_state_serialization_remains_flat() {
    let item = BulkItem {
        index: 2,
        caller_id: Some("row-2".to_owned()),
        analysis_id: None,
        upstream_task_id: None,
        state: BulkItemState::Failed {
            error: json!({"message": "failed"}),
        },
    };

    assert_eq!(
        serde_json::to_value(item).unwrap(),
        json!({
            "index": 2,
            "caller_id": "row-2",
            "status": "failed",
            "error": {"message": "failed"}
        })
    );
}

#[test]
fn unknown_submission_details_reject_missing_duplicate_null_and_unknown_fields() {
    let base = json!({
        "request_sha256": "0".repeat(64),
        "last_status": "sending"
    });
    assert!(serde_json::from_value::<SubmissionOutcomeUnknownDetails>(base.clone()).is_err());

    let mut both = base.clone();
    both["analysis_id"] = json!(AnalysisId::new());
    both["bulk_id"] = json!(BulkId::new());
    assert!(serde_json::from_value::<SubmissionOutcomeUnknownDetails>(both).is_err());

    let mut with_content = base.clone();
    with_content["analysis_id"] = json!(AnalysisId::new());
    with_content["content"] = json!("must not cross the security boundary");
    assert!(serde_json::from_value::<SubmissionOutcomeUnknownDetails>(with_content).is_err());

    let mut null_identity = base;
    null_identity["analysis_id"] = Value::Null;
    assert!(serde_json::from_value::<SubmissionOutcomeUnknownDetails>(null_identity).is_err());
}

#[test]
fn unknown_submission_details_constructor_serializes_one_identity() {
    let details = SubmissionOutcomeUnknownDetails::new(
        LocalOperationId::AnalysisId(AnalysisId::new()),
        Sha256Hash::from_str(&"0".repeat(64)).unwrap(),
        None,
        None,
        "sending".parse().unwrap(),
    );

    let value = serde_json::to_value(details).unwrap();
    assert!(value.get("analysis_id").is_some());
    assert!(value.get("bulk_id").is_none());
}

#[test]
fn analysis_submission_outcomes_require_coherent_status_and_identity() {
    assert!(
        serde_json::from_value::<Analysis<Value>>(analysis_value(
            "running", "accepted", "running", false,
        ))
        .is_err()
    );
    assert!(
        serde_json::from_value::<Analysis<Value>>(analysis_value(
            "queued", "terminal", "queued", false,
        ))
        .is_err()
    );
    assert!(
        serde_json::from_value::<Analysis<Value>>(analysis_value(
            "running",
            "not_submitted",
            "running",
            true,
        ))
        .is_err()
    );
    for (field, evidence) in [
        ("upstream_task_ids", json!([])),
        ("upstream_bulk_id", json!("bulk-123")),
        ("submitted_at", json!("2026-07-23T12:00:00Z")),
        ("completed_at", json!("2026-07-23T12:00:01Z")),
    ] {
        let mut analysis = analysis_value("running", "not_submitted", "running", false);
        analysis["provenance"][field] = evidence;
        assert!(
            serde_json::from_value::<Analysis<Value>>(analysis).is_err(),
            "not_submitted accepted provenance evidence in {field}"
        );
    }

    assert!(
        serde_json::from_value::<Analysis<Value>>(analysis_value(
            "succeeded",
            "accepted",
            "succeeded",
            true,
        ))
        .is_ok(),
        "accepted analyses remain accepted after polling completes"
    );
}

fn bulk_collection(
    status: AnalysisStatus,
    outcome: SubmissionOutcome,
    upstream: Option<UpstreamBulkId>,
    counters: BulkCounters,
) -> Result<BulkCollection, microck_pangram_cli::domain::DomainError> {
    let timestamp = UtcTimestamp::from_str("2026-07-23T12:00:00Z").unwrap();
    BulkCollection::new(
        BulkId::new(),
        upstream,
        status,
        outcome,
        counters,
        2,
        timestamp,
        timestamp,
        None,
    )
}

#[test]
fn bulk_collection_status_matches_exact_terminal_counters() {
    assert!(
        bulk_collection(
            AnalysisStatus::Succeeded,
            SubmissionOutcome::Terminal,
            None,
            BulkCounters::new(2, 2, 2, 0).unwrap(),
        )
        .is_ok()
    );
    assert!(
        bulk_collection(
            AnalysisStatus::Failed,
            SubmissionOutcome::Terminal,
            None,
            BulkCounters::new(2, 0, 0, 2).unwrap(),
        )
        .is_ok()
    );
    assert!(
        bulk_collection(
            AnalysisStatus::Partial,
            SubmissionOutcome::Terminal,
            None,
            BulkCounters::new(2, 1, 1, 1).unwrap(),
        )
        .is_ok()
    );

    for status in [
        AnalysisStatus::Queued,
        AnalysisStatus::Running,
        AnalysisStatus::Succeeded,
        AnalysisStatus::Failed,
        AnalysisStatus::Partial,
    ] {
        assert!(
            bulk_collection(
                status,
                SubmissionOutcome::Terminal,
                None,
                BulkCounters::new(2, 2, 1, 1).unwrap(),
            )
            .is_ok()
                == (status == AnalysisStatus::Partial)
        );
    }
}

#[test]
fn accepted_bulk_collections_require_an_upstream_bulk_id() {
    let counters = BulkCounters::new(2, 1, 0, 0).unwrap();
    assert!(
        bulk_collection(
            AnalysisStatus::Running,
            SubmissionOutcome::Accepted,
            None,
            counters.clone(),
        )
        .is_err()
    );
    assert!(
        bulk_collection(
            AnalysisStatus::Running,
            SubmissionOutcome::Accepted,
            Some(UpstreamBulkId::new("bulk-upstream").unwrap()),
            counters,
        )
        .is_ok()
    );
}

#[test]
fn bulk_submission_outcomes_match_status_and_upstream_identity() {
    let counters = BulkCounters::new(2, 1, 0, 0).unwrap();
    assert!(
        bulk_collection(
            AnalysisStatus::Running,
            SubmissionOutcome::Terminal,
            None,
            counters.clone(),
        )
        .is_err()
    );
    assert!(
        bulk_collection(
            AnalysisStatus::Running,
            SubmissionOutcome::NotSubmitted,
            Some(UpstreamBulkId::new("bulk-upstream").unwrap()),
            counters.clone(),
        )
        .is_err()
    );
    for counters_with_progress in [
        counters.clone(),
        BulkCounters::new(2, 1, 1, 0).unwrap(),
        BulkCounters::new(2, 0, 0, 1).unwrap(),
    ] {
        assert!(
            bulk_collection(
                AnalysisStatus::Running,
                SubmissionOutcome::NotSubmitted,
                None,
                counters_with_progress,
            )
            .is_err()
        );
    }
    assert!(
        bulk_collection(
            AnalysisStatus::Running,
            SubmissionOutcome::NotSubmitted,
            None,
            BulkCounters::new(2, 0, 0, 0).unwrap(),
        )
        .is_ok()
    );
    assert!(
        bulk_collection(
            AnalysisStatus::Running,
            SubmissionOutcome::AcceptanceUnknown,
            Some(UpstreamBulkId::new("bulk-upstream").unwrap()),
            counters,
        )
        .is_ok()
    );
}

fn assert_missing_succeeds_and_null_fails<T>(value: Value, optional_fields: &[&str])
where
    T: DeserializeOwned,
{
    assert!(serde_json::from_value::<T>(value.clone()).is_ok());
    for field in optional_fields {
        let mut with_null = value.clone();
        with_null[*field] = Value::Null;
        assert!(
            serde_json::from_value::<T>(with_null).is_err(),
            "{field} accepted explicit null"
        );
    }
}

#[test]
fn every_optional_domain_field_requires_omission_instead_of_null() {
    assert_missing_succeeds_and_null_fails::<AnalysisInput>(
        json!({
            "type": "text",
            "origin": "literal",
            "sha256": "0".repeat(64),
            "byte_count": 4,
            "word_count": 1
        }),
        &["name", "text"],
    );
    assert_missing_succeeds_and_null_fails::<FileInput>(
        json!({
            "filename": "paper.pdf",
            "media_type": "application/pdf",
            "sha256": "0".repeat(64),
            "size_bytes": 4
        }),
        &["path", "extracted_text"],
    );
    assert_missing_succeeds_and_null_fails::<AiDetectionResult>(ai_result(), &["dashboard_link"]);
    assert_missing_succeeds_and_null_fails::<UpstreamIdentity>(
        json!({}),
        &["task_id", "last_stage"],
    );
    assert_missing_succeeds_and_null_fails::<Provenance>(
        json!({"provider": "pangram"}),
        &[
            "upstream_version",
            "upstream_task_ids",
            "upstream_bulk_id",
            "submitted_at",
            "completed_at",
        ],
    );
    assert_missing_succeeds_and_null_fails::<CheckState<Value, Value>>(
        json!({"status": "queued"}),
        &["upstream"],
    );

    let details = json!({
        "analysis_id": AnalysisId::new(),
        "request_sha256": "0".repeat(64),
        "last_status": "sending"
    });
    assert_missing_succeeds_and_null_fails::<SubmissionOutcomeUnknownDetails>(
        details,
        &["upstream_task_id", "upstream_bulk_id"],
    );
    assert_missing_succeeds_and_null_fails::<Analysis<Value>>(
        analysis_value("queued", "not_submitted", "queued", false),
        &["retry_of", "rerun_of", "completed_at"],
    );
    assert_missing_succeeds_and_null_fails::<BulkCollection>(
        json!({
            "id": BulkId::new(),
            "status": "running",
            "submission_outcome": "acceptance_unknown",
            "total_items": 2,
            "accepted": 0,
            "succeeded": 0,
            "failed": 0,
            "estimated_billable_units": 2,
            "created_at": "2026-07-23T12:00:00Z",
            "updated_at": "2026-07-23T12:00:01Z"
        }),
        &["upstream_bulk_id", "completed_at"],
    );
    assert_missing_succeeds_and_null_fails::<BulkItem<Value>>(
        json!({"index": 0, "status": "running"}),
        &["caller_id", "analysis_id", "upstream_task_id"],
    );
    assert_missing_succeeds_and_null_fails::<BulkPage<Value>>(
        json!({"items": [], "offset": 0, "limit": 1}),
        &["next_offset"],
    );
}

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
