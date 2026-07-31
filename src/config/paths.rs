//! Platform path resolution through `directories`, with explicit overrides.
//!
//! The credentials file always lives under the default platform
//! configuration directory and is computed from `directories` alone, so
//! `PANGRAM_CONFIG` can never relocate it.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use thiserror::Error;

use super::overrides::ConfigOverrides;
use super::{CONFIG_FILE_NAME, CREDENTIALS_FILE_NAME};

const PROJECT_QUALIFIER: &str = "dev";
const PROJECT_ORGANIZATION: &str = "micr";
const PROJECT_APPLICATION: &str = "pangram";

/// Platform directories could not be determined for this user.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PathsError {
    #[error("could not resolve platform configuration directories for this user")]
    Unresolvable,
}

/// The resolved filesystem locations every configuration workflow shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    platform_config_dir: PathBuf,
    platform_data_dir: PathBuf,
    config_file: PathBuf,
    data_dir: PathBuf,
    credentials_file: PathBuf,
}

impl Paths {
    /// Resolves platform locations, applying explicit config/data-dir
    /// overrides from flags or environment. The credentials file is always
    /// the default platform location, never the explicit config path.
    pub fn resolve(overrides: &ConfigOverrides) -> Result<Self, PathsError> {
        let project =
            ProjectDirs::from(PROJECT_QUALIFIER, PROJECT_ORGANIZATION, PROJECT_APPLICATION)
                .ok_or(PathsError::Unresolvable)?;

        let platform_config_dir = project.config_dir().to_path_buf();
        let platform_data_dir = project.data_dir().to_path_buf();
        let config_file = overrides
            .config_file()
            .map(PathBuf::from)
            .unwrap_or_else(|| platform_config_dir.join(CONFIG_FILE_NAME));
        let data_dir = overrides
            .data_dir()
            .map(PathBuf::from)
            .unwrap_or_else(|| platform_data_dir.clone());
        let credentials_file = platform_config_dir.join(CREDENTIALS_FILE_NAME);

        Ok(Self {
            platform_config_dir,
            platform_data_dir,
            config_file,
            data_dir,
            credentials_file,
        })
    }

    /// The default platform configuration directory.
    pub fn platform_config_dir(&self) -> &Path {
        &self.platform_config_dir
    }

    /// The default platform data directory before overrides.
    pub fn platform_data_dir(&self) -> &Path {
        &self.platform_data_dir
    }

    /// The configuration file in effect (explicit path wins).
    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    /// The history and state directory in effect (explicit path wins).
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The dedicated restricted credential file. Never relocatable.
    pub fn credentials_file(&self) -> &Path {
        &self.credentials_file
    }

    /// Direct construction for tests and tooling that need an isolated
    /// filesystem root without process-environment mutation. Public because
    /// integration tests live outside the crate.
    pub fn for_test(platform_config_dir: PathBuf, platform_data_dir: PathBuf) -> Self {
        let config_file = platform_config_dir.join(CONFIG_FILE_NAME);
        let credentials_file = platform_config_dir.join(CREDENTIALS_FILE_NAME);
        Self {
            platform_config_dir,
            data_dir: platform_data_dir.clone(),
            platform_data_dir,
            config_file,
            credentials_file,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_resolution_places_all_files_in_platform_dirs() {
        let paths = Paths::resolve(&ConfigOverrides::default()).unwrap();
        assert_eq!(
            paths.config_file(),
            paths.platform_config_dir().join(CONFIG_FILE_NAME)
        );
        assert_eq!(
            paths.credentials_file(),
            paths.platform_config_dir().join(CREDENTIALS_FILE_NAME)
        );
        assert_eq!(paths.data_dir(), paths.platform_data_dir());
    }

    #[test]
    fn explicit_overrides_relocate_config_and_data_but_never_credentials() {
        let overrides = ConfigOverrides::default()
            .with_config_file("/tmp/explicit/pangram.toml")
            .with_data_dir("/tmp/explicit/data");
        let paths = Paths::resolve(&overrides).unwrap();

        assert_eq!(paths.config_file(), Path::new("/tmp/explicit/pangram.toml"));
        assert_eq!(paths.data_dir(), Path::new("/tmp/explicit/data"));
        assert_eq!(
            paths.credentials_file(),
            paths.platform_config_dir().join(CREDENTIALS_FILE_NAME)
        );
        assert!(
            !paths.credentials_file().starts_with("/tmp/explicit"),
            "credentials must stay in the platform directory: {}",
            paths.credentials_file().display()
        );
    }
}
