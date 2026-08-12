use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::domain::{
    AnalysisId, AnalysisStatus, BulkCounters, BulkId, CheckKind, CheckStatus, NonEmptyString,
    SubmissionOutcomeUnknownDetails, UtcTimestamp,
};

mod value;

pub use value::{
    AnalysisOutput, AuthSource, AuthStatus, BulkDryRun, BulkSubmitOutput, CommandData,
    CommandEnvelope, ConfigGetStatus, ConfigListStatus, ConfigPathStatus, DoctorCheck,
    DoctorCheckStatus, DoctorStatus, EnvelopeMeta, McpClientStatus, McpMutationAction,
    McpMutationReport, McpMutationTarget, McpStatus, MutationAcknowledgement, NonEmptyAnalyses,
    UpdateStatus, UpdateStatusKind,
};

mod projection;

pub(crate) use projection::sanitize_terminal;
pub use projection::{ColorPolicy, OutputFormat, ProjectionCause, render};

/// An output value could not satisfy its public serialization contract.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OutputValidationError {
    #[error("{0} must not be empty")]
    EmptyValue(&'static str),
    #[error("masked API-key suffix must contain no more than 8 characters")]
    AuthSuffixTooLong,
    #[error("{0} must use major.minor.patch numeric form")]
    InvalidVersion(&'static str),
    #[error("{0} has fixed retryability")]
    FixedRetryability(ErrorCode),
    #[error("{0} has fixed recovery guidance")]
    FixedRecovery(ErrorCode),
    #[error("submission_outcome_unknown requires typed submission details")]
    SubmissionDetailsRequired,
    #[error("{0} does not emit a JSON success envelope")]
    NonEnvelopeCommand(ResolvedCommand),
    #[error("MCP mutation path must be absolute")]
    RelativeMcpMutationPath,
    #[error("MCP mutation path must be valid UTF-8")]
    NonUtf8McpMutationPath,
    #[error("MCP mutation reason must not contain control characters")]
    UnsafeMcpMutationReason,
    #[error("analysis-only progress data cannot be attached to a bulk event")]
    AnalysisProgressDataOnBulk,
    #[error("bulk-only progress data cannot be attached to an analysis event")]
    BulkProgressDataOnAnalysis,
}

fn required_string(
    value: impl Into<String>,
    field: &'static str,
) -> Result<NonEmptyString, OutputValidationError> {
    NonEmptyString::new(value).map_err(|_| OutputValidationError::EmptyValue(field))
}

// Optional fields at JSON input boundaries accept omission, never explicit null.
fn deserialize_missing_only<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn deserialize_non_null_value<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(D::Error::custom("explicit null is not allowed"));
    }
    Ok(Some(value))
}

