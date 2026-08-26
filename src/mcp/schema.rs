//! Pangram-owned MCP tool descriptors and closed input schemas.
//!
//! This module contains no RMCP types. The runtime adapter converts these
//! descriptors at its seam, while contract generation serializes the same
//! values into the committed inventory.

use serde_json::{Value, json};

use crate::config::ConfigKey;
use crate::domain::{
    ANALYSIS_ID_PATTERN, BULK_BILLABLE_UNIT_LIMIT, BULK_ID_PATTERN, BULK_PAGE_LIMIT_MAX,
};
use crate::output::ResolvedCommand;

/// Closed MCP tool identity set in deterministic discovery order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolName {
    DetectText,
    CheckPlagiarism,
    AnalyzeText,
    GetTask,
    WaitTask,
    SubmitBulk,
    GetBulk,
    WaitBulk,
    GetBulkResults,
    CheckUpdate,
    HistoryList,
    HistorySearch,
    HistoryGet,
    HistoryRerun,
    HistoryDelete,
    HistoryClear,
    UpdateConfig,
}

impl ToolName {
    pub(crate) const ALL: [Self; 17] = [
        Self::DetectText,
        Self::CheckPlagiarism,
        Self::AnalyzeText,
        Self::GetTask,
        Self::WaitTask,
        Self::SubmitBulk,
        Self::GetBulk,
        Self::WaitBulk,
        Self::GetBulkResults,
        Self::CheckUpdate,
        Self::HistoryList,
        Self::HistorySearch,
        Self::HistoryGet,
        Self::HistoryRerun,
        Self::HistoryDelete,
        Self::HistoryClear,
        Self::UpdateConfig,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::DetectText => "detect_text",
            Self::CheckPlagiarism => "check_plagiarism",
            Self::AnalyzeText => "analyze_text",
            Self::GetTask => "get_task",
            Self::WaitTask => "wait_task",
            Self::SubmitBulk => "submit_bulk",
            Self::GetBulk => "get_bulk",
            Self::WaitBulk => "wait_bulk",
            Self::GetBulkResults => "get_bulk_results",
            Self::CheckUpdate => "check_update",
            Self::HistoryList => "history_list",
            Self::HistorySearch => "history_search",
            Self::HistoryGet => "history_get",
            Self::HistoryRerun => "history_rerun",
            Self::HistoryDelete => "history_delete",
            Self::HistoryClear => "history_clear",
            Self::UpdateConfig => "update_config",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|name| name.as_str() == value)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ToolSpec {
    pub name: ToolName,
    pub description: &'static str,
    pub input_schema: Value,
    pub output_schema: Value,
    pub annotations: Value,
}

pub(crate) fn tool_specs<F>(output_schema: F) -> Vec<ToolSpec>
where
    F: Fn(ResolvedCommand) -> Value,
{
    tool_specs_for(ToolName::ALL, output_schema)
}

pub(crate) fn tool_specs_for<F>(
    names: impl IntoIterator<Item = ToolName>,
    output_schema: F,
) -> Vec<ToolSpec>
where
    F: Fn(ResolvedCommand) -> Value,
{
    names
        .into_iter()
        .map(|identity| match identity {
            ToolName::DetectText => tool(
                identity,
                "Detect AI-written text with Pangram 4.",
                ResolvedCommand::Detect,
                detect_text_input(),
                annotations(false, false, false, true),
                &output_schema,
            ),
            ToolName::CheckPlagiarism => tool(
                identity,
                "Check text for plagiarism against online sources.",
                ResolvedCommand::Plagiarism,
                phase7_text_input(false),
                annotations(false, false, false, true),
                &output_schema,
            ),
            ToolName::AnalyzeText => tool(
                identity,
                "Run Pangram 4 AI detection and plagiarism checks on the same text.",
                ResolvedCommand::Analyze,
                detect_text_input(),
                annotations(false, false, false, true),
                &output_schema,
            ),
            ToolName::GetTask => tool(
                identity,
                "Get one Pangram task without waiting.",
                ResolvedCommand::TaskStatus,
                task_input(false),
                annotations(true, false, true, true),
                &output_schema,
            ),
            ToolName::WaitTask => tool(
                identity,
                "Wait for one Pangram task to reach a terminal state.",
                ResolvedCommand::TaskWait,
                task_input(true),
                annotations(true, false, true, true),
                &output_schema,
            ),
            ToolName::SubmitBulk => tool(
                identity,
                "Submit inline items or one approved JSONL file to Pangram 4.",
                ResolvedCommand::BulkSubmit,
                submit_bulk_input(),
                annotations(false, false, false, true),
                &output_schema,
            ),
            ToolName::GetBulk => tool(
                identity,
                "Get one Pangram bulk job without waiting.",
                ResolvedCommand::BulkStatus,
                bulk_input(false, false),
                annotations(true, false, true, true),
                &output_schema,
            ),
            ToolName::WaitBulk => tool(
                identity,
                "Wait for one Pangram bulk job to reach a terminal state.",
                ResolvedCommand::BulkWait,
                bulk_input(true, false),
                annotations(true, false, true, true),
                &output_schema,
            ),
            ToolName::GetBulkResults => tool(
                identity,
                "Get one explicit results page for a Pangram bulk job.",
                ResolvedCommand::BulkResults,
                bulk_input(false, true),
                annotations(true, false, true, true),
                &output_schema,
            ),
            ToolName::CheckUpdate => tool(
                identity,
                "Check for a Pangram CLI update without installing it.",
                ResolvedCommand::UpdateCheck,
                closed_object(json!({}), &[], Vec::new()),
                annotations(true, false, true, true),
                &output_schema,
            ),
            ToolName::HistoryList => tool(
                identity,
                "List saved local analysis summaries.",
                ResolvedCommand::HistoryList,
                history_query_input(false),
                annotations(true, false, true, false),
                &output_schema,
            ),
            ToolName::HistorySearch => tool(
                identity,
                "Search saved local analysis summaries with literal text.",
                ResolvedCommand::HistorySearch,
                history_query_input(true),
                annotations(true, false, true, false),
                &output_schema,
            ),
            ToolName::HistoryGet => tool(
                identity,
                "Get one saved local analysis, redacted by default.",
                ResolvedCommand::HistoryShow,
                history_id_input(true),
                annotations(true, false, true, false),
                &output_schema,
            ),
            ToolName::HistoryRerun => tool(
                identity,
                "Submit the saved input from one local analysis again.",
                ResolvedCommand::HistoryRerun,
                history_rerun_input(),
                annotations(false, false, false, true),
                &output_schema,
            ),
            ToolName::HistoryDelete => tool(
                identity,
                "Delete one saved local analysis.",
                ResolvedCommand::HistoryDelete,
                history_id_input(false),
                annotations(false, true, false, false),
                &output_schema,
            ),
            ToolName::HistoryClear => tool(
                identity,
                "Delete all saved local analyses.",
                ResolvedCommand::HistoryClear,
                closed_object(json!({}), &[], Vec::new()),
                annotations(false, true, false, false),
                &output_schema,
            ),
            ToolName::UpdateConfig => tool(
                identity,
                "Set one supported non-secret Pangram CLI configuration key.",
                ResolvedCommand::ConfigSet,
                update_config_input(),
                annotations(false, false, false, false),
                &output_schema,
            ),
        })
        .collect()
}

fn tool(
    name: ToolName,
    description: &'static str,
    command: ResolvedCommand,
    input_schema: Value,
    annotations: Value,
    output_schema: &impl Fn(ResolvedCommand) -> Value,
) -> ToolSpec {
    ToolSpec {
        name,
        description,
        input_schema,
        output_schema: output_schema(command),
        annotations,
    }
}

fn annotations(read_only: bool, destructive: bool, idempotent: bool, open_world: bool) -> Value {
    json!({
        "readOnlyHint": read_only,
        "destructiveHint": destructive,
        "idempotentHint": idempotent,
        "openWorldHint": open_world,
    })
}

fn detect_text_input() -> Value {
    closed_object(
        json!({
            "text": {"type": "string", "minLength": 1},
            "max_billable_units": positive_integer(),
            "save": {"type": "boolean", "default": false},
            "public_link": {"type": "boolean", "default": false},
            "include_input": {"type": "boolean", "default": false},
        }),
        &["text", "max_billable_units"],
        Vec::new(),
    )
}

fn phase7_text_input(public_link: bool) -> Value {
    let mut properties = json!({
        "text": {"type": "string", "minLength": 1},
        "max_billable_units": positive_integer(),
        "save": {"type": "boolean", "default": false},
        "include_input": {"type": "boolean", "default": false},
    });
    if public_link {
        properties["public_link"] = json!({"type": "boolean", "default": false});
    }
    closed_object(properties, &["text", "max_billable_units"], Vec::new())
}

fn task_input(with_timeout: bool) -> Value {
    let mut properties = json!({
        "analysis_id": local_id(ANALYSIS_ID_PATTERN),
        "upstream_task_id": opaque_id(),
    });
    if with_timeout {
        properties["timeout_ms"] = positive_integer();
    }
    closed_object(
        properties,
        &[],
        vec![exactly_one("analysis_id", "upstream_task_id")],
    )
}

fn submit_bulk_input() -> Value {
    closed_object(
        json!({
            "items": {
                "type": "array",
                "minItems": 1,
                "maxItems": 1000,
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "minLength": 1},
                        "text": {"type": "string", "minLength": 1}
                    },
                    "required": ["text"],
                    "additionalProperties": false
                }
            },
            "jsonl_path": {"type": "string", "minLength": 1},
            "max_billable_units": {
                "type": "integer",
                "minimum": 1,
                "maximum": BULK_BILLABLE_UNIT_LIMIT
            },
            "save": {"type": "boolean", "default": false},
        }),
        &["max_billable_units"],
        vec![exactly_one("items", "jsonl_path")],
    )
}

