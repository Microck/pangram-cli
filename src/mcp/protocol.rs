//! RMCP-only conversion and stdio protocol handling.

use std::borrow::Cow;
use std::sync::Arc;

use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, CompleteRequestMethod,
    CompleteRequestParams, CompleteResult, ContentBlock, Implementation, InitializeRequestParams,
    InitializeResult, InitializeResultMethod, JsonObject, ListPromptsRequestMethod,
    ListPromptsResult, ListResourceTemplatesRequestMethod, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::service::{MaybeSendFuture, NotificationContext, RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};

use super::embedded::{AGENT_REFERENCE, MCP_RESOURCES, resource};
use super::files::ApprovedFileRoots;
use super::schema::ToolName;
use super::tools::{ToolCallOutcome, ToolRuntime};
use super::{McpOptions, McpStartupError};
use crate::analysis::AnalyzerSource;
use crate::config::ConfigService;

/// One concrete handler keeps inventory, capability gates, file handles, and
/// future tool execution dependencies in one place for the server lifetime.
pub(crate) struct PangramMcpServer {
    tools: Vec<Tool>,
    tool_runtime: ToolRuntime,
}

impl PangramMcpServer {
    pub(crate) fn new(
        options: McpOptions,
        approved_roots: ApprovedFileRoots,
        analyzer_source: AnalyzerSource,
        config_service: ConfigService,
    ) -> Result<Self, McpStartupError> {
        let output_schema = crate::contracts::mcp_output_schema();
        let enabled_names = ToolName::ALL
            .into_iter()
            .filter(|name| tool_enabled(*name, &options));
        let specs = super::schema::tool_specs_for(enabled_names, |command| {
            crate::contracts::specialize_mcp_output_schema(&output_schema, command)
        });
        let tools = specs
            .into_iter()
            .map(tool_from_spec)
            .collect::<Result<_, _>>()?;

        Ok(Self {
            tools,
            tool_runtime: ToolRuntime::new(
                options,
                approved_roots,
                analyzer_source,
                config_service,
            ),
        })
    }

    async fn dispatch_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(name) = ToolName::parse(&request.name) else {
            return Err(McpError::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                request.name.into_owned(),
                None,
            ));
        };
        if !self
            .tools
            .iter()
            .any(|tool| tool.name.as_ref() == name.as_str())
        {
            return Err(McpError::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                request.name.into_owned(),
                None,
            ));
        }

        let outcome = self
            .tool_runtime
            .call(
                name,
                request.arguments.unwrap_or_default(),
                context.ct.clone(),
            )
            .await;
        match outcome {
            ToolCallOutcome::InvalidArguments => {
                Err(McpError::invalid_params("tool arguments are invalid", None))
            }
            ToolCallOutcome::Complete { summary, envelope } => {
                let is_error = envelope.error().is_some();
                let structured = serde_json::to_value(envelope).map_err(|_| {
                    McpError::internal_error("failed to serialize tool result", None)
                })?;
                let mut result = if is_error {
                    CallToolResult::error(vec![ContentBlock::text(summary)])
                } else {
                    CallToolResult::success(vec![ContentBlock::text(summary)])
                };
                result.structured_content = Some(structured);
                Ok(result.into())
            }
            ToolCallOutcome::Cancelled { diagnostic } => {
                if let Some(diagnostic) = diagnostic {
                    eprintln!("pangram: {diagnostic}");
                }
                Err(McpError::internal_error(
                    "tool observation was cancelled",
                    None,
                ))
            }
        }
    }
}