// Wire enums need their spelling, iteration order, and Display form to stay identical.
macro_rules! wire_enum {
    (
        $(#[$metadata:meta])*
        pub enum $name:ident {
            $($variant:ident => $wire_value:literal),+ $(,)?
        }
    ) => {
        $(#[$metadata])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        pub enum $name {
            $(
                #[serde(rename = $wire_value)]
                $variant,
            )+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire_value),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

/// The version shared by command envelopes and progress events.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum OutputSchemaVersion {
    #[default]
    #[serde(rename = "1")]
    V1,
}

wire_enum! {
    /// The closed error categories exposed by every adapter.
    pub enum ErrorCategory {
        Usage => "usage",
        Authentication => "authentication",
        Permission => "permission",
        Payment => "payment",
        RateLimit => "rate_limit",
        Network => "network",
        Upstream => "upstream",
        UpstreamContract => "upstream_contract",
        LocalConfig => "local_config",
        LocalHistory => "local_history",
        Update => "update",
    }
}

wire_enum! {
    /// Stable machine-readable error codes in contract order.
    pub enum ErrorCode {
        InputRequired => "input_required",
        InputConflict => "input_conflict",
        UnsupportedInput => "unsupported_input",
        UnsupportedCombination => "unsupported_combination",
        BulkLimitExceeded => "bulk_limit_exceeded",
        MissingApiKey => "missing_api_key",
        InvalidApiKey => "invalid_api_key",
        PermissionDenied => "permission_denied",
        PaymentRequired => "payment_required",
        RateLimited => "rate_limited",
        NetworkUnavailable => "network_unavailable",
        NetworkTimeout => "network_timeout",
        WaitTimeout => "wait_timeout",
        SubmissionOutcomeUnknown => "submission_outcome_unknown",
        UpstreamError => "upstream_error",
        UpstreamAnalysisFailed => "upstream_analysis_failed",
        UpstreamNotFound => "upstream_not_found",
        UpstreamContractChanged => "upstream_contract_changed",
        InvalidConfig => "invalid_config",
        InsecureConfigPermissions => "insecure_config_permissions",
        InsecureHistoryPermissions => "insecure_history_permissions",
        HistoryDisabled => "history_disabled",
        HistoryUnavailable => "history_unavailable",
        HistoryCorrupt => "history_corrupt",
        HistoryWriteFailed => "history_write_failed",
        LocalTaskUnresolvable => "local_task_unresolvable",
        McpCapabilityRequired => "mcp_capability_required",
        McpRootRequired => "mcp_root_required",
        McpPathOutsideRoot => "mcp_path_outside_root",
        UpdateUnavailable => "update_unavailable",
        UpdateNotOwned => "update_not_owned",
        UpdateVerificationFailed => "update_verification_failed",
        UpdateReplaceFailed => "update_replace_failed",
    }
}

impl ErrorCode {
    pub const fn category(self) -> ErrorCategory {
        match self {
            Self::InputRequired
            | Self::InputConflict
            | Self::UnsupportedInput
            | Self::UnsupportedCombination
            | Self::BulkLimitExceeded => ErrorCategory::Usage,
            Self::MissingApiKey | Self::InvalidApiKey => ErrorCategory::Authentication,
            Self::PermissionDenied
            | Self::McpCapabilityRequired
            | Self::McpRootRequired
            | Self::McpPathOutsideRoot => ErrorCategory::Permission,
            Self::PaymentRequired => ErrorCategory::Payment,
            Self::RateLimited => ErrorCategory::RateLimit,
            Self::NetworkUnavailable
            | Self::NetworkTimeout
            | Self::WaitTimeout
            | Self::SubmissionOutcomeUnknown => ErrorCategory::Network,
            Self::UpstreamError | Self::UpstreamAnalysisFailed | Self::UpstreamNotFound => {
                ErrorCategory::Upstream
            }
            Self::UpstreamContractChanged => ErrorCategory::UpstreamContract,
            Self::InvalidConfig | Self::InsecureConfigPermissions => ErrorCategory::LocalConfig,
            Self::InsecureHistoryPermissions
            | Self::HistoryDisabled
            | Self::HistoryUnavailable
            | Self::HistoryCorrupt
            | Self::HistoryWriteFailed
            | Self::LocalTaskUnresolvable => ErrorCategory::LocalHistory,
            Self::UpdateUnavailable
            | Self::UpdateNotOwned
            | Self::UpdateVerificationFailed
            | Self::UpdateReplaceFailed => ErrorCategory::Update,
        }
    }

    /// Contextual codes default to false so callers never advertise an unsafe retry.
    pub const fn default_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::NetworkUnavailable | Self::WaitTimeout
        )
    }

    /// These codes may be marked retryable only after the owning operation checks its context.
    pub const fn retryability_is_contextual(self) -> bool {
        matches!(
            self,
            Self::NetworkTimeout
                | Self::UpstreamError
                | Self::UpstreamAnalysisFailed
                | Self::HistoryUnavailable
                | Self::HistoryWriteFailed
                | Self::UpdateReplaceFailed
        )
    }
}

pub(crate) const SUBMISSION_OUTCOME_UNKNOWN_RECOVERY_MESSAGE: &str =
    "A manual retry may create a second billable operation.";

/// A safe next step which can be rendered without interpreting shell syntax.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Recovery {
    message: NonEmptyString,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<NonEmptyString>,
}

impl Recovery {
    fn submission_outcome_unknown() -> Self {
        Self::new(SUBMISSION_OUTCOME_UNKNOWN_RECOVERY_MESSAGE)
            .expect("the fixed submission recovery message is non-empty")
    }