fn bulk_input(with_timeout: bool, with_page: bool) -> Value {
    let mut properties = json!({
        "bulk_id": local_id(BULK_ID_PATTERN),
        "upstream_bulk_id": opaque_id(),
    });
    let mut required = Vec::new();
    if with_timeout {
        properties["timeout_ms"] = positive_integer();
    }
    if with_page {
        properties["offset"] = json!({"type": "integer", "minimum": 0});
        properties["limit"] = json!({
            "type": "integer",
            "minimum": 1,
            "maximum": BULK_PAGE_LIMIT_MAX
        });
        required.extend(["offset", "limit"]);
    }
    closed_object(
        properties,
        &required,
        vec![exactly_one("bulk_id", "upstream_bulk_id")],
    )
}

fn history_query_input(with_query: bool) -> Value {
    let properties = json!({
        "query": {"type": "string", "minLength": 1},
        "status": {"enum": ["queued", "running", "succeeded", "failed", "partial"]},
        "check": {"enum": ["ai_detection", "plagiarism"]},
        "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 50},
    });
    let required = if with_query { &["query"][..] } else { &[][..] };
    closed_object(properties, required, Vec::new())
}

fn history_id_input(with_content: bool) -> Value {
    let mut properties = json!({"analysis_id": local_id(ANALYSIS_ID_PATTERN)});
    if with_content {
        properties["include_content"] = json!({"type": "boolean", "default": false});
    }
    closed_object(properties, &["analysis_id"], Vec::new())
}

