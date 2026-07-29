//! Phase 1 filesystem-level contract tests for the configuration/credential
//! core. Each test builds its own temporary filesystem root and passes paths
//! explicitly; no test mutates process-global environment.

use std::fs;
use std::path::PathBuf;

use microck_pangram_cli::config::{ConfigOverrides, ConfigService, IntroMode, Paths};
use microck_pangram_cli::contracts::generated_artifacts;
use tempfile::TempDir;

#[cfg(unix)]
use microck_pangram_cli::config::{
    Config, ConfigError, CredentialResolution, CredentialService, CredentialSource,
    FileConfigStore, OnboardingState,
};

#[cfg(unix)]
const SYNTHETIC_KEY: &str =
    "pangram_synthetic_integration_test_key_abcdef0123456789_NOT_A_REAL_KEY";

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// One isolated filesystem layout mirroring what the CLI test harness builds
/// through XDG environment variables, but constructed directly so no test
/// touches process-global state.
struct Layout {
    root: TempDir,
    platform_config_dir: PathBuf,
    platform_data_dir: PathBuf,
    explicit_config: PathBuf,
}

impl Layout {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let platform_config_dir = root.path().join("xdg-config").join("pangram");
        let platform_data_dir = root.path().join("xdg-data").join("pangram");
        let explicit_config = root.path().join("explicit").join("pangram.toml");
        fs::create_dir_all(&platform_config_dir).unwrap();
        fs::create_dir_all(&platform_data_dir).unwrap();
        fs::create_dir_all(explicit_config.parent().unwrap()).unwrap();
        Self {
            root,
            platform_config_dir,
            platform_data_dir,
            explicit_config,
        }
    }

    fn paths(&self) -> Paths {
        Paths::for_test(
            self.platform_config_dir.clone(),
            self.platform_data_dir.clone(),
        )
    }

    #[cfg(unix)]
    fn credentials_file(&self) -> PathBuf {
        self.platform_config_dir.join("credentials.toml")
    }

    fn service(&self) -> ConfigService {
        ConfigService::new(&self.default_overrides_for_paths())
            .expect("service construction must succeed for test paths")
    }

    /// Overrides containing only path fields matching `paths()`; never reads
    /// or mutates the process environment.
    fn default_overrides_for_paths(&self) -> ConfigOverrides {
        ConfigOverrides::default()
            .with_config_file(self.paths().config_file().to_string_lossy().into_owned())
            .with_data_dir(self.paths().data_dir().to_string_lossy().into_owned())
    }
}

#[test]
fn service_resolves_paths_and_reports_them() {
    let layout = Layout::new();
    let service = layout.service();

    // Explicit overrides relocate the config file and data dir, but the
    // credentials file always resolves through `directories`, so it stays
    // in the real platform config directory even when `PANGRAM_CONFIG` or
    // the test layout points elsewhere.
    assert_eq!(service.paths().config_file(), layout.paths().config_file());
    assert_eq!(service.paths().data_dir(), layout.paths().data_dir());
    assert_ne!(
        service.paths().credentials_file(),
        layout.explicit_config,
        "the credential store never follows the explicit config path"
    );
    assert!(
        !service
            .paths()
            .credentials_file()
            .starts_with(layout.root.path()),
        "directory-resolved credentials never enter the test root: {}",
        service.paths().credentials_file().display()
    );
}

#[test]
fn generated_config_schema_matches_canonical_model() {
    // The committed artifact must regenerate byte-identically from the
    // canonical model owned by `crate::config`.
    let committed =
        fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contracts/config.schema.json"))
            .unwrap();
    let generated = generated_artifacts()
        .unwrap()
        .into_iter()
        .find(|artifact| artifact.path == "contracts/config.schema.json")
        .unwrap();
    assert_eq!(committed, generated.bytes);
}

#[test]
fn effective_config_defaults_when_file_missing() {
    let layout = Layout::new();
    let service = layout.service();
    let effective = service.effective().unwrap();

    assert_eq!(effective.tui.unwrap().intro, Some(IntroMode::Once));
    assert_eq!(effective.history.unwrap().enabled, Some(false));
    assert_eq!(effective.updates, None);
}

#[cfg(unix)]
mod unix_filesystem {
    use super::*;

    #[test]
    fn credential_store_read_remove_roundtrip_with_exact_0600() {
        let layout = Layout::new();
        let credentials = CredentialService::new(layout.credentials_file());

        credentials.store(SYNTHETIC_KEY).unwrap();
        assert_eq!(mode(&layout.credentials_file()), 0o600);

        let (source, suffix) = credentials.status(&ConfigOverrides::default()).unwrap();
        assert_eq!(source, CredentialSource::Stored);
        let suffix = suffix.unwrap();
        assert!(suffix.len() <= 8);
        assert!(SYNTHETIC_KEY.ends_with(&suffix));

        credentials.remove().unwrap();
        assert!(!layout.credentials_file().exists());
        // Idempotent.
        credentials.remove().unwrap();
    }

