use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use jsonschema::Draft;
use microck_pangram_cli::contracts::{
    GeneratedArtifact, generated_artifacts, write_generated_artifacts,
};
use microck_pangram_cli::domain::{AnalysisStatus, CheckStatus, derive_parent_status};
use serde_json::{Value, json};

const EXPECTED_ARTIFACTS: &[&str] = &[
    "contracts/config.schema.json",
    "contracts/install-receipt.schema.json",
    "contracts/manifest-signature.schema.json",
    "contracts/output.schema.json",
    "contracts/tui-state.schema.json",
    "contracts/update-manifest.schema.json",
    "contracts/update-state.schema.json",
    "generated/cli-help.txt",
    "generated/cli-reference.json",
    "generated/error-reference.json",
    "generated/mcp-tools.json",
    "generated/agent-reference.md",
];

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn collect_files(root: &Path, directory: &Path, files: &mut BTreeSet<String>) {
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            collect_files(root, &entry.path(), files);
        } else {
            assert!(
                file_type.is_file(),
                "{} is not a regular file",
                entry.path().display()
            );
            let relative = entry.path().strip_prefix(root).unwrap().to_path_buf();
            files.insert(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn generated_artifact_inventory_is_complete_and_unique() {
    let artifacts = generated_artifacts().unwrap();
    let actual: BTreeSet<_> = artifacts
        .iter()
        .map(|artifact| artifact.path.as_str())
        .collect();
    let expected: BTreeSet<_> = EXPECTED_ARTIFACTS.iter().copied().collect();

    assert_eq!(actual, expected);
}

#[test]
fn owned_directories_contain_exactly_the_generated_inventory() {
    let root = repository_root();
    let mut actual = BTreeSet::new();
    collect_files(&root, &root.join("contracts"), &mut actual);
    collect_files(&root, &root.join("generated"), &mut actual);
    let expected = generated_artifacts()
        .unwrap()
        .into_iter()
        .map(|artifact| artifact.path)
        .collect();

    assert_eq!(actual, expected);
}

#[test]
fn staging_failure_does_not_replace_earlier_artifacts() {
    let root = tempfile::tempdir().unwrap();
    let artifacts = generated_artifacts().unwrap();
    for artifact in artifacts
        .iter()
        .filter(|artifact| artifact.path.starts_with("contracts/"))
    {
        let path = root.path().join(&artifact.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"previous contract").unwrap();
    }
    fs::write(root.path().join("generated"), b"blocks directory creation").unwrap();

    assert!(write_generated_artifacts(root.path()).is_err());
    for artifact in artifacts
        .iter()
        .filter(|artifact| artifact.path.starts_with("contracts/"))
    {
        assert_eq!(
            fs::read(root.path().join(&artifact.path)).unwrap(),
            b"previous contract"
        );
    }
}

#[test]
fn committed_contracts_match_rust_owned_generation() {
    let root = repository_root();

    for GeneratedArtifact { path, bytes } in generated_artifacts().unwrap() {
        let committed = fs::read(root.join(&path))
            .unwrap_or_else(|error| panic!("failed to read {path}: {error}"));
        assert_eq!(
            committed, bytes,
            "{path} differs from its Rust-owned generator"
        );
    }
}

#[test]
fn every_json_artifact_declares_generated_rust_ownership() {
    for GeneratedArtifact { path, bytes } in generated_artifacts().unwrap() {
        if Path::new(&path)
            .extension()
            .and_then(|value| value.to_str())
            != Some("json")
        {
            continue;
        }

        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value
                .get("x-contract-owner")
                .and_then(|owner| owner.as_str()),
            Some("rust:microck_pangram_cli::contracts")
        );
    }
}

fn output_schema_value() -> Value {
    let GeneratedArtifact { bytes, .. } = generated_artifacts()
        .unwrap()
        .into_iter()
        .find(|artifact| artifact.path == "contracts/output.schema.json")
        .expect("the output schema is a generated artifact");
    serde_json::from_slice(&bytes).unwrap()
}

fn generated_json(path: &str) -> Value {
    let GeneratedArtifact { bytes, .. } = generated_artifacts()
        .unwrap()
        .into_iter()
        .find(|artifact| artifact.path == path)
        .unwrap_or_else(|| panic!("{path} is a generated artifact"));
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn mcp_tool_inventory_is_ordered_closed_and_phase_scoped() {
    let inventory = generated_json("generated/mcp-tools.json");
    let tools = inventory["tools"]
        .as_array()
        .expect("mcp-tools.json has a tools array");
    let names: Vec<_> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();

    assert_eq!(
        names,
        [
            "detect_text",
            "get_task",
            "wait_task",
            "submit_bulk",
            "get_bulk",
            "wait_bulk",
            "get_bulk_results",
            "history_list",
            "history_search",
            "history_get",
            "history_rerun",
            "history_delete",
            "history_clear",
            "update_config",
        ]
    );
    assert!(names.iter().all(|name| !matches!(
        *name,
        "detect_files" | "check_plagiarism" | "analyze_text" | "check_update"
    )));
    assert!(names.iter().all(|name| !name.starts_with("test_")));

    for tool in tools {
        assert_eq!(tool["inputSchema"]["additionalProperties"], json!(false));
        jsonschema::options()
            .with_draft(Draft::Draft202012)
            .build(&tool["inputSchema"])
            .unwrap_or_else(|error| panic!("{} input schema: {error}", tool["name"]));
        jsonschema::options()
            .with_draft(Draft::Draft202012)
            .build(&tool["outputSchema"])
            .unwrap_or_else(|error| panic!("{} output schema: {error}", tool["name"]));
    }
}

#[test]
fn shipping_mcp_dependency_excludes_http_transport() {
    let manifest = fs::read_to_string(repository_root().join("Cargo.toml")).unwrap();
    let manifest: toml::Value = toml::from_str(&manifest).unwrap();
    let rmcp = manifest["dependencies"]["rmcp"]
        .as_table()
        .expect("rmcp uses an explicit dependency table");
    let features = rmcp["features"]
        .as_array()
        .expect("rmcp features are explicit")
        .iter()
        .map(|feature| feature.as_str().expect("feature name"))
        .collect::<Vec<_>>();

    assert_eq!(rmcp["default-features"].as_bool(), Some(false));
    assert_eq!(features, ["server", "transport-io"]);
    assert!(features.iter().all(|feature| !feature.contains("http")));
}

#[test]
fn mcp_identifier_and_bulk_sources_are_exactly_one() {
    let inventory = generated_json("generated/mcp-tools.json");
    let tools = inventory["tools"].as_array().unwrap();
    let validator = |name: &str| {
        let schema = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing {name}"))["inputSchema"]
            .clone();
        jsonschema::options()
            .with_draft(Draft::Draft202012)
            .build(&schema)
            .unwrap()
    };

    let task = validator("get_task");
    assert!(task.is_valid(&json!({"analysis_id": "anl_01983c20-0180-7a80-a001-000000000001"})));
    assert!(task.is_valid(&json!({"upstream_task_id": "task-123"})));
    assert!(!task.is_valid(&json!({})));
    assert!(!task.is_valid(&json!({
        "analysis_id": "anl_01983c20-0180-7a80-a001-000000000001",
        "upstream_task_id": "task-123"
    })));
    assert!(!task.is_valid(&json!({"upstream_task_id": "task-123", "unknown": true})));

    let bulk = validator("submit_bulk");
    let item = json!({"text": "Synthetic text."});
    assert!(bulk.is_valid(&json!({"items": [item], "max_billable_units": 1})));
    assert!(
        bulk.is_valid(&json!({"jsonl_path": "/approved/input.jsonl", "max_billable_units": 1}))
    );
    assert!(!bulk.is_valid(&json!({"max_billable_units": 1})));
    assert!(!bulk.is_valid(&json!({
        "items": [{"text": "Synthetic text."}],
        "jsonl_path": "/approved/input.jsonl",
        "max_billable_units": 1
    })));
    assert!(!bulk.is_valid(&json!({
        "items": [{"text": "Synthetic text.", "unknown": true}],
        "max_billable_units": 1
    })));
}

#[test]
fn mcp_output_schemas_specialize_command_and_data_root() {
    let inventory = generated_json("generated/mcp-tools.json");
    let tools = inventory["tools"].as_array().unwrap();
    let expected = [
        ("detect_text", "detect"),
        ("get_task", "task_status"),
        ("wait_task", "task_wait"),
        ("submit_bulk", "bulk_submit"),
        ("get_bulk", "bulk_status"),
        ("wait_bulk", "bulk_wait"),
        ("get_bulk_results", "bulk_results"),
        ("history_list", "history_list"),
        ("history_search", "history_search"),
        ("history_get", "history_show"),
        ("history_rerun", "history_rerun"),
        ("history_delete", "history_delete"),
        ("history_clear", "history_clear"),
        ("update_config", "config_set"),
    ];

    for (tool_name, command) in expected {
        let output = &tools
            .iter()
            .find(|tool| tool["name"] == tool_name)
            .unwrap_or_else(|| panic!("missing {tool_name}"))["outputSchema"];
        assert_eq!(
            output["$defs"]["successEnvelope"]["properties"]["command"],
            json!({"const": command})
        );
        assert_eq!(
            output["$defs"]["errorEnvelope"]["properties"]["command"],
            json!({"const": command})
        );
        let data = &output["$defs"]["successEnvelope"]["properties"]["data"];
        assert_ne!(
            data,
            &json!({"oneOf": [{"type": "object"}, {"type": "array"}]})
        );
        assert!(data.get("$ref").is_some() || data.get("oneOf").is_some());
    }
}

fn check_status_name(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Queued => "queued",
        CheckStatus::Running => "running",
        CheckStatus::Succeeded => "succeeded",
        CheckStatus::Failed => "failed",
    }
}

fn analysis_status_name(status: AnalysisStatus) -> &'static str {
    match status {
        AnalysisStatus::Queued => "queued",
        AnalysisStatus::Running => "running",
        AnalysisStatus::Succeeded => "succeeded",
        AnalysisStatus::Failed => "failed",
        AnalysisStatus::Partial => "partial",
    }
}

