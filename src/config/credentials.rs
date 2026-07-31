//! The dedicated, fail-closed credential store.
//!
//! Contract rules implemented here:
//! - the store is a `credentials.toml` under the default platform config
//!   directory; nothing can relocate it (the path is supplied by `Paths`)
//! - the file contains exactly `credentials_version = 1` and `api_key`
//! - on Unix, writes and every read require mode exactly `0600` with no
//!   setuid/setgid/sticky or other special bits; anything else fails closed
//!   with `InsecurePermissions`
//! - on Windows, the DACL must be protected with exactly one ACCESS_ALLOWED
//!   ACE for the current process-token user and a mask no broader than
//!   read/write/delete plus the security rights the service needs to
//!   re-apply protection; anything else fails closed
//!   (`windows_acl` owns the exact rule)
//! - the in-memory value is `secrecy::SecretString`; no `Debug` or error path
//!   can expose the key; buffers are zeroized after save
//! - persistence is atomic: unique sibling temp, permissions set on the open
//!   handle before rename, `sync_all`, rename, directory sync

use std::fs;
use std::path::{Path, PathBuf};

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::{ConfigError, ConfigOverrides};

pub const CREDENTIALS_FILE_NAME: &str = "credentials.toml";
const CREDENTIALS_VERSION: u8 = 1;

/// Wire shape of the restricted credential file. Unknown fields are
/// rejected so nothing else can hide inside this file.
///
/// This type is intentionally private and deliberately has no `Debug`:
/// `api_key` holds raw credential material, and letting it derive `Debug`
/// (or exposing it) would create a leak channel. Serialization contexts
/// wrap the rendered text in `Zeroizing`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialsToml {
    credentials_version: u8,
    api_key: String,
}

/// Where the effective credential came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    None,
    Environment,
    Stored,
}

/// The outcome of resolving the effective credential. The key, when present,
/// is secret and never `Debug`-printable through this type.
#[derive(Clone)]
pub struct CredentialResolution {
    source: CredentialSource,
    key: Option<SecretString>,
}

impl CredentialResolution {
    pub const fn none() -> Self {
        Self {
            source: CredentialSource::None,
            key: None,
        }
    }

    fn environment(key: SecretString) -> Self {
        Self {
            source: CredentialSource::Environment,
            key: Some(key),
        }
    }

    fn stored(key: SecretString) -> Self {
        Self {
            source: CredentialSource::Stored,
            key: Some(key),
        }
    }

    pub const fn source(&self) -> CredentialSource {
        self.source
    }

    pub const fn is_configured(&self) -> bool {
        self.key.is_some()
    }
}

impl std::fmt::Debug for CredentialResolution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialResolution")
            .field("source", &self.source)
            .field("key_present", &self.key.is_some())
            .finish()
    }
}

/// The restricted credential file plus fail-closed permission enforcement.
#[derive(Debug, Clone)]
pub struct CredentialService {
    path: PathBuf,
}

