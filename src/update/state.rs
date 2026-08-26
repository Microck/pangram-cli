//! Direct-install ownership and local TUI update-check state.

use std::fs;
use std::path::{Component, Path};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Target, UpdateError, UpdateErrorKind, parse_version};
use crate::domain::{Sha256Hash, UtcTimestamp, deserialize_missing_only};

const UPDATE_STATE_FILE_NAME: &str = "update-state.json";
const NPM_PLATFORM_PACKAGES: [&str; 5] = [
    "/node_modules/@microck/pangram-cli-darwin-arm64/",
    "/node_modules/@microck/pangram-cli-darwin-x64/",
    "/node_modules/@microck/pangram-cli-linux-arm64/",
    "/node_modules/@microck/pangram-cli-linux-x64/",
    "/node_modules/@microck/pangram-cli-win32-x64/",
];

/// Package managers recognized only for update instructions. Detection never
/// creates a direct-install receipt or permits binary replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallManager {
    Homebrew,
    Scoop,
    Npm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ManagerAdvisory {
    manager: InstallManager,
    command: &'static str,
}

impl ManagerAdvisory {
    #[must_use]
    pub const fn manager(self) -> InstallManager {
        self.manager
    }

    #[must_use]
    pub const fn command(self) -> &'static str {
        self.command
    }
}

/// Recognizes conventional manager-owned paths for advice only. A caller must
/// still require a valid direct receipt before any mutation.
#[must_use]
pub fn detect_manager_install(executable: &Path) -> Option<ManagerAdvisory> {
    let normalized = executable.to_string_lossy().replace('\\', "/");
    if normalized.contains("/Cellar/pangram/") {
        return Some(ManagerAdvisory {
            manager: InstallManager::Homebrew,
            command: "brew upgrade pangram",
        });
    }
    if normalized
        .to_ascii_lowercase()
        .contains("/scoop/apps/pangram/")
    {
        return Some(ManagerAdvisory {
            manager: InstallManager::Scoop,
            command: "scoop update pangram",
        });
    }
    if NPM_PLATFORM_PACKAGES
        .iter()
        .any(|package| normalized.contains(package))
    {
        return Some(ManagerAdvisory {
            manager: InstallManager::Npm,
            command: "npm update --global @microck/pangram-cli",
        });
    }
    None
}

/// Owner-only evidence that the current executable came from a direct install.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallReceipt {
    schema_version: String,
    method: String,
    #[schemars(length(min = 1))]
    executable_path: String,
    #[schemars(regex(pattern = r"^[0-9]+\.[0-9]+\.[0-9]+$"))]
    installed_version: String,
    target: Target,
    manifest_sha256: Sha256Hash,
    installed_at: UtcTimestamp,
}

