use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::domain::{
    Analysis, AnalysisPage, BulkCollection, BulkPage, NonEmptyString, UtcTimestamp,
};

use super::{
    CanonicalError, OutputSchemaVersion, OutputValidationError, ResolvedCommand,
    deserialize_missing_only, deserialize_non_null_value, required_string,
};

/// A successful command which mutates local state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MutationAcknowledgement {
    #[schemars(extend("const" = true))]
    ok: bool,
}

impl MutationAcknowledgement {
    pub const fn new() -> Self {
        Self { ok: true }
    }

    pub const fn ok(self) -> bool {
        self.ok
    }
}

impl Default for MutationAcknowledgement {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for MutationAcknowledgement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            ok: bool,
        }

        if !Wire::deserialize(deserializer)?.ok {
            return Err(D::Error::custom("mutation acknowledgement must be true"));
        }
        Ok(Self::new())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    None,
    Environment,
    Stored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
struct AuthSuffix(#[schemars(length(max = 8))] String);

impl AuthSuffix {
    fn new(value: impl Into<String>) -> Result<Self, OutputValidationError> {
        let value = value.into();
        if value.chars().count() > 8 {
            return Err(OutputValidationError::AuthSuffixTooLong);
        }
        Ok(Self(value))
    }
}

impl<'de> Deserialize<'de> for AuthSuffix {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuthStatus {
    configured: bool,
    source: AuthSource,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    masked_suffix: Option<AuthSuffix>,
}

impl AuthStatus {
    pub fn new(
        configured: bool,
        source: AuthSource,
        masked_suffix: Option<String>,
    ) -> Result<Self, OutputValidationError> {
        Ok(Self {
            configured,
            source,
            masked_suffix: masked_suffix.map(AuthSuffix::new).transpose()?,
        })
    }

    pub const fn configured(&self) -> bool {
        self.configured
    }

    pub const fn source(&self) -> AuthSource {
        self.source
    }

    pub fn masked_suffix(&self) -> Option<&str> {
        self.masked_suffix.as_ref().map(|suffix| suffix.0.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigListStatus {
    config: BTreeMap<String, Value>,
}

impl ConfigListStatus {
    pub fn new(config: BTreeMap<String, Value>) -> Self {
        Self { config }
    }

    pub const fn config(&self) -> &BTreeMap<String, Value> {
        &self.config
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigGetStatus {
    key: NonEmptyString,
    value: Value,
}

impl ConfigGetStatus {
    pub fn new(key: impl Into<String>, value: Value) -> Result<Self, OutputValidationError> {
        Ok(Self {
            key: required_string(key, "config key")?,
            value,
        })
    }

    pub fn key(&self) -> &str {
        self.key.as_str()
    }

    pub const fn value(&self) -> &Value {
        &self.value
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigPathStatus {
    path: NonEmptyString,
}

impl ConfigPathStatus {
    pub fn new(path: impl Into<String>) -> Result<Self, OutputValidationError> {
        Ok(Self {
            path: required_string(path, "config path")?,
        })
    }

    pub fn path(&self) -> &str {
        self.path.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DoctorCheck {
    name: NonEmptyString,
    status: DoctorCheckStatus,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl DoctorCheck {
    pub fn new(
        name: impl Into<String>,
        status: DoctorCheckStatus,
        message: Option<String>,
    ) -> Result<Self, OutputValidationError> {
        Ok(Self {
            name: required_string(name, "doctor check name")?,
            status,
            message,
        })
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub const fn status(&self) -> DoctorCheckStatus {
        self.status
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DoctorStatus {
    checks: Vec<DoctorCheck>,
}

impl DoctorStatus {
    pub fn new(checks: Vec<DoctorCheck>) -> Self {
        Self { checks }
    }

    pub fn checks(&self) -> &[DoctorCheck] {
        &self.checks
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct McpClientStatus {
    client: NonEmptyString,
    installed: bool,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

impl McpClientStatus {
    pub fn new(
        client: impl Into<String>,
        installed: bool,
        path: Option<String>,
    ) -> Result<Self, OutputValidationError> {
        Ok(Self {
            client: required_string(client, "MCP client")?,
            installed,
            path,
        })
    }

    pub fn client(&self) -> &str {
        self.client.as_str()
    }

    pub const fn installed(&self) -> bool {
        self.installed
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct McpStatus {
    clients: Vec<McpClientStatus>,
}

impl McpStatus {
    pub fn new(clients: Vec<McpClientStatus>) -> Self {
        Self { clients }
    }

    pub fn clients(&self) -> &[McpClientStatus] {
        &self.clients
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatusKind {
    NoUpdate,
    UpdateAvailable,
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
struct ContractVersion(#[schemars(regex(pattern = r"^[0-9]+\.[0-9]+\.[0-9]+$"))] String);

impl ContractVersion {
    fn new(value: impl Into<String>, field: &'static str) -> Result<Self, OutputValidationError> {
        let value = value.into();
        let mut parts = value.split('.');
        let valid = (0..3).all(|_| {
            parts.next().is_some_and(|part| {
                !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
            })
        }) && parts.next().is_none();
        if !valid {
            return Err(OutputValidationError::InvalidVersion(field));
        }
        Ok(Self(value))
    }
}

impl<'de> Deserialize<'de> for ContractVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?, "version").map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateStatus {
    status: UpdateStatusKind,
    current_version: ContractVersion,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    available_version: Option<ContractVersion>,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    manager_command: Option<String>,
}

impl UpdateStatus {
    pub fn new(
        status: UpdateStatusKind,
        current_version: impl Into<String>,
        available_version: Option<String>,
        manager_command: Option<String>,
    ) -> Result<Self, OutputValidationError> {
        Ok(Self {
            status,
            current_version: ContractVersion::new(current_version, "current version")?,
            available_version: available_version
                .map(|version| ContractVersion::new(version, "available version"))
                .transpose()?,
            manager_command,
        })
    }

    pub const fn status(&self) -> UpdateStatusKind {
        self.status
    }

    pub fn current_version(&self) -> &str {
        &self.current_version.0
    }

    pub fn available_version(&self) -> Option<&str> {
        self.available_version
            .as_ref()
            .map(|version| version.0.as_str())
    }

    pub fn manager_command(&self) -> Option<&str> {
        self.manager_command.as_deref()
    }
}

/// Timing metadata shared by success and failure envelopes.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvelopeMeta {
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<UtcTimestamp>,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<UtcTimestamp>,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    failed_at: Option<UtcTimestamp>,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
}

impl EnvelopeMeta {
    #[must_use]
    pub const fn with_started_at(mut self, started_at: UtcTimestamp) -> Self {
        self.started_at = Some(started_at);
        self
    }

    #[must_use]
    pub const fn with_completed_at(mut self, completed_at: UtcTimestamp) -> Self {
        self.completed_at = Some(completed_at);
        self
    }

    #[must_use]
    pub const fn with_failed_at(mut self, failed_at: UtcTimestamp) -> Self {
        self.failed_at = Some(failed_at);
        self
    }

    #[must_use]
    pub const fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }
}

/// A non-empty ordered set produced by repeated-file commands.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct NonEmptyAnalyses(#[schemars(length(min = 1))] Vec<Analysis<CanonicalError>>);

impl NonEmptyAnalyses {
    pub fn new(analyses: Vec<Analysis<CanonicalError>>) -> Result<Self, OutputValidationError> {
        if analyses.is_empty() {
            return Err(OutputValidationError::EmptyValue("analysis output"));
        }
        Ok(Self(analyses))
    }

    #[must_use]
    pub fn as_slice(&self) -> &[Analysis<CanonicalError>] {
        &self.0
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<Analysis<CanonicalError>> {
        self.0
    }
}

impl<'de> Deserialize<'de> for NonEmptyAnalyses {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// One analysis or a non-empty ordered set produced by repeated-file commands.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum AnalysisOutput {
    One(Box<Analysis<CanonicalError>>),
    Many(NonEmptyAnalyses),
}

impl AnalysisOutput {
    pub fn one(analysis: Analysis<CanonicalError>) -> Self {
        Self::One(Box::new(analysis))
    }

    pub fn many(analyses: Vec<Analysis<CanonicalError>>) -> Result<Self, OutputValidationError> {
        NonEmptyAnalyses::new(analyses).map(Self::Many)
    }
}

impl<'de> Deserialize<'de> for AnalysisOutput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        if value.is_array() {
            serde_json::from_value(value)
                .map(Self::Many)
                .map_err(D::Error::custom)
        } else {
            serde_json::from_value(value)
                .map(Box::new)
                .map(Self::One)
                .map_err(D::Error::custom)
        }
    }
}

macro_rules! command_data {
    ($($variant:ident($payload:ty) => $command:ident),+ $(,)?) => {
        /// Closed command-specific success data. Each variant owns its resolved command.
        #[derive(Debug, Clone, PartialEq, Serialize)]
        #[serde(untagged)]
        #[allow(clippy::large_enum_variant)]
        pub enum CommandData {
            $($variant($payload)),+
        }

        impl CommandData {
            pub const fn command(&self) -> ResolvedCommand {
                match self {
                    $(Self::$variant(_) => ResolvedCommand::$command),+
                }
            }

            fn from_value(command: ResolvedCommand, value: Value) -> Result<Self, serde_json::Error> {
                match command {
                    $(
                        ResolvedCommand::$command => {
                            serde_json::from_value::<$payload>(value).map(Self::$variant)
                        }
                    )+
                    _ => Err(serde_json::Error::custom(
                        OutputValidationError::NonEnvelopeCommand(command),
                    )),
                }
            }
        }
    };
}

command_data! {
    Detect(AnalysisOutput) => Detect,
    Plagiarism(AnalysisOutput) => Plagiarism,
    Analyze(AnalysisOutput) => Analyze,
    BulkSubmit(BulkCollection) => BulkSubmit,
    BulkStatus(BulkCollection) => BulkStatus,
    BulkWait(BulkCollection) => BulkWait,
    BulkResults(BulkPage<CanonicalError>) => BulkResults,
    TaskStatus(Analysis<CanonicalError>) => TaskStatus,
    TaskWait(Analysis<CanonicalError>) => TaskWait,
    HistoryList(AnalysisPage<CanonicalError>) => HistoryList,
    HistorySearch(AnalysisPage<CanonicalError>) => HistorySearch,
    HistoryShow(Analysis<CanonicalError>) => HistoryShow,
    HistoryRerun(Analysis<CanonicalError>) => HistoryRerun,
    HistoryDelete(MutationAcknowledgement) => HistoryDelete,
    HistoryClear(MutationAcknowledgement) => HistoryClear,
    AuthSet(MutationAcknowledgement) => AuthSet,
    AuthStatus(AuthStatus) => AuthStatus,
    AuthLogout(MutationAcknowledgement) => AuthLogout,
    ConfigList(ConfigListStatus) => ConfigList,
    ConfigGet(ConfigGetStatus) => ConfigGet,
    ConfigSet(MutationAcknowledgement) => ConfigSet,
    ConfigPath(ConfigPathStatus) => ConfigPath,
    Doctor(DoctorStatus) => Doctor,
    McpInstall(MutationAcknowledgement) => McpInstall,
    McpUninstall(MutationAcknowledgement) => McpUninstall,
    McpStatus(McpStatus) => McpStatus,
    UpdateCheck(UpdateStatus) => UpdateCheck,
    UpdateInstall(UpdateStatus) => UpdateInstall,
}

/// A single canonical envelope with success and failure made exclusive by construction.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandEnvelope {
    Success {
        data: CommandData,
        meta: EnvelopeMeta,
    },
    Failure {
        command: ResolvedCommand,
        error: CanonicalError,
        meta: EnvelopeMeta,
    },
}

impl CommandEnvelope {
    pub fn success(data: CommandData, meta: EnvelopeMeta) -> Self {
        Self::Success { data, meta }
    }

    pub fn failure(command: ResolvedCommand, error: CanonicalError, meta: EnvelopeMeta) -> Self {
        Self::Failure {
            command,
            error,
            meta,
        }
    }
}

impl Serialize for CommandEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut envelope = serializer.serialize_struct("CommandEnvelope", 4)?;
        envelope.serialize_field("schema_version", &OutputSchemaVersion::V1)?;
        match self {
            Self::Success { data, meta } => {
                envelope.serialize_field("command", &data.command())?;
                envelope.serialize_field("data", data)?;
                envelope.serialize_field("meta", meta)?;
            }
            Self::Failure {
                command,
                error,
                meta,
            } => {
                envelope.serialize_field("command", command)?;
                envelope.serialize_field("error", error)?;
                envelope.serialize_field("meta", meta)?;
            }
        }
        envelope.end()
    }
}

#[derive(Deserialize)]
struct CommandEnvelopeWire {
    schema_version: OutputSchemaVersion,
    command: ResolvedCommand,
    #[serde(default, deserialize_with = "deserialize_non_null_value")]
    data: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    error: Option<CanonicalError>,
    meta: EnvelopeMeta,
}

impl<'de> Deserialize<'de> for CommandEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CommandEnvelopeWire::deserialize(deserializer)?;
        let _ = wire.schema_version;
        match (wire.data, wire.error) {
            (Some(data), None) => CommandData::from_value(wire.command, data)
                .map(|data| Self::success(data, wire.meta))
                .map_err(D::Error::custom),
            (None, Some(error)) => Ok(Self::failure(wire.command, error, wire.meta)),
            _ => Err(D::Error::custom(
                "an envelope must contain exactly one of data or error",
            )),
        }
    }
}
