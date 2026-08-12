//! Source-preserving MCP client registration.
//!
//! The installer is a deep module: callers select targets and provide the
//! server name, while this module owns every client path, entry shape,
//! conflict rule, race check, and atomic filesystem mutation. Planning is a
//! separate operation so `--dry-run` can report the same changes that a real
//! invocation would attempt without opening a write path.

use std::fmt;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use thiserror::Error;

mod jsonc;
mod paths;
mod toml_config;

use jsonc::{JsonFormat, edit_json};
use paths::InstallContext;
#[cfg(test)]
use paths::InstallPlatform;
use toml_config::edit_codex_toml;

/// One client whose global user configuration Pangram can inspect or edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ClientTarget {
    ClaudeCode,
    ClaudeDesktop,
    Codex,
    Cursor,
    Vscode,
    Windsurf,
    Gemini,
    OpenCode,
    Cline,
    RooCode,
    Droid,
    Antigravity,
}

impl ClientTarget {
    pub const ALL: &'static [Self] = &[
        Self::ClaudeCode,
        Self::ClaudeDesktop,
        Self::Codex,
        Self::Cursor,
        Self::Vscode,
        Self::Windsurf,
        Self::Gemini,
        Self::OpenCode,
        Self::Cline,
        Self::RooCode,
        Self::Droid,
        Self::Antigravity,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::ClaudeDesktop => "claude-desktop",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::Vscode => "vscode",
            Self::Windsurf => "windsurf",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
            Self::Cline => "cline",
            Self::RooCode => "roo-code",
            Self::Droid => "droid",
            Self::Antigravity => "antigravity",
        }
    }
}

