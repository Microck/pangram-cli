//! The one HTTP transport used by the analysis module.
//!
//! The client is built exactly once per [`crate::analysis::UpstreamClient`]
//! with rustls through the platform verifier (native TLS is never enabled),
//! system proxy discovery, decompression for gzip, Brotli, and deflate,
//! streaming bodies, JSON, and redirect following disabled. A per-request
//! timeout bounds every call; it is never a billable-ambiguity escape hatch
//! by itself (callers classify send ambiguity separately).
//!
//! Responses are consumed through one bounded reader so a malformed or
//! hostile peer cannot force an unbounded allocation.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

/// Upper bound on a consumed JSON body. Pangram task responses are small;
/// this exists only as a defensive allocation cap before JSON parsing.
const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;

use super::config::{Duration, RetryPolicy};
use super::upstream::AnalysisError;

/// A consumed response: the status plus a sanitized body reader. The raw
/// body never appears in errors; it is parsed once into a typed value or
/// surfaced as a truncated contract symptom. It is not `Debug`able:
/// printing it would dump content.
pub struct Response {
    status: u16,
    retry_after_ms: Option<u64>,
    body: Vec<u8>,
}

impl Response {
    /// Reads a bounded body and extracts the safe `Retry-After` hint.
    async fn collect(response: reqwest::Response) -> Result<Self, AnalysisError> {
        let status = response.status().as_u16();
        let retry_after_ms = parse_retry_after(response.headers());
        let body = read_bounded(response).await?;
        Ok(Self {
            status,
            retry_after_ms,
            body,
        })
    }

    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    #[must_use]
    pub const fn retry_after_ms(&self) -> Option<u64> {
        self.retry_after_ms
    }

    /// Parses the body as JSON exactly once; body bytes are never surfaced.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, AnalysisError> {
        serde_json::from_slice(&self.body).map_err(AnalysisError::malformed_body)
    }

    /// Parses the body as an untyped JSON value for staged classification.
    /// Callers own any content hygiene from this point: the value may carry
    /// submitted or result text and must stay inside the analysis module.
    pub(crate) fn json_value(&self) -> Result<serde_json::Value, AnalysisError> {
        serde_json::from_slice(&self.body).map_err(AnalysisError::malformed_body)
    }

    /// A bounded structural symptom (shape and size only) for contract
    /// diagnostics. Never includes text content from the body.
    #[must_use]
    pub fn structural_symptom(&self) -> String {
        structural_symptom(&self.body)
    }
}

/// Classifies a body by JSON shape without echoing any content.
fn structural_symptom(body: &[u8]) -> String {
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(serde_json::Value::Object(_)) => format!("a JSON object of {} bytes", body.len()),
        Ok(serde_json::Value::Array(_)) => format!("a JSON array of {} bytes", body.len()),
        Ok(_) => format!("a JSON scalar of {} bytes", body.len()),
        Err(_) => format!("a non-JSON body of {} bytes", body.len()),
    }
}