#[test]
fn output_schema_history_summaries_enforce_canonical_check_order() {
    let schema = output_schema_value();
    let validator = jsonschema::options()
        .with_draft(Draft::Draft202012)
        .build(&schema)
        .expect("generated output schema compiles");
    let envelope = |command: &str, checks: Value| {
        json!({
            "schema_version": "1",
            "command": command,
            "data": {
                "items": [{
                    "id": "anl_01983c20-0180-7a80-a001-000000000001",
                    "status": "succeeded",
                    "checks": checks,
                    "save_state": "saved_history",
                    "input_kind": "text",
                    "created_at": "2026-07-23T12:00:00Z"
                }]
            },
            "meta": {"started_at": "2026-07-23T12:00:00Z"}
        })
    };

    assert!(validator.is_valid(&envelope(
        "history_list",
        json!(["ai_detection", "plagiarism"]),
    )));
    assert!(!validator.is_valid(&envelope(
        "history_list",
        json!(["plagiarism", "ai_detection"]),
    )));
    assert!(!validator.is_valid(&envelope(
        "history_search",
        json!(["ai_detection", "ai_detection"]),
    )));
}

// The generated output schema's parent-status constraint must accept exactly
// the declared status that `derive_parent_status` computes for every non-empty
// set of one or two check statuses, and reject every other status (PR #14
// review d). This exercises the real emitted schema, not a rebuilt copy, so it
// pins the order-independent set-based encoding against the canonical
// derivation.
// Extracts the parent-status derivation clause (the five `if status ... then
// checks ...` implications) from the generated `Analysis` definition so the
// invariant can be validated against minimal status/check objects without
// tripping the unrelated required-field and check-state constraints of a full
// analysis envelope.
fn parent_status_clause(schema: &Value) -> Value {
    let analysis = &schema["$defs"]["Analysis"];
    analysis["allOf"]
        .as_array()
        .expect("analysis has an allOf")
        .iter()
        .find(|clause| {
            clause
                .get("allOf")
                .and_then(Value::as_array)
                .is_some_and(|cases| {
                    cases.len() == 5
                        && cases
                            .iter()
                            .all(|case| case.get("if").is_some() && case.get("then").is_some())
                })
        })
        .expect("analysis contains the parent-status allOf clause")
        .clone()
}

