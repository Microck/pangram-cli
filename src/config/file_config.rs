//! Strict TOML loading and atomic persistence of the non-secret config file.
//!
//! An existing file must declare the current `config_version`; unknown keys
//! fail; closed values and the network rate ceiling are validated. Writes go
//! through a unique sibling temporary file, `sync_all`, and rename so a crash
//! never leaves a torn config. Credential keys are rejected at key-parsing
//! time and can never reach this file.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::{self, Unexpected, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::model::validate_rate;
use super::{
    CONFIG_VERSION, Config, ConfigError, HistoryConfig, IntroMode, Keymap, Motion, NetworkConfig,
    TuiConfig, UpdatesConfig,
};

pub const CONFIG_FILE_NAME: &str = "config.toml";

/// A typed value for one settable configuration key, as a JSON scalar for
/// output projection (`config get`, `config list`).
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    Bool(bool),
    Number(f64),
    Text(String),
    /// A closed-value spelling in transit from `parse_value` to `apply`.
    /// Never projected to output.
    Unknown(String),
    /// No value is configured and the key has no built-in default (only
    /// `updates.check_on_tui_start` before onboarding). Projects to `null`.
    Unset,
}

impl ConfigValue {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Bool(value) => Value::Bool(*value),
            Self::Number(value) => serde_json::Number::from_f64(*value)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Self::Text(value) | Self::Unknown(value) => Value::String(value.clone()),
            Self::Unset => Value::Null,
        }
    }
}

/// One settable configuration key. Closed sections and fields only; anything
/// in the credential namespace is rejected before touching the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfigKey {
    HistoryEnabled,
    TuiIntro,
    TuiKeymap,
    TuiMotion,
    UpdatesCheckOnTuiStart,
    NetworkMaxRequestsPerSecond,
}

