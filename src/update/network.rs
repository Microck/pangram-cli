//! Explicit signed-manifest checks. No adapter calls this automatically.

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{ETAG, IF_NONE_MATCH};
use url::Url;

use super::{
    ReleaseDecision, Target, TrustedManifestKey, UpdateError, UpdateErrorKind, UpdateManifest,
    UpdateState, verify_manifest,
};
use crate::domain::UtcTimestamp;

const MANIFEST_URL: &str =
    "https://github.com/Microck/pangram-cli/releases/latest/download/pangram-update-manifest.json";
const SIGNATURE_URL: &str = "https://github.com/Microck/pangram-cli/releases/latest/download/pangram-update-manifest.json.sig";
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_SIGNATURE_BYTES: usize = 16 * 1024;

/// Result class for one explicit update check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateCheckKind {
    NotModified,
    NoUpdate,
    UpdateAvailable,
}

/// Verified check result plus the state that may be committed atomically.
#[derive(Clone, Debug)]
pub struct UpdateCheck {
    kind: UpdateCheckKind,
    state: UpdateState,
    manifest: Option<UpdateManifest>,
}

impl UpdateCheck {
    #[must_use]
    pub const fn kind(&self) -> UpdateCheckKind {
        self.kind
    }

    #[must_use]
    pub const fn state(&self) -> &UpdateState {
        &self.state
    }

    #[must_use]
    pub const fn manifest(&self) -> Option<&UpdateManifest> {
        self.manifest.as_ref()
    }
}

/// Fixed-production-endpoint update checker.
#[derive(Clone, Debug)]
pub struct UpdateChecker {
    client: reqwest::Client,
    manifest_url: Url,
    signature_url: Url,
}

impl UpdateChecker {
    pub fn production() -> Result<Self, UpdateError> {
        Self::build(MANIFEST_URL, SIGNATURE_URL, false)
    }

    /// Loopback-only constructor compiled solely with the repository test
    /// feature. Production code cannot override release endpoints.
    #[cfg(feature = "dev-tools")]
    #[doc(hidden)]
    pub fn for_test(
        manifest_url: impl AsRef<str>,
        signature_url: impl AsRef<str>,
    ) -> Result<Self, UpdateError> {
        Self::build(manifest_url.as_ref(), signature_url.as_ref(), true)
    }

    fn build(manifest_url: &str, signature_url: &str, no_proxy: bool) -> Result<Self, UpdateError> {
        let manifest_url = Url::parse(manifest_url).map_err(|_| network_error())?;
        let signature_url = Url::parse(signature_url).map_err(|_| network_error())?;
        if !no_proxy
            && (manifest_url.as_str() != MANIFEST_URL || signature_url.as_str() != SIGNATURE_URL)
        {
            return Err(network_error());
        }
        let mut builder = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("pangram/", env!("CARGO_PKG_VERSION")));
        if no_proxy {
            builder = builder.no_proxy();
        }
        let client = builder.build().map_err(|_| network_error())?;
        Ok(Self {
            client,
            manifest_url,
            signature_url,
        })
    }

    /// Performs one explicit check. The caller receives a replacement state
    /// value only on a verified 200 or a valid 304, so failures cannot mutate
    /// the prior state.
    pub async fn check(
        &self,
        prior_state: Option<&UpdateState>,
        checked_at: UtcTimestamp,
        current_version: &str,
        updater_version: &str,
        target: Target,
        trusted_keys: &[TrustedManifestKey],
    ) -> Result<UpdateCheck, UpdateError> {
        let mut request = self.client.get(self.manifest_url.clone());
        if let Some(etag) = prior_state.and_then(UpdateState::etag) {
            request = request.header(IF_NONE_MATCH, etag);
        }
        let response = request.send().await.map_err(|_| network_error())?;

        if response.status() == StatusCode::NOT_MODIFIED {
            let prior = prior_state.ok_or_else(network_error)?;
            return Ok(UpdateCheck {
                kind: UpdateCheckKind::NotModified,
                state: prior.not_modified(checked_at),
                manifest: None,
            });
        }
        if response.status() != StatusCode::OK {
            return Err(network_error());
        }

        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let manifest_bytes = bounded_body(response, MAX_MANIFEST_BYTES).await?;
        let signature_response = self
            .client
            .get(self.signature_url.clone())
            .send()
            .await
            .map_err(|_| network_error())?;
        if signature_response.status() != StatusCode::OK {
            return Err(network_error());
        }
        let signature_bytes = bounded_body(signature_response, MAX_SIGNATURE_BYTES).await?;
        let manifest = verify_manifest(&manifest_bytes, &signature_bytes, trusted_keys)?;
        let decision = manifest.release_for(current_version, updater_version, target)?;
        let (kind, available_version) = match decision {
            ReleaseDecision::NoUpdate => (UpdateCheckKind::NoUpdate, None),
            ReleaseDecision::Update(_) => (
                UpdateCheckKind::UpdateAvailable,
                Some(manifest.version().to_owned()),
            ),
        };
        let state = UpdateState::checked(checked_at, etag, available_version)?;
        Ok(UpdateCheck {
            kind,
            state,
            manifest: Some(manifest),
        })
    }
}

async fn bounded_body(
    mut response: reqwest::Response,
    maximum: usize,
) -> Result<Vec<u8>, UpdateError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        return Err(network_error());
    }
    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(maximum);
    let mut bytes = Vec::with_capacity(initial_capacity);
    while let Some(chunk) = response.chunk().await.map_err(|_| network_error())? {
        if chunk.len() > maximum.saturating_sub(bytes.len()) {
            return Err(network_error());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

const fn network_error() -> UpdateError {
    UpdateError::new(
        UpdateErrorKind::Network,
        "The update check could not retrieve a valid release manifest.",
    )
}