#[test]
fn output_schema_parent_status_matches_the_canonical_derivation() {
    let clause = parent_status_clause(&output_schema_value());
    // Wrap the extracted clause in a minimal object schema carrying only the
    // fields the invariant reads, so the canonical derivation is checked
    // directly and independently of the full envelope shape.
    let isolated = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "status": {"enum": ["queued", "running", "succeeded", "failed", "partial"]},
            "checks": {
                "type": "array",
                "minItems": 1,
                "maxItems": 2,
                "items": {
                    "type": "object",
                    "properties": {
                        "status": {"enum": ["queued", "running", "succeeded", "failed"]}
                    },
                    "required": ["status"]
                }
            }
        },
        "required": ["status", "checks"],
        "allOf": [clause]
    });
    let validator = jsonschema::options()
        .with_draft(Draft::Draft202012)
        .build(&isolated)
        .expect("parent-status clause is a valid Draft 2020-12 schema");

    let statuses = [
        CheckStatus::Queued,
        CheckStatus::Running,
        CheckStatus::Succeeded,
        CheckStatus::Failed,
    ];
    let parents = [
        AnalysisStatus::Queued,
        AnalysisStatus::Running,
        AnalysisStatus::Succeeded,
        AnalysisStatus::Failed,
        AnalysisStatus::Partial,
    ];

    let mut combos: Vec<Vec<CheckStatus>> = statuses.iter().map(|status| vec![*status]).collect();
    for first in statuses {
        for second in statuses {
            combos.push(vec![first, second]);
        }
    }

    for combo in combos {
        let expected = derive_parent_status(&combo).unwrap();
        for parent in parents {
            let analysis: Value = json!({
                "status": analysis_status_name(parent),
                "checks": combo
                    .iter()
                    .map(|status| json!({"status": check_status_name(*status)}))
                    .collect::<Vec<_>>()
            });
            assert_eq!(
                validator.is_valid(&analysis),
                parent == expected,
                "checks {:?} derive {:?}; schema must {} status {:?}",
                combo,
                expected,
                if parent == expected {
                    "accept"
                } else {
                    "reject"
                },
                parent
            );
        }
    }
}