impl ConfigKey {
    pub const ALL: &'static [Self] = &[
        Self::HistoryEnabled,
        Self::TuiIntro,
        Self::TuiKeymap,
        Self::TuiMotion,
        Self::UpdatesCheckOnTuiStart,
        Self::NetworkMaxRequestsPerSecond,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HistoryEnabled => "history.enabled",
            Self::TuiIntro => "tui.intro",
            Self::TuiKeymap => "tui.keymap",
            Self::TuiMotion => "tui.motion",
            Self::UpdatesCheckOnTuiStart => "updates.check_on_tui_start",
            Self::NetworkMaxRequestsPerSecond => "network.max_requests_per_second",
        }
    }

    /// Parses a typed key. Credential namespaces (`credentials`, `auth`,
    /// `api_key`, `secret`) and unknown keys are distinct failures so the CLI
    /// can map them to the right stable error code.
    pub fn parse(input: &str) -> Result<Self, ConfigError> {
        let normalized = input.trim().to_ascii_lowercase();
        let head = normalized.split('.').next().unwrap_or("");
        if matches!(
            head,
            "credentials" | "credential" | "auth" | "api_key" | "secret" | "secrets"
        ) {
            return Err(ConfigError::CredentialKeyRejected);
        }
        match normalized.as_str() {
            "history.enabled" => Ok(Self::HistoryEnabled),
            "tui.intro" => Ok(Self::TuiIntro),
            "tui.keymap" => Ok(Self::TuiKeymap),
            "tui.motion" => Ok(Self::TuiMotion),
            "updates.check_on_tui_start" => Ok(Self::UpdatesCheckOnTuiStart),
            "network.max_requests_per_second" => Ok(Self::NetworkMaxRequestsPerSecond),
            "" => Err(ConfigError::UnknownKey(input.trim().into())),
            other => {
                if !other.contains('.') && Config::SECTIONS.contains(&other) {
                    Err(ConfigError::UnknownSection(other.into()))
                } else {
                    Err(ConfigError::UnknownKey(other.into()))
                }
            }
        }
    }

    /// Parses the CLI-supplied value for this key, enforcing closed values
    /// and numeric bounds before any write.
    pub fn parse_value(self, raw: &str) -> Result<ConfigValue, ConfigError> {
        let trimmed = raw.trim();
        match self {
            Self::HistoryEnabled | Self::UpdatesCheckOnTuiStart => parse_bool(self, trimmed),
            Self::TuiIntro => closed_value(
                self,
                trimmed,
                IntroMode::ALL.iter().map(|mode| mode.as_str()),
            ),
            Self::TuiKeymap => closed_value(
                self,
                trimmed,
                Keymap::ALL.iter().map(|keymap| keymap.as_str()),
            ),
            Self::TuiMotion => closed_value(
                self,
                trimmed,
                Motion::ALL.iter().map(|motion| motion.as_str()),
            ),
            Self::NetworkMaxRequestsPerSecond => {
                let rate: f64 = trimmed.parse().map_err(|_| ConfigError::InvalidValue {
                    key: self.as_str().into(),
                    reason: "not a number".into(),
                })?;
                validate_rate(rate)?;
                Ok(ConfigValue::Number(rate))
            }
        }
    }

    /// Applies a validated value to a config, creating sections as needed.
    ///
    /// `pub(crate)` because a mismatched `(key, value)` pair is impossible only
    /// by construction: `parse_value` is the sole caller and validates the
    /// pairing before reaching `apply`. Keeping this entry point crate-local
    /// prevents an out-of-crate caller from reaching the `unreachable!` below
    /// with a raw `ConfigValue::Text`, which would both abort and echo
    /// caller-supplied content.
    pub(crate) fn apply(self, config: &mut Config, value: ConfigValue) {
        match (self, value) {
            (Self::HistoryEnabled, ConfigValue::Bool(value)) => {
                config
                    .history
                    .get_or_insert_with(HistoryConfig::default)
                    .enabled = Some(value);
            }
            (Self::TuiIntro, ConfigValue::Unknown(value)) => {
                let mode = IntroMode::ALL
                    .iter()
                    .copied()
                    .find(|candidate| candidate.as_str() == value)
                    .expect("closed values are validated before apply");
                config.tui.get_or_insert_with(TuiConfig::default).intro = Some(mode);
            }
            (Self::TuiKeymap, ConfigValue::Unknown(value)) => {
                let keymap = Keymap::ALL
                    .iter()
                    .copied()
                    .find(|candidate| candidate.as_str() == value)
                    .expect("closed values are validated before apply");
                config.tui.get_or_insert_with(TuiConfig::default).keymap = Some(keymap);
            }
            (Self::TuiMotion, ConfigValue::Unknown(value)) => {
                let motion = Motion::ALL
                    .iter()
                    .copied()
                    .find(|candidate| candidate.as_str() == value)
                    .expect("closed values are validated before apply");
                config.tui.get_or_insert_with(TuiConfig::default).motion = Some(motion);
            }
            (Self::UpdatesCheckOnTuiStart, ConfigValue::Bool(value)) => {
                config
                    .updates
                    .get_or_insert(UpdatesConfig {
                        check_on_tui_start: None,
                    })
                    .check_on_tui_start = Some(value);
            }
            (Self::NetworkMaxRequestsPerSecond, ConfigValue::Number(value)) => {
                config
                    .network
                    .get_or_insert_with(NetworkConfig::default)
                    .max_requests_per_second = Some(value);
            }
            (key, value) => unreachable!(
                "config key {} cannot carry value kind {:?} after parse_value",
                key.as_str(),
                // Only the discriminant kind is named; a caller-supplied
                // `ConfigValue::Text` payload is never echoed into the message.
                std::mem::discriminant(&value)
            ),
        }
    }

    /// Reads the effective value of this key from a config on which defaults
    /// have already been applied (`Config::with_defaults`). Keys with no
    /// built-in default and no persisted value report "not configured".
    pub fn read_from(self, config: &Config) -> ConfigValue {
        match self {
            Self::HistoryEnabled => config
                .history
                .and_then(|section| section.enabled)
                .map(ConfigValue::Bool)
                .unwrap_or(ConfigValue::Unset),
            Self::TuiIntro => config
                .tui
                .and_then(|section| section.intro)
                .map(|value| ConfigValue::Text(value.as_str().into()))
                .unwrap_or(ConfigValue::Unset),
            Self::TuiKeymap => config
                .tui
                .and_then(|section| section.keymap)
                .map(|value| ConfigValue::Text(value.as_str().into()))
                .unwrap_or(ConfigValue::Unset),
            Self::TuiMotion => config
                .tui
                .and_then(|section| section.motion)
                .map(|value| ConfigValue::Text(value.as_str().into()))
                .unwrap_or(ConfigValue::Unset),
            Self::UpdatesCheckOnTuiStart => config
                .updates
                .and_then(|section| section.check_on_tui_start)
                .map(ConfigValue::Bool)
                .unwrap_or(ConfigValue::Unset),
            Self::NetworkMaxRequestsPerSecond => config
                .network
                .and_then(|section| section.max_requests_per_second)
                .map(ConfigValue::Number)
                .unwrap_or(ConfigValue::Unset),
        }
    }
}

