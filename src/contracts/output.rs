//! Canonical JSON output schema generation.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::{
    CONTRACT_OWNER, DRAFT_2020_12, data_condition, object_mut, patch_output_definitions,
    rust_schema, schema_ref, schema_types, strip_optional_nulls,
};
use crate::domain::{Analysis, AnalysisSummaryPage, BulkCollection, BulkPage};
use crate::output::{CanonicalError, EnvelopeMeta, OutputSchemaVersion, ResolvedCommand};

// The output schema combines Schemars-owned definitions with cross-field JSON
// Schema constraints which Rust's type system cannot express by derivation.
pub(super) fn output_schema() -> Value {
    let mut registry = rust_schema::<schema_types::OutputRegistry>();
    strip_optional_nulls(&mut registry);
    let mut definitions = object_mut(&mut registry)
        .remove("$defs")
        .expect("the output registry has definitions");

    patch_output_definitions(&mut definitions);
    let analysis = schema_ref::<Analysis<CanonicalError>>();
    let array = json!({"type": "array", "minItems": 1, "items": analysis});
    let conditions = vec![
        data_condition(
            &["detect", "plagiarism", "analyze"],
            json!({"oneOf": [analysis, array]}),
        ),
        data_condition(
            &["task_status", "task_wait", "history_show", "history_rerun"],
            schema_ref::<Analysis<CanonicalError>>(),
        ),
        data_condition(
            &["bulk_submit"],
            json!({"oneOf": [
                schema_ref::<BulkCollection>(),
                schema_ref::<crate::output::BulkDryRun>(),
            ]}),
        ),
        data_condition(
            &["bulk_status", "bulk_wait"],
            schema_ref::<BulkCollection>(),
        ),
        data_condition(&["bulk_results"], schema_ref::<BulkPage<CanonicalError>>()),
        data_condition(
            &["history_list", "history_search"],
            schema_ref::<AnalysisSummaryPage>(),
        ),
        data_condition(
            &[
                "history_delete",
                "history_clear",
                "auth_set",
                "auth_logout",
                "config_set",
            ],
            schema_ref::<crate::output::MutationAcknowledgement>(),
        ),
        data_condition(
            &["mcp_install", "mcp_uninstall"],
            schema_ref::<crate::output::McpMutationReport>(),
        ),
        data_condition(&["auth_status"], schema_ref::<crate::output::AuthStatus>()),
        data_condition(
            &["config_list"],
            schema_ref::<crate::output::ConfigListStatus>(),
        ),
        data_condition(
            &["config_get"],
            schema_ref::<crate::output::ConfigGetStatus>(),
        ),
        data_condition(
            &["config_path"],
            schema_ref::<crate::output::ConfigPathStatus>(),
        ),
        data_condition(&["doctor"], schema_ref::<crate::output::DoctorStatus>()),
        data_condition(&["mcp_status"], schema_ref::<crate::output::McpStatus>()),
        data_condition(
            &["update_check", "update_install"],
            schema_ref::<crate::output::UpdateStatus>(),
        ),
    ];
    let json_commands: Vec<_> = ResolvedCommand::ALL
        .iter()
        .copied()
        .filter(|command| command.uses_json_envelope())
        .map(ResolvedCommand::as_str)
        .collect();
    let all_commands: Vec<_> = ResolvedCommand::ALL
        .iter()
        .copied()
        .map(ResolvedCommand::as_str)
        .collect();

    let definitions = object_mut(&mut definitions);
    definitions.insert(
        "successEnvelope".into(),
        json!({
            "type": "object",
            "required": ["schema_version", "command", "data", "meta"],
            "properties": {
                "schema_version": {"const": OutputSchemaVersion::V1},
                "command": {"enum": json_commands},
                "data": {"oneOf": [{"type": "object"}, {"type": "array"}]},
                "meta": schema_ref::<EnvelopeMeta>(),
            },
            "allOf": conditions,
            "not": {"required": ["error"]},
            "additionalProperties": true,
        }),
    );
    definitions.insert(
        "errorEnvelope".into(),
        json!({
            "type": "object",
            "required": ["schema_version", "command", "error", "meta"],
            "properties": {
                "schema_version": {"const": OutputSchemaVersion::V1},
                "command": {"enum": all_commands},
                "error": schema_ref::<CanonicalError>(),
                "meta": schema_ref::<EnvelopeMeta>(),
            },
            "not": {"required": ["data"]},
            "additionalProperties": true,
        }),
    );

    json!({
        "$schema": DRAFT_2020_12,
        "$id": "https://pangram.micr.dev/schemas/output-v1.json",
        "x-contract-owner": CONTRACT_OWNER,
        "title": "Pangram CLI output envelope v1",
        "oneOf": [
            {"$ref": "#/$defs/successEnvelope"},
            {"$ref": "#/$defs/errorEnvelope"}
        ],
        "$defs": definitions,
    })
}

/// Narrows the canonical envelope to one resolved command and its data root.
///
/// Deriving the data shape from the canonical output schema keeps MCP output
/// descriptors synchronized without a second command-to-data mapping.
pub(super) fn specialized_output_schema(base: &Value, command: &str) -> Value {
    let mut schema = base.clone();
    let success = &mut schema["$defs"]["successEnvelope"];
    let data = success["allOf"]
        .as_array()
        .expect("the success envelope has command conditions")
        .iter()
        .find_map(|condition| {
            let command_schema = &condition["if"]["properties"]["command"];
            let matches = command_schema.get("const").and_then(Value::as_str) == Some(command)
                || command_schema
                    .get("enum")
                    .and_then(Value::as_array)
                    .is_some_and(|commands| commands.iter().any(|value| value == command));
            matches.then(|| condition["then"]["properties"]["data"].clone())
        })
        .unwrap_or_else(|| panic!("{command} has no canonical output data root"));
    success["properties"]["command"] = json!({"const": command});
    success["properties"]["data"] = data;
    object_mut(success).remove("allOf");
    schema["$defs"]["errorEnvelope"]["properties"]["command"] = json!({"const": command});
    prune_unreachable_definitions(&mut schema);
    schema
}

fn prune_unreachable_definitions(schema: &mut Value) {
    let all_definitions = schema["$defs"]
        .as_object()
        .expect("the output schema has definitions")
        .clone();
    let mut reachable = BTreeSet::new();
    collect_local_refs(schema, true, &mut reachable);

    let mut pending = reachable.iter().cloned().collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let definition = all_definitions
            .get(&name)
            .unwrap_or_else(|| panic!("output schema references missing definition {name}"));
        let mut discovered = BTreeSet::new();
        collect_local_refs(definition, false, &mut discovered);
        for candidate in discovered {
            if reachable.insert(candidate.clone()) {
                pending.push(candidate);
            }
        }
    }

    schema["$defs"] = Value::Object(
        all_definitions
            .into_iter()
            .filter(|(name, _)| reachable.contains(name))
            .collect(),
    );
}

fn collect_local_refs(value: &Value, skip_definitions: bool, references: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str)
                && let Some(name) = reference.strip_prefix("#/$defs/")
            {
                references.insert(name.to_owned());
            }
            for (name, child) in object {
                if !(skip_definitions && name == "$defs") {
                    collect_local_refs(child, false, references);
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_local_refs(child, false, references);
            }
        }
        _ => {}
    }
}
