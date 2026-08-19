//! Configuration, credential, and onboarding core.
//!
//! One canonical runtime model (owned by this module) drives both TOML I/O
//! and the committed `config.schema.json` generator. Credentials never enter
//! the general configuration file; they live in a dedicated restricted
//! `credentials.toml` under the default platform configuration directory that
//! `PANGRAM_CONFIG` cannot relocate.

mod credentials;
mod file_config;
mod model;
mod onboarding;
mod overrides;
mod paths;
#[cfg(windows)]
pub(crate) mod windows_acl;

pub use credentials::{
    CREDENTIALS_FILE_NAME, CredentialResolution, CredentialService, CredentialSource,
};
pub(crate) use credentials::{atomic_secret_write, enforce_protected_permissions};
use zeroize::Zeroizing;

/// Reads one owner-only file into zeroizing memory for release tooling.
#[doc(hidden)]
pub fn read_protected_file(path: &std::path::Path) -> Result<Zeroizing<Vec<u8>>, ConfigError> {
    enforce_protected_permissions(path)?;
    std::fs::read(path)
        .map(Zeroizing::new)
        .map_err(|error| ConfigError::Io(format!("cannot read protected file: {error}")))
}
pub(crate) use file_config::ConfigSetOutcome;
pub use file_config::{CONFIG_FILE_NAME, ConfigKey, ConfigValue, FileConfigStore};
pub use model::{
    CONFIG_VERSION, Config, HistoryConfig, IntroMode, Keymap, MAX_REQUESTS_PER_SECOND, Motion,
    NetworkConfig, TuiConfig, UpdatesConfig,
};
pub use onboarding::OnboardingState;
pub use overrides::{ConfigOverrides, ENV_API_KEY, ENV_CONFIG, ENV_DATA_DIR};
pub use paths::{Paths, PathsError};

use std::sync::Arc;

use thiserror::Error;

/// Configuration or credential failures with adapter-safe, sanitized messages.
///
/// Messages name keys, paths, and permission bits only. They never include
/// credential material, and no variant stores a credential. `source` errors
/// are I/O or TOML parse errors whose default rendering can only contain
/// parser positions and OS error strings.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("configuration file is invalid: {0}")]
    Invalid(String),
    #[error("unknown configuration key `{0}`")]
    UnknownKey(String),
    #[error("unknown configuration section `{0}`")]
    UnknownSection(String),
    #[error("credential keys cannot be read or changed through general configuration")]
    CredentialKeyRejected,
    #[error("invalid value for configuration key `{key}`: {reason}")]
    InvalidValue { key: String, reason: String },
    #[error("stored credentials are not protected by owner-only permissions")]
    InsecurePermissions,
    #[error("owner-only permissions could not be established for stored credentials")]
    RestrictionFailed,
    #[error("failed to access configuration data: {0}")]
    Io(String),
}

/// Resolved configuration files, default-data view, and credential service
/// for one set of overrides. Adapters hold one of these instead of touching
/// the filesystem or environment themselves.
///
/// The service retains its `ConfigOverrides` so credential resolution (for
/// example `onboarding_state`) honors `PANGRAM_API_KEY` precedence. All
/// `Debug` surfaces on this type stay redacted: overrides can carry the
/// ephemeral credential.
#[derive(Clone)]
pub struct ConfigService {
    paths: Paths,
    store: FileConfigStore,
    credentials: CredentialService,
    overrides: ConfigOverrides,
}

impl ConfigService {
    pub fn new(overrides: &ConfigOverrides) -> Result<Self, ConfigError> {
        let paths = Paths::resolve(overrides)?;
        Ok(Self::for_test(paths, overrides.clone()))
    }

    #[doc(hidden)]
    pub fn for_test(paths: Paths, overrides: ConfigOverrides) -> Self {
        Self {
            store: FileConfigStore::new(paths.config_file()),
            credentials: CredentialService::new(paths.credentials_file()),
            paths,
            overrides,
        }
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn store(&self) -> &FileConfigStore {
        &self.store
    }

    pub fn credentials(&self) -> &CredentialService {
        &self.credentials
    }

    /// The overrides this service was built from (redacted `Debug`).
    pub fn overrides(&self) -> &ConfigOverrides {
        &self.overrides
    }

    /// Loads the persisted configuration without applying defaults.
    pub fn persisted(&self) -> Result<Config, ConfigError> {
        self.store.load()
    }

    /// Loads the persisted configuration with built-in defaults applied.
    pub fn effective(&self) -> Result<Config, ConfigError> {
        self.store.load().map(Config::with_defaults)
    }

    pub fn list(&self) -> Result<Vec<(ConfigKey, ConfigValue)>, ConfigError> {
        self.store.list()
    }

    pub fn get(&self, key: &str) -> Result<ConfigValue, ConfigError> {
        self.store.get(key)
    }

    pub fn set(&self, key: &str, value: &str) -> Result<Config, ConfigError> {
        self.store.set(key, value)
    }

    /// Mutates one persisted key and reports transition facts derived inside
    /// the storage lock rather than from a racy adapter-side read.
    pub(crate) fn set_with_outcome(
        &self,
        key: &str,
        value: &str,
    ) -> Result<ConfigSetOutcome, ConfigError> {
        self.store.set_with_outcome(key, value)
    }

    /// Reads the onboarding/setup state (credential status plus platform
    /// paths). Uses this service's original overrides so an environment
    /// credential reports the correct onboarding status.
    pub fn onboarding_state(&self) -> Result<OnboardingState, ConfigError> {
        OnboardingState::read_with(&self.paths, &self.credentials, &self.overrides, None)
    }
}

impl std::fmt::Debug for ConfigService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfigService")
            .field("paths", &self.paths)
            .field("overrides", &self.overrides)
            .finish_non_exhaustive()
    }
}

