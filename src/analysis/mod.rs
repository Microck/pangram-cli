//! The shared Pangram 4 text-analysis module.
//!
//! This module is the sole owner of Pangram protocol behavior: explicit
//! Pangram 4 text submission, safe task polling, upstream response
//! normalization, rate limiting, safe-GET retry with bounded backoff, local
//! wait timeouts, and local cancellation. CLI, TUI, and MCP adapters call
//! the typed [`Analyzer`] surface; they never construct HTTP requests
//! themselves.
//!
//! Safety invariants:
//!
//! - Production requests go only to the fixed Pangram 4 text endpoints with
//!   `x-api-key` authentication, TLS through rustls and the platform
//!   verifier, system proxy discovery, and redirect following disabled.
//!   There is no production endpoint, environment, or flag override.
//! - A billable POST is never replayed after an ambiguous send outcome.
//!   Ambiguity yields `submission_outcome_unknown` (non-retryable) with the
//!   fixed duplicate-billing recovery.
//! - Safe GET polling retries bounded transient failures, honors
//!   `Retry-After`, and uses decorrelated bounded backoff with jitter. The
//!   retry chain observes the caller's wait deadline and a cumulative
//!   retry-time budget, so a wait timeout or cancellation interrupts pending
//!   retry sleeps promptly.
//! - Every request (submit and poll) is issued through one shared time-based
//!   issue gate that enforces the hard 5-requests-per-second ceiling on
//!   request issue timing; configuration may only lower the rate.
//! - Wait timeouts and cancellation stop local observation only. No remote
//!   cancellation request is ever sent.
//! - Credentials, auth headers, submitted content, and raw response bodies
//!   never enter errors, `Debug` output, or serialized error details.
//!   Upstream-reported failure text is reduced (control sequences stripped,
//!   non-printable bytes removed, bounded length) before it can appear in
//!   canonical details.

mod bulk;
mod config;
mod handle;
mod http;
mod normalize;
mod pacemaker;
mod task;
mod upstream;

pub use crate::domain::BulkSubmissionPlan;
pub use bulk::{
    BulkAnalysisError, BulkAnalysisRequest, BulkOperationIdentity, BulkPageResult, BulkProgress,
    InterruptedBulk, RunningBulk,
};
pub use config::{AnalysisConfig, PollPolicy, RetryPolicy, WaitOptions};
pub use handle::{
    AnalysisProgress, Analyzer, InterruptedAnalysis, OperationIdentity, RunningAnalysis,
    StopObserving,
};
pub(crate) use task::canonical_text_word_count;
pub use task::{Accepted, AcceptedInput, AnalysisRequest, AnalysisResult, TaskError};
pub use upstream::{
    AcceptedBulk, AnalysisError, SubmissionFailure, UpstreamClient, UpstreamEndpoints,
};

pub use crate::config::MAX_REQUESTS_PER_SECOND;
pub use tokio::time::{Duration, Instant};

use secrecy::SecretString;

use crate::config::{ConfigError, ConfigService};
use crate::output::{CanonicalError, ErrorCode};

/// Selects the immutable analysis dependency used by one adapter execution.
/// Production resolves configuration and credentials at the point of use;
/// the development driver supplies a loopback-validated analyzer directly.
#[derive(Clone, Default)]
pub(crate) enum AnalyzerSource {
    #[default]
    Production,
    #[cfg(feature = "dev-tools")]
    Injected(Box<Analyzer>),
}

/// Session-lifetime state shared by analyzers which refresh configuration and
/// credentials independently. It owns only request-issue pacing state, never
/// an analyzer, credential, endpoint, or configuration snapshot.
#[derive(Clone, Default)]
pub(crate) struct AnalyzerSession {
    pacing: pacemaker::PacingSchedule,
}

impl AnalyzerSource {
    #[allow(clippy::result_large_err)]
    pub(crate) fn resolve(&self, service: &ConfigService) -> Result<Analyzer, CanonicalError> {
        match self {
            Self::Production => build_analyzer(service, resolved_api_key(service)?),
            #[cfg(feature = "dev-tools")]
            Self::Injected(analyzer) => Ok(analyzer.as_ref().clone()),
        }
    }

    /// Resolves fresh configuration and credentials while preserving the
    /// caller's session-shared request-issue schedule.
    #[allow(clippy::result_large_err)]
    pub(crate) fn resolve_in_session(
        &self,
        service: &ConfigService,
        session: &AnalyzerSession,
    ) -> Result<Analyzer, CanonicalError> {
        match self {
            Self::Production => {
                let api_key = resolved_api_key(service)?;
                build_analyzer_in_session(
                    api_key,
                    configured_rate(service)?,
                    session.pacing.clone(),
                )
            }
            #[cfg(feature = "dev-tools")]
            Self::Injected(analyzer) => {
                let rate = configured_rate(service)?.unwrap_or(MAX_REQUESTS_PER_SECOND);
                Ok(analyzer
                    .as_ref()
                    .clone()
                    .with_pacing_schedule(rate, session.pacing.clone()))
            }
        }
    }

