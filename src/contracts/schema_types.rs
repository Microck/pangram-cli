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

#[derive(JsonSchema)]
pub(super) enum Target {
    #[serde(rename = "x86_64-unknown-linux-gnu")]
    X86_64UnknownLinuxGnu,
    #[serde(rename = "aarch64-unknown-linux-gnu")]
    Aarch64UnknownLinuxGnu,
    #[serde(rename = "x86_64-apple-darwin")]
    X86_64AppleDarwin,
    #[serde(rename = "aarch64-apple-darwin")]
    Aarch64AppleDarwin,
    #[serde(rename = "x86_64-pc-windows-msvc")]
    X86_64PcWindowsMsvc,
}

#[derive(JsonSchema)]
pub(super) enum ArchiveFormat {
    #[serde(rename = "tar.xz")]
    TarXz,
    #[serde(rename = "zip")]
    Zip,
}

#[derive(JsonSchema)]
#[serde(transparent)]
pub(super) struct SemVerString(
    #[schemars(regex(
        pattern = r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-((0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*))?(\+([0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*))?$"
    ))]
    String,
);

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateArtifact {
    pub target: Target,
    pub archive_format: ArchiveFormat,
    #[schemars(url, regex(pattern = r"^https://"))]
    pub url: String,
    #[schemars(range(min = 1))]
    pub size_bytes: u64,
    #[schemars(range(min = 1))]
    pub executable_size_bytes: u64,
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"))]
    pub sha256: String,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateManifest {
    pub schema_version: String,
    pub channel: String,
    pub version: SemVerString,
    #[schemars(extend("format" = "date-time"))]
    pub published_at: String,
    #[schemars(url, regex(pattern = r"^https://"))]
    pub notes_url: String,
    pub minimum_updater_version: SemVerString,
    #[schemars(length(min = 1))]
    pub artifacts: Vec<UpdateArtifact>,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestSignature {
    pub schema_version: String,
    pub algorithm: String,
    #[schemars(length(min = 1))]
    pub key_id: String,
    pub signature: String,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateState {
    pub schema_version: String,
    #[schemars(extend("format" = "date-time"))]
    pub last_checked_at: String,
    pub etag: Option<String>,
    pub available_version: Option<SemVerString>,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct InstallReceipt {
    pub schema_version: String,
    pub method: String,
    #[schemars(length(min = 1))]
    pub executable_path: String,
    pub installed_version: SemVerString,
    pub target: Target,
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"))]
    pub manifest_sha256: String,
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"))]
    pub executable_sha256: String,
    #[schemars(extend("format" = "date-time"))]
    pub installed_at: String,
}