impl ServerHandler for PangramMcpServer {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        static SUPPORTED: [ProtocolVersion; 1] = [ProtocolVersion::V_2026_07_28];
        Cow::Borrowed(&SUPPORTED)
    }

    fn get_info(&self) -> ServerInfo {
        let instructions = std::str::from_utf8(AGENT_REFERENCE.bytes)
            .expect("the generated agent reference is UTF-8")
            .to_owned();
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_resources()
                .enable_tools()
                .build(),
        )
        .with_protocol_version(ProtocolVersion::V_2026_07_28)
        .with_server_info(Implementation::new("pangram", env!("CARGO_PKG_VERSION")))
        .with_instructions(instructions)
    }

    fn initialize(
        &self,
        _request: InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<InitializeResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Err(McpError::method_not_found::<InitializeResultMethod>()))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(self.tools.clone())
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private)))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, McpError>> + MaybeSendFuture + '_ {
        self.dispatch_tool(request, context)
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + MaybeSendFuture + '_ {
        let resources = MCP_RESOURCES
            .iter()
            .map(|embedded| {
                Resource::new(embedded.uri, embedded.uri)
                    .with_mime_type(embedded.mime_type)
                    .with_size(embedded.bytes.len() as u64)
            })
            .collect();
        std::future::ready(Ok(ListResourcesResult::with_all_items(resources)
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private)))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, McpError>> + MaybeSendFuture + '_ {
        let result = resource(&request.uri)
            .ok_or_else(|| McpError::resource_not_found("resource not found", None))
            .and_then(|embedded| {
                let text = std::str::from_utf8(embedded.bytes).map_err(|_| {
                    McpError::internal_error("embedded resource is not UTF-8", None)
                })?;
                Ok(ReadResourceResult::new(vec![
                    ResourceContents::text(text, embedded.uri).with_mime_type(embedded.mime_type),
                ])
                .with_ttl_ms(0)
                .with_cache_scope(CacheScope::Private)
                .into())
            });
        std::future::ready(result)
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Err(McpError::method_not_found::<ListPromptsRequestMethod>()))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, McpError>> + MaybeSendFuture + '_
    {
        std::future::ready(Err(McpError::method_not_found::<
            ListResourceTemplatesRequestMethod,
        >()))
    }

    fn complete(
        &self,
        _request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CompleteResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Err(McpError::method_not_found::<CompleteRequestMethod>()))
    }

    fn on_initialized(
        &self,
        _context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        std::future::ready(())
    }
}

pub(crate) async fn serve(server: PangramMcpServer) -> Result<(), McpStartupError> {
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|_| McpStartupError::Serve)?;
    running
        .waiting()
        .await
        .map_err(|_| McpStartupError::Serve)?;
    Ok(())
}

fn tool_from_spec(spec: super::schema::ToolSpec) -> Result<Tool, McpStartupError> {
    let input_schema = object_schema(spec.input_schema)?;
    let output_schema = object_schema(spec.output_schema)?;
    let annotations: ToolAnnotations = serde_json::from_value(spec.annotations)
        .map_err(|_| McpStartupError::InvalidEmbeddedContract)?;
    Ok(
        Tool::new(spec.name.as_str(), spec.description, input_schema)
            .with_raw_output_schema(Arc::new(output_schema))
            .with_annotations(annotations),
    )
}

fn object_schema(value: serde_json::Value) -> Result<JsonObject, McpStartupError> {
    value
        .as_object()
        .cloned()
        .ok_or(McpStartupError::InvalidEmbeddedContract)
}

fn tool_enabled(name: ToolName, options: &McpOptions) -> bool {
    match name {
        ToolName::HistoryList | ToolName::HistorySearch | ToolName::HistoryGet => options.history,
        ToolName::HistoryRerun | ToolName::HistoryDelete | ToolName::HistoryClear => {
            options.history && options.allow_history_mutations
        }
        ToolName::UpdateConfig => options.allow_config_mutations,
        ToolName::DetectText
        | ToolName::CheckPlagiarism
        | ToolName::AnalyzeText
        | ToolName::GetTask
        | ToolName::WaitTask
        | ToolName::SubmitBulk
        | ToolName::GetBulk
        | ToolName::WaitBulk
        | ToolName::GetBulkResults
        | ToolName::CheckUpdate => true,
    }
}
