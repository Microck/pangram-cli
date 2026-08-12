use std::str::FromStr;

use microck_pangram_cli::domain::{
    Analysis, AnalysisId, AnalysisStatus, BulkCounters, BulkId, CheckKind, CheckStatus,
    LocalOperationId, NonEmptyString, Sha256Hash, SubmissionOutcomeUnknownDetails, UtcTimestamp,
};
use microck_pangram_cli::output::{
    AnalysisOutput, AuthSource, AuthStatus, CanonicalError, CommandData, CommandEnvelope,
    ConfigGetStatus, ConfigPathStatus, DoctorCheck, DoctorCheckStatus, EnvelopeMeta, ErrorCategory,
    ErrorCode, ExitCode, McpClientStatus, McpMutationAction, McpMutationReport, McpMutationTarget,
    MutationAcknowledgement, NonEmptyAnalyses, ProgressEvent, Recovery, ResolvedCommand,
    UpdateStatus, UpdateStatusKind,
};
use proptest::prelude::*;
use schemars::JsonSchema;
use serde_json::{Value, json};

const EXPECTED_UNKNOWN_SUBMISSION_RECOVERY: &str =
    "A manual retry may create a second billable operation.";

fn timestamp() -> UtcTimestamp {
    UtcTimestamp::from_str("2026-07-23T12:00:00Z").unwrap()
}

fn unknown_submission_details() -> SubmissionOutcomeUnknownDetails {
    SubmissionOutcomeUnknownDetails::new(
        LocalOperationId::AnalysisId(AnalysisId::new()),
        Sha256Hash::from_str(&"0".repeat(64)).unwrap(),
        None,
        None,
        NonEmptyString::new("request sent").unwrap(),
    )
}

fn queued_analysis_value() -> Value {
    json!({
        "id": AnalysisId::new(),
        "status": "queued",
        "submission_outcome": "not_submitted",
        "input": {
            "type": "text",
            "origin": "literal",
            "sha256": "0".repeat(64),
            "byte_count": 4,
            "word_count": 1
        },
        "checks": [{"kind": "ai_detection", "status": "queued"}],
        "save_state": "ephemeral",
        "provenance": {"provider": "pangram"},
        "created_at": "2026-07-23T12:00:00Z",
        "updated_at": "2026-07-23T12:00:01Z"
    })
}

#[test]
fn repeated_analysis_output_is_structurally_nonempty() {
    let empty: Vec<Analysis<CanonicalError>> = Vec::new();
    assert!(NonEmptyAnalyses::new(empty).is_err());
    assert!(serde_json::from_value::<AnalysisOutput>(json!([])).is_err());
    assert!(!schema_accepts::<NonEmptyAnalyses>(&json!([])));

    let analysis = serde_json::from_value(queued_analysis_value()).unwrap();
    let repeated = NonEmptyAnalyses::new(vec![analysis]).unwrap();
    let value = serde_json::to_value(AnalysisOutput::Many(repeated)).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 1);
    assert!(schema_accepts::<NonEmptyAnalyses>(&json!([
        queued_analysis_value()
    ])));
    assert!(serde_json::from_value::<AnalysisOutput>(json!([queued_analysis_value()])).is_ok());
}

#[test]
fn success_and_failure_envelopes_are_structurally_exclusive() {
    let success = CommandEnvelope::success(
        CommandData::AuthSet(MutationAcknowledgement::new()),
        EnvelopeMeta::default(),
    );
    let failure = CommandEnvelope::failure(
        ResolvedCommand::Detect,
        CanonicalError::new(
            ErrorCode::MissingApiKey,
            "No Pangram API key is configured.",
        )
        .unwrap(),
        EnvelopeMeta::default(),
    );

    let success = serde_json::to_value(success).unwrap();
    let failure = serde_json::to_value(failure).unwrap();

    assert!(success.get("data").is_some());
    assert!(success.get("error").is_none());
    assert!(failure.get("error").is_some());
    assert!(failure.get("data").is_none());
}