    #[test]
    fn insecure_permissions_fail_closed_on_every_read() {
        let layout = Layout::new();
        let credentials = CredentialService::new(layout.credentials_file());
        credentials.store(SYNTHETIC_KEY).unwrap();

        set_mode(&layout.credentials_file(), 0o644);
        let error = credentials.read().unwrap_err();
        assert!(matches!(error, ConfigError::InsecurePermissions), "{error}");
        let rendered = format!("{error}");
        assert!(!rendered.contains(SYNTHETIC_KEY));
        assert_eq!(
            rendered,
            "stored credentials are not protected by owner-only permissions"
        );

        // resolve() sees the same failure.
        let error = credentials
            .resolve(&ConfigOverrides::default())
            .unwrap_err();
        assert!(matches!(error, ConfigError::InsecurePermissions));
    }

    #[test]
    fn stored_file_contains_only_version_and_key() {
        let layout = Layout::new();
        let credentials = CredentialService::new(layout.credentials_file());
        credentials.store(SYNTHETIC_KEY).unwrap();

        let contents = fs::read_to_string(layout.credentials_file()).unwrap();
        let value: toml::Value = toml::from_str(&contents).unwrap();
        let table = value.as_table().unwrap();
        let mut keys: Vec<&String> = table.keys().collect();
        keys.sort();
        assert_eq!(keys, vec!["api_key", "credentials_version"]);
    }

    #[test]
    fn persisted_config_path_overrides_live_only_in_general_config() {
        let layout = Layout::new();
        let service = FileConfigStore::new(layout.explicit_config.clone());

        service.set("tui.intro", "off").unwrap();
        service.set("updates.check_on_tui_start", "false").unwrap();
        service.set("network.max_requests_per_second", "2").unwrap();

        let persisted: Config =
            toml::from_str(&fs::read_to_string(&layout.explicit_config).unwrap()).unwrap();
        assert_eq!(persisted.tui.unwrap().intro, Some(IntroMode::Off));
        // The updates preference is preserved exactly as set.
        assert_eq!(persisted.updates.unwrap().check_on_tui_start, Some(false));

        // The credential file is untouched by general config writes.
        assert!(!layout.credentials_file().exists());
    }

    #[test]
    fn onboarding_reflects_credential_and_preference_state() {
        let layout = Layout::new();
        let paths = layout.paths();
        let credentials = CredentialService::new(paths.credentials_file());

        let state = OnboardingState::read(&paths, &credentials).unwrap();
        assert!(state.needs_credential_setup());
        assert!(state.needs_update_preference());

        credentials.store(SYNTHETIC_KEY).unwrap();
        FileConfigStore::new(paths.config_file())
            .set("updates.check_on_tui_start", "true")
            .unwrap();

        let state = OnboardingState::read(&paths, &credentials).unwrap();
        assert!(!state.needs_credential_setup());
        assert!(!state.needs_update_preference());
        assert!(state.onboarding_complete());
        assert_eq!(state.credential_source(), CredentialSource::Stored);
    }

    #[test]
    fn environment_credential_wins_without_touching_disk() {
        let layout = Layout::new();
        let paths = layout.paths();
        let credentials = CredentialService::new(paths.credentials_file());

        let overrides =
            ConfigOverrides::default().with_env_api_key("pangram_env_only_key_NOT_REAL");
        let resolution: CredentialResolution = credentials.resolve(&overrides).unwrap();
        assert_eq!(resolution.source(), CredentialSource::Environment);
        // Nothing was persisted.
        assert!(!layout.credentials_file().exists());
    }

    #[test]
    fn config_set_rejects_credential_keys_without_writing() {
        let layout = Layout::new();
        let store = FileConfigStore::new(layout.explicit_config.clone());

        let error = store.set("credentials.api_key", SYNTHETIC_KEY).unwrap_err();
        assert!(matches!(error, ConfigError::CredentialKeyRejected));
        let rendered = format!("{error}");
        assert!(!rendered.contains(SYNTHETIC_KEY));

        assert!(!layout.explicit_config.exists());
        assert!(!layout.credentials_file().exists());
    }

    #[test]
    fn unknown_keys_fail_load_with_the_key_named() {
        let layout = Layout::new();
        let store = FileConfigStore::new(layout.explicit_config.clone());
        fs::write(
            &layout.explicit_config,
            "config_version = 1\nunknown_key = true\n",
        )
        .unwrap();

        let error = store.load().unwrap_err();
        assert!(format!("{error}").contains("unknown_key"), "{error}");
    }