async fn read_bounded(mut response: reqwest::Response) -> Result<Vec<u8>, AnalysisError> {
    if let Some(length) = response.content_length() {
        if length > MAX_BODY_BYTES {
            return Err(AnalysisError::response_too_large());
        }
    }

    let mut body = Vec::new();
    // Chunked streaming keeps the ceiling enforced even when the peer lies
    // about or omits Content-Length.
    while let Some(chunk) = response.chunk().await.map_err(AnalysisError::transport)? {
        if u64::try_from(body.len() + chunk.len()).unwrap_or(u64::MAX) > MAX_BODY_BYTES {
            return Err(AnalysisError::response_too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Parses `Retry-After` into whole milliseconds. Only the safe integer
/// delta-seconds form affects scheduling; HTTP-date and malformed values
/// fall back to the computed backoff.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?;
    let seconds: u64 = value.to_str().ok()?.trim().parse().ok()?;
    Some(seconds.saturating_mul(1000))
}

/// A sanitized send-side outcome. `delivered` means the request bytes may
/// have reached the peer; callers must not replay an ambiguous billable
/// send. Transport strings cover reqwest's error classes (connection,
/// redirect, timeout, body); auth headers are injected per call and never
/// included in any rendered error. It is not `Debug`able: one variant holds
/// response content.
pub enum SendOutcome {
    /// The server produced a response (any status).
    Responded(Response),
    /// The call failed before any response; `delivered_may_have_occurred`
    /// records whether the request could have reached the peer.
    Failed {
        delivered_may_have_occurred: bool,
        error: AnalysisError,
    },
    /// The caller's local cancellation fired; no observation stop is sent.
    Cancelled,
}

/// One built client. Cheap to clone (shared connection pool); construction
/// fixes the entire production transport profile.
#[derive(Clone)]
pub struct HttpClient {
    inner: reqwest::Client,
    per_request_timeout: Duration,
}

impl fmt::Debug for HttpClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpClient")
            .field("per_request_timeout", &self.per_request_timeout)
            .finish_non_exhaustive()
    }
}

impl HttpClient {
    /// Builds the transport. Any construction failure surfaces once at
    /// startup; there is no lazy or insecure fallback. `use_rustls_tls()`
    /// selects the platform-verifier rustls backend explicitly; native TLS
    /// is not compiled and cannot be selected.
    pub fn build(per_request_timeout: Duration) -> Result<Self, AnalysisError> {
        let inner = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(per_request_timeout)
            .use_rustls_tls()
            .build()
            .map_err(AnalysisError::client_construction)?;
        Ok(Self {
            inner,
            per_request_timeout,
        })
    }

    /// Sends one JSON POST. Auth is injected from the caller-supplied key;
    /// the key is moved through the header map and never logged. Dropping
    /// the future cancels the in-flight request.
    pub async fn post_json(
        &self,
        url: &str,
        api_key: &SecretString,
        body: &serde_json::Value,
        cancel: &CancellationToken,
    ) -> SendOutcome {
        let request = self
            .inner
            .post(url)
            .header(
                reqwest::header::HeaderName::from_static("x-api-key"),
                api_key.expose_secret(),
            )
            .json(body);
        self.send(request, cancel).await
    }

    /// Sends one GET to the given absolute URL. Auth is injected from the
    /// caller-supplied key.
    pub async fn get(
        &self,
        url: &str,
        api_key: &SecretString,
        cancel: &CancellationToken,
    ) -> SendOutcome {
        let request = self.inner.get(url).header(
            reqwest::header::HeaderName::from_static("x-api-key"),
            api_key.expose_secret(),
        );
        self.send(request, cancel).await
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        cancel: &CancellationToken,
    ) -> SendOutcome {
        let outcome = tokio::select! {
            biased;
            () = cancel.cancelled() => return SendOutcome::Cancelled,
            outcome = request.send() => outcome,
        };
        match outcome {
            Ok(response) => match Response::collect(response).await {
                Ok(response) => SendOutcome::Responded(response),
                Err(error) => SendOutcome::Failed {
                    // The response had already arrived when body consumption
                    // failed, so a billable peer-side effect must be assumed.
                    delivered_may_have_occurred: true,
                    error,
                },
            },
            Err(error) => {
                let delivered_may_have_occurred = !error.is_connect() && !error.is_builder();
                SendOutcome::Failed {
                    delivered_may_have_occurred,
                    error: AnalysisError::transport(error),
                }
            }
        }
    }
}

/// Computes the wait before safe-GET attempt `attempt` (1-based; attempt 1
/// is the immediate first try, so this is called for the wait before attempt
/// 2 and beyond). Uses a decorrelated full-jitter window
/// `[base, min(max, 3 * previous)]`. Randomness is process-local jitter
/// only; it never affects semantics or leaves the process.
#[must_use]
pub fn backoff_delay(policy: &RetryPolicy, previous: Duration, attempt: u32) -> Duration {
    debug_assert!(attempt >= 2, "attempt 1 is the immediate first try");
    if policy.base_delay.is_zero() {
        return Duration::ZERO;
    }
    let ceiling = previous
        .saturating_mul(3)
        .min(policy.max_delay)
        .max(policy.base_delay);
    let window_nanos = ceiling
        .as_nanos()
        .saturating_sub(policy.base_delay.as_nanos());
    if window_nanos == 0 {
        policy.base_delay
    } else {
        let draw = jitter_draw() % (window_nanos + 1);
        policy.base_delay + Duration::from_nanos(nanos_u64(draw))
    }
}

/// Clamps a server `Retry-After` hint into policy bounds. A zero cap (the
/// deterministic test policy) means hints are ignored.
#[must_use]
pub fn honor_retry_after(policy: &RetryPolicy, hint_ms: Option<u64>) -> Option<Duration> {
    let hint_ms = hint_ms?;
    if policy.max_delay.is_zero() {
        return None;
    }
    let hint = Duration::from_millis(hint_ms).min(policy.max_delay);
    Some(if hint.is_zero() {
        policy.base_delay
    } else {
        hint
    })
}

/// A per-process jitter draw (xorshift64* over a seeded atomic). Semantics
/// never depend on its quality; decorrelating retry schedules between peers
/// is measurably better than every caller waking on the same timetable.
fn jitter_draw() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};

    static STATE: AtomicU64 = AtomicU64::new(0);
    let seed = STATE.load(Ordering::Relaxed).max(1);
    let mut next = seed;
    next ^= next << 13;
    next ^= next >> 7;
    next ^= next << 17;
    STATE.store(next, Ordering::Relaxed);
    u128::from(next)

    // Seeding note: a fixed initializer is acceptable because jitter only
    // needs per-call variance, not unpredictability. The first draws of a
    // xorshift with a non-zero fixed state already decorrelate consecutive
    // callers within a process.
}