#[test]
fn stable_error_codes_own_their_category_and_default_retryability() {
    let missing_key = CanonicalError::new(
        ErrorCode::MissingApiKey,
        "No Pangram API key is configured.",
    )
    .unwrap();
    let rate_limited =
        CanonicalError::new(ErrorCode::RateLimited, "Pangram is rate limiting requests.").unwrap();
    let unknown_submission = CanonicalError::submission_outcome_unknown(
        "The submission outcome is unknown.",
        unknown_submission_details(),
    )
    .unwrap();

    assert_eq!(missing_key.category(), ErrorCategory::Authentication);
    assert!(!missing_key.retryable());
    assert_eq!(rate_limited.category(), ErrorCategory::RateLimit);
    assert!(rate_limited.retryable());
    assert_eq!(unknown_submission.category(), ErrorCategory::Network);
    assert!(!unknown_submission.retryable());
    assert!(unknown_submission.details().is_some());
}

#[test]
fn unknown_submission_constructor_always_supplies_exact_recovery() {
    let error = CanonicalError::submission_outcome_unknown(
        "The submission outcome is unknown.",
        unknown_submission_details(),
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(error).unwrap()["recovery"],
        json!({"message": EXPECTED_UNKNOWN_SUBMISSION_RECOVERY})
    );
}

#[test]
fn unknown_submission_deserialization_requires_exact_recovery() {
    let valid = json!({
        "code": "submission_outcome_unknown",
        "category": "network",
        "message": "unknown",
        "retryable": false,
        "recovery": {"message": EXPECTED_UNKNOWN_SUBMISSION_RECOVERY},
        "details": unknown_submission_details()
    });
    assert!(serde_json::from_value::<CanonicalError>(valid.clone()).is_ok());

    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("recovery");
    assert!(serde_json::from_value::<CanonicalError>(missing).is_err());

    let mut wrong_message = valid.clone();
    wrong_message["recovery"]["message"] = json!("Retry the request.");
    assert!(serde_json::from_value::<CanonicalError>(wrong_message).is_err());

    let mut command_bearing = valid;
    command_bearing["recovery"]["command"] = json!("pangram analyze --retry");
    assert!(serde_json::from_value::<CanonicalError>(command_bearing).is_err());
}

#[test]
fn unknown_submission_recovery_cannot_be_overwritten() {
    let error = CanonicalError::submission_outcome_unknown(
        "The submission outcome is unknown.",
        unknown_submission_details(),
    )
    .unwrap();
    assert!(
        error
            .with_recovery(Recovery::new("Retry the request.").unwrap())
            .is_err()
    );

    let generic = CanonicalError::new(ErrorCode::MissingApiKey, "missing")
        .unwrap()
        .with_recovery(Recovery::new("Configure a key.").unwrap())
        .unwrap();
    assert_eq!(generic.recovery().unwrap().message(), "Configure a key.");
}

#[test]
fn canonical_errors_reject_invalid_construction_and_deserialization() {
    assert!(CanonicalError::new(ErrorCode::MissingApiKey, "").is_err());
    assert!(
        CanonicalError::new(
            ErrorCode::SubmissionOutcomeUnknown,
            "The submission outcome is unknown."
        )
        .is_err()
    );

    let base = json!({
        "code": "missing_api_key",
        "category": "authentication",
        "message": "missing",
        "retryable": false
    });
    assert!(serde_json::from_value::<CanonicalError>(base.clone()).is_ok());

    let mut wrong_category = base.clone();
    wrong_category["category"] = json!("network");
    assert!(serde_json::from_value::<CanonicalError>(wrong_category).is_err());

    let mut wrong_retryability = base.clone();
    wrong_retryability["retryable"] = json!(true);
    assert!(serde_json::from_value::<CanonicalError>(wrong_retryability).is_err());

    let mut empty_message = base;
    empty_message["message"] = json!("");
    assert!(serde_json::from_value::<CanonicalError>(empty_message).is_err());

    let unknown_without_details = json!({
        "code": "submission_outcome_unknown",
        "category": "network",
        "message": "unknown",
        "retryable": false,
        "recovery": {"message": EXPECTED_UNKNOWN_SUBMISSION_RECOVERY}
    });
    assert!(serde_json::from_value::<CanonicalError>(unknown_without_details).is_err());
}

#[test]
fn contextual_retryability_can_only_change_contextual_codes() {
    assert!(
        CanonicalError::new(ErrorCode::NetworkTimeout, "read timed out")
            .unwrap()
            .with_contextual_retryability(true)
            .is_ok()
    );
    assert!(
        CanonicalError::new(ErrorCode::MissingApiKey, "missing")
            .unwrap()
            .with_contextual_retryability(true)
            .is_err()
    );
}