fn history_rerun_input() -> Value {
    closed_object(
        json!({
            "analysis_id": local_id(ANALYSIS_ID_PATTERN),
            "max_billable_units": positive_integer(),
        }),
        &["analysis_id", "max_billable_units"],
        Vec::new(),
    )
}

fn update_config_input() -> Value {
    let keys = ConfigKey::ALL
        .iter()
        .copied()
        .map(ConfigKey::as_str)
        .collect::<Vec<_>>();
    closed_object(
        json!({
            "key": {"enum": keys},
            "value": {"type": "string", "minLength": 1}
        }),
        &["key", "value"],
        Vec::new(),
    )
}

fn closed_object(properties: Value, required: &[&str], constraints: Vec<Value>) -> Value {
    let mut schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    });
    if !constraints.is_empty() {
        schema["allOf"] = Value::Array(constraints);
    }
    schema
}

fn exactly_one(first: &str, second: &str) -> Value {
    json!({
        "oneOf": [
            {"required": [first], "not": {"required": [second]}},
            {"required": [second], "not": {"required": [first]}}
        ]
    })
}

fn positive_integer() -> Value {
    json!({"type": "integer", "minimum": 1})
}

fn opaque_id() -> Value {
    json!({"type": "string", "minLength": 1})
}

fn local_id(pattern: &str) -> Value {
    json!({"type": "string", "pattern": pattern})
}