    pub fn new(message: impl Into<String>) -> Result<Self, OutputValidationError> {
        Ok(Self {
            message: required_string(message, "recovery message")?,
            command: None,
        })
    }

    pub fn with_command(
        mut self,
        command: impl Into<String>,
    ) -> Result<Self, OutputValidationError> {
        self.command = Some(required_string(command, "recovery command")?);
        Ok(self)
    }

    pub fn message(&self) -> &str {
        self.message.as_str()
    }

    pub fn command(&self) -> Option<&str> {
        self.command.as_ref().map(NonEmptyString::as_str)
    }
}

/// Safe code-specific fields attached to a canonical error.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CanonicalErrorDetails {
    SubmissionOutcomeUnknown(SubmissionOutcomeUnknownDetails),
    Fields(BTreeMap<String, Value>),
}

/// The adapter-independent error object used by CLI, TUI, and MCP output.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CanonicalError {
    code: ErrorCode,
    category: ErrorCategory,
    message: NonEmptyString,
    retryable: bool,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_ms: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<Recovery>,
    /// Callers must remove credentials, submitted text, matches, and raw responses first.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<BTreeMap<String, Value>>")]
    details: Option<CanonicalErrorDetails>,
}

impl CanonicalError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Result<Self, OutputValidationError> {
        if code == ErrorCode::SubmissionOutcomeUnknown {
            return Err(OutputValidationError::SubmissionDetailsRequired);
        }
        Ok(Self {
            code,
            category: code.category(),
            message: required_string(message, "error message")?,
            retryable: code.default_retryable(),
            retry_after_ms: None,
            recovery: None,
            details: None,
        })
    }

    pub fn submission_outcome_unknown(
        message: impl Into<String>,
        details: SubmissionOutcomeUnknownDetails,
    ) -> Result<Self, OutputValidationError> {
        Ok(Self {
            code: ErrorCode::SubmissionOutcomeUnknown,
            category: ErrorCategory::Network,
            message: required_string(message, "error message")?,
            retryable: false,
            retry_after_ms: None,
            recovery: Some(Recovery::submission_outcome_unknown()),
            details: Some(CanonicalErrorDetails::SubmissionOutcomeUnknown(details)),
        })
    }

    pub fn with_contextual_retryability(
        mut self,
        retryable: bool,
    ) -> Result<Self, OutputValidationError> {
        if !self.code.retryability_is_contextual() {
            return Err(OutputValidationError::FixedRetryability(self.code));
        }
        self.retryable = retryable;
        Ok(self)
    }

    #[must_use]
    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    pub fn with_recovery(mut self, recovery: Recovery) -> Result<Self, OutputValidationError> {
        if self.code == ErrorCode::SubmissionOutcomeUnknown {
            return Err(OutputValidationError::FixedRecovery(self.code));
        }
        self.recovery = Some(recovery);
        Ok(self)
    }

    /// Attaches details that the caller has already reduced to its safe contract fields.
    pub fn with_details(
        mut self,
        details: BTreeMap<String, Value>,
    ) -> Result<Self, OutputValidationError> {
        if self.code == ErrorCode::SubmissionOutcomeUnknown {
            return Err(OutputValidationError::SubmissionDetailsRequired);
        }
        self.details = Some(CanonicalErrorDetails::Fields(details));
        Ok(self)
    }

    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    pub const fn category(&self) -> ErrorCategory {
        self.category
    }

    pub fn message(&self) -> &str {
        self.message.as_str()
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    pub const fn retry_after_ms(&self) -> Option<u64> {
        self.retry_after_ms
    }

    pub const fn recovery(&self) -> Option<&Recovery> {
        self.recovery.as_ref()
    }

    pub const fn details(&self) -> Option<&CanonicalErrorDetails> {
        self.details.as_ref()
    }
}

/// The adapter-independent missing-credential failure and recovery guidance.
pub(crate) fn missing_api_key_error() -> CanonicalError {
    let recovery = Recovery::new("Configure a persistent key or set PANGRAM_API_KEY.")
        .and_then(|recovery| recovery.with_command("pangram auth"))
        .expect("fixed recovery text is non-empty");
    CanonicalError::new(
        ErrorCode::MissingApiKey,
        "No Pangram API key is configured.",
    )
    .and_then(|error| error.with_recovery(recovery))
    .expect("recovery is valid for missing_api_key")
}