impl CredentialService {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The effective credential: `PANGRAM_API_KEY` wins over the stored key.
    /// An absent stored file is not an error.
    pub fn resolve(
        &self,
        overrides: &ConfigOverrides,
    ) -> Result<CredentialResolution, ConfigError> {
        // A present-but-non-UTF-8 `PANGRAM_API_KEY` must not silently fall
        // back to the stored credential (wrong key): the environment override
        // would win if it were decodable, so undecodable input fails closed.
        // No key material or raw bytes are echoed into the message.
        if overrides.env_api_key_invalid() {
            return Err(ConfigError::InvalidValue {
                key: "PANGRAM_API_KEY".into(),
                reason: "must be valid UTF-8 when set".into(),
            });
        }
        if let Some(key) = overrides.env_api_key() {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                return Ok(CredentialResolution::environment(SecretString::from(
                    trimmed.to_owned(),
                )));
            }
        }
        match self.read()? {
            Some(key) => Ok(CredentialResolution::stored(key)),
            None => Ok(CredentialResolution::none()),
        }
    }

    /// `auth status`: source plus a masked suffix of at most 8 characters.
    /// The suffix is the trailing characters only; the service never hands
    /// out the key through this path.
    pub fn status(
        &self,
        overrides: &ConfigOverrides,
    ) -> Result<(CredentialSource, Option<String>), ConfigError> {
        let resolution = self.resolve(overrides)?;
        let suffix = resolution
            .key
            .as_ref()
            .map(|key| masked_suffix(key.expose_secret()));
        Ok((resolution.source(), suffix))
    }

    /// The trailing characters of the resolved key, bounded to 8, for
    /// display in auth status. Full keys are never exposed.
    pub fn masked_suffix_for(
        overrides: &ConfigOverrides,
        resolution: &CredentialResolution,
    ) -> Option<String> {
        let _ = overrides;
        resolution
            .key
            .as_ref()
            .map(|key| masked_suffix(key.expose_secret()))
    }

    /// Reads the stored credential. Returns `Ok(None)` when the file does
    /// not exist; fails closed on insecure permissions or malformed content.
    ///
    /// Existence is probed with an error-preserving metadata call rather than
    /// `Path::exists()`: `Path::exists()` swallows lookup errors (an
    /// unsearchable parent directory or a denied `stat`) and reports them as
    /// absence, which would make `auth status` and `doctor` claim "no key is
    /// configured" while the store may exist but be unreadable. Only an
    /// explicit `NotFound` means absent; every other lookup error fails closed.
    pub fn read(&self) -> Result<Option<SecretString>, ConfigError> {
        match fs::metadata(&self.path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(ConfigError::from(error)),
        }
        platform::enforce_permissions_on_read(&self.path)?;
        let contents = fs::read_to_string(&self.path).map_err(ConfigError::from)?;
        let parsed: CredentialsToml = toml::from_str(&contents).map_err(super::sanitize_toml_de)?;
        if parsed.credentials_version != CREDENTIALS_VERSION {
            return Err(ConfigError::InvalidValue {
                key: "credentials_version".into(),
                reason: format!(
                    "must be {CREDENTIALS_VERSION}, found {}",
                    parsed.credentials_version
                ),
            });
        }
        let key = parsed.api_key.trim();
        if key.is_empty() {
            return Err(ConfigError::Invalid(
                "stored credential file contains an empty api_key".into(),
            ));
        }
        Ok(Some(SecretString::from(key.to_owned())))
    }

    /// Validates and atomically persists an API key. Fails closed when
    /// restrictive permissions cannot be established.
    pub fn store(&self, api_key: &str) -> Result<(), ConfigError> {
        let key = api_key.trim();
        if key.is_empty() {
            return Err(ConfigError::Invalid(
                "an empty API key cannot be stored".into(),
            ));
        }
        let version = CREDENTIALS_VERSION;
        let mut serialized = Zeroizing::new(
            toml::to_string(&CredentialsToml {
                credentials_version: version,
                api_key: key.to_owned(),
            })
            .map_err(ConfigError::from)?,
        );
        atomic_secret_write(&self.path, serialized.as_bytes())?;
        serialized.zeroize();
        Ok(())
    }

    /// Removes the stored credential. Idempotent: a missing file is success.
    ///
    /// Absence is recognized with an error-preserving metadata probe before
    /// the ACL check: on Windows, `enforce_permissions_before_remove` queries
    /// the file's security descriptor and cannot run against a path that does
    /// not exist, so an absent `credentials.toml` must be matched as success
    /// before that verification (a non-Windows platform treats the pre-remove
    /// check as a no-op, which is why the bug only surfaced on Windows).
    pub fn remove(&self) -> Result<(), ConfigError> {
        match fs::metadata(&self.path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(ConfigError::from(error)),
        }
        platform::enforce_permissions_before_remove(&self.path)?;
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ConfigError::Io(format!(
                "cannot remove {}: {error}",
                self.path.display()
            ))),
        }
    }

    /// Interactive masked read from the controlling terminal. Never echoes.
    /// Rejects empty input; the caller decides whether to store.
    pub fn prompt_masked(prompt: &str) -> Result<SecretString, ConfigError> {
        let value = rpassword::prompt_password(prompt)
            .map_err(|error| ConfigError::Io(format!("masked prompt failed: {error}")))?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(ConfigError::Invalid(
                "an empty API key was not accepted".into(),
            ));
        }
        Ok(SecretString::from(trimmed.to_owned()))
    }

    /// Noninteractive stdin read for `auth set --api-key-stdin`: exactly one
    /// UTF-8 line. Anything more is a usage failure surfaced by the adapter.
    pub fn read_stdin_line(input: &str) -> Result<&str, ConfigError> {
        let mut lines = input.lines();
        let first = lines.next().unwrap_or("").trim();
        if first.is_empty() {
            return Err(ConfigError::Invalid(
                "no API key was received on stdin".into(),
            ));
        }
        if lines.next().is_some() {
            return Err(ConfigError::Invalid(
                "--api-key-stdin accepts exactly one line".into(),
            ));
        }
        Ok(first)
    }
}

