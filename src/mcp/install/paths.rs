//! Current global configuration paths for supported MCP clients.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{ClientTarget, InstallError, Installer, io_error};

// Tests construct every platform to verify the pinned path table. A normal
// build constructs only its native variant, so the other variants are
// deliberately dormant on that target.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallPlatform {
    Linux,
    Macos,
    Windows,
}

/// Process facts used for deterministic path resolution.
#[derive(Debug, Clone)]
pub struct InstallContext {
    pub(super) platform: InstallPlatform,
    pub(super) home: PathBuf,
    pub(super) executable: PathBuf,
    env_paths: BTreeMap<String, PathBuf>,
}

impl InstallContext {
    #[cfg(test)]
    pub fn for_test(platform: InstallPlatform, home: PathBuf) -> Self {
        Self {
            platform,
            home,
            executable: PathBuf::new(),
            env_paths: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn with_executable(mut self, executable: PathBuf) -> Self {
        self.executable = executable;
        self
    }

    #[cfg(test)]
    #[must_use]
    pub fn with_env_path(mut self, name: impl Into<String>, value: PathBuf) -> Self {
        self.env_paths.insert(name.into(), value);
        self
    }

    pub(super) fn from_process() -> Result<Self, InstallError> {
        let home = environment_path("HOME")
            .or_else(|| environment_path("USERPROFILE"))
            .ok_or(InstallError::MissingHome)?;
        let executable =
            fs::canonicalize(std::env::current_exe().map_err(io_error)?).map_err(io_error)?;
        let env_paths = [
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_USER_DATA_DIR",
            "CODEX_HOME",
            "GEMINI_CLI_HOME",
            "XDG_CONFIG_HOME",
            "APPDATA",
            "LOCALAPPDATA",
            "CLINE_MCP_SETTINGS_PATH",
            "CLINE_DATA_DIR",
            "CLINE_DIR",
        ]
        .into_iter()
        .filter_map(|name| environment_path(name).map(|value| (name.to_owned(), value)))
        .collect();
        #[cfg(target_os = "windows")]
        let platform = InstallPlatform::Windows;
        #[cfg(target_os = "macos")]
        let platform = InstallPlatform::Macos;
        #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
        let platform = InstallPlatform::Linux;
        Ok(Self {
            platform,
            home,
            executable,
            env_paths,
        })
    }

    fn env_path(&self, name: &str) -> Option<&Path> {
        self.env_paths.get(name).map(PathBuf::as_path)
    }
}

fn environment_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

impl Installer {
    pub fn path_for(&self, target: ClientTarget) -> Result<PathBuf, InstallError> {
        let home = &self.context.home;
        let xdg = self
            .context
            .env_path("XDG_CONFIG_HOME")
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".config"));
        let appdata = || {
            self.context
                .env_path("APPDATA")
                .ok_or(InstallError::MissingEnvironment {
                    target,
                    variable: "APPDATA",
                })
        };
        let path = match target {
            ClientTarget::ClaudeCode => self
                .context
                .env_path("CLAUDE_CONFIG_DIR")
                .unwrap_or(home)
                .join(".claude.json"),
            ClientTarget::ClaudeDesktop => {
                if let Some(root) = self.context.env_path("CLAUDE_USER_DATA_DIR") {
                    root.join("claude_desktop_config.json")
                } else {
                    match self.context.platform {
                        InstallPlatform::Linux => xdg.join("Claude/claude_desktop_config.json"),
                        InstallPlatform::Macos => home
                            .join("Library/Application Support/Claude/claude_desktop_config.json"),
                        InstallPlatform::Windows => {
                            let conventional = appdata()?.join("Claude");
                            let msix = self.context.env_path("LOCALAPPDATA").map(|root| {
                                root.join("Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude")
                            });
                            if conventional.is_dir() {
                                conventional.join("claude_desktop_config.json")
                            } else if let Some(msix) = msix.filter(|path| path.is_dir()) {
                                msix.join("claude_desktop_config.json")
                            } else {
                                conventional.join("claude_desktop_config.json")
                            }
                        }
                    }
                }
            }
            ClientTarget::Codex => self
                .context
                .env_path("CODEX_HOME")
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home.join(".codex"))
                .join("config.toml"),
            ClientTarget::Cursor => home.join(".cursor/mcp.json"),
            ClientTarget::Vscode => match self.context.platform {
                InstallPlatform::Linux => xdg.join("Code/User/mcp.json"),
                InstallPlatform::Macos => {
                    home.join("Library/Application Support/Code/User/mcp.json")
                }
                InstallPlatform::Windows => appdata()?.join("Code/User/mcp.json"),
            },
            ClientTarget::Windsurf => match self.context.platform {
                InstallPlatform::Windows => appdata()?.join("devin/mcp_config.json"),
                InstallPlatform::Linux | InstallPlatform::Macos => {
                    home.join(".config/devin/mcp_config.json")
                }
            },
            ClientTarget::Gemini => self
                .context
                .env_path("GEMINI_CLI_HOME")
                .unwrap_or(home)
                .join(".gemini/settings.json"),
            ClientTarget::OpenCode => {
                let root = xdg.join("opencode");
                let json = root.join("opencode.json");
                let jsonc = root.join("opencode.jsonc");
                if json.exists() || !jsonc.exists() {
                    json
                } else {
                    jsonc
                }
            }
            ClientTarget::Cline => {
                if let Some(path) = self.context.env_path("CLINE_MCP_SETTINGS_PATH") {
                    path.to_path_buf()
                } else if let Some(root) = self.context.env_path("CLINE_DATA_DIR") {
                    root.join("settings/cline_mcp_settings.json")
                } else if let Some(root) = self.context.env_path("CLINE_DIR") {
                    root.join("data/settings/cline_mcp_settings.json")
                } else {
                    home.join(".cline/data/settings/cline_mcp_settings.json")
                }
            }
            ClientTarget::RooCode => {
                return Err(InstallError::UnsupportedPath {
                    target,
                    reason: "VS Code global storage can be relocated and Roo Code exposes no authoritative effective-path query",
                });
            }
            ClientTarget::Droid => home.join(".factory/mcp.json"),
            ClientTarget::Antigravity => home.join(".gemini/config/mcp_config.json"),
        };
        if !path.is_absolute() {
            return Err(InstallError::UnsupportedPath {
                target,
                reason: "the resolved global configuration path is not absolute",
            });
        }
        if path
            .to_str()
            .is_none_or(|value| value.chars().any(char::is_control))
        {
            return Err(InstallError::UnsupportedPath {
                target,
                reason: "the resolved global configuration path is not safe UTF-8",
            });
        }
        Ok(path)
    }
}
