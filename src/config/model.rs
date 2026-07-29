//! The canonical configuration model.
//!
//! These types are the single source of truth for the non-secret TOML file:
//! they drive strict serde loading/saving, `config set` key parsing, and the
//! generated `config.schema.json` (re-export and aliased by
//! `crate::contracts::schema_types`). Credentials have no representation here
//! by contract.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ConfigError;

/// The only supported `config_version` value.
pub const CONFIG_VERSION: u8 = 1;

/// Pangram's documented ceiling for `network.max_requests_per_second`.
pub const MAX_REQUESTS_PER_SECOND: f64 = 5.0;

// Canonical non-secret configuration. Unknown keys are rejected on load.
// The field-level note on `updates` is a plain comment deliberately:
// Schemars would emit doc strings as `description` keywords and break the
// byte-identical contract with the committed schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub config_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<HistoryConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tui: Option<TuiConfig>,
    // `check_on_tui_start` remains `None` until the first-launch preference
    // is resolved; saving must preserve that `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<UpdatesConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkConfig>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryConfig {
    #[schemars(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TuiConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<IntroMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keymap: Option<Keymap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IntroMode {
    Once,
    Always,
    Off,
}

impl IntroMode {
    pub const ALL: &'static [Self] = &[Self::Once, Self::Always, Self::Off];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Always => "always",
            Self::Off => "off",
        }
    }
}

impl std::fmt::Display for IntroMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Keymap {
    Regular,
    Vim,
}

impl Keymap {
    pub const ALL: &'static [Self] = &[Self::Regular, Self::Vim];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Regular => "regular",
            Self::Vim => "vim",
        }
    }
}

impl std::fmt::Display for Keymap {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    Full,
    Reduced,
    Off,
}

impl Motion {
    pub const ALL: &'static [Self] = &[Self::Full, Self::Reduced, Self::Off];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Reduced => "reduced",
            Self::Off => "off",
        }
    }
}

impl std::fmt::Display for Motion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatesConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_on_tui_start: Option<bool>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    #[schemars(range(min = 0.0, max = 5.0))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_requests_per_second: Option<f64>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: CONFIG_VERSION,
            history: None,
            tui: None,
            updates: None,
            network: None,
        }
    }
}

impl Config {
    /// Built-in defaults merged over the persisted shape. `None` stays `None`
    /// for `updates.check_on_tui_start`; the TUI resolves the preference.
    pub fn with_defaults(mut self) -> Self {
        let history = self.history.get_or_insert_with(HistoryConfig::default);
        history.enabled.get_or_insert(false);
        let tui = self.tui.get_or_insert_with(TuiConfig::default);
        tui.intro.get_or_insert(IntroMode::Once);
        tui.keymap.get_or_insert(Keymap::Regular);
        tui.motion.get_or_insert(Motion::Full);
        let network = self.network.get_or_insert_with(NetworkConfig::default);
        network
            .max_requests_per_second
            .get_or_insert(MAX_REQUESTS_PER_SECOND);
        self
    }

    /// The semantic rule mirrored by the schema's `const` and range bounds.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.config_version != CONFIG_VERSION {
            return Err(ConfigError::InvalidValue {
                key: "config_version".into(),
                reason: format!("must be {CONFIG_VERSION}, found {}", self.config_version),
            });
        }
        if let Some(rate) = self
            .network
            .as_ref()
            .and_then(|network| network.max_requests_per_second)
        {
            validate_rate(rate)?;
        }
        Ok(())
    }
}

pub(super) fn validate_rate(rate: f64) -> Result<(), ConfigError> {
    if !rate.is_finite() || rate <= 0.0 || rate > MAX_REQUESTS_PER_SECOND {
        return Err(ConfigError::InvalidValue {
            key: "network.max_requests_per_second".into(),
            reason: format!("must be greater than 0 and at most {MAX_REQUESTS_PER_SECOND}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_document_parses_with_version_only() {
        let config: Config = toml::from_str("config_version = 1").unwrap();
        config.validate().unwrap();
        assert_eq!(config.history, None);
        assert_eq!(config.updates, None);
    }

    #[test]
    fn unknown_keys_are_rejected_strictly() {
        let error =
            toml::from_str::<Config>("config_version = 1\nunknown_key = true\n").unwrap_err();
        assert!(error.to_string().contains("unknown_key"), "{error}");
    }

    #[test]
    fn unknown_nested_key_is_rejected() {
        toml::from_str::<Config>("config_version = 1\n[tui]\nbrightness = 3\n").unwrap_err();
    }

    #[test]
    fn wrong_version_is_invalid() {
        let config: Config = toml::from_str("config_version = 2").unwrap();
        let error = config.validate().unwrap_err();
        assert!(matches!(error, ConfigError::InvalidValue { .. }), "{error}");
    }

    #[test]
    fn rate_bounds_are_enforced() {
        for rate in [0.0, -1.0, 5.000_000_1, f64::INFINITY, f64::NAN] {
            let config = Config {
                config_version: 1,
                history: None,
                tui: None,
                updates: None,
                network: Some(NetworkConfig {
                    max_requests_per_second: Some(rate),
                }),
            };
            assert!(config.validate().is_err(), "rate {rate} must be rejected");
        }
        for rate in [0.5, 2.0, 5.0] {
            let config = Config {
                config_version: 1,
                history: None,
                tui: None,
                updates: None,
                network: Some(NetworkConfig {
                    max_requests_per_second: Some(rate),
                }),
            };
            config.validate().unwrap();
        }
    }

    #[test]
    fn defaults_apply_without_touching_updates_preference() {
        let config = Config::default().with_defaults();
        assert_eq!(config.tui.unwrap().intro, Some(IntroMode::Once));
        assert_eq!(config.history.unwrap().enabled, Some(false));
        assert_eq!(
            config.network.unwrap().max_requests_per_second,
            Some(MAX_REQUESTS_PER_SECOND)
        );
        assert_eq!(config.updates, None);
    }

    #[test]
    fn closed_values_have_fixed_wire_spelling() {
        let intro: IntroMode = serde_json::from_value(serde_json::json!("once")).unwrap();
        assert_eq!(intro, IntroMode::Once);
        assert_eq!(serde_json::to_value(Motion::Reduced).unwrap(), "reduced");
        assert_eq!(serde_json::to_value(Keymap::Vim).unwrap(), "vim");
        serde_json::from_value::<IntroMode>(serde_json::json!("sometimes")).unwrap_err();
    }
}