// The generated output schema must use only standard Draft 2020-12 keywords;
// any invented cross-field arithmetic keyword would be silently ignored by
// validators and must never appear (PR #14 review d). Cross-field arithmetic
// bounds remain constructor-owned (see contracts.md section 9).
/// The generated CLI grammar reference must keep `bulk submit` free of any
/// `--public-link` argument: contracts.md 14.3 and docs/mcp-contract.md lock
/// bulk submission against Pangram's Bulk API, which documents no
/// public-dashboard-link request or response field. The assertion runs against
/// the real generator output (which the drift test proves byte-identical to
/// the committed artifact), so a grammar regression cannot be hidden by a
/// stale committed file.
#[test]
fn generated_cli_reference_keeps_bulk_submit_free_of_public_link() {
    let GeneratedArtifact { bytes, .. } = generated_artifacts()
        .unwrap()
        .into_iter()
        .find(|artifact| artifact.path == "generated/cli-reference.json")
        .expect("the CLI reference is a generated artifact");
    let reference: Value = serde_json::from_slice(&bytes).unwrap();

    let commands = reference["commands"]
        .as_array()
        .expect("cli-reference.json has a commands array");
    let bulk_submit = commands
        .iter()
        .find(|command| command["path"] == json!(["bulk", "submit"]))
        .expect("cli-reference.json contains the bulk submit command");

    let arguments = bulk_submit["arguments"]
        .as_array()
        .expect("bulk submit has an arguments array");
    assert!(
        !arguments
            .iter()
            .any(|argument| argument["name"] == json!("--public-link")),
        "generated cli-reference.json must not list --public-link for bulk submit"
    );

    // The corrected grammar matches contracts.md 14.3 exactly: a JSONL path
    // or implicit stdin, the required ceiling, one output format, and
    // progress control. No flag beyond that contract may reseed itself.
    let names: Vec<&str> = arguments
        .iter()
        .map(|argument| argument["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "JSONL_PATH",
            "--max-billable-units",
            "--dry-run",
            "--wait",
            "--format",
            "--progress"
        ]
    );

    // The contracted detect `--public-link` stays available; the correction
    // removes only the bulk seed entry.
    let detect = commands
        .iter()
        .find(|command| command["path"] == json!(["detect"]))
        .expect("cli-reference.json contains the detect command");
    let detect_public_link = detect["arguments"]
        .as_array()
        .expect("detect has an arguments array")
        .iter()
        .find(|argument| argument["name"] == json!("--public-link"))
        .expect("detect keeps its contracted --public-link flag");
    assert_eq!(detect_public_link["availability"], json!("available"));

    // Bulk submit itself is available at the Phase 3 activation packet.
    assert_eq!(bulk_submit["availability"], json!("available"));
}

// The generated help fixture lists the implemented (available) top-level
// commands; the Phase 3 bulk and task namespaces surface in it.
#[test]
fn generated_cli_help_lists_the_activated_bulk_and_task_surface() {
    let GeneratedArtifact { bytes, .. } = generated_artifacts()
        .unwrap()
        .into_iter()
        .find(|artifact| artifact.path == "generated/cli-help.txt")
        .expect("the CLI help fixture is a generated artifact");
    let help = String::from_utf8(bytes).unwrap();
    let listed: Vec<&str> = help
        .lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();

    for name in ["bulk", "task"] {
        assert!(
            listed.contains(&name),
            "generated cli-help.txt must list the available {name} command"
        );
    }
}

#[test]
fn output_schema_declares_no_nonstandard_keywords() {
    let schema = output_schema_value();
    let text = serde_json::to_string(&schema).unwrap();
    for keyword in ["maximum_field", "minimum_field"] {
        assert!(
            !text.contains(keyword),
            "non-standard keyword {keyword} must not appear in the generated output schema"
        );
    }
}
