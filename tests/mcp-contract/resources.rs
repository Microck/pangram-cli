use std::fs;

use serde_json::json;

use crate::mcp_stdio::{McpProcess, result};

const RESOURCES: &[(&str, &str, &str)] = &[
    (
        "pangram://schema/output/v1",
        "contracts/output.schema.json",
        "application/schema+json",
    ),
    (
        "pangram://schema/errors/v1",
        "generated/error-reference.json",
        "application/json",
    ),
    (
        "pangram://skills/pangram",
        "skills/pangram/SKILL.md",
        "text/markdown",
    ),
];

#[test]
fn resource_inventory_and_reads_are_private_noncacheable_exact_bytes() {
    let mut server = McpProcess::spawn(&[]);
    result(&server.discover());
    let response = server.request("resources/list", json!({}), true);
    let listed = result(&response);

    assert_eq!(listed["resultType"], "complete");
    assert_eq!(listed["ttlMs"], 0);
    assert_eq!(listed["cacheScope"], "private");
    let resources = listed["resources"].as_array().unwrap();
    assert_eq!(resources.len(), RESOURCES.len());

    for (index, (uri, path, mime_type)) in RESOURCES.iter().enumerate() {
        assert_eq!(resources[index]["uri"], *uri);
        assert_eq!(resources[index]["mimeType"], *mime_type);
        assert_eq!(resources[index]["size"], fs::metadata(path).unwrap().len());

        let response = server.request("resources/read", json!({"uri": uri}), true);
        let read = result(&response);
        assert_eq!(read["resultType"], "complete");
        assert_eq!(read["ttlMs"], 0);
        assert_eq!(read["cacheScope"], "private");
        assert_eq!(read["contents"][0]["uri"], *uri);
        assert_eq!(read["contents"][0]["mimeType"], *mime_type);
        assert_eq!(
            read["contents"][0]["text"],
            fs::read_to_string(path).unwrap()
        );
    }
    assert_eq!(server.shutdown(), "");
}

#[test]
fn unknown_resources_are_protocol_errors_and_no_templates_or_prompts_exist() {
    let mut server = McpProcess::spawn(&[]);
    result(&server.discover());

    let missing = server.request(
        "resources/read",
        json!({"uri": "pangram://history/secret"}),
        true,
    );
    assert_eq!(missing["error"]["code"], -32602);

    let templates = server.request("resources/templates/list", json!({}), true);
    assert_eq!(templates["error"]["code"], -32601);
    let prompts = server.request("prompts/list", json!({}), true);
    assert_eq!(prompts["error"]["code"], -32601);
    assert_eq!(server.shutdown(), "");
}