fn parse_bool(key: ConfigKey, raw: &str) -> Result<ConfigValue, ConfigError> {
    match raw.to_ascii_lowercase().as_str() {
        "true" => Ok(ConfigValue::Bool(true)),
        "false" => Ok(ConfigValue::Bool(false)),
        _ => Err(ConfigError::InvalidValue {
            key: key.as_str().into(),
            reason: "must be `true` or `false`".into(),
        }),
    }
}

fn closed_value(
    key: ConfigKey,
    raw: &str,
    candidates: impl Iterator<Item = &'static str>,
) -> Result<ConfigValue, ConfigError> {
    let allowed: Vec<&str> = candidates.collect();
    let normalized = raw.trim().to_ascii_lowercase();
    if allowed.iter().any(|candidate| *candidate == normalized) {
        return Ok(ConfigValue::Unknown(normalized));
    }
    Err(ConfigError::InvalidValue {
        key: key.as_str().into(),
        reason: format!("must be one of {}", allowed.join(", ")),
    })
}

impl Config {
    const SECTIONS: &'static [&'static str] = &["history", "tui", "updates", "network"];
}

/// A strict `config_version` field: the file must declare the integer `1`.
struct ConfigVersionVisitor;

impl Visitor<'_> for ConfigVersionVisitor {
    type Value = u8;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the integer config_version = 1")
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        if value == u64::from(CONFIG_VERSION) {
            Ok(CONFIG_VERSION)
        } else {
            Err(de::Error::invalid_value(Unexpected::Unsigned(value), &self))
        }
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        if value == i64::from(CONFIG_VERSION) {
            Ok(CONFIG_VERSION)
        } else {
            Err(de::Error::invalid_value(Unexpected::Signed(value), &self))
        }
    }
}

/// Wire shape used only for parsing: enforces the version rule during the
/// TOML decode, rejects unknown keys, then hands control to the canonical
/// model for validation.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigWire {
    #[serde(deserialize_with = "deserialize_config_version")]
    config_version: u8,
    #[serde(default)]
    history: Option<HistoryConfig>,
    #[serde(default)]
    tui: Option<TuiConfig>,
    #[serde(default)]
    updates: Option<UpdatesConfig>,
    #[serde(default)]
    network: Option<NetworkConfig>,
}

fn deserialize_config_version<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(ConfigVersionVisitor)
}

/// One configuration file path and its strict load/save behavior.
#[derive(Debug, Clone)]
pub struct FileConfigStore {
    path: PathBuf,
}