impl From<PathsError> for ConfigError {
    fn from(error: PathsError) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(redact_io(&error))
    }
}

impl From<toml::ser::Error> for ConfigError {
    fn from(error: toml::ser::Error) -> Self {
        Self::Invalid(format!("could not encode TOML: {error}"))
    }
}

/// Renders a TOML deserialization error without ever echoing the source
/// document. `toml::de::Error`'s default `Display` embeds the offending
/// source line, which in `credentials.toml` (or a malformed key line in
/// `config.toml`) could print raw credential material. We only carry the
/// parser's own message plus the byte span, never rendered source text.
pub(crate) fn sanitize_toml_de(error: toml::de::Error) -> ConfigError {
    let mut message = error.message().to_owned();
    if let Some(span) = error.span() {
        message.push_str(&format!(" (near byte offset {})", span.start));
    }
    ConfigError::Invalid(format!("could not parse TOML: {message}"))
}

/// Renders an I/O error by kind and OS code only, never carrying paths or
/// caller-supplied/strings into adapter-facing messages.
pub(crate) fn redact_io(error: &std::io::Error) -> String {
    format!(
        "{} ({})",
        error.kind(),
        error
            .raw_os_error()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "no OS code".into())
    )
}

/// Shares one service across adapter code without `'static` borrowing.
pub type SharedConfigService = Arc<ConfigService>;

#[cfg(test)]
mod mutation_tests {
    use super::*;
    use std::fs;
    use std::sync::{Barrier, mpsc};
    use std::time::Duration;

    fn store(root: &tempfile::TempDir) -> FileConfigStore {
        FileConfigStore::new(root.path().join(CONFIG_FILE_NAME))
    }

    #[test]
    fn config_mutation_waits_for_the_production_sidecar_lock() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        let held_lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.path().join(format!("{CONFIG_FILE_NAME}.lock")))
            .unwrap();
        fs4::FileExt::lock(&held_lock).unwrap();

        let started = Barrier::new(2);
        let (completed_tx, completed_rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                started.wait();
                completed_tx
                    .send(store.set_with_outcome("history.enabled", "true"))
                    .unwrap();
            });

            started.wait();
            assert!(
                completed_rx.recv_timeout(Duration::from_secs(1)).is_err(),
                "the mutation completed without respecting the held config lock"
            );

            fs4::FileExt::unlock(&held_lock).unwrap();
            let outcome = completed_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("mutation completes after the config lock is released")
                .unwrap();
            worker.join().unwrap();
            assert!(outcome.history_became_enabled());
        });
    }

    #[test]
    fn concurrent_history_enable_has_exactly_one_transition_winner() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        let started = Barrier::new(3);
        let winners: usize = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        started.wait();
                        store
                            .set_with_outcome("history.enabled", "true")
                            .unwrap()
                            .history_became_enabled()
                    })
                })
                .collect();

            started.wait();
            workers
                .into_iter()
                .map(|worker| usize::from(worker.join().unwrap()))
                .sum()
        });
        assert_eq!(winners, 1, "only the persisted false-to-true writer wins");
        assert_eq!(
            store.get("history.enabled").unwrap(),
            ConfigValue::Bool(true)
        );
    }

    #[test]
    fn concurrent_disable_cannot_hide_a_persisted_reenable_transition() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        store.set("history.enabled", "true").unwrap();
        let started = Barrier::new(3);

        let outcomes: Vec<(bool, bool)> = std::thread::scope(|scope| {
            let workers: Vec<_> = [false, true]
                .into_iter()
                .map(|enabled| {
                    let store = &store;
                    let started = &started;
                    scope.spawn(move || {
                        started.wait();
                        let value = if enabled { "true" } else { "false" };
                        let outcome = store.set_with_outcome("history.enabled", value).unwrap();
                        (enabled, outcome.history_became_enabled())
                    })
                })
                .collect();

            started.wait();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect()
        });
        let enable_transition = outcomes
            .iter()
            .find_map(|(enabled, transitioned)| enabled.then_some(*transitioned))
            .unwrap();
        let disable_transition = outcomes
            .iter()
            .find_map(|(enabled, transitioned)| (!enabled).then_some(*transitioned))
            .unwrap();
        let ConfigValue::Bool(final_enabled) = store.get("history.enabled").unwrap() else {
            panic!("history.enabled must remain a persisted bool");
        };

        assert!(
            !disable_transition,
            "disabling never owns an enable warning"
        );
        assert_eq!(
            enable_transition, final_enabled,
            "from an enabled start, a final true means disable serialized first \
             and the enable call must report that persisted false-to-true transition"
        );
    }
}