#[derive(Deserialize)]
struct CanonicalErrorWire {
    code: ErrorCode,
    category: ErrorCategory,
    message: NonEmptyString,
    retryable: bool,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    retry_after_ms: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    recovery: Option<Recovery>,
    #[serde(default, deserialize_with = "deserialize_non_null_value")]
    details: Option<Value>,
}

impl<'de> Deserialize<'de> for CanonicalError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CanonicalErrorWire::deserialize(deserializer)?;
        if wire.category != wire.code.category() {
            return Err(D::Error::custom("error category does not match its code"));
        }
        if !wire.code.retryability_is_contextual()
            && wire.retryable != wire.code.default_retryable()
        {
            return Err(D::Error::custom(
                "error retryability does not match its code",
            ));
        }

        let recovery = if wire.code == ErrorCode::SubmissionOutcomeUnknown {
            let expected = Recovery::submission_outcome_unknown();
            if wire.recovery.as_ref() != Some(&expected) {
                return Err(D::Error::custom(OutputValidationError::FixedRecovery(
                    wire.code,
                )));
            }
            wire.recovery
        } else {
            wire.recovery
        };

        let details = if wire.code == ErrorCode::SubmissionOutcomeUnknown {
            let details = wire.details.ok_or_else(|| {
                D::Error::custom(OutputValidationError::SubmissionDetailsRequired)
            })?;
            Some(CanonicalErrorDetails::SubmissionOutcomeUnknown(
                serde_json::from_value(details).map_err(D::Error::custom)?,
            ))
        } else {
            wire.details
                .map(|details| {
                    serde_json::from_value(details)
                        .map(CanonicalErrorDetails::Fields)
                        .map_err(D::Error::custom)
                })
                .transpose()?
        };

        Ok(Self {
            code: wire.code,
            category: wire.category,
            message: wire.message,
            retryable: wire.retryable,
            retry_after_ms: wire.retry_after_ms,
            recovery,
            details,
        })
    }
}

wire_enum! {
    /// The canonical command name after aliases and bare-command dispatch are resolved.
    pub enum ResolvedCommand {
        Detect => "detect",
        Plagiarism => "plagiarism",
        Analyze => "analyze",
        BulkSubmit => "bulk_submit",
        BulkStatus => "bulk_status",
        BulkWait => "bulk_wait",
        BulkResults => "bulk_results",
        TaskStatus => "task_status",
        TaskWait => "task_wait",
        HistoryList => "history_list",
        HistorySearch => "history_search",
        HistoryShow => "history_show",
        HistoryRerun => "history_rerun",
        HistoryDelete => "history_delete",
        HistoryClear => "history_clear",
        HistoryExport => "history_export",
        AuthSet => "auth_set",
        AuthStatus => "auth_status",
        AuthLogout => "auth_logout",
        ConfigList => "config_list",
        ConfigGet => "config_get",
        ConfigSet => "config_set",
        ConfigPath => "config_path",
        Doctor => "doctor",
        McpServe => "mcp_serve",
        McpInstall => "mcp_install",
        McpUninstall => "mcp_uninstall",
        McpStatus => "mcp_status",
        Agent => "agent",
        SkillsList => "skills_list",
        SkillsGet => "skills_get",
        SkillsPath => "skills_path",
        Completions => "completions",
        UpdateCheck => "update_check",
        UpdateInstall => "update_install",
    }
}

impl ResolvedCommand {
    /// Commands with non-envelope success output still use this value in failure envelopes.
    pub const fn uses_json_envelope(self) -> bool {
        !matches!(
            self,
            Self::HistoryExport
                | Self::McpServe
                | Self::Agent
                | Self::SkillsList
                | Self::SkillsGet
                | Self::SkillsPath
                | Self::Completions
        )
    }
}

/// Stable process exit values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ExitCode {
    Success = 0,
    GeneralFailure = 1,
    Usage = 2,
    Partial = 3,
    AuthenticationOrPermission = 4,
    PaymentOrRateLimit = 5,
    NetworkOrUpstream = 6,
    LocalState = 7,
    Interrupted = 130,
}