impl FileConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the persisted config. A missing file yields built-in defaults;
    /// an existing file must declare `config_version = 1` and contain no
    /// unknown keys or keys outside the closed model.
    pub fn load(&self) -> Result<Config, ConfigError> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Config::default());
            }
            Err(error) => return Err(error.into()),
        };
        let wire: ConfigWire = toml::from_str(&contents).map_err(super::sanitize_toml_de)?;
        let config = Config {
            config_version: wire.config_version,
            history: wire.history,
            tui: wire.tui,
            updates: wire.updates,
            network: wire.network,
        };
        config.validate()?;
        Ok(config)
    }

    /// Atomically persists a config: unique sibling temp, os-set 0600 on
    /// Unix (defense in depth for owner-readable content), sync, rename,
    /// best-effort directory sync.
    pub fn save(&self, config: &Config) -> Result<(), ConfigError> {
        config.validate()?;
        let serialized = toml::to_string_pretty(config)?;
        atomic_write(&self.path, serialized.as_bytes(), Some(0o600)).map_err(ConfigError::Io)
    }

    /// `config get KEY`: reads the effective value after built-in defaults
    /// apply, so it always agrees with `config list`. A key absent from the
    /// file resolves to its documented default rather than a sentinel; the
    /// pre-onboarding `updates.check_on_tui_start` has no default and reports
    /// "not configured" without inventing one.
    pub fn get(&self, key: &str) -> Result<ConfigValue, ConfigError> {
        let key = ConfigKey::parse(key)?;
        let effective = self.load()?.with_defaults();
        Ok(key.read_from(&effective))
    }

    /// `config set KEY VALUE`: parses, applies, validates, saves atomically.
    /// The returned config is the new persisted shape.
    pub fn set(&self, key: &str, raw_value: &str) -> Result<Config, ConfigError> {
        let key = ConfigKey::parse(key)?;
        let value = key.parse_value(raw_value)?;
        let mut config = self.load()?;
        key.apply(&mut config, value);
        config.validate()?;
        self.save(&config)?;
        Ok(config)
    }

    /// `config list`: effective values for every settable key in a stable
    /// key order, matching the nested projection and `config get` semantics.
    pub fn list(&self) -> Result<Vec<(ConfigKey, ConfigValue)>, ConfigError> {
        let effective = self.load()?.with_defaults();
        Ok(ConfigKey::ALL
            .iter()
            .copied()
            .map(|key| (key, key.read_from(&effective)))
            .collect())
    }

    /// The canonical nested view for `config list` envelope data.
    pub fn list_as_nested(&self) -> Result<serde_json::Map<String, Value>, ConfigError> {
        let persisted = self.load()?.with_defaults();
        let value = serde_json::to_value(&persisted)
            .map_err(|error| ConfigError::Invalid(error.to_string()))?;
        value.as_object().cloned().ok_or_else(|| {
            ConfigError::Invalid("config model did not serialize to an object".into())
        })
    }
}

