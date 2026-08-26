#[path = "support/schema-contract.rs"]
mod support;

use serde_json::{Value, json};
use support::{Case, assert_cases};

fn analysis(status: &str) -> Value {
    let (submission_outcome, check) = match status {
        "queued" => (
            "not_submitted",
            json!({"kind": "ai_detection", "status": "queued"}),
        ),
        "running" => (
            "accepted",
            json!({
                "kind": "ai_detection",
                "status": "running",
                "upstream": {"task_id": "task-123"}
            }),
        ),
        "succeeded" => (
            "terminal",
            json!({
                "kind": "ai_detection",
                "status": "succeeded",
                "upstream": {"task_id": "task-123", "last_stage": "STAGE_SUCCESS"},
                "result": {
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
                }
            }),
        ),
        _ => panic!("unsupported fixture status"),
    };

    json!({
        "id": "anl_01983c20-0180-7a80-a001-000000000001",
        "status": status,
        "submission_outcome": submission_outcome,
        "input": {
            "type": "text",
            "origin": "literal",
            "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
            "byte_count": 15,
            "word_count": 3
        },
        "checks": [check],
        "save_state": "ephemeral",
        "provenance": {"provider": "pangram"},
        "created_at": "2026-07-23T12:00:00Z",
        "updated_at": "2026-07-23T12:00:00Z"
    })
}

fn bulk_collection() -> Value {
    json!({
        "id": "bulk_01983c20-0180-7a80-a001-000000000001",
        "upstream_bulk_id": "bulk-123",
        "status": "running",
        "submission_outcome": "accepted",
        "total_items": 1,
        "accepted": 1,
        "succeeded": 0,
        "failed": 0,
        "estimated_billable_units": 1,
        "created_at": "2026-07-23T12:00:00Z",
        "updated_at": "2026-07-23T12:00:00Z"
    })
}

fn success(command: &str, data: Value) -> Value {
    json!({
        "schema_version": "1",
        "command": command,
        "data": data,
        "meta": {"started_at": "2026-07-23T12:00:00Z"}
    })
}

fn canonical_error(code: &str, category: &str, retryable: bool) -> Value {
    json!({
        "code": code,
        "category": category,
        "message": "Synthetic contract failure.",
        "retryable": retryable
    })
}

