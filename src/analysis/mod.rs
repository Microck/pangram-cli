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

/// Maps configuration failures into the canonical categories shared by every
/// adapter that constructs an analyzer or resolves credentials.
pub(crate) fn config_error(error: ConfigError) -> CanonicalError {
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
/// Production endpoints stay fixed inside `UpstreamClient`; the environment
/// override exists only in test/dev-tools builds and accepts loopback hosts.
#[allow(clippy::result_large_err)]
pub(crate) fn build_analyzer(
    service: &ConfigService,
    api_key: SecretString,
) -> Result<Analyzer, CanonicalError> {
    let config = service.effective().map_err(config_error)?;
    let rate = config
        .network
        .as_ref()
        .and_then(|network| network.max_requests_per_second);
    let client = configured_client(api_key, AnalysisConfig::production(rate))?;
    Ok(Analyzer::from_client(client))
}

#[cfg(any(test, feature = "dev-tools", doctest))]
#[allow(clippy::result_large_err)]
fn configured_client(
    api_key: SecretString,
    config: AnalysisConfig,
) -> Result<UpstreamClient, CanonicalError> {
    let base = std::env::var("PANGRAM_DETECT_ENDPOINT").ok();
    let endpoints = base
        .as_deref()
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .map(UpstreamEndpoints::loopback);
    let client = match endpoints {
        Some(Ok(endpoints)) => UpstreamClient::for_loopback(api_key, config, endpoints),
        Some(Err(error)) => {
            return Err(CanonicalError::new(
                ErrorCode::InvalidConfig,
                format!("PANGRAM_DETECT_ENDPOINT is not a loopback fixture address: {error}"),
            )
            .expect("static template"));
        }
        None => UpstreamClient::production(api_key, config),
    };
    map_client(client)
}

#[cfg(not(any(test, feature = "dev-tools", doctest)))]
#[allow(clippy::result_large_err)]
fn configured_client(
    api_key: SecretString,
    config: AnalysisConfig,
) -> Result<UpstreamClient, CanonicalError> {
    map_client(UpstreamClient::production(api_key, config))
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
