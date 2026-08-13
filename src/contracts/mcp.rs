//! Generated MCP tool inventory and compact embedded agent reference.

use serde_json::{Value, json};

use super::{CONTRACT_OWNER, DRAFT_2020_12};
use crate::mcp::schema::{self, ToolSpec};

pub(super) fn artifacts(output_schema: &Value) -> (Value, String) {
    let specs = schema::tool_specs(|command| {
        super::output::specialized_output_schema(output_schema, command.as_str())
    });
    let reference = agent_reference(&specs);
    let tools = specs
        .into_iter()
        .map(|tool| {
            json!({
                "name": tool.name.as_str(),
                "description": tool.description,
                "inputSchema": tool.input_schema,
                "outputSchema": tool.output_schema,
                "annotations": tool.annotations,
            })
        })
        .collect::<Vec<_>>();

    let inventory = json!({
        "$schema": DRAFT_2020_12,
        "$id": "https://pangram.micr.dev/generated/mcp-tools-v1.json",
        "x-contract-owner": CONTRACT_OWNER,
        "title": "Pangram CLI MCP tool inventory v1",
        "protocol_version": "2026-07-28",
        "tools": tools,
    });
    (inventory, reference)
}

fn agent_reference(specs: &[ToolSpec]) -> String {
    let mut reference = String::from(
        "# Pangram MCP agent reference\n\n\
         Protocol: MCP 2026-07-28 over stdio.\n\n\
         ## Tools\n\n",
    );
    for tool in specs {
        reference.push_str("- `");
        reference.push_str(tool.name.as_str());
        reference.push_str("`: ");
        reference.push_str(tool.description);
        reference.push('\n');
    }
    reference.push_str(
        "\n## Safety\n\n\
         Every billable submission requires `max_billable_units`. Local IDs require history; upstream IDs do not. History reads, history mutations, configuration mutations, public links, and file roots require their explicit startup capabilities. `save: true` requires history and history mutations. File paths must be inside an approved root. Cancellation stops local observation only.\n",
    );
    reference
}