    #[test]
    fn malformed_toml_reports_invalid_config() {
        let layout = Layout::new();
        let store = FileConfigStore::new(layout.explicit_config.clone());
        fs::write(&layout.explicit_config, "config_version = [broken\n").unwrap();

        let error = store.load().unwrap_err();
        assert!(matches!(error, ConfigError::Invalid(_)), "{error}");
    }

    #[test]
    fn onboarding_never_exposes_key_material_in_debug() {
        let layout = Layout::new();
        let paths = layout.paths();
        let credentials = CredentialService::new(paths.credentials_file());
        credentials.store(SYNTHETIC_KEY).unwrap();
        let state = OnboardingState::read(&paths, &credentials).unwrap();

        let rendered = format!("{state:?}");
        assert!(!rendered.contains(SYNTHETIC_KEY));
    }

    #[test]
    fn service_onboarding_state_honors_environment_credential() {
        let layout = Layout::new();
        let overrides = layout
            .default_overrides_for_paths()
            .with_env_api_key(SYNTHETIC_KEY);
        let service = ConfigService::new(&overrides).unwrap();

        let state = service.onboarding_state().unwrap();
        assert!(
            !state.needs_credential_setup(),
            "PANGRAM_API_KEY must count as configured: {state:?}"
        );
        assert_eq!(state.credential_source(), CredentialSource::Environment);
    }

    #[test]
    fn overrides_and_service_debug_render_redacted_key() {
        let layout = Layout::new();
        let overrides = layout
            .default_overrides_for_paths()
            .with_env_api_key(SYNTHETIC_KEY);
        let service = ConfigService::new(&overrides).unwrap();

        let overrides_debug = format!("{overrides:?}");
        assert!(
            !overrides_debug.contains(SYNTHETIC_KEY),
            "key leaked into ConfigOverrides debug: {overrides_debug}"
        );
        assert!(overrides_debug.contains("[redacted]"), "{overrides_debug}");

        let service_debug = format!("{service:?}");
        assert!(
            !service_debug.contains(SYNTHETIC_KEY),
            "key leaked into ConfigService debug: {service_debug}"
        );
    }

    #[test]
    fn invalid_values_are_reported_without_echoing_the_value() {
        let layout = Layout::new();
        let store = FileConfigStore::new(layout.explicit_config.clone());

        let error = store.set("history.enabled", SYNTHETIC_KEY).unwrap_err();
        let rendered = format!("{error:?} {error}");
        assert!(matches!(error, ConfigError::InvalidValue { .. }), "{error}");
        assert!(
            !rendered.contains(SYNTHETIC_KEY),
            "raw invalid value echoed: {rendered}"
        );

        let error = store.set("tui.intro", SYNTHETIC_KEY).unwrap_err();
        let rendered = format!("{error:?} {error}");
        assert!(
            !rendered.contains(SYNTHETIC_KEY),
            "raw invalid value echoed: {rendered}"
        );
        assert!(rendered.contains("once, always, off"), "{rendered}");
    }

    #[test]
    fn malformed_toml_never_renders_source_line() {
        let layout = Layout::new();
        let store = FileConfigStore::new(layout.explicit_config.clone());
        // A malformed line embedding the synthetic key, so an unsanitized
        // `toml::de::Error` display would print it back.
        fs::write(
            &layout.explicit_config,
            format!("config_version = 1\n[SYNTHETIC_MARK_{SYNTHETIC_KEY}\n"),
        )
        .unwrap();

        let error = store.load().unwrap_err();
        let rendered = format!("{error:?} {error}");
        assert!(matches!(error, ConfigError::Invalid(_)), "{error}");
        assert!(
            !rendered.contains(SYNTHETIC_KEY),
            "malformed source line leaked: {rendered}"
        );
    }

    #[test]
    fn special_permission_bits_fail_closed() {
        let layout = Layout::new();
        let credentials = CredentialService::new(layout.credentials_file());
        credentials.store(SYNTHETIC_KEY).unwrap();

        set_mode(&layout.credentials_file(), 0o4600);
        let error = credentials.read().unwrap_err();
        assert!(
            matches!(error, ConfigError::InsecurePermissions),
            "setuid bit must fail closed: {error}"
        );

        set_mode(&layout.credentials_file(), 0o2600);
        let error = credentials.read().unwrap_err();
        assert!(
            matches!(error, ConfigError::InsecurePermissions),
            "setgid bit must fail closed: {error}"
        );
    }
}
