//! Credential and endpoint plumbing for detection: effective-key resolution,
//! the fixed-production or (dev-tools-only) loopback client, and the process
//! SIGINT cancellation slot. Secret material lives only inside the
//! endpoint-bearing client; this module never logs or renders it. The
//! loopback override is compiled out of every production build.

use secrecy::SecretString;
use tokio_util::sync::CancellationToken;

#[cfg(any(test, feature = "dev-tools", doctest))]
use crate::analysis::UpstreamEndpoints;
use crate::analysis::{AnalysisConfig, Analyzer};
use crate::config::{ConfigError, CredentialSource};
use crate::output::{CanonicalError, ErrorCode};

/// Maps a credential-resolution failure onto the authentication category.
pub(crate) fn credential_error(error: ConfigError) -> CanonicalError {
    let code = match &error {
        ConfigError::InsecurePermissions | ConfigError::RestrictionFailed => {
            ErrorCode::InsecureConfigPermissions
        }
        ConfigError::InvalidValue { .. } => ErrorCode::InvalidApiKey,
        _ => ErrorCode::InvalidConfig,
    };
    CanonicalError::new(code, error.to_string()).unwrap_or_else(|_| {
        CanonicalError::new(code, "credential resolution failed").expect("fixed message")
    })
}

/// Resolves the effective API key (`PANGRAM_API_KEY` over stored) into
/// secret material for client construction. Returns `MissingApiKey` when no
/// credential is configured.
pub(crate) fn resolve_api_key(
    service: &crate::config::ConfigService,
) -> Result<SecretString, CanonicalError> {
    let resolution = service
        .credentials()
        .resolve(service.overrides())
        .map_err(credential_error)?;
    match resolution.source() {
        CredentialSource::None => Err(super::render::missing_api_key_error()),
        CredentialSource::Environment | CredentialSource::Stored => {
            // The resolution owns the SecretString; cloning shares the secrecy
            // wrapper without exposing the value.
            Ok(resolution_key(&resolution))
        }
    }
}

fn resolution_key(resolution: &crate::config::CredentialResolution) -> SecretString {
    resolution
        .key_for_client()
        .expect("a configured resolution always carries its key")
}

/// Builds an `Analyzer` from the resolved service and (test-only) loopback
/// endpoint override. Production uses the fixed text endpoint; the loopback
/// path exists only for the `dev-tools` fixture and refuses non-loopback
/// hosts at construction.
pub(crate) fn build_analyzer(
    service: &crate::config::ConfigService,
    api_key: SecretString,
) -> Result<Analyzer, CanonicalError> {
    let config = service.effective().map_err(credential_error)?;
    let rate = config
        .network
        .as_ref()
        .and_then(|network| network.max_requests_per_second);
    let analysis_config = AnalysisConfig::production(rate);

    let client = build_client(api_key, analysis_config)?;

    Ok(Analyzer::from_client(client))
}

/// Builds the endpoint-bearing client. The loopback override exists only
/// when the dev-tools loopback constructor is compiled in; a normal build
/// always selects the fixed production endpoint.
#[cfg(any(test, feature = "dev-tools", doctest))]
fn build_client(
    api_key: SecretString,
    config: AnalysisConfig,
) -> Result<crate::analysis::UpstreamClient, CanonicalError> {
    let base = std::env::var("PANGRAM_DETECT_ENDPOINT").ok();
    let endpoints = base
        .as_deref()
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .map(UpstreamEndpoints::loopback);
    let client = match endpoints {
        Some(Ok(endpoints)) => {
            crate::analysis::UpstreamClient::for_loopback(api_key, config, endpoints)
        }
        Some(Err(error)) => {
            // A non-loopback or malformed override is a usage failure, never
            // silently ignored toward production.
            return Err(CanonicalError::new(
                ErrorCode::InvalidConfig,
                format!("PANGRAM_DETECT_ENDPOINT is not a loopback fixture address: {error}"),
            )
            .expect("static template"));
        }
        None => crate::analysis::UpstreamClient::production(api_key, config),
    };
    client.map_err(|error| {
        CanonicalError::new(
            ErrorCode::UpstreamError,
            format!("could not build the Pangram client: {error}"),
        )
        .and_then(|error| error.with_contextual_retryability(false))
        .expect("static template")
    })
}

/// The production-only client: there is no endpoint override of any kind.
#[cfg(not(any(test, feature = "dev-tools", doctest)))]
fn build_client(
    api_key: SecretString,
    config: AnalysisConfig,
) -> Result<crate::analysis::UpstreamClient, CanonicalError> {
    crate::analysis::UpstreamClient::production(api_key, config).map_err(|error| {
        CanonicalError::new(
            ErrorCode::UpstreamError,
            format!("could not build the Pangram client: {error}"),
        )
        .and_then(|error| error.with_contextual_retryability(false))
        .expect("static template")
    })
}

/// The CTRL+C/SIGINT target token. The signal handler is installed once per
/// process (SIGINT delivery is process-global); the slot is refreshed for
/// each observation flow so an in-flight wait is cancelled and a completed
/// flow leaves no stale handle.
fn cancel_slot() -> &'static std::sync::Mutex<Option<CancellationToken>> {
    static SLOT: std::sync::OnceLock<std::sync::Mutex<Option<CancellationToken>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| std::sync::Mutex::new(None))
}

/// Installs the SIGINT driver exactly once and registers `token` as the
/// current observation's cancellation target. Returns a guard that clears
/// the slot on drop. A driver-install failure is non-fatal: without it no
/// SIGINT is trapped, so the interruption path is simply never exercised.
pub(super) fn set_active_cancel(token: &CancellationToken) -> CancelGuard {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Register a single cross-platform low-level SIGINT handler. On Unix
        // this is the signal action; on Windows it is a console control
        // handler for CTRL_C_EVENT. The handler body only reads the slot and
        // cancels the current token, so it never touches shared mutable
        // application state beyond a mutex-guarded read. Registering maps
        // every target the signal-hook crate supports, unlike the
        // Unix-only `iterator`/`Signals` driver.
        unsafe {
            if signal_hook::low_level::register(signal_hook::consts::SIGINT, || {
                if let Some(token) = cancel_slot().lock().expect("cancel slot").clone() {
                    token.cancel();
                }
            })
            .is_err()
            {
                // A registration failure is non-fatal: without it no SIGINT
                // is trapped and the interruption path is simply unexercised.
            }
        }
    });
    *cancel_slot().lock().expect("cancel slot") = Some(token.clone());
    CancelGuard
}

/// Clears the SIGINT target when one observation flow ends.
pub(super) struct CancelGuard;

impl Drop for CancelGuard {
    fn drop(&mut self) {
        *cancel_slot().lock().expect("cancel slot") = None;
    }
}
