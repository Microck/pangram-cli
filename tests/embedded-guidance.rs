#[path = "../src/mcp/embedded.rs"]
mod embedded;

use embedded::{
    AGENT_REFERENCE, ERROR_REFERENCE, MCP_RESOURCES, OUTPUT_SCHEMA, PANGRAM_SKILL,
    PANGRAM_SKILL_PATH, SKILL_LIST, SKILL_ROOT_PATH, resource,
};

#[test]
fn embedded_resources_are_the_contracted_build_time_bytes() {
    let actual: Vec<_> = MCP_RESOURCES
        .iter()
        .map(|resource| (resource.uri, resource.mime_type))
        .collect();

    assert_eq!(
        actual,
        [
            ("pangram://schema/output/v1", "application/schema+json"),
            ("pangram://schema/errors/v1", "application/json"),
            ("pangram://skills/pangram", "text/markdown"),
        ]
    );
    assert_eq!(
        OUTPUT_SCHEMA.bytes,
        include_bytes!("../contracts/output.schema.json")
    );
    assert_eq!(
        ERROR_REFERENCE.bytes,
        include_bytes!("../generated/error-reference.json")
    );
    assert_eq!(
        PANGRAM_SKILL.bytes,
        include_bytes!("../skills/pangram/SKILL.md")
    );
    for (resource, asset) in
        MCP_RESOURCES
            .iter()
            .zip([OUTPUT_SCHEMA, ERROR_REFERENCE, PANGRAM_SKILL])
    {
        assert_eq!(resource.mime_type, asset.mime_type);
        assert_eq!(resource.bytes, asset.bytes);
    }
}

#[test]
fn embedded_resource_lookup_is_closed_to_the_published_inventory() {
    for expected in MCP_RESOURCES {
        assert_eq!(resource(expected.uri), Some(expected));
    }

    assert_eq!(resource("pangram://schema/task/v1"), None);
    assert_eq!(resource("pangram://history"), None);
}

#[test]
fn agent_reference_uses_the_exact_generated_bytes() {
    assert_eq!(AGENT_REFERENCE.mime_type, "text/markdown");
    assert_eq!(
        AGENT_REFERENCE.bytes,
        include_bytes!("../generated/agent-reference.md")
    );
}

#[test]
fn embedded_skill_inventory_and_locators_are_exact_bytes() {
    assert_eq!(SKILL_LIST, b"# Embedded skills\n\n- `pangram`\n");
    assert_eq!(SKILL_ROOT_PATH, b"embedded://skills\n");
    assert_eq!(PANGRAM_SKILL_PATH, b"embedded://skills/pangram/SKILL.md\n");
}
