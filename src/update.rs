//! Signed update policy and mechanics.
//!
//! Private `0.x` builds stop at the outer policy boundary. The verifier still
//! ships behind that boundary so release artifacts can use one implementation
//! for signature, manifest, target, archive, and ownership decisions.

use std::collections::HashSet;
use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, VerifyingKey};
use schemars::JsonSchema;
use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::domain::{Sha256Hash, UtcTimestamp};
use crate::output::{CanonicalError, ErrorCode};

mod archive;
mod install;
mod network;
mod replace;
mod state;

pub use archive::validate_archive;
pub(crate) use install::run_direct_install_helper;
pub use network::{UpdateCheck, UpdateCheckKind, UpdateChecker};
#[cfg(windows)]
pub(crate) use replace::run_windows_replace_helper;
pub use replace::{
    DirectReplacement, DirectUpdateCandidate, finalize_pending_receipt, install_direct_candidate,
    replace_direct_install,
};
pub use state::{
    InstallManager, InstallReceipt, ManagerAdvisory, UpdateState, detect_manager_install,
    load_update_state, store_update_state, validate_install_receipt,
};

/// One public key accepted for detached release-manifest signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedManifestKey {
    key_id: String,
    public_key: [u8; 32],
}

impl TrustedManifestKey {
    #[must_use]
    pub fn new(key_id: impl Into<String>, public_key: [u8; 32]) -> Self {
        Self {
            key_id: key_id.into(),
            public_key,
        }
    }

    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    #[must_use]
    pub const fn public_key(&self) -> &[u8; 32] {
        &self.public_key
    }
}

/// Public keys trusted by production update checks and direct installers.
///
/// Rotation adds the next key here before release signing switches to it. The
/// corresponding private seed exists only in the protected GitHub release
/// environment.
#[must_use]
pub fn production_manifest_keys() -> Vec<TrustedManifestKey> {
    vec![TrustedManifestKey::new(
        "pangram-release-2026-01",
        [
            0xbb, 0x21, 0x97, 0x24, 0x90, 0xc1, 0x32, 0xe7, 0xf4, 0x9c, 0xaa, 0xd9, 0xb2, 0x5d,
            0x1d, 0x7e, 0x6a, 0xc3, 0xc2, 0x54, 0x0b, 0xc4, 0x27, 0x60, 0x9b, 0xe7, 0x92, 0x11,
            0x43, 0xa1, 0x26, 0x42,
        ],
    )]
}

/// Stable updater failure classes used by the contract suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateErrorKind {
    SignatureDocumentInvalid,
    UnknownManifestKey,
    DuplicateManifestKey,
    ManifestSignature,
    ManifestInvalid,
    UpdaterTooOld,
    Downgrade,
    TargetUnavailable,
    ArchiveSize,
    ArchiveHash,
    ArchiveLayout,
    InstallReceiptInvalid,
    InstallNotOwned,
    UpdateStateInvalid,
    Network,
    ReplaceFailed,
}

/// A sanitized update-contract failure. It never contains downloaded bytes.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct UpdateError {
    kind: UpdateErrorKind,
    message: &'static str,
}

impl UpdateError {
    const fn new(kind: UpdateErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    #[must_use]
    pub const fn kind(&self) -> UpdateErrorKind {
        self.kind
    }
}

/// Release target identifiers accepted by the signed manifest.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Target {
    #[serde(rename = "x86_64-unknown-linux-gnu")]
    X86_64UnknownLinuxGnu,
    #[serde(rename = "aarch64-unknown-linux-gnu")]
    Aarch64UnknownLinuxGnu,
    #[serde(rename = "x86_64-apple-darwin")]
    X86_64AppleDarwin,
    #[serde(rename = "aarch64-apple-darwin")]
    Aarch64AppleDarwin,
    #[serde(rename = "x86_64-pc-windows-msvc")]
    X86_64PcWindowsMsvc,
}