impl InstallReceipt {
    pub fn new(
        executable_path: &Path,
        installed_version: impl Into<String>,
        target: Target,
        manifest_sha256: Sha256Hash,
        installed_at: UtcTimestamp,
    ) -> Result<Self, UpdateError> {
        let executable_path = executable_path
            .to_str()
            .ok_or_else(invalid_receipt)?
            .to_owned();
        let receipt = Self {
            schema_version: "1".into(),
            method: "direct".into(),
            executable_path,
            installed_version: installed_version.into(),
            target,
            manifest_sha256,
            installed_at,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    #[must_use]
    pub fn executable_path(&self) -> &str {
        &self.executable_path
    }

    #[must_use]
    pub fn installed_version(&self) -> &str {
        &self.installed_version
    }

    #[must_use]
    pub const fn target(&self) -> Target {
        self.target
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> Sha256Hash {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn installed_at(&self) -> UtcTimestamp {
        self.installed_at
    }

    fn validate(&self) -> Result<(), UpdateError> {
        let executable_path = Path::new(&self.executable_path);
        if self.schema_version != "1"
            || self.method != "direct"
            || !is_clean_absolute_path(executable_path)
            || parse_version(&self.installed_version).is_err()
        {
            return Err(invalid_receipt());
        }
        Ok(())
    }
}

pub(super) fn parse_install_receipt(receipt_bytes: &[u8]) -> Result<InstallReceipt, UpdateError> {
    let receipt: InstallReceipt =
        serde_json::from_slice(receipt_bytes).map_err(|_| invalid_receipt())?;
    receipt.validate()?;
    Ok(receipt)
}

/// Parses a receipt and proves that it owns this exact executable instance.
pub fn validate_install_receipt(
    receipt_bytes: &[u8],
    current_executable: &Path,
    current_version: &str,
    current_target: Target,
) -> Result<InstallReceipt, UpdateError> {
    let receipt = parse_install_receipt(receipt_bytes)?;
    validate_parsed_install_receipt(
        &receipt,
        current_executable,
        current_version,
        current_target,
    )?;
    Ok(receipt)
}

pub(super) fn validate_parsed_install_receipt(
    receipt: &InstallReceipt,
    current_executable: &Path,
    current_version: &str,
    current_target: Target,
) -> Result<(), UpdateError> {
    if !is_clean_absolute_path(current_executable)
        || Path::new(&receipt.executable_path) != current_executable
        || receipt.installed_version != current_version
        || receipt.target != current_target
    {
        return Err(UpdateError::new(
            UpdateErrorKind::InstallNotOwned,
            "The current executable is not owned by this direct-install receipt.",
        ));
    }
    Ok(())
}

/// Local TUI update-check state. CLI and MCP never consult it automatically.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateState {
    schema_version: String,
    last_checked_at: UtcTimestamp,
    #[serde(
        default,
        deserialize_with = "deserialize_missing_only",
        skip_serializing_if = "Option::is_none"
    )]
    etag: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_missing_only",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(regex(pattern = r"^[0-9]+\.[0-9]+\.[0-9]+$"))]
    available_version: Option<String>,
}

impl UpdateState {
    pub fn checked(
        checked_at: UtcTimestamp,
        etag: Option<String>,
        available_version: Option<String>,
    ) -> Result<Self, UpdateError> {
        if available_version
            .as_deref()
            .is_some_and(|version| parse_version(version).is_err())
        {
            return Err(invalid_state());
        }
        Ok(Self {
            schema_version: "1".into(),
            last_checked_at: checked_at,
            etag,
            available_version,
        })
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, UpdateError> {
        let state: Self = serde_json::from_slice(bytes).map_err(|_| invalid_state())?;
        if state.schema_version != "1"
            || state
                .available_version
                .as_deref()
                .is_some_and(|version| parse_version(version).is_err())
        {
            return Err(invalid_state());
        }
        Ok(state)
    }

    /// Clock rollback is due immediately; otherwise checks are at least 24
    /// hours apart, including subsecond timestamps.
    #[must_use]
    pub fn should_check(&self, now: UtcTimestamp) -> bool {
        let now = now.get();
        let last = self.last_checked_at.get();
        if now < last {
            return true;
        }
        let elapsed_seconds = now.as_second() - last.as_second();
        elapsed_seconds > 86_400
            || (elapsed_seconds == 86_400 && now.subsec_nanosecond() >= last.subsec_nanosecond())
    }

    #[must_use]
    pub fn not_modified(&self, checked_at: UtcTimestamp) -> Self {
        Self {
            schema_version: "1".into(),
            last_checked_at: checked_at,
            etag: self.etag.clone(),
            available_version: self.available_version.clone(),
        }
    }

    #[must_use]
    pub const fn last_checked_at(&self) -> UtcTimestamp {
        self.last_checked_at
    }

    #[must_use]
    pub fn etag(&self) -> Option<&str> {
        self.etag.as_deref()
    }

    #[must_use]
    pub fn available_version(&self) -> Option<&str> {
        self.available_version.as_deref()
    }
}

/// Loads owner-only update-check state. A missing file means no check has run.
pub fn load_update_state(data_dir: &Path) -> Result<Option<UpdateState>, UpdateError> {
    let path = data_dir.join(UPDATE_STATE_FILE_NAME);
    match fs::metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return Err(invalid_state()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(invalid_state()),
    }
    crate::config::enforce_protected_permissions(&path).map_err(|_| invalid_state())?;
    let bytes = fs::read(path).map_err(|_| invalid_state())?;
    UpdateState::parse(&bytes).map(Some)
}

/// Atomically stores update-check state with the shared owner-only file rule.
pub fn store_update_state(data_dir: &Path, state: &UpdateState) -> Result<(), UpdateError> {
    let path = data_dir.join(UPDATE_STATE_FILE_NAME);
    let mut bytes = serde_json::to_vec_pretty(state).map_err(|_| invalid_state())?;
    bytes.push(b'\n');
    crate::config::atomic_secret_write(&path, &bytes).map_err(|_| invalid_state())
}

fn is_clean_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
}

const fn invalid_receipt() -> UpdateError {
    UpdateError::new(
        UpdateErrorKind::InstallReceiptInvalid,
        "The direct-install receipt is invalid.",
    )
}

const fn invalid_state() -> UpdateError {
    UpdateError::new(
        UpdateErrorKind::UpdateStateInvalid,
        "The local update-check state is invalid.",
    )
}