#[test]
fn envelope_deserialization_rejects_invalid_shapes_and_command_data_pairs() {
    let error = json!({
        "code": "missing_api_key",
        "category": "authentication",
        "message": "missing",
        "retryable": false
    });
    let success = json!({
        "schema_version": "1",
        "command": "auth_set",
        "data": {"ok": true},
        "meta": {}
    });
    assert!(serde_json::from_value::<CommandEnvelope>(success).is_ok());

    let both = json!({
        "schema_version": "1",
        "command": "auth_set",
        "data": {"ok": true},
        "error": error,
        "meta": {}
    });
    assert!(serde_json::from_value::<CommandEnvelope>(both).is_err());

    let neither = json!({
        "schema_version": "1",
        "command": "auth_set",
        "meta": {}
    });
    assert!(serde_json::from_value::<CommandEnvelope>(neither).is_err());

    let non_envelope = json!({
        "schema_version": "1",
        "command": "history_export",
        "data": {},
        "meta": {}
    });
    assert!(serde_json::from_value::<CommandEnvelope>(non_envelope).is_err());

    let mismatched = json!({
        "schema_version": "1",
        "command": "auth_status",
        "data": {"ok": true},
        "meta": {}
    });
    assert!(serde_json::from_value::<CommandEnvelope>(mismatched).is_err());
}

#[test]
fn progress_variants_keep_analysis_and_bulk_payloads_separate() {
    let analysis = ProgressEvent::analysis(
        AnalysisId::new(),
        CheckKind::AiDetection,
        CheckStatus::Running,
        timestamp(),
    )
    .with_upstream_stage("STAGE_PREPROCESSING")
    .unwrap();
    let bulk = ProgressEvent::bulk(BulkId::new(), AnalysisStatus::Partial, timestamp())
        .with_counters(BulkCounters::new(3, 2, 2, 1).unwrap())
        .unwrap();

    let analysis = serde_json::to_value(analysis).unwrap();
    assert!(analysis.get("analysis_id").is_some());
    assert!(analysis.get("bulk_id").is_none());
    assert!(analysis.get("counters").is_none());
    assert_eq!(analysis["status"], "running");

    let bulk = serde_json::to_value(bulk).unwrap();
    assert!(bulk.get("bulk_id").is_some());
    assert!(bulk.get("analysis_id").is_none());
    assert!(bulk.get("check").is_none());
    assert_eq!(bulk["status"], "partial");
    assert_eq!(bulk["counters"]["total_items"], 3);
}