    #[cfg(feature = "dev-tools")]
    pub(crate) fn injected(analyzer: Analyzer) -> Self {
        Self::Injected(Box::new(analyzer))
    }
}

#[allow(clippy::result_large_err)]
fn resolved_api_key(service: &ConfigService) -> Result<SecretString, CanonicalError> {
    let resolution = service
        .credentials()
        .resolve(service.overrides())
        .map_err(credential_error)?;
    resolution
        .key_for_client()
        .ok_or_else(crate::output::missing_api_key_error)
}

#[allow(clippy::result_large_err)]
fn configured_rate(service: &ConfigService) -> Result<Option<f64>, CanonicalError> {
    let config = service.effective().map_err(config_error)?;
    Ok(config
        .network
        .as_ref()
        .and_then(|network| network.max_requests_per_second))
}

/// Maps configuration failures into the canonical categories shared by every
/// adapter that constructs an analyzer or resolves credentials.
pub(crate) fn config_error(error: ConfigError) -> CanonicalError {
    let code = match &error {
        ConfigError::InsecurePermissions | ConfigError::RestrictionFailed => {
            ErrorCode::InsecureConfigPermissions
        }
        _ => ErrorCode::InvalidConfig,
    };
    CanonicalError::new(code, error.to_string()).unwrap_or_else(|_| {
        CanonicalError::new(code, "local configuration is invalid").expect("fixed message")
    })
}

/// Maps failures from the credential store or effective-key resolution.
/// Only this credential-owned boundary may classify an invalid value as an
/// invalid API key; general configuration values remain `invalid_config`.
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

/// Builds the one adapter-facing analyzer from effective configuration.
/// Production endpoints stay fixed inside `UpstreamClient`. Loopback fixtures
/// construct a client through the typed test-only seam instead of changing
/// endpoint selection in the product binary.
#[allow(clippy::result_large_err)]
pub(crate) fn build_analyzer(
    service: &ConfigService,
    api_key: SecretString,
) -> Result<Analyzer, CanonicalError> {
    let rate = configured_rate(service)?;
    let client = map_client(UpstreamClient::production(
        api_key,
        AnalysisConfig::production(rate),
    ))?;
    Ok(Analyzer::from_client(client))
}

#[allow(clippy::result_large_err)]
fn build_analyzer_in_session(
    api_key: SecretString,
    rate: Option<f64>,
    schedule: pacemaker::PacingSchedule,
) -> Result<Analyzer, CanonicalError> {
    let config = AnalysisConfig::production(rate);
    let rate = config.max_requests_per_second();
    let client = map_client(UpstreamClient::production(api_key, config))?
        .with_pacing_schedule(rate, schedule);
    Ok(Analyzer::from_client(client))
}

#[allow(clippy::result_large_err)]
fn map_client(
    client: Result<UpstreamClient, AnalysisError>,
) -> Result<UpstreamClient, CanonicalError> {
    client.map_err(|error| {
        CanonicalError::new(
            ErrorCode::UpstreamError,
            format!("could not build the Pangram client: {error}"),
        )
        .and_then(|error| error.with_contextual_retryability(false))
        .expect("static template")
    })
}

/// Builds the development driver's loopback-only analyzer. URL validation
/// remains inside the analysis module, and its production request policy is
/// identical to the shipped client apart from the endpoint set.
#[cfg(feature = "dev-tools")]
#[allow(clippy::result_large_err)]
pub(crate) fn build_loopback_analyzer(
    base_url: &str,
    api_key: SecretString,
) -> Result<Analyzer, CanonicalError> {
    let endpoints = UpstreamEndpoints::loopback(base_url).map_err(|_| {
        CanonicalError::new(
            ErrorCode::InvalidConfig,
            "the fixture endpoint must be an HTTP loopback base URL",
        )
        .expect("fixed URL validation error")
    })?;
    let client = map_client(UpstreamClient::for_loopback(
        api_key,
        AnalysisConfig::production(None),
        endpoints,
    ))?;
    Ok(Analyzer::from_client(client))
}

#[cfg(test)]
mod tests {
    use super::{config_error, credential_error};
    use crate::config::ConfigError;
    use crate::output::ErrorCode;

    #[test]
    fn general_invalid_values_are_configuration_errors() {
        let error = config_error(ConfigError::InvalidValue {
            key: "tui.keymap".into(),
            reason: "must be regular or vim".into(),
        });

        assert_eq!(error.code(), ErrorCode::InvalidConfig);
    }

    #[test]
    fn credential_invalid_values_are_api_key_errors() {
        let error = credential_error(ConfigError::InvalidValue {
            key: "PANGRAM_API_KEY".into(),
            reason: "must be valid UTF-8 when set".into(),
        });

        assert_eq!(error.code(), ErrorCode::InvalidApiKey);
    }
}