/// Shared atomic-write: `path.NNNN-PID-NONCE.tmp` sibling -> write -> 0600 on
/// Unix when requested -> sync -> rename -> cleanup on failure.
pub(super) fn atomic_write(
    path: &Path,
    contents: &[u8],
    restrict_unix_mode: Option<u32>,
) -> Result<(), String> {
    #[cfg(not(unix))]
    let _ = restrict_unix_mode;

    let parent = path
        .parent()
        .ok_or_else(|| format!("path {} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("path {} is not valid UTF-8", path.display()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary = parent.join(format!(".{file_name}.{}-{nonce}.tmp", std::process::id()));

    let write_result = (|| -> std::io::Result<()> {
        {
            use std::io::Write as _;
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            if let Some(mode) = restrict_unix_mode {
                // Set the restrictive mode atomically at creation: `create_new`
                // would otherwise momentarily publish the file as
                // `0666 & ~umask`, opening a window where a local watcher can
                // open and retain a readable descriptor across the later
                // `set_permissions` narrow. `OpenOptionsExt::mode` applies at
                // the initial `open(2)`; the `set_permissions` below remains as
                // defense in depth.
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(mode);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(contents)?;
            #[cfg(unix)]
            if let Some(mode) = restrict_unix_mode {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(mode))?;
            }
            file.sync_all()?;
        }
        fs::rename(&temporary, path)?;
        // Best-effort directory sync for rename durability.
        #[cfg(unix)]
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("cannot write {}: {error}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MAX_REQUESTS_PER_SECOND;
    use tempfile::TempDir;

    fn store(root: &TempDir) -> FileConfigStore {
        FileConfigStore::new(root.path().join("config.toml"))
    }

    #[test]
    fn missing_file_uses_builtins_and_does_not_create_one() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        let config = store.load().unwrap();
        assert_eq!(config, Config::default());
        assert!(!store.path().exists());
    }

    #[test]
    fn existing_file_without_version_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        fs::write(store.path(), "[tui]\nintro = \"off\"\n").unwrap();
        let error = store.load().unwrap_err();
        assert!(format!("{error}").contains("config_version"), "{error}");
    }

    #[test]
    fn wrong_version_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        fs::write(store.path(), "config_version = 2\n").unwrap();
        store.load().unwrap_err();
    }

    #[test]
    fn string_version_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        fs::write(store.path(), "config_version = \"1\"\n").unwrap();
        store.load().unwrap_err();
    }

    #[test]
    fn unknown_top_level_key_names_itself() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        fs::write(store.path(), "config_version = 1\nunknown_key = true\n").unwrap();
        let error = store.load().unwrap_err();
        assert!(format!("{error}").contains("unknown_key"), "{error}");
    }

    #[test]
    fn api_key_in_file_is_rejected_as_unknown() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        fs::write(store.path(), "config_version = 1\napi_key = \"no\"\n").unwrap();
        store.load().unwrap_err();
    }

    #[test]
    fn out_of_range_rate_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        fs::write(
            store.path(),
            "config_version = 1\n[network]\nmax_requests_per_second = 9.5\n",
        )
        .unwrap();
        let error = store.load().unwrap_err();
        assert!(matches!(error, ConfigError::InvalidValue { .. }), "{error}");
    }

    #[test]
    fn set_and_get_roundtrip_and_persist() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);

        store.set("tui.intro", "off").unwrap();
        store.set("network.max_requests_per_second", "2").unwrap();

        let get = store.get("tui.intro").unwrap();
        assert_eq!(get, ConfigValue::Text("off".into()));
        let rate = store.get("network.max_requests_per_second").unwrap();
        assert_eq!(rate, ConfigValue::Number(2.0));

        let reloaded: ConfigWire =
            toml::from_str(&fs::read_to_string(store.path()).unwrap()).unwrap();
        assert_eq!(reloaded.config_version, CONFIG_VERSION);
    }

    #[test]
    fn updates_preference_survives_unrelated_sets() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);

        store.set("tui.intro", "off").unwrap();
        let config = store.load().unwrap();
        assert_eq!(config.updates, None, "unset preference must stay None");

        store.set("updates.check_on_tui_start", "false").unwrap();
        let config = store.load().unwrap();
        assert_eq!(
            config.updates.and_then(|u| u.check_on_tui_start),
            Some(false)
        );
    }

    #[test]
    fn list_yields_every_key_in_stable_order() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        store.set("tui.keymap", "vim").unwrap();
        let listed = store.list().unwrap();
        let names: Vec<&str> = listed.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "history.enabled",
                "tui.intro",
                "tui.keymap",
                "tui.motion",
                "updates.check_on_tui_start",
                "network.max_requests_per_second"
            ]
        );
        assert_eq!(listed[2].1, ConfigValue::Text("vim".into()));
    }

    #[test]
    fn credential_keys_are_rejected_by_key_parsing() {
        for key in [
            "credentials.api_key",
            "auth.api_key",
            "api_key",
            "secret.stuff",
        ] {
            let error = ConfigKey::parse(key).unwrap_err();
            assert!(
                matches!(error, ConfigError::CredentialKeyRejected),
                "{key}: {error}"
            );
        }
    }

    #[test]
    fn unknown_section_and_unknown_key_are_distinguished() {
        let error = ConfigKey::parse("telemetry").unwrap_err();
        assert!(matches!(error, ConfigError::UnknownKey(_)), "{error}");
        let error = ConfigKey::parse("tui.brightness").unwrap_err();
        assert!(matches!(error, ConfigError::UnknownKey(_)), "{error}");
        let error = ConfigKey::parse("tui").unwrap_err();
        assert!(matches!(error, ConfigError::UnknownSection(_)), "{error}");
    }

    #[test]
    fn closed_value_rejection_names_allowed_values() {
        let error = ConfigKey::TuiIntro.parse_value("sometimes").unwrap_err();
        match error {
            ConfigError::InvalidValue { key, reason } => {
                assert_eq!(key, "tui.intro");
                assert!(reason.contains("once, always, off"), "{reason}");
            }
            other => panic!("expected InvalidValue, got {other}"),
        }
    }

    #[test]
    fn writes_are_atomic_and_leave_no_temp_files() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        store.set("history.enabled", "true").unwrap();
        store.set("history.enabled", "false").unwrap();
        let leftovers: Vec<_> = fs::read_dir(root.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "stray temp files: {leftovers:?}");
    }

    #[test]
    fn saved_config_roundtrips_through_canonical_model() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        store.set("tui.intro", "once").unwrap();
        let contents = fs::read_to_string(store.path()).unwrap();
        assert!(contents.contains("config_version = 1"), "{contents}");
        let reloaded = store.load().unwrap();
        assert_eq!(reloaded.tui.and_then(|t| t.intro), Some(IntroMode::Once));
    }

    #[test]
    fn read_from_resolves_effective_defaults_for_unset_keys() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        let config = store.load().unwrap().with_defaults();

        assert_eq!(
            ConfigKey::TuiIntro.read_from(&config),
            ConfigValue::Text("once".into()),
            "tui.intro resolves to its documented default, not a sentinel"
        );
        assert_eq!(
            ConfigKey::HistoryEnabled.read_from(&config),
            ConfigValue::Bool(false),
            "history.enabled stays a typed bool"
        );
        assert_eq!(
            ConfigKey::TuiKeymap.read_from(&config),
            ConfigValue::Text("regular".into())
        );
        assert_eq!(
            ConfigKey::TuiMotion.read_from(&config),
            ConfigValue::Text("full".into())
        );
        assert_eq!(
            ConfigKey::NetworkMaxRequestsPerSecond.read_from(&config),
            ConfigValue::Number(MAX_REQUESTS_PER_SECOND),
            "the rate ceiling stays a typed number"
        );
    }

    #[test]
    fn get_returns_the_same_effective_value_list_reports_for_every_defaulted_key() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        let config = store.load().unwrap().with_defaults();

        let nested = store.list_as_nested().unwrap();
        for key in ConfigKey::ALL {
            let from_get = key.read_from(&config).to_json();
            let (section, leaf) = key.as_str().split_once('.').unwrap();
            // `updates.check_on_tui_start` has no built-in default before
            // onboarding, so the list projection omits the section rather
            // than emitting a sentinel.
            if *key == ConfigKey::UpdatesCheckOnTuiStart {
                assert!(
                    nested.get("updates").is_none(),
                    "pre-onboarding updates section stays omitted: {nested:?}"
                );
                continue;
            }
            let from_list = &nested[section][leaf];
            assert_eq!(
                from_get,
                *from_list,
                "config get {} must equal the config list projection",
                key.as_str()
            );
        }
    }

    #[test]
    fn get_after_set_matches_list_projection() {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root);
        store.set("tui.intro", "off").unwrap();
        store.set("history.enabled", "true").unwrap();
        store.set("network.max_requests_per_second", "2.5").unwrap();
        let config = store.load().unwrap().with_defaults();
        let nested = store.list_as_nested().unwrap();

        for key in [
            ConfigKey::TuiIntro,
            ConfigKey::HistoryEnabled,
            ConfigKey::NetworkMaxRequestsPerSecond,
        ] {
            let (section, leaf) = key.as_str().split_once('.').unwrap();
            assert_eq!(
                key.read_from(&config).to_json(),
                nested[section][leaf],
                "set value for {} matches the list projection",
                key.as_str()
            );
        }
        assert_eq!(
            ConfigKey::TuiIntro.read_from(&config),
            ConfigValue::Text("off".into())
        );
        assert_eq!(
            ConfigKey::NetworkMaxRequestsPerSecond.read_from(&config),
            ConfigValue::Number(2.5)
        );
    }
}
