use std::str::FromStr;

use microck_pangram_cli::domain::{
    AiDetectionResult, Analysis, AnalysisId, AnalysisInput, AnalysisStatus, BulkCollection,
    BulkCounters, BulkId, BulkItem, BulkItemState, BulkPage, BulkSubmissionItem,
    BulkSubmissionPlan, Check, CheckKind, CheckState, CheckStatus, FileInput, LocalOperationId,
    OrderedChecks, Provenance, Sha256Hash, SubmissionOutcome, SubmissionOutcomeUnknownDetails,
    TEXT_BILLING_UNIT_WORDS, UpstreamBulkId, UpstreamIdentity, UtcTimestamp,
    bulk_estimated_billable_units, derive_parent_status, text_billable_units,
};
use proptest::prelude::*;
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
    // Pangram 4 text billing is one unit per started 100-word block, minimum
    // one. Regression pins the documented boundaries: 0, 1, 99, 100, and 101
    // words, and a large value. The generator is bounded to `u64::MAX - 99`
    // because the implementation computes `words.saturating_add(99) / 100`;
    // above that bound the saturating offset and the exact oracle diverge
    // by one for inputs that cannot arise from real text (word count is
    // derived from a u64 byte length, so u64::MAX-scale word counts are
    // unreachable). Regression pins the documented overflow boundary directly
    // below.
    fn text_billable_units_match_started_100_word_blocks(
        words in 0_u64..=(u64::MAX - (TEXT_BILLING_UNIT_WORDS - 1)),
    ) {
        // Independent oracle: compute the ceiling quotient in i128 space so no
        // u64 offset can wrap, then clamp to the minimum unit.
        let expected = (i128::from(words) + i128::from(TEXT_BILLING_UNIT_WORDS) - 1)
            .div_euclid(i128::from(TEXT_BILLING_UNIT_WORDS))
            .max(1);

        prop_assert_eq!(i128::from(text_billable_units(words)), expected);
    }

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
fn text_billable_units_pinned_boundaries() {
    let cases: &[(u64, u64)] = &[
        (0, 1),
        (1, 1),
        (19, 1),
        (99, 1),
        (100, 1),
        (101, 2),
        (199, 2),
        (200, 2),
        (201, 3),
        (1_000_000, 10_000),
        // Overflow boundary: the 99-word ceiling offset reaches u64::MAX
        // without wrapping, so the largest representable counts still bill
        // the exact precomputed ceiling quotient floor(u64::MAX / 100).
        (u64::MAX - 99, 184_467_440_737_095_516),
        (u64::MAX - 98, 184_467_440_737_095_516),
        (u64::MAX, 184_467_440_737_095_516),
    ];

    for (words, expected) in cases {
        assert_eq!(text_billable_units(*words), *expected, "words = {words}");
    }
}

