//! RMCP-free execution seam for Pangram MCP tools.
//!
//! The protocol adapter owns JSON-RPC and RMCP conversion. This module owns
//! one immutable runtime and returns canonical command envelopes so every
//! adapter projects the same domain values and failures.

mod analysis;
mod bulk;
mod local;

use std::sync::Arc;

use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use super::McpOptions;
use super::files::ApprovedFileRoots;
use super::schema::ToolName;
use crate::analysis::{Analyzer, AnalyzerSession, AnalyzerSource};
use crate::config::ConfigService;
use crate::domain::UtcTimestamp;
use crate::output::{
    CanonicalError, CommandData, CommandEnvelope, EnvelopeMeta, ErrorCode, ResolvedCommand,
};

/// Immutable dependencies shared by every call in one MCP server process.
pub(crate) struct ToolRuntime {
    options: McpOptions,
    approved_roots: Arc<ApprovedFileRoots>,
    analyzer_source: AnalyzerSource,
    analyzer_session: AnalyzerSession,
    config_service: ConfigService,
}

impl ToolRuntime {
    pub(crate) fn new(
        options: McpOptions,
        approved_roots: ApprovedFileRoots,
        analyzer_source: AnalyzerSource,
        config_service: ConfigService,
    ) -> Self {
        Self {
            options,
            approved_roots: Arc::new(approved_roots),
            analyzer_source,
            analyzer_session: AnalyzerSession::default(),
            config_service,
        }
    }

    /// Dispatches one protocol-validated identity through the RMCP-free tool
    /// interface while preserving the analysis/bulk/local handler split.
    pub(crate) async fn call(
        &self,
        name: ToolName,
        arguments: Map<String, Value>,
        cancellation: CancellationToken,
    ) -> ToolCallOutcome {
        let started = UtcTimestamp::now();
        let context = ToolCallContext {
            options: &self.options,
            approved_roots: &self.approved_roots,
            analyzer_source: &self.analyzer_source,
            analyzer_session: &self.analyzer_session,
            config_service: &self.config_service,
            cancellation: &cancellation,
        };

        match name {
            ToolName::DetectText => analysis::detect_text(&context, arguments, started).await,
            ToolName::GetTask => {
                analysis::task(&context, arguments, started, analysis::TaskOperation::Get).await
            }
            ToolName::WaitTask => {
                analysis::task(&context, arguments, started, analysis::TaskOperation::Wait).await
            }
            ToolName::SubmitBulk => bulk::submit(&context, arguments, started).await,
            ToolName::GetBulk => {
                bulk::observe(&context, arguments, started, bulk::BulkOperation::Get).await
            }
            ToolName::WaitBulk => {
                bulk::observe(&context, arguments, started, bulk::BulkOperation::Wait).await
            }
            ToolName::GetBulkResults => {
                bulk::observe(&context, arguments, started, bulk::BulkOperation::Results).await
            }
            ToolName::HistoryList => {
                local::history_query(&context, arguments, false, started).await
            }
            ToolName::HistorySearch => {
                local::history_query(&context, arguments, true, started).await
            }
            ToolName::HistoryGet => local::history_get(&context, arguments, started).await,
            ToolName::HistoryRerun => local::history_rerun(&context, arguments, started).await,
            ToolName::HistoryDelete => local::history_delete(&context, arguments, started).await,
            ToolName::HistoryClear => local::history_clear(&context, arguments, started).await,
            ToolName::UpdateConfig => local::update_config(&context, arguments, started).await,
        }
    }
}

/// The handler result before RMCP conversion.
#[derive(Debug)]
pub(crate) enum ToolCallOutcome {
    InvalidArguments,
    Complete {
        summary: String,
        envelope: Box<CommandEnvelope>,
    },
    /// RMCP suppresses the response for a cancelled request. The optional
    /// content-free diagnostic may be written to stderr by the protocol seam.
    Cancelled {
        diagnostic: Option<String>,
    },
}

pub(super) struct ToolCallContext<'a> {
    options: &'a McpOptions,
    approved_roots: &'a Arc<ApprovedFileRoots>,
    analyzer_source: &'a AnalyzerSource,
    analyzer_session: &'a AnalyzerSession,
    config_service: &'a ConfigService,
    cancellation: &'a CancellationToken,
}

impl ToolCallContext<'_> {
    pub(super) const fn options(&self) -> &McpOptions {
        self.options
    }

    pub(super) fn approved_roots_handle(&self) -> Arc<ApprovedFileRoots> {
        Arc::clone(self.approved_roots)
    }

    pub(super) const fn analyzer_source(&self) -> &AnalyzerSource {
        self.analyzer_source
    }

    pub(super) const fn analyzer_session(&self) -> &AnalyzerSession {
        self.analyzer_session
    }

    pub(super) const fn service(&self) -> &ConfigService {
        self.config_service
    }

    pub(super) const fn cancellation(&self) -> &CancellationToken {
        self.cancellation
    }

    pub(super) const fn history_mutations_enabled(&self) -> bool {
        self.options.history && self.options.allow_history_mutations
    }
}

pub(super) fn invalid_arguments() -> ToolCallOutcome {
    ToolCallOutcome::InvalidArguments
}

pub(super) fn success(
    data: CommandData,
    summary: impl Into<String>,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    let envelope = CommandEnvelope::success(
        data,
        EnvelopeMeta::default()
            .with_started_at(started)
            .with_completed_at(UtcTimestamp::now()),
    );
    ToolCallOutcome::Complete {
        summary: summary.into(),
        envelope: Box::new(envelope),
    }
}

pub(super) fn failure(
    command: ResolvedCommand,
    error: CanonicalError,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    let summary = error.message().to_owned();
    let envelope = CommandEnvelope::failure(
        command,
        error,
        EnvelopeMeta::default()
            .with_started_at(started)
            .with_failed_at(UtcTimestamp::now()),
    );
    ToolCallOutcome::Complete {
        summary,
        envelope: Box::new(envelope),
    }
}

pub(super) fn canonical_error(code: ErrorCode, message: &str) -> CanonicalError {
    CanonicalError::new(code, message).expect("static error")
}

pub(super) async fn resolve_analyzer(
    context: &ToolCallContext<'_>,
) -> Result<Analyzer, Box<CanonicalError>> {
    // Resolve for every call so update_config changes become visible within
    // the same MCP session, while keeping credential and config I/O off the
    // async executor.
    let source = context.analyzer_source().clone();
    let session = context.analyzer_session().clone();
    let service = context.service().clone();
    tokio::task::spawn_blocking(move || {
        source
            .resolve_in_session(&service, &session)
            .map_err(Box::new)
    })
    .await
    .map_err(|_| Box::new(blocking_operation_error()))?
}

pub(super) fn blocking_operation_error() -> CanonicalError {
    canonical_error(
        ErrorCode::InvalidConfig,
        "the MCP tool operation did not complete",
    )
}