impl Target {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::X86_64UnknownLinuxGnu => "x86_64-unknown-linux-gnu",
            Self::Aarch64UnknownLinuxGnu => "aarch64-unknown-linux-gnu",
            Self::X86_64AppleDarwin => "x86_64-apple-darwin",
            Self::Aarch64AppleDarwin => "aarch64-apple-darwin",
            Self::X86_64PcWindowsMsvc => "x86_64-pc-windows-msvc",
        }
    }

    #[must_use]
    pub const fn current() -> Option<Self> {
        if cfg!(all(target_arch = "x86_64", target_os = "linux")) {
            Some(Self::X86_64UnknownLinuxGnu)
        } else if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
            Some(Self::Aarch64UnknownLinuxGnu)
        } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
            Some(Self::X86_64AppleDarwin)
        } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
            Some(Self::Aarch64AppleDarwin)
        } else if cfg!(all(target_arch = "x86_64", target_os = "windows")) {
            Some(Self::X86_64PcWindowsMsvc)
        } else {
            None
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Archive encodings emitted by the release workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum ArchiveFormat {
    #[serde(rename = "tar.xz")]
    TarXz,
    #[serde(rename = "zip")]
    Zip,
}

/// One target-bound archive in a verified manifest.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateArtifact {
    target: Target,
    archive_format: ArchiveFormat,
    #[schemars(url, regex(pattern = r"^https://"))]
    url: String,
    #[schemars(range(min = 1))]
    size_bytes: u64,
    #[schemars(range(min = 1))]
    executable_size_bytes: u64,
    sha256: Sha256Hash,
}

impl UpdateArtifact {
    #[must_use]
    pub const fn target(&self) -> Target {
        self.target
    }

    #[must_use]
    pub const fn archive_format(&self) -> ArchiveFormat {
        self.archive_format
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn executable_size_bytes(&self) -> u64 {
        self.executable_size_bytes
    }

    #[must_use]
    pub const fn sha256(&self) -> Sha256Hash {
        self.sha256
    }
}

/// The signed update manifest after exact-byte signature verification.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateManifest {
    schema_version: String,
    channel: String,
    #[schemars(regex(pattern = r"^[0-9]+\.[0-9]+\.[0-9]+$"))]
    version: String,
    published_at: UtcTimestamp,
    #[schemars(url, regex(pattern = r"^https://"))]
    notes_url: String,
    #[schemars(regex(pattern = r"^[0-9]+\.[0-9]+\.[0-9]+$"))]
    minimum_updater_version: String,
    #[schemars(length(min = 1))]
    artifacts: Vec<UpdateArtifact>,
}

impl UpdateManifest {
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub fn published_at(&self) -> UtcTimestamp {
        self.published_at
    }

    #[must_use]
    pub fn notes_url(&self) -> &str {
        &self.notes_url
    }

    #[must_use]
    pub fn artifacts(&self) -> &[UpdateArtifact] {
        &self.artifacts
    }

    /// Applies updater-floor, downgrade, equality, and exact-target policy.
    pub fn release_for(
        &self,
        current_version: &str,
        updater_version: &str,
        target: Target,
    ) -> Result<ReleaseDecision<'_>, UpdateError> {
        let current = parse_version(current_version)?;
        let updater = parse_version(updater_version)?;
        let release = parse_version(&self.version)?;
        let minimum_updater = parse_version(&self.minimum_updater_version)?;

        if updater < minimum_updater {
            return Err(UpdateError::new(
                UpdateErrorKind::UpdaterTooOld,
                "The running updater is too old for this release.",
            ));
        }
        if current > release {
            return Err(UpdateError::new(
                UpdateErrorKind::Downgrade,
                "The release manifest would downgrade the installed version.",
            ));
        }
        if current == release {
            return Ok(ReleaseDecision::NoUpdate);
        }

        self.artifacts
            .iter()
            .find(|artifact| artifact.target == target)
            .map(ReleaseDecision::Update)
            .ok_or_else(|| {
                UpdateError::new(
                    UpdateErrorKind::TargetUnavailable,
                    "The release manifest has no artifact for this target.",
                )
            })
    }

    fn validate(&self) -> Result<(), UpdateError> {
        if self.schema_version != "1" || self.channel != "stable" {
            return Err(invalid_manifest());
        }
        parse_version(&self.version)?;
        parse_version(&self.minimum_updater_version)?;
        validate_https_url(&self.notes_url)?;
        if self.artifacts.is_empty() {
            return Err(invalid_manifest());
        }

        let mut targets = HashSet::with_capacity(self.artifacts.len());
        for artifact in &self.artifacts {
            if artifact.size_bytes == 0
                || artifact.executable_size_bytes == 0
                || !targets.insert(artifact.target)
                || !archive_matches_target(artifact.archive_format, artifact.target)
            {
                return Err(invalid_manifest());
            }
            validate_https_url(&artifact.url)?;
        }
        Ok(())
    }
}