#[test]
fn output_schema_preserves_envelope_and_domain_invariants() {
    // One running check makes the parent running regardless of the other
    // check.
    let mut both_checks = analysis("running");
    both_checks["checks"] = json!([
        {"kind": "ai_detection", "status": "running"},
        {"kind": "plagiarism", "status": "queued"}
    ]);

    let mut reversed_checks = both_checks.clone();
    reversed_checks["checks"].as_array_mut().unwrap().reverse();

    // With no running or queued check, a mixed succeeded/failed parent is
    // partial (the AI-detection result is well-formed; only the leading kind
    // is mislabeled, so the AI-first ordering invariant rejects it).
    let mut partial_two_checks = analysis("running");
    partial_two_checks["status"] = json!("partial");
    partial_two_checks["submission_outcome"] = json!("accepted");
    partial_two_checks["checks"] = json!([
        {
            "kind": "ai_detection",
            "status": "succeeded",
            "upstream": {"task_id": "task-123", "last_stage": "STAGE_SUCCESS"},
            "result": analysis("succeeded")["checks"][0]["result"].clone()
        },
        {
            "kind": "plagiarism",
            "status": "failed",
            "error": canonical_error("upstream_analysis_failed", "upstream", false)
        }
    ]);

    let mut offset_timestamp = analysis("queued");
    offset_timestamp["created_at"] = json!("2026-07-23T13:00:00+01:00");

    let mut queued_with_result = analysis("queued");
    queued_with_result["checks"][0]["result"] =
        analysis("succeeded")["checks"][0]["result"].clone();

    let mut succeeded_without_result = analysis("succeeded");
    succeeded_without_result["checks"][0]
        .as_object_mut()
        .unwrap()
        .remove("result");

    let mut failed_without_error = analysis("succeeded");
    failed_without_error["status"] = json!("failed");
    failed_without_error["checks"][0]["status"] = json!("failed");
    failed_without_error["checks"][0]
        .as_object_mut()
        .unwrap()
        .remove("result");

    // Parent-status derivation from contracts.md section 4.1: a declared
    // parent status must agree with the checks it summarizes.
    let mut succeeded_parent_with_running_check = analysis("succeeded");
    succeeded_parent_with_running_check["checks"][0]["status"] = json!("running");
    succeeded_parent_with_running_check["checks"][0]["upstream"]
        .as_object_mut()
        .unwrap()
        .remove("last_stage");
    succeeded_parent_with_running_check["checks"][0]
        .as_object_mut()
        .unwrap()
        .remove("result");

    let mut running_parent_with_terminal_checks = analysis("running");
    running_parent_with_terminal_checks["checks"] = json!([
        {
            "kind": "ai_detection",
            "status": "succeeded",
            "upstream": {"task_id": "task-123", "last_stage": "STAGE_SUCCESS"},
            "result": analysis("succeeded")["checks"][0]["result"].clone()
        },
        {
            "kind": "plagiarism",
            "status": "failed",
            "error": canonical_error("upstream_analysis_failed", "upstream", false)
        }
    ]);

    // A queued check makes the parent queued, never running.
    let mut running_parent_with_queued_checks = analysis("running");
    running_parent_with_queued_checks["checks"] = json!([
        {"kind": "plagiarism", "status": "queued"},
        {"kind": "ai_detection", "status": "queued"}
    ]);

    let mut explicit_null = analysis("succeeded");
    explicit_null["completed_at"] = Value::Null;

    let mut missing_humanizer_score = analysis("succeeded");
    missing_humanizer_score["checks"][0]["result"]["segments"][0]
        .as_object_mut()
        .unwrap()
        .remove("humanizer_score");

    let mut missing_is_humanized = analysis("succeeded");
    missing_is_humanized["checks"][0]["result"]["segments"][0]
        .as_object_mut()
        .unwrap()
        .remove("is_humanized");

    let mut ai_assisted_document_classification = analysis("succeeded");
    ai_assisted_document_classification["checks"][0]["result"]["classification"] =
        json!("ai_assisted");

    let mut additive_analysis_field = analysis("queued");
    additive_analysis_field["future_field"] = json!(true);

    let mut duplicate_provenance_task_ids = analysis("queued");
    duplicate_provenance_task_ids["provenance"]["upstream_task_ids"] =
        json!(["task-123", "task-123"]);

    let mut terminal_not_submitted_analysis = analysis("succeeded");
    terminal_not_submitted_analysis["submission_outcome"] = json!("not_submitted");
    terminal_not_submitted_analysis["checks"][0]
        .as_object_mut()
        .unwrap()
        .remove("upstream");

    let mut not_submitted_with_check_task_id = analysis("queued");
    not_submitted_with_check_task_id["checks"][0]["upstream"] = json!({"task_id": "task-123"});

    let mut not_submitted_with_provenance_task_ids = analysis("queued");
    not_submitted_with_provenance_task_ids["provenance"]["upstream_task_ids"] = json!(["task-123"]);

    let mut not_submitted_with_empty_provenance_task_ids = analysis("queued");
    not_submitted_with_empty_provenance_task_ids["provenance"]["upstream_task_ids"] = json!([]);

    let mut not_submitted_with_provenance_bulk_id = analysis("queued");
    not_submitted_with_provenance_bulk_id["provenance"]["upstream_bulk_id"] = json!("bulk-123");

    let mut not_submitted_with_submitted_at = analysis("queued");
    not_submitted_with_submitted_at["provenance"]["submitted_at"] = json!("2026-07-23T12:00:00Z");

    let mut not_submitted_with_completed_at = analysis("queued");
    not_submitted_with_completed_at["provenance"]["completed_at"] = json!("2026-07-23T12:00:00Z");

    let mut empty_bulk_collection = bulk_collection();
    empty_bulk_collection["total_items"] = json!(0);

    let mut zero_billable_units = bulk_collection();
    zero_billable_units["estimated_billable_units"] = json!(0);

    let mut not_submitted_bulk = bulk_collection();
    not_submitted_bulk["submission_outcome"] = json!("not_submitted");
    not_submitted_bulk["accepted"] = json!(0);
    not_submitted_bulk
        .as_object_mut()
        .unwrap()
        .remove("upstream_bulk_id");

    let mut not_submitted_bulk_with_upstream_id = not_submitted_bulk.clone();
    not_submitted_bulk_with_upstream_id["upstream_bulk_id"] = json!("bulk-123");

    let mut not_submitted_bulk_with_accepted_item = not_submitted_bulk.clone();
    not_submitted_bulk_with_accepted_item["accepted"] = json!(1);

    let mut not_submitted_bulk_with_succeeded_item = not_submitted_bulk.clone();
    not_submitted_bulk_with_succeeded_item["succeeded"] = json!(1);

    let mut not_submitted_bulk_with_failed_item = not_submitted_bulk.clone();
    not_submitted_bulk_with_failed_item["failed"] = json!(1);

    // An accepted submission must carry the upstream bulk identifier (PR #14
    // review: encode the expressible BulkCollection invariants).
    let mut accepted_bulk_without_upstream_id = bulk_collection();
    accepted_bulk_without_upstream_id
        .as_object_mut()
        .unwrap()
        .remove("upstream_bulk_id");

    // A terminally resolved submission cannot still report an active status.
    let mut terminal_bulk_still_running = bulk_collection();
    terminal_bulk_still_running["submission_outcome"] = json!("terminal");
    terminal_bulk_still_running["status"] = json!("running");

    let mut terminal_bulk_still_queued = bulk_collection();
    terminal_bulk_still_queued["submission_outcome"] = json!("terminal");
    terminal_bulk_still_queued["status"] = json!("queued");
    terminal_bulk_still_queued["accepted"] = json!(0);

    // A `succeeded` collection must not carry failures and must record at
    // least one succeeded item; the base running fixture has both terminal
    // counters at zero.
    let mut terminal_succeeded_bulk_with_unfinished_items = bulk_collection();
    terminal_succeeded_bulk_with_unfinished_items["status"] = json!("succeeded");

    let mut succeeded_bulk_with_failures = bulk_collection();
    succeeded_bulk_with_failures["status"] = json!("succeeded");
    succeeded_bulk_with_failures["submission_outcome"] = json!("terminal");
    succeeded_bulk_with_failures["succeeded"] = json!(1);
    succeeded_bulk_with_failures["failed"] = json!(1);
    succeeded_bulk_with_failures["total_items"] = json!(2);
    succeeded_bulk_with_failures["accepted"] = json!(2);

    let mut failed_bulk_with_successes = bulk_collection();
    failed_bulk_with_successes["status"] = json!("failed");
    failed_bulk_with_successes["submission_outcome"] = json!("terminal");
    failed_bulk_with_successes["succeeded"] = json!(1);
    failed_bulk_with_successes["failed"] = json!(1);
    failed_bulk_with_successes["total_items"] = json!(2);
    failed_bulk_with_successes["accepted"] = json!(2);

    let mut partial_bulk_with_unfinished_items = bulk_collection();
    partial_bulk_with_unfinished_items["status"] = json!("partial");

    // A `partial` collection requires both outcomes, so `failed: 0` is invalid
    // even when the counters otherwise remain terminal and consistent.
    let mut partial_bulk_without_failures = bulk_collection();
    partial_bulk_without_failures["status"] = json!("partial");
    partial_bulk_without_failures["submission_outcome"] = json!("terminal");
    partial_bulk_without_failures["succeeded"] = json!(1);
    partial_bulk_without_failures["failed"] = json!(0);
    partial_bulk_without_failures["total_items"] = json!(1);
    partial_bulk_without_failures["accepted"] = json!(1);

    let mut partial_bulk_may_finish_beyond_accepted = bulk_collection();
    partial_bulk_may_finish_beyond_accepted["status"] = json!("partial");
    partial_bulk_may_finish_beyond_accepted["submission_outcome"] = json!("terminal");
    partial_bulk_may_finish_beyond_accepted["total_items"] = json!(3);
    partial_bulk_may_finish_beyond_accepted["accepted"] = json!(2);
    partial_bulk_may_finish_beyond_accepted["succeeded"] = json!(2);
    partial_bulk_may_finish_beyond_accepted["failed"] = json!(1);
    partial_bulk_may_finish_beyond_accepted["completed_at"] = json!("2026-07-23T12:00:30Z");

    let mut both_data_and_error = success("detect", analysis("running"));
    both_data_and_error["error"] = canonical_error("missing_api_key", "authentication", false);

    let mut success_with_malformed_error = success("detect", analysis("running"));
    success_with_malformed_error["error"] = json!({"malformed": true});

    let error_with_malformed_data = json!({
        "schema_version": "1",
        "command": "detect",
        "data": false,
        "error": canonical_error("missing_api_key", "authentication", false),
        "meta": {"failed_at": "2026-07-23T12:00:00Z"}
    });

    let failed_item = json!({
        "index": 0,
        "status": "failed",
        "error": canonical_error("upstream_error", "upstream", false)
    });

    assert_cases(
        "output.schema.json",
        vec![
            Case {
                name: "single analysis",
                instance: success("detect", analysis("succeeded")),
                valid: true,
            },
            Case {
                name: "repeated detect analyses",
                instance: success("detect", json!([analysis("queued")])),
                valid: true,
            },
            Case {
                name: "task result is never an array",
                instance: success("task_status", json!([analysis("queued")])),
                valid: false,
            },
            Case {
                name: "success and failure are exclusive",
                instance: both_data_and_error,
                valid: false,
            },
            Case {
                name: "success envelopes reject malformed error fields",
                instance: success_with_malformed_error,
                valid: false,
            },
            Case {
                name: "error envelopes reject malformed data fields",
                instance: error_with_malformed_data,
                valid: false,
            },
            Case {
                name: "ai check precedes plagiarism",
                instance: success("analyze", both_checks),
                valid: true,
            },
            Case {
                name: "partial parents allow mixed succeeded and failed checks",
                instance: success("analyze", partial_two_checks),
                valid: true,
            },
            Case {
                name: "reversed checks are rejected",
                instance: success("analyze", reversed_checks),
                valid: false,
            },
            Case {
                name: "a succeeded parent cannot contain a running check",
                instance: success("detect", succeeded_parent_with_running_check),
                valid: false,
            },
            Case {
                name: "a running parent cannot contain only terminal checks",
                instance: success("analyze", running_parent_with_terminal_checks),
                valid: false,
            },
            Case {
                name: "a running parent cannot contain only finished checks",
                instance: success("detect", running_parent_with_queued_checks),
                valid: false,
            },
            Case {
                name: "timestamps use UTC Z",
                instance: success("detect", offset_timestamp),
                valid: false,
            },
            Case {
                name: "queued checks cannot contain results",
                instance: success("detect", queued_with_result),
                valid: false,
            },
            Case {
                name: "succeeded checks require results",
                instance: success("detect", succeeded_without_result),
                valid: false,
            },
            Case {
                name: "failed checks require errors",
                instance: success("detect", failed_without_error),
                valid: false,
            },
            Case {
                name: "optional fields reject explicit null",
                instance: success("detect", explicit_null),
                valid: false,
            },
            Case {
                name: "segments allow file responses without humanizer scores",
                instance: success("detect", missing_humanizer_score),
                valid: true,
            },
            Case {
                name: "segments allow file responses without humanized decisions",
                instance: success("detect", missing_is_humanized),
                valid: true,
            },
            Case {
                name: "AI-assisted evidence is not a Pangram 4 document classification",
                instance: success("detect", ai_assisted_document_classification),
                valid: false,
            },
            Case {
                name: "additive analysis fields remain compatible",
                instance: success("detect", additive_analysis_field),
                valid: true,
            },
            Case {
                name: "config list shape",
                instance: success("config_list", json!({"config": {}})),
                valid: true,
            },
            Case {
                name: "config get shape",
                instance: success(
                    "config_get",
                    json!({"key": "history.enabled", "value": false}),
                ),
                valid: true,
            },
            Case {
                name: "config path shape",
                instance: success("config_path", json!({"path": "/tmp/pangram/config.toml"})),
                valid: true,
            },
            Case {
                name: "config command shapes do not cross",
                instance: success("config_list", json!({"path": "/tmp/pangram/config.toml"})),
                valid: false,
            },
            Case {
                name: "bulk collections require at least one item",
                instance: success("bulk_status", empty_bulk_collection),
                valid: false,
            },
            Case {
                name: "bulk collections require positive estimated units",
                instance: success("bulk_status", zero_billable_units),
                valid: false,
            },
            // The cross-field arithmetic bounds (accepted <= total_items,
            // succeeded <= accepted, succeeded + failed <= total_items) and the
            // exact terminal equation compare or sum distinct integer fields.
            // Draft 2020-12 has no keyword for that over unbounded integers, so
            // the canonical `BulkCounters`/`BulkCollection` constructor remains
            // authoritative for them; they are proven in the domain-contract
            // counter and terminal-status regressions (see contracts.md
            // section 9). The schema encodes the expressible status-driven
            // counter minimums and zero-pins instead.
            Case {
                name: "terminal succeeded bulk collections cannot leave unfinished items",
                instance: success("bulk_status", terminal_succeeded_bulk_with_unfinished_items),
                valid: false,
            },
            Case {
                name: "terminal succeeded bulk collections cannot carry failures",
                instance: success("bulk_status", succeeded_bulk_with_failures),
                valid: false,
            },
            Case {
                name: "terminal failed bulk collections cannot carry successes",
                instance: success("bulk_status", failed_bulk_with_successes),
                valid: false,
            },
            Case {
                name: "terminal partial bulk collections cannot leave unfinished items",
                instance: success("bulk_status", partial_bulk_with_unfinished_items),
                valid: false,
            },
            Case {
                name: "terminal partial bulk collections require failures",
                instance: success("bulk_status", partial_bulk_without_failures),
                valid: false,
            },
            // Failed includes immediate upstream rejection, so a terminal
            // partial may finish beyond accepted (PR #14 disputed counter).
            Case {
                name: "terminal partial bulk collections may finish beyond accepted",
                instance: success("bulk_status", partial_bulk_may_finish_beyond_accepted),
                valid: true,
            },
            Case {
                name: "not-submitted bulk collections accept zero progress",
                instance: success("bulk_status", not_submitted_bulk),
                valid: true,
            },
            Case {
                name: "not-submitted bulk collections reject upstream IDs",
                instance: success("bulk_status", not_submitted_bulk_with_upstream_id),
                valid: false,
            },
            Case {
                name: "not-submitted bulk collections reject accepted items",
                instance: success("bulk_status", not_submitted_bulk_with_accepted_item),
                valid: false,
            },
            Case {
                name: "not-submitted bulk collections reject succeeded items",
                instance: success("bulk_status", not_submitted_bulk_with_succeeded_item),
                valid: false,
            },
            Case {
                name: "not-submitted bulk collections reject failed items",
                instance: success("bulk_status", not_submitted_bulk_with_failed_item),
                valid: false,
            },
            Case {
                name: "accepted bulk collections require an upstream bulk id",
                instance: success("bulk_status", accepted_bulk_without_upstream_id),
                valid: false,
            },
            Case {
                name: "terminal bulk collections cannot still be running",
                instance: success("bulk_status", terminal_bulk_still_running),
                valid: false,
            },
            Case {
                name: "terminal bulk collections cannot still be queued",
                instance: success("bulk_status", terminal_bulk_still_queued),
                valid: false,
            },
            Case {
                name: "bulk result limits require at least one item",
                instance: success(
                    "bulk_results",
                    json!({"items": [], "offset": 0, "limit": 0}),
                ),
                valid: false,
            },
            Case {
                name: "bulk result limits stop at one thousand items",
                instance: success(
                    "bulk_results",
                    json!({"items": [], "offset": 0, "limit": 1001}),
                ),
                valid: false,
            },
            Case {
                name: "failed bulk item contains an error",
                instance: success(
                    "bulk_results",
                    json!({"items": [failed_item], "offset": 0, "limit": 1}),
                ),
                valid: true,
            },
            Case {
                name: "running bulk items may carry a sanitized last stage",
                instance: success(
                    "bulk_results",
                    json!({
                        "items": [{
                            "index": 0,
                            "status": "running",
                            "last_stage": "STAGE_INFERENCE"
                        }],
                        "offset": 0,
                        "limit": 1
                    }),
                ),
                valid: true,
            },
            Case {
                name: "queued bulk items reject last stage",
                instance: success(
                    "bulk_results",
                    json!({
                        "items": [{
                            "index": 0,
                            "status": "queued",
                            "last_stage": "STAGE_QUEUED"
                        }],
                        "offset": 0,
                        "limit": 1
                    }),
                ),
                valid: false,
            },
            Case {
                name: "succeeded bulk items reject last stage",
                instance: success(
                    "bulk_results",
                    json!({
                        "items": [{
                            "index": 0,
                            "status": "succeeded",
                            "last_stage": "STAGE_SUCCESS",
                            "analysis": analysis("succeeded")
                        }],
                        "offset": 0,
                        "limit": 1
                    }),
                ),
                valid: false,
            },
            Case {
                name: "bulk item last stage must be non-empty",
                instance: success(
                    "bulk_results",
                    json!({
                        "items": [{
                            "index": 0,
                            "status": "running",
                            "last_stage": ""
                        }],
                        "offset": 0,
                        "limit": 1
                    }),
                ),
                valid: false,
            },
            Case {
                name: "bulk item last stage must be sanitized",
                instance: success(
                    "bulk_results",
                    json!({
                        "items": [{
                            "index": 0,
                            "status": "failed",
                            "last_stage": "STAGE_\u{1b}[31mFAILED",
                            "error": canonical_error("upstream_error", "upstream", false)
                        }],
                        "offset": 0,
                        "limit": 1
                    }),
                ),
                valid: false,
            },
            Case {
                name: "failed bulk item cannot omit its error",
                instance: success(
                    "bulk_results",
                    json!({"items": [{"index": 0, "status": "failed"}], "offset": 0, "limit": 1}),
                ),
                valid: false,
            },
            Case {
                name: "bulk items require a status",
                instance: success(
                    "bulk_results",
                    json!({"items": [{"index": 0}], "offset": 0, "limit": 1}),
                ),
                valid: false,
            },
            Case {
                name: "succeeded bulk items require typed analyses",
                instance: success(
                    "bulk_results",
                    json!({
                        "items": [{"index": 0, "status": "succeeded", "analysis": false}],
                        "offset": 0,
                        "limit": 1
                    }),
                ),
                valid: false,
            },
            Case {
                name: "failed bulk items require typed errors",
                instance: success(
                    "bulk_results",
                    json!({
                        "items": [{"index": 0, "status": "failed", "error": false}],
                        "offset": 0,
                        "limit": 1
                    }),
                ),
                valid: false,
            },
            Case {
                name: "queued bulk items reject terminal analyses",
                instance: success(
                    "bulk_results",
                    json!({
                        "items": [{
                            "index": 0,
                            "status": "queued",
                            "analysis": analysis("queued")
                        }],
                        "offset": 0,
                        "limit": 1
                    }),
                ),
                valid: false,
            },
            Case {
                name: "running bulk items reject terminal errors",
                instance: success(
                    "bulk_results",
                    json!({
                        "items": [{
                            "index": 0,
                            "status": "running",
                            "error": canonical_error("upstream_error", "upstream", false)
                        }],
                        "offset": 0,
                        "limit": 1
                    }),
                ),
                valid: false,
            },
            Case {
                name: "error code owns its category",
                instance: json!({
                    "schema_version": "1",
                    "command": "detect",
                    "error": canonical_error("missing_api_key", "authentication", false),
                    "meta": {"failed_at": "2026-07-23T12:00:00Z"}
                }),
                valid: true,
            },
            Case {
                name: "mismatched error category",
                instance: json!({
                    "schema_version": "1",
                    "command": "detect",
                    "error": canonical_error("missing_api_key", "network", false),
                    "meta": {"failed_at": "2026-07-23T12:00:00Z"}
                }),
                valid: false,
            },
            Case {
                name: "fixed retryability cannot be overridden",
                instance: json!({
                    "schema_version": "1",
                    "command": "detect",
                    "error": canonical_error("rate_limited", "rate_limit", false),
                    "meta": {"failed_at": "2026-07-23T12:00:00Z"}
                }),
                valid: false,
            },
            Case {
                name: "error messages are nonempty",
                instance: json!({
                    "schema_version": "1",
                    "command": "detect",
                    "error": {
                        "code": "missing_api_key",
                        "category": "authentication",
                        "message": "",
                        "retryable": false
                    },
                    "meta": {"failed_at": "2026-07-23T12:00:00Z"}
                }),
                valid: false,
            },
            Case {
                name: "unknown submission details identify one local operation",
                instance: json!({
                    "schema_version": "1",
                    "command": "detect",
                    "error": {
                        "code": "submission_outcome_unknown",
                        "category": "network",
                        "message": "The submission outcome is unknown.",
                        "retryable": false,
                        "details": {
                            "analysis_id": "anl_01983c20-0180-7a80-a001-000000000001",
                            "bulk_id": "bulk_01983c20-0180-7a80-a001-000000000001",
                            "request_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                            "last_status": "sending"
                        }
                    },
                    "meta": {"failed_at": "2026-07-23T12:00:00Z"}
                }),
                valid: false,
            },
            Case {
                name: "unknown submission details reject unknown fields",
                instance: json!({
                    "schema_version": "1",
                    "command": "detect",
                    "error": {
                        "code": "submission_outcome_unknown",
                        "category": "network",
                        "message": "The submission outcome is unknown.",
                        "retryable": false,
                        "details": {
                            "analysis_id": "anl_01983c20-0180-7a80-a001-000000000001",
                            "request_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                            "last_status": "sending",
                            "content": "must not be accepted"
                        }
                    },
                    "meta": {"failed_at": "2026-07-23T12:00:00Z"}
                }),
                valid: false,
            },
            Case {
                name: "unknown submissions require recovery guidance",
                instance: json!({
                    "schema_version": "1",
                    "command": "detect",
                    "error": {
                        "code": "submission_outcome_unknown",
                        "category": "network",
                        "message": "The submission outcome is unknown.",
                        "retryable": false,
                        "details": {
                            "analysis_id": "anl_01983c20-0180-7a80-a001-000000000001",
                            "request_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                            "last_status": "sending"
                        }
                    },
                    "meta": {"failed_at": "2026-07-23T12:00:00Z"}
                }),
                valid: false,
            },
            Case {
                name: "unknown submission recovery uses the canonical message",
                instance: json!({
                    "schema_version": "1",
                    "command": "detect",
                    "error": {
                        "code": "submission_outcome_unknown",
                        "category": "network",
                        "message": "The submission outcome is unknown.",
                        "retryable": false,
                        "details": {
                            "analysis_id": "anl_01983c20-0180-7a80-a001-000000000001",
                            "request_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                            "last_status": "sending"
                        },
                        "recovery": {"message": "Retry this command."}
                    },
                    "meta": {"failed_at": "2026-07-23T12:00:00Z"}
                }),
                valid: false,
            },
            Case {
                name: "unknown submission recovery cannot contain commands",
                instance: json!({
                    "schema_version": "1",
                    "command": "detect",
                    "error": {
                        "code": "submission_outcome_unknown",
                        "category": "network",
                        "message": "The submission outcome is unknown.",
                        "retryable": false,
                        "details": {
                            "analysis_id": "anl_01983c20-0180-7a80-a001-000000000001",
                            "request_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                            "last_status": "sending"
                        },
                        "recovery": {
                            "message": "A manual retry may create a second billable operation.",
                            "command": "pangram detect --text retry"
                        }
                    },
                    "meta": {"failed_at": "2026-07-23T12:00:00Z"}
                }),
                valid: false,
            },
            Case {
                name: "unknown submission recovery accepts the canonical object",
                instance: json!({
                    "schema_version": "1",
                    "command": "detect",
                    "error": {
                        "code": "submission_outcome_unknown",
                        "category": "network",
                        "message": "The submission outcome is unknown.",
                        "retryable": false,
                        "details": {
                            "analysis_id": "anl_01983c20-0180-7a80-a001-000000000001",
                            "request_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                            "last_status": "sending"
                        },
                        "recovery": {
                            "message": "A manual retry may create a second billable operation."
                        }
                    },
                    "meta": {"failed_at": "2026-07-23T12:00:00Z"}
                }),
                valid: true,
            },
            Case {
                name: "not-submitted analyses can be terminal",
                instance: success("detect", terminal_not_submitted_analysis),
                valid: true,
            },
            Case {
                name: "not-submitted analyses reject check task IDs",
                instance: success("detect", not_submitted_with_check_task_id),
                valid: false,
            },
            Case {
                name: "not-submitted analyses reject provenance task IDs",
                instance: success("detect", not_submitted_with_provenance_task_ids),
                valid: false,
            },
            Case {
                name: "not-submitted analyses reject empty provenance task IDs",
                instance: success("detect", not_submitted_with_empty_provenance_task_ids),
                valid: false,
            },
            Case {
                name: "not-submitted analyses reject provenance bulk IDs",
                instance: success("detect", not_submitted_with_provenance_bulk_id),
                valid: false,
            },
            Case {
                name: "not-submitted analyses reject submitted provenance timestamps",
                instance: success("detect", not_submitted_with_submitted_at),
                valid: false,
            },
            Case {
                name: "not-submitted analyses reject completed provenance timestamps",
                instance: success("detect", not_submitted_with_completed_at),
                valid: false,
            },
            Case {
                name: "provenance task IDs are unique",
                instance: success("detect", duplicate_provenance_task_ids),
                valid: false,
            },
            Case {
                name: "non-envelope commands cannot claim JSON data",
                instance: success("completions", json!({"source": "complete -W pangram"})),
                valid: false,
            },
        ],
    );
}