impl fmt::Display for ClientTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ClientTarget {
    type Err = InstallError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|target| target.as_str() == value)
            .ok_or_else(|| InstallError::UnknownTarget(value.to_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallAction {
    Install,
    Uninstall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallChange {
    Create,
    Update,
    Remove,
    Unchanged,
}

/// Inputs shared by planning and execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRequest {
    action: InstallAction,
    targets: Vec<ClientTarget>,
    server_name: String,
    dry_run: bool,
}

impl InstallRequest {
    pub fn new(
        action: InstallAction,
        targets: Vec<ClientTarget>,
        server_name: impl Into<String>,
        dry_run: bool,
    ) -> Result<Self, InstallError> {
        let server_name = server_name.into();
        if server_name.is_empty()
            || server_name.len() > 128
            || server_name.chars().any(char::is_control)
        {
            return Err(InstallError::InvalidServerName);
        }
        if targets.is_empty() {
            return Err(InstallError::NoTargets);
        }
        let mut unique = Vec::with_capacity(targets.len());
        for target in targets {
            if !unique.contains(&target) {
                unique.push(target);
            }
        }
        Ok(Self {
            action,
            targets: unique,
            server_name,
            dry_run,
        })
    }

    pub const fn action(&self) -> InstallAction {
        self.action
    }

    #[cfg(test)]
    pub fn targets(&self) -> &[ClientTarget] {
        &self.targets
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }
}

pub struct TargetPlan {
    target: ClientTarget,
    path: PathBuf,
    change: InstallChange,
    reason: Option<&'static str>,
    expected: Option<Vec<u8>>,
    replacement: Option<Vec<u8>>,
    permissions: Option<fs::Permissions>,
}

pub struct InstallPlan {
    targets: Vec<TargetPlan>,
    dry_run: bool,
}

#[derive(Debug)]
pub struct InstallReport {
    targets: Vec<TargetReport>,
    dry_run: bool,
}

impl InstallReport {
    #[cfg(test)]
    pub fn targets(&self) -> &[TargetReport] {
        &self.targets
    }

    #[cfg(test)]
    pub fn changed(&self) -> usize {
        self.targets
            .iter()
            .filter(|target| target.change != InstallChange::Unchanged)
            .count()
    }

    #[cfg(test)]
    pub const fn dry_run(&self) -> bool {
        self.dry_run
    }

    /// Projects the filesystem report into the closed command-output type.
    /// Keeping this mapping here prevents the CLI adapter from duplicating
    /// installer action or client-name knowledge.
    pub fn to_output(
        &self,
    ) -> Result<crate::output::McpMutationReport, crate::output::OutputValidationError> {
        let targets = self
            .targets
            .iter()
            .map(|target| {
                let action = match target.change {
                    InstallChange::Create => crate::output::McpMutationAction::Create,
                    InstallChange::Update => crate::output::McpMutationAction::Update,
                    InstallChange::Remove => crate::output::McpMutationAction::Remove,
                    InstallChange::Unchanged => crate::output::McpMutationAction::Unchanged,
                };
                crate::output::McpMutationTarget::new(
                    target.target.as_str(),
                    &target.path,
                    action,
                    target.reason.map(str::to_owned),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        crate::output::McpMutationReport::new(self.dry_run, targets)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetReport {
    target: ClientTarget,
    path: PathBuf,
    change: InstallChange,
    reason: Option<&'static str>,
}

impl TargetReport {
    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
    #[cfg(test)]
    pub const fn change(&self) -> InstallChange {
        self.change
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientStatus {
    target: ClientTarget,
    path: Option<PathBuf>,
    installed: bool,
}

impl ClientStatus {
    pub const fn target(&self) -> ClientTarget {
        self.target
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub const fn installed(&self) -> bool {
        self.installed
    }
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("unknown MCP client target {0}")]
    UnknownTarget(String),
    #[error("at least one MCP client target is required")]
    NoTargets,
    #[error("MCP server name must be 1 through 128 characters with no control characters")]
    InvalidServerName,
    #[error("could not resolve the current user's home directory")]
    MissingHome,
    #[error("could not resolve {variable} for {target}")]
    MissingEnvironment {
        target: ClientTarget,
        variable: &'static str,
    },
    #[error("{target} configuration path is not safely discoverable: {reason}")]
    UnsupportedPath {
        target: ClientTarget,
        reason: &'static str,
    },
    #[error("{path} is a symbolic link; refusing to edit it")]
    Symlink { path: PathBuf },
    #[error("malformed {target} MCP configuration at {path}: {message}")]
    Malformed {
        target: ClientTarget,
        path: PathBuf,
        message: String,
    },
    #[error("duplicate key {key:?} in {target} MCP configuration at {path}")]
    DuplicateKey {
        target: ClientTarget,
        path: PathBuf,
        key: String,
    },
    #[error("MCP server {server_name:?} conflicts with an existing {target} entry at {path}")]
    Conflict {
        target: ClientTarget,
        path: PathBuf,
        server_name: String,
    },
    #[error("{path} changed after the MCP installation plan was created")]
    ConcurrentChange { path: PathBuf },
    #[error(
        "MCP installer stopped after changing {completed}; later targets were not changed: {cause}"
    )]
    PartialWrite {
        completed: String,
        cause: Box<InstallError>,
    },
    #[error("MCP installer filesystem operation failed: {0}")]
    Io(String),
}

/// Global-client installer and status reader.
#[derive(Debug, Clone)]
pub struct Installer {
    context: InstallContext,
}

impl Installer {
    pub fn from_process() -> Result<Self, InstallError> {
        Ok(Self {
            context: InstallContext::from_process()?,
        })
    }

    #[cfg(test)]
    pub const fn with_context(context: InstallContext) -> Self {
        Self { context }
    }

    /// Builds an all-target plan before any mutation. A conflict in one target
    /// therefore prevents writes to every selected target.
    pub fn plan(&self, request: &InstallRequest) -> Result<InstallPlan, InstallError> {
        let mut targets = Vec::with_capacity(request.targets.len());
        for target in &request.targets {
            targets.push(self.plan_target(*target, request)?);
        }
        Ok(InstallPlan {
            targets,
            dry_run: request.dry_run,
        })
    }

    pub fn apply(&self, request: InstallRequest) -> Result<InstallReport, InstallError> {
        let plan = self.plan(&request)?;
        self.apply_plan(plan)
    }

    pub fn apply_plan(&self, plan: InstallPlan) -> Result<InstallReport, InstallError> {
        if !plan.dry_run {
            for (index, target) in plan.targets.iter().enumerate() {
                if target.change == InstallChange::Unchanged {
                    continue;
                }
                let Err(cause) = write_guarded(target) else {
                    continue;
                };
                if index == 0 {
                    return Err(cause);
                }
                let completed = plan.targets[..index]
                    .iter()
                    .filter(|target| target.change != InstallChange::Unchanged)
                    .map(|target| {
                        format!(
                            "{} at {}",
                            target.target,
                            target.path.to_str().expect("validated UTF-8 path")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                if completed.is_empty() {
                    return Err(cause);
                }
                return Err(InstallError::PartialWrite {
                    completed,
                    cause: Box::new(cause),
                });
            }
        }
        let targets = plan
            .targets
            .into_iter()
            .map(|target| TargetReport {
                target: target.target,
                path: target.path,
                change: target.change,
                reason: target.reason,
            })
            .collect();
        Ok(InstallReport {
            targets,
            dry_run: plan.dry_run,
        })
    }

    pub fn status(
        &self,
        targets: &[ClientTarget],
        server_name: &str,
    ) -> Result<Vec<ClientStatus>, InstallError> {
        let mut statuses = Vec::with_capacity(targets.len());
        for target in targets {
            let path = match self.path_for(*target) {
                Ok(path) => path,
                Err(InstallError::UnsupportedPath { .. }) => {
                    statuses.push(ClientStatus {
                        target: *target,
                        path: None,
                        installed: false,
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };
            let request =
                InstallRequest::new(InstallAction::Install, vec![*target], server_name, true)?;
            let installed = match self.plan_target(*target, &request) {
                Ok(planned) => planned.change == InstallChange::Unchanged,
                // Status answers exact Pangram ownership. A same-named entry
                // owned by the user or another server is not installed; it is
                // a mutation conflict only when install/uninstall is asked to
                // change it.
                Err(InstallError::Conflict { .. }) => false,
                Err(error) => return Err(error),
            };
            statuses.push(ClientStatus {
                target: *target,
                path: Some(path),
                installed,
            });
        }
        Ok(statuses)
    }

    fn plan_target(
        &self,
        target: ClientTarget,
        request: &InstallRequest,
    ) -> Result<TargetPlan, InstallError> {
        let path = self.path_for(target)?;
        let (source, permissions) = read_regular_or_missing(&path)?;
        let source_text = match &source {
            Some(bytes) => std::str::from_utf8(bytes).map_err(|_| InstallError::Malformed {
                target,
                path: path.clone(),
                message: "configuration is not UTF-8".into(),
            })?,
            None => "",
        };
        let executable = self
            .context
            .executable
            .to_str()
            .ok_or_else(|| InstallError::Io("Pangram executable path is not UTF-8".into()))?;
        if !self.context.executable.is_absolute() {
            return Err(InstallError::Io(
                "Pangram executable path must be absolute".into(),
            ));
        }
        let edited = if target == ClientTarget::Codex {
            edit_codex_toml(
                source_text,
                request.server_name(),
                executable,
                request.action(),
            )
            .map_err(|error| map_format_error(target, &path, request.server_name(), error))?
        } else {
            let spec = json_spec(target, executable);
            edit_json(
                source_text,
                spec.container,
                request.server_name(),
                &spec.entry,
                request.action(),
                spec.format,
            )
            .map_err(|error| map_format_error(target, &path, request.server_name(), error))?
        };
        let change = match (edited.change, source.is_some()) {
            (InstallChange::Create, true) => InstallChange::Update,
            (change, _) => change,
        };
        let reason = match (request.action(), change) {
            (InstallAction::Install, InstallChange::Unchanged) => {
                Some("The exact Pangram entry is already installed.")
            }
            (InstallAction::Uninstall, InstallChange::Unchanged) => {
                Some("No exact Pangram-owned entry is installed.")
            }
            _ => None,
        };
        Ok(TargetPlan {
            target,
            path,
            change,
            reason,
            expected: source,
            replacement: edited.replacement.map(String::into_bytes),
            permissions,
        })
    }
}

struct JsonSpec {
    container: &'static str,
    entry: Value,
    format: JsonFormat,
}

fn json_spec(target: ClientTarget, executable: &str) -> JsonSpec {
    let (container, entry, format) = match target {
        ClientTarget::ClaudeCode | ClientTarget::Cursor | ClientTarget::Droid => (
            "mcpServers",
            json!({"type":"stdio","command":executable,"args":["mcp"]}),
            JsonFormat::Strict,
        ),
        ClientTarget::ClaudeDesktop | ClientTarget::Antigravity => (
            "mcpServers",
            json!({"command":executable,"args":["mcp"]}),
            JsonFormat::Strict,
        ),
        ClientTarget::Windsurf | ClientTarget::Gemini => (
            "mcpServers",
            json!({"command":executable,"args":["mcp"]}),
            JsonFormat::Jsonc,
        ),
        ClientTarget::Vscode => (
            "servers",
            json!({"type":"stdio","command":executable,"args":["mcp"]}),
            JsonFormat::Jsonc,
        ),
        ClientTarget::OpenCode => (
            "mcp",
            json!({"type":"local","command":[executable,"mcp"]}),
            JsonFormat::Jsonc,
        ),
        ClientTarget::Cline => (
            "mcpServers",
            json!({"transport":{"type":"stdio","command":executable,"args":["mcp"]}}),
            JsonFormat::Strict,
        ),
        ClientTarget::Codex | ClientTarget::RooCode => unreachable!("non-JSON target"),
    };
    JsonSpec {
        container,
        entry,
        format,
    }
}

fn map_format_error(
    target: ClientTarget,
    path: &Path,
    server_name: &str,
    error: jsonc::EditError,
) -> InstallError {
    match error {
        jsonc::EditError::Malformed(message) => InstallError::Malformed {
            target,
            path: path.to_path_buf(),
            message,
        },
        jsonc::EditError::DuplicateKey(key) => InstallError::DuplicateKey {
            target,
            path: path.to_path_buf(),
            key,
        },
        jsonc::EditError::Conflict => InstallError::Conflict {
            target,
            path: path.to_path_buf(),
            server_name: server_name.to_owned(),
        },
    }
}

fn read_regular_or_missing(
    path: &Path,
) -> Result<(Option<Vec<u8>>, Option<fs::Permissions>), InstallError> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsMaybeDirExt as _};

    let parent = path
        .parent()
        .ok_or_else(|| InstallError::Io(format!("{} has no parent", path.display())))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| InstallError::Io(format!("{} has no file name", path.display())))?;
    let directory = match cap_std::fs::Dir::open_ambient_dir(parent, cap_std::ambient_authority()) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((None, None)),
        Err(error) => return Err(io_error(error)),
    };
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .read(true)
        .follow(FollowSymlinks::No)
        // Windows otherwise rejects a directory before the opened-handle
        // metadata can preserve the existing non-regular-file error.
        .maybe_dir(true);
    let file = match directory.open_with(file_name, &options) {
        Ok(file) => file,
        Err(error) => {
            return match directory.symlink_metadata(file_name) {
                Ok(metadata) if metadata.file_type().is_symlink() => Err(InstallError::Symlink {
                    path: path.to_path_buf(),
                }),
                Err(metadata_error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && metadata_error.kind() == std::io::ErrorKind::NotFound =>
                {
                    Ok((None, None))
                }
                _ => Err(io_error(error)),
            };
        }
    };

    // Metadata and bytes come from the same no-follow handle. A concurrent
    // path replacement can change what later callers see at `path`, but it
    // cannot redirect this read to the replacement or a symlink referent.
    let mut file = file.into_std();
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() {
        return Err(InstallError::Io(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    let permissions = metadata.permissions();
    let mut source = Vec::new();
    file.read_to_end(&mut source).map_err(io_error)?;
    Ok((Some(source), Some(permissions)))
}

fn write_guarded(plan: &TargetPlan) -> Result<(), InstallError> {
    let parent = plan
        .path
        .parent()
        .ok_or_else(|| InstallError::Io(format!("{} has no parent", plan.path.display())))?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let lock_path = parent.join(".pangram-mcp.lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        // This persistent sidecar carries no payload. Opening it must never
        // shorten an existing inode while another process may hold the lock.
        .truncate(false)
        .open(&lock_path)
        .map_err(io_error)?;
    fs4::FileExt::lock(&lock).map_err(io_error)?;

    let current = read_regular_or_missing(&plan.path)?.0;
    if current != plan.expected {
        return Err(InstallError::ConcurrentChange {
            path: plan.path.clone(),
        });
    }
    let replacement = plan
        .replacement
        .as_deref()
        .ok_or_else(|| InstallError::Io("changed plan did not contain replacement bytes".into()))?;
    atomic_replace(&plan.path, replacement, plan.permissions.as_ref())
}

fn atomic_replace(
    path: &Path,
    contents: &[u8],
    permissions: Option<&fs::Permissions>,
) -> Result<(), InstallError> {
    let parent = path.parent().expect("validated parent");
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| InstallError::Io(format!("{} is not valid UTF-8", path.display())))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary = parent.join(format!(
        ".{file_name}.{}-{nonce}.pangram.tmp",
        std::process::id()
    ));
    let result = (|| -> Result<(), InstallError> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            use std::os::unix::fs::PermissionsExt as _;
            options.mode(permissions.map(fs::Permissions::mode).unwrap_or(0o600));
        }
        let mut file = options.open(&temporary).map_err(io_error)?;
        file.write_all(contents).map_err(io_error)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions.clone())
                .map_err(io_error)?;
        }
        file.sync_all().map_err(io_error)?;
        drop(file);
        replace_file(&temporary, path)?;
        #[cfg(unix)]
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(temporary: &Path, path: &Path) -> Result<(), InstallError> {
    fs::rename(temporary, path).map_err(io_error)
}

#[cfg(windows)]
fn replace_file(temporary: &Path, path: &Path) -> Result<(), InstallError> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let mut from = temporary.as_os_str().encode_wide().collect::<Vec<_>>();
    from.push(0);
    let mut to = path.as_os_str().encode_wide().collect::<Vec<_>>();
    to.push(0);
    // SAFETY: both pointers address NUL-terminated UTF-16 buffers for the
    // duration of the call. Flags request same-directory atomic replacement.
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

fn io_error(error: impl fmt::Display) -> InstallError {
    let _ = error;
    InstallError::Io("local filesystem access failed".into())
}

#[cfg(test)]
mod tests;
