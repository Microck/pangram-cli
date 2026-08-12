//! Shipping MCP server composition and startup validation.
//!
//! The process adapter calls this module only after Clap has selected `mcp`.
//! Startup configuration is resolved before the stdio handles are created, so
//! an invalid capability set cannot consume protocol input or write stdout.

use std::path::PathBuf;

use thiserror::Error;

use crate::analysis::AnalyzerSource;
use crate::config::ConfigService;

pub(crate) mod embedded;
mod files;
pub(crate) mod install;
mod protocol;
pub(crate) mod schema;
mod tools;

/// Immutable capabilities selected for one MCP server process.
#[derive(Clone, Debug, Default)]
pub(crate) struct McpOptions {
    pub history: bool,
    pub allow_history_mutations: bool,
    pub allow_config_mutations: bool,
    pub allow_public_links: bool,
    pub allow_file_roots: Vec<PathBuf>,
}

/// A sanitized startup failure suitable for one stderr line.
#[derive(Debug, Error)]
pub(crate) enum McpStartupError {
    #[error("MCP history mutations require --history")]
    HistoryMutationsRequireHistory,
    #[error("invalid MCP file root")]
    InvalidFileRoot(#[from] files::ApprovedFileError),
    #[error("invalid embedded MCP contract")]
    InvalidEmbeddedContract,
    #[error("failed to start the MCP runtime")]
    Runtime,
    #[error("failed to serve MCP stdio")]
    Serve,
}

/// Runs the shipping stdio server until the client closes the transport.
///
/// This is deliberately blocking: process dispatch owns the server lifecycle,
/// while RMCP and the shared analysis module own asynchronous work inside it.
pub(crate) fn serve_stdio(
    options: McpOptions,
    analyzer_source: AnalyzerSource,
    config_service: ConfigService,
) -> Result<(), McpStartupError> {
    if options.allow_history_mutations && !options.history {
        return Err(McpStartupError::HistoryMutationsRequireHistory);
    }

    let approved_roots = files::ApprovedFileRoots::preopen(&options.allow_file_roots)?;
    let server =
        protocol::PangramMcpServer::new(options, approved_roots, analyzer_source, config_service)?;
    let runtime = tokio::runtime::Runtime::new().map_err(|_| McpStartupError::Runtime)?;
    runtime.block_on(protocol::serve(server))
}
