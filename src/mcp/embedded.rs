//! Build-time-owned guidance and schemas exposed by the MCP adapter.
//!
//! Keeping the bytes here prevents runtime configuration or the current
//! working directory from changing the server's immutable resource inventory.

/// One immutable document compiled into the Pangram binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddedAsset {
    pub mime_type: &'static str,
    pub bytes: &'static [u8],
}

/// One immutable MCP resource and its exact build-time bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddedResource {
    pub uri: &'static str,
    pub mime_type: &'static str,
    pub bytes: &'static [u8],
}

/// Canonical output envelope schema shared by every adapter.
pub const OUTPUT_SCHEMA: EmbeddedAsset = EmbeddedAsset {
    mime_type: "application/schema+json",
    bytes: include_bytes!("../../contracts/output.schema.json"),
};

/// Canonical generated error and exit-code reference.
pub const ERROR_REFERENCE: EmbeddedAsset = EmbeddedAsset {
    mime_type: "application/json",
    bytes: include_bytes!("../../generated/error-reference.json"),
};

/// Full Pangram skill for clients that can load repository skills.
pub const PANGRAM_SKILL: EmbeddedAsset = EmbeddedAsset {
    mime_type: "text/markdown",
    bytes: include_bytes!("../../skills/pangram/SKILL.md"),
};

/// Compact guidance for clients that do not load the full Pangram skill.
pub const AGENT_REFERENCE: EmbeddedAsset = EmbeddedAsset {
    mime_type: "text/markdown",
    bytes: include_bytes!("../../generated/agent-reference.md"),
};

/// Exact Markdown inventory emitted by `pangram skills list`.
pub(crate) const SKILL_LIST: &[u8] = b"# Embedded skills\n\n- `pangram`\n";

/// Stable locator emitted when no individual embedded skill is selected.
pub(crate) const SKILL_ROOT_PATH: &[u8] = b"embedded://skills\n";

/// Stable locator for the full embedded Pangram skill.
pub(crate) const PANGRAM_SKILL_PATH: &[u8] = b"embedded://skills/pangram/SKILL.md\n";

/// Ordered public resource inventory for one server lifetime.
pub static MCP_RESOURCES: &[EmbeddedResource] = &[
    EmbeddedResource {
        uri: "pangram://schema/output/v1",
        mime_type: OUTPUT_SCHEMA.mime_type,
        bytes: OUTPUT_SCHEMA.bytes,
    },
    EmbeddedResource {
        uri: "pangram://schema/errors/v1",
        mime_type: ERROR_REFERENCE.mime_type,
        bytes: ERROR_REFERENCE.bytes,
    },
    EmbeddedResource {
        uri: "pangram://skills/pangram",
        mime_type: PANGRAM_SKILL.mime_type,
        bytes: PANGRAM_SKILL.bytes,
    },
];

/// Looks up only a resource published by the immutable MCP inventory.
pub fn resource(uri: &str) -> Option<&'static EmbeddedResource> {
    MCP_RESOURCES.iter().find(|resource| resource.uri == uri)
}