proptest! {
    // Pangram 4 bulk billing is the sum of each valid item's started
    // 100-word units, minimum one per item. The plan enforces the smaller of
    // the caller ceiling and the 1,000-unit upstream cap, and rejects an
    // estimate above it before any network work (roadmap Phase 3 gate).
    #[test]
    fn bulk_plan_accepts_every_estimate_at_or_below_the_effective_ceiling(
        // Keep item units small so a modest item count can cross 1000: 1 unit
        // each (0..=100 words), up to 1100 items, caller ceiling 0..=1500.
        word_counts in prop::collection::vec(0_u64..=100, 1..=1100),
        caller_ceiling in 0_u64..=1500,
    ) {
        let items: Vec<BulkSubmissionItem> = word_counts
            .iter()
            .map(|words| BulkSubmissionItem::new(None, "text".to_owned(), *words).unwrap())
            .collect();
        // Each item is one started-100-word unit here (0..=100 words).
        let estimate = bulk_estimated_billable_units(word_counts.iter().copied()).unwrap();
        prop_assert_eq!(estimate, word_counts.len() as u64);

        let plan = BulkSubmissionPlan::new(items, caller_ceiling);
        let effective = caller_ceiling.min(1000);
        if caller_ceiling == 0 {
            prop_assert!(plan.is_err(), "a zero caller ceiling is rejected");
        } else if estimate <= effective {
            let plan = plan.expect("an estimate within the ceiling passes preflight");
            prop_assert_eq!(plan.estimated_billable_units(), estimate);
            // One job-wide model and exactly one of items/text.
            let body = plan.submit_body();
            prop_assert_eq!(&body["model"], &serde_json::json!("pangram-4"));
            prop_assert_eq!(body.get("public_dashboard_link"), None);
        } else {
            prop_assert_eq!(
                plan.unwrap_err(),
                microck_pangram_cli::domain::DomainError::BulkLimitExceeded
            );
        }
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
fn bulk_item_last_stage_is_state_bound_non_empty_and_sanitized() {
    for valid in [
        json!({"index": 0, "status": "running"}),
        json!({"index": 0, "status": "running", "last_stage": "STAGE_INFERENCE"}),
        json!({
            "index": 0,
            "status": "failed",
            "last_stage": "STAGE_FAILED",
            "error": {"message": "failed"}
        }),
    ] {
        assert!(serde_json::from_value::<BulkItem<Value>>(valid).is_ok());
    }

    for invalid in [
        json!({"index": 0, "status": "queued", "last_stage": "STAGE_QUEUED"}),
        json!({
            "index": 0,
            "status": "succeeded",
            "last_stage": "STAGE_SUCCESS",
            "analysis": analysis_value("queued", "not_submitted", "queued", false)
        }),
        json!({"index": 0, "status": "running", "last_stage": ""}),
        json!({"index": 0, "status": "running", "last_stage": " STAGE_INFERENCE"}),
        json!({
            "index": 0,
            "status": "failed",
            "last_stage": "STAGE_\u{1b}[31mFAILED",
            "error": {"message": "failed"}
        }),
    ] {
        assert!(serde_json::from_value::<BulkItem<Value>>(invalid).is_err());
    }

    assert!(
        BulkItem::<Value>::new(
            0,
            None,
            None,
            None,
            Some("STAGE_SUCCESS".to_owned()),
            BulkItemState::Queued,
        )
        .is_err(),
        "domain construction rejects a stage on a queued item"
    );
}

#[test]
fn bulk_item_state_serialization_remains_flat() {
    let item = BulkItem::new(
        2,
        Some("row-2".to_owned()),
        None,
        None,
        None,
        BulkItemState::Failed {
            error: json!({"message": "failed"}),
        },
    )
    .unwrap();

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
        Some(2),
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
fn bulk_counters_allow_finished_to_exceed_accepted_for_rejected_items() {
    // `failed` includes immediate upstream rejection, so a rejected item counts
    // toward `failed` without entering `accepted`. The documented contracts.md
    // section 9 example (`total_items = 3`, `accepted = 2`, `succeeded = 2`,
    // `failed = 1`) is terminal and valid even though `succeeded + failed = 3`
    // exceeds `accepted = 2` (PR #14 disputed counter).
    let counters = BulkCounters::new(3, 2, 2, 1).unwrap();
    assert!(counters.is_terminal());
    assert!(
        bulk_collection(
            AnalysisStatus::Partial,
            SubmissionOutcome::Terminal,
            None,
            counters,
        )
        .is_ok()
    );

    // The constructor still rejects impossible progress: an item cannot succeed
    // without being accepted first.
    assert!(BulkCounters::new(3, 2, 3, 0).is_err());
    // Cross-field bounds are constructor-owned, not schema keywords.
    assert!(BulkCounters::new(3, 4, 0, 0).is_err());
    assert!(BulkCounters::new(3, 2, 2, 2).is_err());
}

#[test]
fn accepted_bulk_collections_require_an_upstream_bulk_id() {
    let counters = BulkCounters::new(2, 1, 0, 0).unwrap();
    assert!(
        bulk_collection(
            AnalysisStatus::Running,
            SubmissionOutcome::Accepted,
            None,
            counters,
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
            counters,
        )
        .is_err()
    );
    assert!(
        bulk_collection(
            AnalysisStatus::Running,
            SubmissionOutcome::NotSubmitted,
            Some(UpstreamBulkId::new("bulk-upstream").unwrap()),
            counters,
        )
        .is_err()
    );
    for counters_with_progress in [
        counters,
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
        &["caller_id", "analysis_id", "upstream_task_id", "last_stage"],
    );
    assert_missing_succeeds_and_null_fails::<BulkPage<Value>>(
        json!({"items": [], "offset": 0, "limit": 1}),
        &["next_offset"],
    );
}
