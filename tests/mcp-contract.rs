#[cfg(feature = "dev-tools")]
#[path = "support/protocol_loopback/mod.rs"]
mod fixture;
#[path = "support/mcp_stdio.rs"]
mod mcp_stdio;

#[path = "mcp-contract/analysis.rs"]
mod analysis;
#[path = "mcp-contract/bulk.rs"]
mod bulk;
#[path = "mcp-contract/capabilities.rs"]
mod capabilities;
#[path = "mcp-contract/history.rs"]
mod history;
#[path = "mcp-contract/inventory.rs"]
mod inventory;
#[path = "mcp-contract/protocol.rs"]
mod protocol;
#[path = "mcp-contract/resources.rs"]
mod resources;
