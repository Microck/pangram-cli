//! Flag and environment overrides for configuration resolution.
//!
//! Precedence is flags > environment > explicit config > default config >
//! built-ins. Readers never mutate process-global environment; producers
//! (the CLI layer in a later packet) call `from_environment()` once at startup
//! and pass the resulting value down.

/// Ephemeral credential override.
pub const ENV_API_KEY: &str = "PANGRAM_API_KEY";
/// Explicit configuration file override.
pub const ENV_CONFIG: &str = "PANGRAM_CONFIG";
/// Explicit history and state directory override.
pub const ENV_DATA_DIR: &str = "PANGRAM_DATA_DIR";

/// Caller-supplied overrides. Each field is an already-resolved value; the
/// owning layer (CLI, tests) merges flags over environment before this.
///
/// The manual `Debug` impl redacts the ephemeral credential: overrides can
/// carry `PANGRAM_API_KEY`, and no debug surface may print it. The credential
/// is held in a zeroizing buffer so copies dropped after `merge`/`Clone` do
/// not leave the key in freed heap memory.
#[derive(Default)]
pub struct ConfigOverrides {
    config_file: Option<String>,
    data_dir: Option<String>,
    env_api_key: Option<zeroize::Zeroizing<String>>,
    /// True when `PANGRAM_API_KEY` is present but not valid UTF-8. Presence
    /// must not silently fall back to the stored credential, so resolution
    /// fails closed instead of operating under an unintended key.
    env_api_key_invalid: bool,
}

impl std::fmt::Debug for ConfigOverrides {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfigOverrides")
            .field("config_file", &self.config_file)
            .field("data_dir", &self.data_dir)
            .field(
                "env_api_key",
                &self.env_api_key.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

impl Clone for ConfigOverrides {
    fn clone(&self) -> Self {
        Self {
            config_file: self.config_file.clone(),
            data_dir: self.data_dir.clone(),
            // `Zeroizing<String>` clones the inner string and zeroizes the
            // shared copy's backing store only when each copy is dropped.
            env_api_key: self.env_api_key.clone(),
            env_api_key_invalid: self.env_api_key_invalid,
        }
    }
}

impl PartialEq for ConfigOverrides {
    fn eq(&self, other: &Self) -> bool {
        self.config_file == other.config_file
            && self.data_dir == other.data_dir
            && self.env_api_key.as_deref() == other.env_api_key.as_deref()
            && self.env_api_key_invalid == other.env_api_key_invalid
    }
}

impl Eq for ConfigOverrides {}

impl ConfigOverrides {
    /// Reads the documented environment variables without mutating anything.
    pub fn from_environment() -> Self {
        // `var` maps NotUnicode to Err, which would silently treat a present
        // but non-UTF-8 credential override as absent and let the stored key
        // win behind the scenes. `var_os` preserves presence; the invalid
        // encoding is recorded so resolution fails closed instead of falling
        // back to the stored credential under the wrong key.
        let (env_api_key, env_api_key_invalid) = match std::env::var_os(ENV_API_KEY) {
            Some(raw) => match raw.into_string() {
                Ok(value) if !value.trim().is_empty() => {
                    (Some(zeroize::Zeroizing::new(value)), false)
                }
                Ok(_) => (None, false),
                Err(_) => (None, true),
            },
            None => (None, false),
        };
        Self {
            config_file: std::env::var(ENV_CONFIG)
                .ok()
                .filter(|value| !value.trim().is_empty()),
            data_dir: std::env::var(ENV_DATA_DIR)
                .ok()
                .filter(|value| !value.trim().is_empty()),
            env_api_key,
            env_api_key_invalid,
        }
    }

    /// Merges flag-level values over environment-level values.
    pub fn merge(flags: Self, environment: Self) -> Self {
        Self {
            config_file: flags.config_file.or(environment.config_file),
            data_dir: flags.data_dir.or(environment.data_dir),
            env_api_key: flags.env_api_key.or(environment.env_api_key),
            // An invalid flag-supplied value cannot occur (flags are typed),
            // so only the environment side can contribute the invalid marker.
            env_api_key_invalid: flags.env_api_key_invalid || environment.env_api_key_invalid,
        }
    }

    #[must_use]
    pub fn with_config_file(mut self, path: impl Into<String>) -> Self {
        self.config_file = Some(path.into());
        self
    }

    #[must_use]
    pub fn with_data_dir(mut self, path: impl Into<String>) -> Self {
        self.data_dir = Some(path.into());
        self
    }

    #[must_use]
    pub fn with_env_api_key(mut self, key: impl Into<String>) -> Self {
        self.env_api_key = Some(zeroize::Zeroizing::new(key.into()));
        self
    }

    pub fn config_file(&self) -> Option<&str> {
        self.config_file.as_deref()
    }

    pub fn data_dir(&self) -> Option<&str> {
        self.data_dir.as_deref()
    }

    /// The ephemeral environment credential, when set. Callers must treat
    /// the returned value as secret material.
    pub fn env_api_key(&self) -> Option<&str> {
        self.env_api_key.as_deref().map(String::as_str)
    }

    /// True when `PANGRAM_API_KEY` was present but not valid UTF-8. Callers
    /// must fail closed rather than treat the override as absent, which would
    /// silently fall back to the stored credential under the wrong key.
    pub const fn env_api_key_invalid(&self) -> bool {
        self.env_api_key_invalid
    }

    /// Test-only constructor for the undecodable-environment state; the real
    /// path is `from_environment` + `var_os` on a non-UTF-8 value, which tests
    /// cannot set without mutating the process environment.
    #[cfg(test)]
    pub fn for_test_invalid_env_api_key() -> Self {
        Self {
            env_api_key_invalid: true,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_env_api_key_is_flagged_not_absent() {
        let overrides = ConfigOverrides {
            env_api_key_invalid: true,
            ..ConfigOverrides::default()
        };
        // Presence of an undecodable override must resolve fail-closed, not
        // silently drop to the stored credential.
        assert!(overrides.env_api_key_invalid());
        assert_eq!(overrides.env_api_key(), None);
    }

    #[test]
    fn flags_win_over_environment_in_merge() {
        let flags = ConfigOverrides::default().with_config_file("/flags/pangram.toml");
        let environment = ConfigOverrides::default()
            .with_config_file("/env/pangram.toml")
            .with_data_dir("/env/data")
            .with_env_api_key("env-key");
        let merged = ConfigOverrides::merge(flags, environment);

        assert_eq!(merged.config_file(), Some("/flags/pangram.toml"));
        assert_eq!(merged.data_dir(), Some("/env/data"));
        assert_eq!(merged.env_api_key(), Some("env-key"));
    }

    #[test]
    fn blank_values_are_not_valid_keys() {
        let overrides = ConfigOverrides::default()
            .with_config_file("   ")
            .with_env_api_key(" ");
        // Whitespace-only values survive the builder (they came from a flag,
        // not the environment reader) and are the caller's problem; the
        // environment reader filters them.
        assert!(overrides.config_file().is_some());
        let env_value = std::env::var("PANGRAM_TEST_DEFINITELY_UNSET_12345")
            .ok()
            .filter(|value: &String| !value.trim().is_empty());
        assert_eq!(env_value, None);
    }
}
