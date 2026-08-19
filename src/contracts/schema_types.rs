//! Private types which own the shapes of generated contract artifacts.

#![allow(dead_code)]

use schemars::JsonSchema;

use crate::domain::{
    Analysis, AnalysisPage, AnalysisSummaryPage, BulkCollection, BulkPage,
    SubmissionOutcomeUnknownDetails,
};
use crate::output::{
    AuthStatus, BulkDryRun, CanonicalError, ConfigGetStatus, ConfigListStatus, ConfigPathStatus,
    DoctorStatus, EnvelopeMeta, McpMutationReport, McpStatus, MutationAcknowledgement,
    UpdateStatus,
};

/// A Schemars-only registry which causes every output type to share one `$defs`.
#[derive(JsonSchema)]
#[allow(clippy::large_enum_variant)]
pub(super) enum OutputRegistry {
    Analysis(Analysis<CanonicalError>),
    BulkCollection(BulkCollection),
    BulkDryRun(BulkDryRun),
    BulkPage(BulkPage<CanonicalError>),
    AnalysisPage(AnalysisPage<CanonicalError>),
    AnalysisSummaryPage(AnalysisSummaryPage),
    Mutation(MutationAcknowledgement),
    McpMutation(McpMutationReport),
    Auth(AuthStatus),
    ConfigList(ConfigListStatus),
    ConfigGet(ConfigGetStatus),
    ConfigPath(ConfigPathStatus),
    Doctor(DoctorStatus),
    Mcp(McpStatus),
    Update(UpdateStatus),
    Error(CanonicalError),
    Meta(EnvelopeMeta),
    UnknownSubmission(SubmissionOutcomeUnknownDetails),
}

// The configuration schema is owned by the canonical runtime model in
// `crate::config`. Aliasing `Config` keeps the generated `$defs` names and
// the committed `config.schema.json` byte-identical to the Schemars-only
// predecessors they replaced.
pub(super) use crate::config::Config;

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct TuiState {
    pub schema_version: String,
    pub intro_seen: bool,
}