/// Whether the verified release is newer than the installed version.
#[derive(Clone, Copy, Debug)]
pub enum ReleaseDecision<'a> {
    NoUpdate,
    Update(&'a UpdateArtifact),
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestSignature {
    schema_version: String,
    algorithm: String,
    #[schemars(length(min = 1))]
    key_id: String,
    signature: String,
}

/// Verifies the detached signature over the exact bytes, then parses and
/// validates the signed value. Do not move manifest parsing above `verify_strict`.
pub fn verify_manifest(
    manifest_bytes: &[u8],
    signature_document: &[u8],
    trusted_keys: &[TrustedManifestKey],
) -> Result<UpdateManifest, UpdateError> {
    let descriptor: ManifestSignature =
        serde_json::from_slice(signature_document).map_err(|_| {
            UpdateError::new(
                UpdateErrorKind::SignatureDocumentInvalid,
                "The detached manifest signature document is invalid.",
            )
        })?;
    if descriptor.schema_version != "1"
        || descriptor.algorithm != "ed25519"
        || descriptor.key_id.is_empty()
    {
        return Err(UpdateError::new(
            UpdateErrorKind::SignatureDocumentInvalid,
            "The detached manifest signature document is invalid.",
        ));
    }

    let matching_keys = trusted_keys
        .iter()
        .filter(|key| key.key_id == descriptor.key_id)
        .collect::<Vec<_>>();
    let trusted_key = match matching_keys.as_slice() {
        [] => {
            return Err(UpdateError::new(
                UpdateErrorKind::UnknownManifestKey,
                "The manifest was signed by an unknown key.",
            ));
        }
        [key] => *key,
        _ => {
            return Err(UpdateError::new(
                UpdateErrorKind::DuplicateManifestKey,
                "The embedded manifest key ring contains a duplicate key ID.",
            ));
        }
    };

    let signature_bytes = STANDARD.decode(descriptor.signature).map_err(|_| {
        UpdateError::new(
            UpdateErrorKind::SignatureDocumentInvalid,
            "The detached manifest signature document is invalid.",
        )
    })?;
    let signature = Signature::from_slice(&signature_bytes).map_err(|_| {
        UpdateError::new(
            UpdateErrorKind::SignatureDocumentInvalid,
            "The detached manifest signature document is invalid.",
        )
    })?;
    let verifying_key = VerifyingKey::from_bytes(&trusted_key.public_key).map_err(|_| {
        UpdateError::new(
            UpdateErrorKind::DuplicateManifestKey,
            "The embedded manifest key ring contains an invalid public key.",
        )
    })?;
    verifying_key
        .verify_strict(manifest_bytes, &signature)
        .map_err(|_| {
            UpdateError::new(
                UpdateErrorKind::ManifestSignature,
                "The update manifest signature is invalid.",
            )
        })?;

    let manifest: UpdateManifest = serde_json::from_slice(manifest_bytes).map_err(|_| {
        UpdateError::new(
            UpdateErrorKind::ManifestInvalid,
            "The signed update manifest is invalid.",
        )
    })?;
    manifest.validate()?;
    Ok(manifest)
}

fn parse_version(value: &str) -> Result<Version, UpdateError> {
    let version = Version::parse(value).map_err(|_| invalid_manifest())?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(invalid_manifest());
    }
    Ok(version)
}

fn validate_https_url(value: &str) -> Result<(), UpdateError> {
    let url = Url::parse(value).map_err(|_| invalid_manifest())?;
    if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() {
        return Err(invalid_manifest());
    }
    Ok(())
}

const fn archive_matches_target(format: ArchiveFormat, target: Target) -> bool {
    matches!(
        (format, target),
        (ArchiveFormat::Zip, Target::X86_64PcWindowsMsvc)
            | (
                ArchiveFormat::TarXz,
                Target::X86_64UnknownLinuxGnu
                    | Target::Aarch64UnknownLinuxGnu
                    | Target::X86_64AppleDarwin
                    | Target::Aarch64AppleDarwin
            )
    )
}

const fn invalid_manifest() -> UpdateError {
    UpdateError::new(
        UpdateErrorKind::ManifestInvalid,
        "The signed update manifest is invalid.",
    )
}

pub(crate) fn private_build_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::UpdateUnavailable,
        "Updates are unavailable for private development builds.",
    )
    .expect("the fixed update-unavailable message is non-empty")
}