#[test]
fn output_status_constructors_and_deserializers_enforce_schema_constraints() {
    assert!(Recovery::new("").is_err());
    assert!(Recovery::new("retry").unwrap().with_command("").is_err());
    assert!(serde_json::from_value::<MutationAcknowledgement>(json!({"ok": false})).is_err());

    assert!(AuthStatus::new(true, AuthSource::Stored, Some("12345678".into())).is_ok());
    assert!(AuthStatus::new(true, AuthSource::Stored, Some("123456789".into())).is_err());
    assert!(ConfigGetStatus::new("", Value::Null).is_err());
    assert!(ConfigPathStatus::new("").is_err());
    assert!(DoctorCheck::new("", DoctorCheckStatus::Pass, None).is_err());
    assert!(McpClientStatus::new("", false, None).is_err());
    assert!(McpMutationReport::new(false, Vec::new()).is_err());
    assert!(
        McpMutationTarget::new(
            "",
            "/home/example/.config/client/mcp.json",
            McpMutationAction::Create,
            None,
        )
        .is_err()
    );
    assert!(
        McpMutationTarget::new(
            "codex",
            "relative/config.toml",
            McpMutationAction::Update,
            None,
        )
        .is_err()
    );
    assert!(
        McpMutationTarget::new(
            "codex",
            "/home/example/.codex/config.toml",
            McpMutationAction::Unchanged,
            Some("unsafe\u{1b}[31m".into()),
        )
        .is_err()
    );
    assert!(UpdateStatus::new(UpdateStatusKind::NoUpdate, "1.2", None, None).is_err());
    assert!(UpdateStatus::new(UpdateStatusKind::NoUpdate, "1.2.3", None, None).is_ok());

    assert!(
        serde_json::from_value::<AuthStatus>(json!({
            "configured": true,
            "source": "stored",
            "masked_suffix": "123456789"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<UpdateStatus>(json!({
            "status": "no_update",
            "current_version": "v1.2.3"
        }))
        .is_err()
    );
}

#[test]
fn mcp_mutation_report_preserves_exact_order_and_closed_json_shape() {
    let report = McpMutationReport::new(
        true,
        vec![
            McpMutationTarget::new(
                "windsurf",
                "/home/example/.config/devin/mcp_config.json",
                McpMutationAction::Create,
                None,
            )
            .unwrap(),
            McpMutationTarget::new(
                "codex",
                r"C:\Users\example\.codex\config.toml",
                McpMutationAction::Unchanged,
                Some("The exact Pangram entry is already installed.".into()),
            )
            .unwrap(),
        ],
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(CommandEnvelope::success(
            CommandData::McpInstall(report),
            EnvelopeMeta::default(),
        ))
        .unwrap(),
        json!({
            "schema_version": "1",
            "command": "mcp_install",
            "data": {
                "dry_run": true,
                "targets": [
                    {
                        "client": "windsurf",
                        "path": "/home/example/.config/devin/mcp_config.json",
                        "action": "create"
                    },
                    {
                        "client": "codex",
                        "path": r"C:\Users\example\.codex\config.toml",
                        "action": "unchanged",
                        "reason": "The exact Pangram entry is already installed."
                    }
                ]
            },
            "meta": {}
        })
    );
}

#[test]
fn mcp_mutation_report_json_boundary_rejects_invalid_or_open_values() {
    assert!(
        serde_json::from_value::<McpMutationReport>(json!({
            "dry_run": false,
            "targets": []
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<McpMutationReport>(json!({
            "dry_run": false,
            "targets": [{
                "client": "codex",
                "path": "relative/config.toml",
                "action": "update"
            }]
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<McpMutationReport>(json!({
            "dry_run": false,
            "targets": [{
                "client": "codex",
                "path": "/home/example/.codex/config.toml",
                "action": "unchanged",
                "reason": null
            }]
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<McpMutationReport>(json!({
            "dry_run": false,
            "targets": [{
                "client": "codex",
                "path": "/home/example/.codex/config.toml",
                "action": "unchanged",
                "reason": "unsafe\nreason"
            }]
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<McpMutationReport>(json!({
            "dry_run": false,
            "targets": [{
                "client": "codex",
                "path": "/home/example/.codex/config.toml",
                "action": "unchanged",
                "extra": true
            }]
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<McpMutationReport>(json!({
            "dry_run": false,
            "targets": [{
                "client": "codex",
                "path": "/home/example/.codex/config.toml",
                "action": "unchanged"
            }],
            "extra": true
        }))
        .is_err()
    );
}

fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
    let schema = serde_json::to_value(schemars::schema_for!(T)).unwrap();
    jsonschema::validator_for(&schema).unwrap().is_valid(value)
}

#[test]
fn derived_status_schemas_preserve_the_runtime_constraints() {
    assert!(schema_accepts::<MutationAcknowledgement>(
        &json!({"ok": true})
    ));
    assert!(!schema_accepts::<MutationAcknowledgement>(
        &json!({"ok": false})
    ));

    assert!(schema_accepts::<Recovery>(&json!({"message": "retry"})));
    assert!(!schema_accepts::<Recovery>(&json!({"message": ""})));
    assert!(!schema_accepts::<Recovery>(
        &json!({"message": "retry", "command": ""})
    ));

    assert!(schema_accepts::<AuthStatus>(&json!({
        "configured": true,
        "source": "stored",
        "masked_suffix": "12345678"
    })));
    assert!(!schema_accepts::<AuthStatus>(&json!({
        "configured": true,
        "source": "stored",
        "masked_suffix": "123456789"
    })));
    assert!(!schema_accepts::<ConfigGetStatus>(
        &json!({"key": "", "value": null})
    ));
    assert!(!schema_accepts::<ConfigPathStatus>(&json!({"path": ""})));
    assert!(!schema_accepts::<DoctorCheck>(
        &json!({"name": "", "status": "pass"})
    ));
    assert!(!schema_accepts::<McpClientStatus>(
        &json!({"client": "", "installed": false})
    ));
    let mcp_report_schema = serde_json::to_value(schemars::schema_for!(McpMutationReport)).unwrap();
    let mcp_report_validator = jsonschema::validator_for(&mcp_report_schema).unwrap();
    assert!(mcp_report_validator.is_valid(&json!({
        "dry_run": true,
        "targets": [{
            "client": "codex",
            "path": "/home/example/.codex/config.toml",
            "action": "update"
        }]
    })));
    assert!(!mcp_report_validator.is_valid(&json!({
        "dry_run": true,
        "targets": []
    })));
    assert!(!mcp_report_validator.is_valid(&json!({
        "dry_run": true,
        "targets": [{
            "client": "codex",
            "path": "/home/example/.codex/config.toml",
            "action": "replace"
        }]
    })));
    assert!(!mcp_report_validator.is_valid(&json!({
        "dry_run": true,
        "targets": [{
            "client": "codex",
            "path": "/home/example/.codex/config.toml",
            "action": "unchanged",
            "extra": true
        }]
    })));
    assert!(!mcp_report_validator.is_valid(&json!({
        "dry_run": true,
        "targets": [{
            "client": "codex",
            "path": "/home/example/.codex/config.toml",
            "action": "unchanged"
        }],
        "extra": true
    })));
    assert!(schema_accepts::<UpdateStatus>(
        &json!({"status": "no_update", "current_version": "1.2.3"})
    ));
    assert!(!schema_accepts::<UpdateStatus>(
        &json!({"status": "no_update", "current_version": "v1.2.3"})
    ));
}

#[test]
fn explicit_null_is_rejected_at_output_json_boundaries() {
    assert!(
        serde_json::from_value::<Recovery>(json!({"message": "retry", "command": null})).is_err()
    );
    assert!(
        serde_json::from_value::<AuthStatus>(json!({
            "configured": true,
            "source": "stored",
            "masked_suffix": null
        }))
        .is_err()
    );
    assert!(serde_json::from_value::<EnvelopeMeta>(json!({"started_at": null})).is_err());
    assert!(
        serde_json::from_value::<CanonicalError>(json!({
            "code": "missing_api_key",
            "category": "authentication",
            "message": "missing",
            "retryable": false,
            "details": null
        }))
        .is_err()
    );
}

#[test]
fn error_categories_and_partial_results_map_to_stable_exit_codes() {
    assert_eq!(ExitCode::for_error(ErrorCategory::Usage), ExitCode::Usage);
    assert_eq!(
        ExitCode::for_error(ErrorCategory::Authentication),
        ExitCode::AuthenticationOrPermission
    );
    assert_eq!(
        ExitCode::for_error(ErrorCategory::Permission),
        ExitCode::AuthenticationOrPermission
    );
    assert_eq!(
        ExitCode::for_error(ErrorCategory::RateLimit),
        ExitCode::PaymentOrRateLimit
    );
    assert_eq!(
        ExitCode::for_error(ErrorCategory::UpstreamContract),
        ExitCode::NetworkOrUpstream
    );
    assert_eq!(
        ExitCode::for_error(ErrorCategory::LocalHistory),
        ExitCode::LocalState
    );
    assert_eq!(
        ExitCode::for_status(AnalysisStatus::Partial),
        ExitCode::Partial
    );
    assert_eq!(
        ExitCode::for_status(AnalysisStatus::Succeeded),
        ExitCode::Success
    );
}

proptest! {
    #[test]
    fn serialized_envelopes_never_contain_both_data_and_error(is_success in any::<bool>()) {
        let value = if is_success {
            serde_json::to_value(CommandEnvelope::success(
                CommandData::AuthSet(MutationAcknowledgement::new()),
                EnvelopeMeta::default(),
            )).unwrap()
        } else {
            serde_json::to_value(CommandEnvelope::failure(
                ResolvedCommand::Detect,
                CanonicalError::new(ErrorCode::MissingApiKey, "missing").unwrap(),
                EnvelopeMeta::default(),
            )).unwrap()
        };

        prop_assert_ne!(value.get("data").is_some(), value.get("error").is_some());
    }
}
