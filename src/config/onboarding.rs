//! The first-launch setup state the TUI overlay and diagnostics consume.
//!
//! This is state, not configuration: it reports whether a credential exists
//! (and from where), whether the update-check preference has been resolved,
//! and the resolved paths. It never exposes key material.

use super::credentials::{CredentialService, CredentialSource};
use super::{Config, ConfigError, ConfigOverrides, IntroMode, Keymap, Motion, Paths};

/// The setup state for one launch. All fields are safe to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingState {
    credential_configured: bool,
    credential_source: CredentialSource,
    update_check_preference_set: bool,
    intro_mode: IntroMode,
    keymap: Keymap,
    motion: Motion,
    config_file: String,
    credentials_file: String,
    data_dir: String,
}

impl OnboardingState {
    /// Reads persisted configuration plus credential status without ever
    /// surfacing key material.
    pub fn read(paths: &Paths, credentials: &CredentialService) -> Result<Self, ConfigError> {
        Self::read_with(paths, credentials, &ConfigOverrides::default(), None)
    }

    /// Same as `read`, with environment/flag overrides and an optional
    /// pre-loaded config so callers that already loaded it never parse twice.
    pub fn read_with(
        paths: &Paths,
        credentials: &CredentialService,
        overrides: &ConfigOverrides,
        persisted: Option<&Config>,
    ) -> Result<Self, ConfigError> {
        let resolution = credentials.resolve(overrides)?;
        let config = match persisted {
            Some(config) => config.clone(),
            None => super::file_config::FileConfigStore::new(paths.config_file()).load()?,
        };
        let effective = config.with_defaults();
        Ok(Self {
            credential_configured: resolution.is_configured(),
            credential_source: resolution.source(),
            update_check_preference_set: effective
                .updates
                .and_then(|updates| updates.check_on_tui_start)
                .is_some(),
            intro_mode: effective
                .tui
                .and_then(|tui| tui.intro)
                .unwrap_or(IntroMode::Once),
            keymap: effective
                .tui
                .and_then(|tui| tui.keymap)
                .unwrap_or(Keymap::Regular),
            motion: effective
                .tui
                .and_then(|tui| tui.motion)
                .unwrap_or(Motion::Full),
            config_file: paths.config_file().display().to_string(),
            credentials_file: paths.credentials_file().display().to_string(),
            data_dir: paths.data_dir().display().to_string(),
        })
    }

    /// Whether the first-launch credential overlay should offer setup.
    pub const fn needs_credential_setup(&self) -> bool {
        !self.credential_configured
    }

    /// Whether the update-check preference overlay must run this launch.
    pub const fn needs_update_preference(&self) -> bool {
        !self.update_check_preference_set
    }

    /// True when neither overlay needs to run.
    pub const fn onboarding_complete(&self) -> bool {
        self.credential_configured && self.update_check_preference_set
    }

    pub const fn credential_configured(&self) -> bool {
        self.credential_configured
    }

    pub const fn credential_source(&self) -> CredentialSource {
        self.credential_source
    }

    pub const fn update_check_preference_set(&self) -> bool {
        self.update_check_preference_set
    }

    pub const fn intro_mode(&self) -> IntroMode {
        self.intro_mode
    }

    pub const fn keymap(&self) -> Keymap {
        self.keymap
    }

    pub const fn motion(&self) -> Motion {
        self.motion
    }

    pub fn config_file(&self) -> &str {
        &self.config_file
    }

    pub fn credentials_file(&self) -> &str {
        &self.credentials_file
    }

    pub fn data_dir(&self) -> &str {
        &self.data_dir
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    mod unix {
        use super::super::*;
        use std::fs;
        use tempfile::TempDir;

        fn paths(root: &TempDir) -> Paths {
            let config_dir = root.path().join("platform-config");
            let data_dir = root.path().join("platform-data");
            fs::create_dir_all(&config_dir).unwrap();
            fs::create_dir_all(&data_dir).unwrap();
            Paths::for_test(config_dir, data_dir)
        }

        #[test]
        fn unsetup_state_offers_both_overlays() {
            let root = tempfile::tempdir().unwrap();
            let paths = paths(&root);
            let credentials = CredentialService::new(paths.credentials_file());
            let state = OnboardingState::read(&paths, &credentials).unwrap();

            assert!(state.needs_credential_setup());
            assert!(state.needs_update_preference());
            assert!(!state.onboarding_complete());
            assert_eq!(state.intro_mode(), IntroMode::Once);
        }

        #[test]
        fn credential_setup_alone_does_not_close_preference_overlay() {
            let root = tempfile::tempdir().unwrap();
            let paths = paths(&root);
            let credentials = CredentialService::new(paths.credentials_file());
            credentials
                .store("pangram_synthetic_onboarding_key_NOT_REAL")
                .unwrap();

            let state = OnboardingState::read(&paths, &credentials).unwrap();
            assert!(!state.needs_credential_setup());
            assert!(state.needs_update_preference());

            // Persisting the preference closes onboarding.
            crate::config::FileConfigStore::new(paths.config_file())
                .set("updates.check_on_tui_start", "true")
                .unwrap();
            let state = OnboardingState::read(&paths, &credentials).unwrap();
            assert!(state.onboarding_complete());
        }

        #[test]
        fn environment_credential_counts_as_configured() {
            let root = tempfile::tempdir().unwrap();
            let paths = paths(&root);
            let credentials = CredentialService::new(paths.credentials_file());
            let overrides =
                ConfigOverrides::default().with_env_api_key("pangram_synthetic_env_key_NOT_REAL");
            let state = OnboardingState::read_with(&paths, &credentials, &overrides, None).unwrap();
            assert!(!state.needs_credential_setup());
            assert_eq!(state.credential_source(), CredentialSource::Environment);
        }
    }
}