/// Write with permissions established on the open handle before the rename,
/// so no window exists where a world-readable temp holds the key.
fn atomic_secret_write(path: &Path, contents: &[u8]) -> Result<(), ConfigError> {
    let parent = path
        .parent()
        .ok_or_else(|| ConfigError::Io(format!("{} has no parent directory", path.display())))?;
    fs::create_dir_all(parent)
        .map_err(|error| ConfigError::Io(format!("cannot create {}: {error}", parent.display())))?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ConfigError::Io(format!("{} is not valid UTF-8", path.display())))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary = parent.join(format!(".{file_name}.{}-{nonce}.tmp", std::process::id()));

    let result = (|| -> Result<(), ConfigError> {
        {
            use std::io::Write as _;
            // create_new fails if the unique sibling already exists, so the
            // temp file can never be pre-created with lax permissions. On Unix
            // the 0600 mode is applied atomically at creation (not only by the
            // later `set_permissions`) so the file never exists with a wider
            // mode, even though content is written only after the restrict
            // step below.
            #[allow(unused_mut)]
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary).map_err(|error| {
                ConfigError::Io(format!("cannot stage credential write: {error}"))
            })?;
            platform::restrict_handle_permissions(&file)?;
            file.write_all(contents).map_err(|error| {
                ConfigError::Io(format!("cannot stage credential write: {error}"))
            })?;
            file.sync_all().map_err(|error| {
                ConfigError::Io(format!("cannot sync credential file: {error}"))
            })?;
        }
        fs::rename(&temporary, path).map_err(|error| {
            ConfigError::Io(format!("cannot publish {}: {error}", path.display()))
        })?;
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

/// The trailing suffix for display, at most 8 characters. Constant-shape
/// work over a trimmed key; never logs the key itself.
///
/// A key of 8 or fewer characters is never returned even partially: any
/// returned suffix would expose the whole key, which the contract forbids.
/// In that case a fixed masked marker is returned instead of the suffix.
fn masked_suffix(key: &str) -> String {
    let grapheme_buffer: Vec<char> = key.chars().collect();
    if grapheme_buffer.len() <= 8 {
        // The suffix would be the entire key; emit a constant ASCII masked
        // marker so `auth status` never reproduces the stored credential and
        // the source/documentation stays within the repository's ASCII
        // punctuation rule.
        return "********".to_owned();
    }
    let take = 8;
    let start = grapheme_buffer.len() - take;
    let mut buffer = grapheme_buffer;
    buffer.drain(..start);
    buffer.into_iter().collect()
}

#[cfg(windows)]
mod platform {
    use crate::config::ConfigError;
    use crate::config::windows_acl;
    use std::fs::File;
    use std::path::Path;

    /// Every read requires the exact owner-only protected DACL; anything
    /// else fails closed with `InsecurePermissions` (or `RestrictionFailed`
    /// when enforcement machinery itself is unreachable).
    pub fn enforce_permissions_on_read(path: &Path) -> Result<(), ConfigError> {
        windows_acl::enforce_owner_only(path)
    }

    /// The owner-only ACL is applied to the staged temp file before content
    /// is synced, so no window exists with a readable temp; after rename the
    /// final path is verified again by the caller re-reading.
    pub fn restrict_handle_permissions(file: &File) -> Result<(), ConfigError> {
        windows_acl::restrict_handle_permissions(file)
    }

    /// Removal verifies the ACL too: a tampered file fails closed rather
    /// than being unlinked unseen.
    pub fn enforce_permissions_before_remove(path: &Path) -> Result<(), ConfigError> {
        windows_acl::enforce_owner_only(path)
    }
}

#[cfg(not(windows))]
mod platform {
    #[cfg(unix)]
    mod imp {
        use super::super::ConfigError;
        use std::fs::{self, File};
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;

        /// Every read requires exactly 0600 with no setuid/setgid/sticky or
        /// other special bits. Masking against `0o7777` (not `0o777`) keeps
        /// special bits from slipping through; too-open or unexpected modes
        /// fail closed and the adapter maps this to
        /// `insecure_config_permissions`.
        pub fn enforce_permissions_on_read(path: &Path) -> Result<(), ConfigError> {
            let metadata = fs::metadata(path).map_err(|error| {
                ConfigError::Io(format!("cannot stat {}: {error}", path.display()))
            })?;
            let mode = metadata.permissions().mode() & 0o7777;
            if mode != 0o600 {
                return Err(ConfigError::InsecurePermissions);
            }
            Ok(())
        }

        /// Permissions are tightened on the open handle before any content
        /// is durably synced, so the file never exists world-readable.
        pub fn restrict_handle_permissions(file: &File) -> Result<(), ConfigError> {
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| ConfigError::RestrictionFailed)
        }

        /// Removal is the one state change allowed on an insecure file: it
        /// only ever reduces exposure.
        pub fn enforce_permissions_before_remove(_path: &Path) -> Result<(), ConfigError> {
            Ok(())
        }
    }

    #[cfg(not(unix))]
    mod imp {
        use super::super::ConfigError;
        use std::fs::File;
        use std::path::Path;

        // Other platforms without a defined permission model fail closed.
        pub fn enforce_permissions_on_read(_path: &Path) -> Result<(), ConfigError> {
            Err(ConfigError::RestrictionFailed)
        }
        pub fn restrict_handle_permissions(_file: &File) -> Result<(), ConfigError> {
            Err(ConfigError::RestrictionFailed)
        }
        pub fn enforce_permissions_before_remove(_path: &Path) -> Result<(), ConfigError> {
            Err(ConfigError::RestrictionFailed)
        }
    }

    pub use imp::*;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn service(root: &TempDir) -> CredentialService {
        CredentialService::new(root.path().join(CREDENTIALS_FILE_NAME))
    }

    #[test]
    fn invalid_environment_credential_fails_closed_instead_of_falling_back() {
        let root = tempfile::tempdir().unwrap();
        let service = service(&root);
        // A present-but-undecodable PANGRAM_API_KEY must not silently resolve
        // to the stored (or absent) credential; environment precedence means
        // undecodable input is a fail-closed error.
        let overrides = ConfigOverrides::for_test_invalid_env_api_key();
        let error = service.resolve(&overrides).unwrap_err();
        let rendered = error.to_string();
        assert!(
            rendered.contains("PANGRAM_API_KEY"),
            "the offending variable is named: {rendered}"
        );
        assert!(
            !rendered.contains("API key") || rendered.contains("must be valid UTF-8"),
            "message only describes the encoding requirement: {rendered}"
        );
    }

    #[test]
    fn missing_file_resolves_to_none() {
        let root = tempfile::tempdir().unwrap();
        let service = service(&root);
        let resolution = service.resolve(&ConfigOverrides::default()).unwrap();
        assert_eq!(resolution.source(), CredentialSource::None);
        assert!(!resolution.is_configured());
    }

    #[test]
    fn remove_absent_file_is_success_on_every_platform() {
        let root = tempfile::tempdir().unwrap();
        let service = service(&root);
        assert!(!service.path().exists());
        // Idempotence holds on every platform: absence must be matched as
        // success before Windows ACL verification can attempt to query a
        // nonexistent security descriptor. Prior to this fix, `remove()` on
        // Windows ran `enforce_owner_only` first and returned
        // `RestrictionFailed` for an absent file.
        service.remove().unwrap();
        service.remove().unwrap();
        assert!(!service.path().exists());
    }

    #[cfg(unix)]
    mod unix {
        use super::super::*;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::TempDir;

        const SYNTHETIC_KEY: &str =
            "pangram_synthetic_contract_test_key_0000000000000000_NOT_A_REAL_KEY";

        fn service(root: &TempDir) -> CredentialService {
            CredentialService::new(root.path().join(CREDENTIALS_FILE_NAME))
        }

        #[test]
        fn store_persists_with_exact_0600_and_roundtrips() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            service.store(SYNTHETIC_KEY).unwrap();

            let mode = fs::metadata(service.path()).unwrap().permissions().mode() & 0o7777;
            assert_eq!(mode, 0o600, "credentials.toml must be owner-only");

            let contents = fs::read_to_string(service.path()).unwrap();
            assert!(contents.contains("credentials_version = 1"), "{contents}");
            assert!(contents.contains("api_key"), "{contents}");

            let resolution = service.resolve(&ConfigOverrides::default()).unwrap();
            assert_eq!(resolution.source(), CredentialSource::Stored);
            assert!(resolution.is_configured());
        }

        #[test]
        fn stored_file_content_is_exactly_two_fields() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            service.store(SYNTHETIC_KEY).unwrap();
            let parsed: CredentialsToml =
                toml::from_str(&fs::read_to_string(service.path()).unwrap()).unwrap();
            assert_eq!(parsed.credentials_version, 1);
            assert_eq!(parsed.api_key, SYNTHETIC_KEY);
        }

        #[test]
        fn read_fails_closed_on_too_open_permissions() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            service.store(SYNTHETIC_KEY).unwrap();
            fs::set_permissions(service.path(), fs::Permissions::from_mode(0o644)).unwrap();

            let error = service.read().unwrap_err();
            assert!(matches!(error, ConfigError::InsecurePermissions), "{error}");
        }

        #[test]
        fn read_fails_closed_on_too_tight_permissions() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            service.store(SYNTHETIC_KEY).unwrap();
            fs::set_permissions(service.path(), fs::Permissions::from_mode(0o400)).unwrap();

            let error = service.read().unwrap_err();
            assert!(
                matches!(error, ConfigError::InsecurePermissions),
                "exact 0600 is the contract: {error}"
            );
        }

        #[test]
        fn read_fails_closed_on_special_permission_bits() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            service.store(SYNTHETIC_KEY).unwrap();
            // Set the setuid bit on top of 0600; exactness must consider
            // special bits, not just the low 0o777 mask.
            fs::set_permissions(service.path(), fs::Permissions::from_mode(0o4600)).unwrap();

            let error = service.read().unwrap_err();
            assert!(
                matches!(error, ConfigError::InsecurePermissions),
                "special bits must fail closed: {error}"
            );
        }

        #[test]
        fn malformed_credential_error_never_renders_the_key_line() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            let path = service.path().to_path_buf();
            // A broken TOML line that embeds the synthetic key so the parser
            // error's rendered source snippet would contain it unless
            // sanitized.
            fs::write(
                &path,
                format!("credentials_version = 1\napi_key = \"{SYNTHETIC_KEY}\n"),
            )
            .unwrap();
            fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

            let error = service.read().unwrap_err();
            let rendered = format!("{error:?} {error}");
            assert!(
                !rendered.contains(SYNTHETIC_KEY),
                "key leaked into TOML parse error: {rendered}"
            );
        }

        #[test]
        fn remove_is_idempotent_and_works_after_insecure_change() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            service.store(SYNTHETIC_KEY).unwrap();
            fs::set_permissions(service.path(), fs::Permissions::from_mode(0o644)).unwrap();

            // Removal is allowed even from an insecure state: it only reduces
            // exposure.
            service.remove().unwrap();
            assert!(!service.path().exists());
            service.remove().unwrap();
        }

        #[test]
        fn rewrite_replaces_key_and_keeps_0600() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            service.store(SYNTHETIC_KEY).unwrap();
            service
                .store("pangram_second_synthetic_key_2_NOT_REAL")
                .unwrap();

            let mode = fs::metadata(service.path()).unwrap().permissions().mode() & 0o7777;
            assert_eq!(mode, 0o600);
            let contents = fs::read_to_string(service.path()).unwrap();
            assert!(contents.contains("pangram_second_synthetic_key_2_NOT_REAL"));
            assert!(!contents.contains(SYNTHETIC_KEY));
        }

        #[test]
        fn no_temp_files_survive_a_write() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            service.store(SYNTHETIC_KEY).unwrap();
            let leftovers: Vec<_> = fs::read_dir(root.path())
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
                .collect();
            assert!(leftovers.is_empty());
        }

        #[test]
        fn environment_key_wins_over_stored() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            service.store(SYNTHETIC_KEY).unwrap();

            let overrides = ConfigOverrides::default()
                .with_env_api_key("pangram_env_override_synthetic_key_NOT_REAL");
            let resolution = service.resolve(&overrides).unwrap();
            assert_eq!(resolution.source(), CredentialSource::Environment);
            let (_, suffix) = service.status(&overrides).unwrap();
            assert!(
                "pangram_env_override_synthetic_key_NOT_REAL".ends_with(suffix.as_deref().unwrap()),
                "suffix must come from the environment key"
            );
        }

        #[test]
        fn masked_suffix_is_bounded_and_trailing() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            service.store(SYNTHETIC_KEY).unwrap();

            let (source, suffix) = service.status(&ConfigOverrides::default()).unwrap();
            assert_eq!(source, CredentialSource::Stored);
            let suffix = suffix.unwrap();
            assert!(suffix.len() <= 8, "{suffix}");
            assert!(SYNTHETIC_KEY.ends_with(&suffix), "{suffix}");
        }

        #[test]
        fn short_keys_are_never_reproduced_in_status() {
            // A suffix of a key of 8 or fewer characters would be the whole
            // key; the contract forbids ever exposing full keys, so these
            // must collapse to a constant masked marker rather than echo any
            // key material.
            for short in ["abcd", "12345678", "x", ""] {
                let suffix = masked_suffix(short);
                assert_ne!(suffix, short, "short key leaked verbatim: {short:?}");
                assert!(
                    !short.is_empty() && !suffix.ends_with(short) || short.is_empty(),
                    "short key leaked as a suffix: {short:?}"
                );
            }
            let suffix = masked_suffix(SYNTHETIC_KEY);
            assert_eq!(suffix.chars().count(), 8);
        }

        #[test]
        fn errors_and_debug_carry_no_key_material() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            service.store(SYNTHETIC_KEY).unwrap();
            fs::set_permissions(service.path(), std::fs::Permissions::from_mode(0o644)).unwrap();

            let error = service.read().unwrap_err();
            let rendered = format!("{error:?} {error}");
            assert!(
                !rendered.contains(SYNTHETIC_KEY),
                "key leaked into error: {rendered}"
            );
            assert!(
                !rendered.contains("api_key value"),
                "no credential payload in debug: {rendered}"
            );

            let overrides = ConfigOverrides::default().with_env_api_key(SYNTHETIC_KEY);
            let overrides_debug = format!("{overrides:?}");
            assert!(
                !overrides_debug.contains(SYNTHETIC_KEY),
                "key leaked into ConfigOverrides Debug: {overrides_debug}"
            );
            assert!(overrides_debug.contains("[redacted]"), "{overrides_debug}");

            let resolution = CredentialResolution::environment(SecretString::from(SYNTHETIC_KEY));
            let debug = format!("{resolution:?}");
            assert!(!debug.contains(SYNTHETIC_KEY), "key leaked into Debug");
        }

        #[test]
        fn wrong_stored_version_is_invalid() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            let path = service.path().to_path_buf();
            fs::write(&path, "credentials_version = 2\napi_key = \"whatever\"\n").unwrap();
            fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            let error = service.read().unwrap_err();
            assert!(matches!(error, ConfigError::InvalidValue { .. }), "{error}");
        }

        #[test]
        fn extra_stored_fields_are_rejected() {
            let root = tempfile::tempdir().unwrap();
            let service = service(&root);
            let path = service.path().to_path_buf();
            fs::write(
                &path,
                "credentials_version = 1\napi_key = \"whatever\"\nextra = 1\n",
            )
            .unwrap();
            fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            service.read().unwrap_err();
        }

        #[test]
        fn stdin_line_accepts_exactly_one_line() {
            assert_eq!(
                CredentialService::read_stdin_line("key-only\n").unwrap(),
                "key-only"
            );
            CredentialService::read_stdin_line("first\nsecond\n").unwrap_err();
            CredentialService::read_stdin_line("").unwrap_err();
            CredentialService::read_stdin_line("\n\n").unwrap_err();
        }
    }
}