impl ExitCode {
    pub const ALL: &'static [Self] = &[
        Self::Success,
        Self::GeneralFailure,
        Self::Usage,
        Self::Partial,
        Self::AuthenticationOrPermission,
        Self::PaymentOrRateLimit,
        Self::NetworkOrUpstream,
        Self::LocalState,
        Self::Interrupted,
    ];

    pub const fn for_error(category: ErrorCategory) -> Self {
        match category {
            ErrorCategory::Usage => Self::Usage,
            ErrorCategory::Authentication | ErrorCategory::Permission => {
                Self::AuthenticationOrPermission
            }
            ErrorCategory::Payment | ErrorCategory::RateLimit => Self::PaymentOrRateLimit,
            ErrorCategory::Network | ErrorCategory::Upstream | ErrorCategory::UpstreamContract => {
                Self::NetworkOrUpstream
            }
            ErrorCategory::LocalConfig | ErrorCategory::LocalHistory | ErrorCategory::Update => {
                Self::LocalState
            }
        }
    }

    pub const fn for_status(status: AnalysisStatus) -> Self {
        match status {
            AnalysisStatus::Queued | AnalysisStatus::Running | AnalysisStatus::Succeeded => {
                Self::Success
            }
            AnalysisStatus::Partial => Self::Partial,
            AnalysisStatus::Failed => Self::GeneralFailure,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

impl From<ExitCode> for u8 {
    fn from(code: ExitCode) -> Self {
        code.as_u8()
    }
}

impl From<ExitCode> for i32 {
    fn from(code: ExitCode) -> Self {
        code.as_i32()
    }
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(code: ExitCode) -> Self {
        Self::from(code.as_u8())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProgressEventType {
    #[default]
    Progress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct AnalysisProgressEvent {
    schema_version: OutputSchemaVersion,
    #[serde(rename = "type")]
    event_type: ProgressEventType,
    analysis_id: AnalysisId,
    check: CheckKind,
    status: CheckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_stage: Option<NonEmptyString>,
    observed_at: UtcTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct BulkProgressEvent {
    schema_version: OutputSchemaVersion,
    #[serde(rename = "type")]
    event_type: ProgressEventType,
    bulk_id: BulkId,
    status: AnalysisStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    counters: Option<BulkCounters>,
    observed_at: UtcTimestamp,
}

/// A content-free observation event with structurally exclusive payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ProgressEvent {
    Analysis(AnalysisProgressEvent),
    Bulk(BulkProgressEvent),
}

impl ProgressEvent {
    pub fn analysis(
        analysis_id: AnalysisId,
        check: CheckKind,
        status: CheckStatus,
        observed_at: UtcTimestamp,
    ) -> Self {
        Self::Analysis(AnalysisProgressEvent {
            schema_version: OutputSchemaVersion::V1,
            event_type: ProgressEventType::Progress,
            analysis_id,
            check,
            status,
            upstream_stage: None,
            observed_at,
        })
    }

    pub fn bulk(bulk_id: BulkId, status: AnalysisStatus, observed_at: UtcTimestamp) -> Self {
        Self::Bulk(BulkProgressEvent {
            schema_version: OutputSchemaVersion::V1,
            event_type: ProgressEventType::Progress,
            bulk_id,
            status,
            counters: None,
            observed_at,
        })
    }

    pub fn with_upstream_stage(
        mut self,
        upstream_stage: impl Into<String>,
    ) -> Result<Self, OutputValidationError> {
        match &mut self {
            Self::Analysis(event) => {
                event.upstream_stage = Some(required_string(upstream_stage, "upstream stage")?);
                Ok(self)
            }
            Self::Bulk(_) => Err(OutputValidationError::AnalysisProgressDataOnBulk),
        }
    }

    pub fn with_counters(mut self, counters: BulkCounters) -> Result<Self, OutputValidationError> {
        match &mut self {
            Self::Bulk(event) => {
                event.counters = Some(counters);
                Ok(self)
            }
            Self::Analysis(_) => Err(OutputValidationError::BulkProgressDataOnAnalysis),
        }
    }
}
