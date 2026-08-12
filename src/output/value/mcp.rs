//! Canonical MCP installer mutation values.
//!
//! These types own the cross-platform path validation and closed serialized
//! shapes for install, uninstall, and dry-run reports. The parent `value`
//! module re-exports them so the public output interface stays unchanged.

use std::path::Path;

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::domain::NonEmptyString;

use super::{OutputValidationError, deserialize_missing_only, required_string};

/// The exact filesystem change made, or planned by a dry run, for one MCP
/// client configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpMutationAction {
    Create,
    Update,
    Remove,
    Unchanged,
}

/// One ordered MCP client configuration result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpMutationTarget {
    client: NonEmptyString,
    path: NonEmptyString,
    action: McpMutationAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<NonEmptyString>,
}

impl McpMutationTarget {
    pub fn new(
        client: impl Into<String>,
        path: impl AsRef<Path>,
        action: McpMutationAction,
        reason: Option<String>,
    ) -> Result<Self, OutputValidationError> {
        let path = path
            .as_ref()
            .to_str()
            .ok_or(OutputValidationError::NonUtf8McpMutationPath)?;
        Self::from_parts(client.into(), path.to_owned(), action, reason)
    }

    fn from_parts(
        client: String,
        path: String,
        action: McpMutationAction,
        reason: Option<String>,
    ) -> Result<Self, OutputValidationError> {
        if !is_portable_absolute_path(&path) {
            return Err(OutputValidationError::RelativeMcpMutationPath);
        }
        let reason = reason
            .map(|reason| {
                if reason.chars().any(char::is_control) {
                    return Err(OutputValidationError::UnsafeMcpMutationReason);
                }
                required_string(reason, "MCP mutation reason")
            })
            .transpose()?;
        Ok(Self {
            client: required_string(client, "MCP client")?,
            path: required_string(path, "MCP mutation path")?,
            action,
            reason,
        })
    }

    pub fn client(&self) -> &str {
        self.client.as_str()
    }

    pub fn path(&self) -> &str {
        self.path.as_str()
    }

    pub const fn action(&self) -> McpMutationAction {
        self.action
    }

    pub fn reason(&self) -> Option<&str> {
        self.reason.as_ref().map(NonEmptyString::as_str)
    }
}

impl<'de> Deserialize<'de> for McpMutationTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            client: String,
            path: String,
            action: McpMutationAction,
            #[serde(default, deserialize_with = "deserialize_missing_only")]
            reason: Option<String>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::from_parts(wire.client, wire.path, wire.action, wire.reason).map_err(D::Error::custom)
    }
}

/// Successful MCP installer output. Target order is part of the public
/// contract, so this type preserves the caller-selected order without sorting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpMutationReport {
    dry_run: bool,
    #[schemars(length(min = 1))]
    targets: Vec<McpMutationTarget>,
}

impl McpMutationReport {
    pub fn new(
        dry_run: bool,
        targets: Vec<McpMutationTarget>,
    ) -> Result<Self, OutputValidationError> {
        if targets.is_empty() {
            return Err(OutputValidationError::EmptyValue("MCP mutation targets"));
        }
        Ok(Self { dry_run, targets })
    }

    pub const fn dry_run(&self) -> bool {
        self.dry_run
    }

    pub fn targets(&self) -> &[McpMutationTarget] {
        &self.targets
    }
}

impl<'de> Deserialize<'de> for McpMutationReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            dry_run: bool,
            targets: Vec<McpMutationTarget>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.dry_run, wire.targets).map_err(D::Error::custom)
    }
}

// Installer reports can be deserialized on a different operating system from
// the one which produced them. Native `Path::is_absolute` handles the current
// platform; the small Windows checks preserve valid drive and UNC paths on
// Unix without accepting drive-relative forms such as `C:config.toml`.
fn is_portable_absolute_path(path: &str) -> bool {
    if Path::new(path).is_absolute() {
        return true;
    }
    let bytes = path.as_bytes();
    let drive_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    let unc_absolute = (path.starts_with(r"\\") || path.starts_with("//"))
        && path[2..]
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .take(2)
            .count()
            == 2;
    drive_absolute || unc_absolute
}