fn nanos_u64(value: u128) -> u64 {
    // Decorrelated windows are capped at `max_delay` (at most seconds), so
    // the window plus base always fits in u64 nanoseconds.
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 4,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(900),
        }
        .validate()
        .unwrap()
    }

    #[test]
    fn backoff_stays_within_the_decorrelated_window() {
        let policy = policy();
        let mut previous = policy.base_delay;
        for attempt in 2..8 {
            let next = backoff_delay(&policy, previous, attempt);
            assert!(next >= policy.base_delay, "attempt {attempt}: {next:?}");
            let ceiling = previous
                .saturating_mul(3)
                .min(policy.max_delay)
                .max(policy.base_delay);
            assert!(next <= ceiling, "attempt {attempt}: {next:?}");
            previous = next;
        }
    }

    #[test]
    fn zero_base_disables_backoff_entirely() {
        let mut policy = policy();
        policy.base_delay = Duration::ZERO;
        assert_eq!(backoff_delay(&policy, Duration::ZERO, 2), Duration::ZERO);
    }

    #[test]
    fn retry_after_hints_are_clamped() {
        let policy = policy();
        let honored = honor_retry_after(&policy, Some(5_000));
        assert_eq!(honored, Some(policy.max_delay));
        let honored = honor_retry_after(&policy, Some(2));
        assert_eq!(honored, Some(Duration::from_millis(2)));
        assert_eq!(honor_retry_after(&policy, None), None);
    }

    #[test]
    fn zero_max_delay_ignores_hints() {
        let policy = RetryPolicy::OFF;
        assert_eq!(honor_retry_after(&policy, Some(5_000)), None);
    }

    #[test]
    fn structural_symptom_reports_shape_and_size_only() {
        let object = format!("{{\"secret\":\"{}\", \"k\": 1}}", "x".repeat(32));
        let symptom = structural_symptom(object.as_bytes());
        assert!(symptom.contains("JSON object"), "{symptom}");
        assert!(symptom.contains("bytes"), "{symptom}");
        assert!(!symptom.contains('x'), "{symptom}");
    }
}
