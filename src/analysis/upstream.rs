//! The concrete upstream client: the only code path that talks to Pangram.
//!
//! Production construction is fixed to the documented Pangram 4 text
//! endpoints. A loopback-only constructor exists for the `dev-tools`-gated
//! protocol fixture; it refuses any non-loopback URL at construction so no
//! path from a test or adapter can aim the client elsewhere.
//!
//! Method inventory stays intentionally narrow: one billable text POST and
//! one safe GET poll. Bulk, file, and plagiarism paths are Phase 3/7 work
//! and have no protocol surface here yet.

use std::fmt;

use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
#[cfg(any(test, feature = "dev-tools", doctest))]
use url::{Host, Url};

use crate::domain::UpstreamTaskId;
use crate::output::{CanonicalError, ErrorCode, OutputValidationError};

use super::config::{AnalysisConfig, Clock, Duration, Instant};
use super::http::{self, HttpClient, Response, SendOutcome};
use super::pacemaker::Pacemaker;

/// The fixed production submit URL. Never construct it from configuration.
const PRODUCTION_SUBMIT_URL: &str = "https://text.external-api.pangram.com/task";
/// The fixed production poll prefix; `/task/{id}` is appended by the client.
const PRODUCTION_POLL_PREFIX: &str = "https://text.external-api.pangram.com/task";

/// The endpoint set for one client. Production values are compile-time
/// constants; only the loopback-gated constructor accepts alternates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamEndpoints {
    submit: String,
    poll_prefix: String,
}

impl UpstreamEndpoints {
    /// The exact production text endpoints.
    #[must_use]
    pub fn production() -> Self {
        Self {
            submit: PRODUCTION_SUBMIT_URL.to_owned(),
            poll_prefix: PRODUCTION_POLL_PREFIX.to_owned(),
        }
    }

    /// A loopback-only endpoint set for the protocol fixture. Rejects any
    /// non-loopback host, any non-HTTP(S) scheme, and any URL that is not
    /// `http(s)://loopback[:port]` plus a path.
    #[cfg(any(test, feature = "dev-tools", doctest))]
    #[doc(hidden)]
    pub fn loopback(base_url: &str) -> Result<Self, AnalysisError> {
        let url = Url::parse(base_url)
            .map_err(|_| AnalysisError::InvalidEndpoint("unable to parse the base URL"))?;
        let scheme = url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(AnalysisError::InvalidEndpoint(
                "the loopback fixture requires an http or https scheme",
            ));
        }
        let loopback = match url.host() {
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
            _ => false,
        };
        if !loopback {
            return Err(AnalysisError::InvalidEndpoint(
                "test endpoints are restricted to loopback hosts",
            ));
        }
        if url.path() != "/" && !url.path().is_empty() {
            return Err(AnalysisError::InvalidEndpoint(
                "test endpoints use the server root; paths are protocol-owned",
            ));
        }
        let submit = format!("{base_url}/task");
        let poll_prefix = submit.clone();
        Ok(Self {
            submit,
            poll_prefix,
        })
    }

    /// The URL polled for one task. The task ID is URL-encoded; Pangram IDs
    /// are opaque to us.
    #[must_use]
    pub fn poll_url(&self, task_id: &UpstreamTaskId) -> String {
        format!("{}/{}", self.poll_prefix, encode_path(task_id.as_str()))
    }
}

/// Percent-encodes the characters a path segment cannot carry. Pangram task
/// IDs are documented as simple tokens; this is belt-and-braces so a future
/// opaque ID cannot break URL structure.
fn encode_path(id: &str) -> String {
    let mut encoded = String::with_capacity(id.len());
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The analysis module's error surface. Every failure is already reduced to
/// the sanitized canonical vocabulary; transport details live in the
/// `Display` rendering of reqwest's error classes only (schema classes such
/// as `connection`, `timeout`, `body`) and never include URLs with keys,
/// headers, request bodies, or response bodies.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AnalysisError {
    #[error("could not build the Pangram HTTP client: {0}")]
    ClientConstruction(String),
    #[error("test-only endpoint rejected: {0}")]
    InvalidEndpoint(&'static str),
    #[error("the request failed before a response: {0}")]
    Transport(String),
    #[error("the response body exceeded the supported size")]
    ResponseTooLarge,
    #[error("the response body was not valid JSON: {0}")]
    MalformedBody(String),
    #[error("the task state was cancelled locally; no remote action was taken")]
    Cancelled,
    #[error("canonical output validation failed: {0}")]
    OutputValidation(#[from] OutputValidationError),
}

impl AnalysisError {
    pub(crate) fn client_construction(error: reqwest::Error) -> Self {
        Self::ClientConstruction(sanitize_reqwest(&error))
    }

    pub(crate) fn transport(error: reqwest::Error) -> Self {
        Self::Transport(sanitize_reqwest(&error))
    }

    pub(crate) fn response_too_large() -> Self {
        Self::ResponseTooLarge
    }

    pub(crate) fn malformed_body(error: serde_json::Error) -> Self {
        Self::MalformedBody(error.to_string())
    }
}

/// Renders a reqwest error without its `url` fragment (which would repeat
/// the endpoint and, in_path segments, caller-supplied IDs). The class and
/// source chain of reqwest errors contain no secrets by construction: every
/// header we send is generated internally and bodies are JSON we own.
fn sanitize_reqwest(error: &reqwest::Error) -> String {
    let mut message = if error.is_timeout() {
        "the request timed out".to_owned()
    } else if error.is_connect() {
        "the connection could not be established".to_owned()
    } else if error.is_body() || error.is_decode() {
        "the response body could not be read".to_owned()
    } else if error.is_request() {
        "the request could not be sent".to_owned()
    } else if error.is_redirect() {
        "an unexpected redirect was refused".to_owned()
    } else {
        "the HTTP call failed".to_owned()
    };
    if let Some(source) = std::error::Error::source(error) {
        // Source chains are hyper/transport classes (e.g. "connection reset").
        // They never carry payloads; include one level for diagnosability.
        message.push_str(&format!(": {source}"));
    }
    message
}

/// An accepted asynchronous task: exactly the upstream identifier. This is
/// the only possible success shape of the submit endpoint; a missing or
/// misplaced `task_id` is an upstream contract change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedTask {
    pub task_id: UpstreamTaskId,
}

#[derive(Deserialize)]
struct TaskCreatedWire {
    task_id: String,
}

/// The concrete client. Cloning shares the connection pool and pacing gate.
#[derive(Clone)]
pub struct UpstreamClient<C = super::config::SystemClock> {
    http: HttpClient,
    endpoints: UpstreamEndpoints,
    api_key: SecretString,
    config: AnalysisConfig<C>,
    pacemaker: Pacemaker<C>,
}

impl<C: Clock> fmt::Debug for UpstreamClient<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = &self.api_key; // never rendered
        formatter
            .debug_struct("UpstreamClient")
            .field("endpoints", &self.endpoints)
            .field("api_key", &"[redacted]")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl UpstreamClient<super::config::SystemClock> {
    /// The production client. Only [`UpstreamEndpoints::production`] is ever
    /// supplied here; there is no other way to obtain this constructor's
    /// behavior.
    pub fn production(
        api_key: SecretString,
        config: AnalysisConfig<super::config::SystemClock>,
    ) -> Result<Self, AnalysisError> {
        Self::with_endpoints(api_key, config, UpstreamEndpoints::production())
    }
}

impl<C: Clock> UpstreamClient<C> {
    fn with_endpoints(
        api_key: SecretString,
        config: AnalysisConfig<C>,
        endpoints: UpstreamEndpoints,
    ) -> Result<Self, AnalysisError> {
        let http = HttpClient::build(config.per_request_timeout())?;
        let pacemaker = Pacemaker::new(config.max_requests_per_second(), config.clock());
        Ok(Self {
            http,
            endpoints,
            api_key,
            config,
            pacemaker,
        })
    }

    /// The loopback-gated client used by the protocol fixture. The endpoint
    /// set must already be loopback-validated by
    /// [`UpstreamEndpoints::loopback`].
    #[cfg(any(test, feature = "dev-tools", doctest))]
    #[doc(hidden)]
    pub fn for_loopback(
        api_key: SecretString,
        config: AnalysisConfig<C>,
        endpoints: UpstreamEndpoints,
    ) -> Result<Self, AnalysisError> {
        Self::with_endpoints(api_key, config, endpoints)
    }

    #[must_use]
    pub const fn config(&self) -> &AnalysisConfig<C> {
        &self.config
    }

    /// Submits one Pangram 4 text-analysis task. This is billable: on any
    /// ambiguous outcome the caller must produce `submission_outcome_unknown`
    /// rather than retry. Returns the acceptance, the ambiguity marker with
    /// the last-known state, or a classified terminal failure.
    pub async fn submit_text(
        &self,
        text: &str,
        public_dashboard_link: bool,
        cancel: &CancellationToken,
    ) -> Result<AcceptedTask, SubmitOutcome> {
        let body = serde_json::json!({
            "text": text,
            "model": "pangram-4",
            "public_dashboard_link": public_dashboard_link,
        });
        // There is no caller wait deadline on the submit path; a cancelled
        // token is the only early release. Distinct deadline semantics live
        // on the observation path where the caller supplies a wait budget.
        match self.pacemaker.hurdle(cancel, None).await {
            super::pacemaker::Gate::Released => {}
            super::pacemaker::Gate::Cancelled | super::pacemaker::Gate::DeadlinePassed => {
                return Err(SubmitOutcome::Cancelled);
            }
        }
        let outcome = self
            .http
            .post_json(&self.endpoints.submit, &self.api_key, &body, cancel)
            .await;
        match outcome {
            SendOutcome::Responded(response) => classify_submit(response),
            SendOutcome::Cancelled => Err(SubmitOutcome::Cancelled),
            SendOutcome::Failed {
                delivered_may_have_occurred,
                error,
            } => {
                if delivered_may_have_occurred {
                    Err(SubmitOutcome::Ambiguous(error))
                } else {
                    Err(SubmitOutcome::Failed(Box::new(map_transport_failure(
                        &error,
                    ))))
                }
            }
        }
    }

    /// One safe GET observation of a task. Transient failures may be retried
    /// by the internal bounded policy; a server `Retry-After` hint is honored
    /// and clamped by the configured window. The chain additionally honors the
    /// caller's wait `deadline` and the policy's cumulative retry-time budget
    /// so a small wait timeout interrupts promptly even through repeated
    /// 429/503 responses carrying long hints. Returns the consumed response
    /// for the caller to classify (success, failure, in-progress).
    ///
    /// Every attempt is issued through the shared pacing gate, so even a
    /// retried chain cannot burst past the configured per-second ceiling.
    pub async fn poll_task(
        &self,
        task_id: &UpstreamTaskId,
        cancel: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<TaskPoll, PollError> {
        let policy = self.config.retry();
        let mut attempt: u32 = 1;
        let mut previous_delay = policy.base_delay;
        let mut spent = Duration::ZERO;
        let clock = self.config.clock();

        loop {
            let url = self.endpoints.poll_url(task_id);
            match self.pacemaker.hurdle(cancel, deadline).await {
                super::pacemaker::Gate::Released => {}
                super::pacemaker::Gate::Cancelled => return Err(PollError::Cancelled),
                // The caller's wait budget ran out before the next request
                // could issue: surface the wait timeout, not an interruption.
                super::pacemaker::Gate::DeadlinePassed => {
                    return Err(PollError::DeadlineExceeded);
                }
            }
            let outcome = self.http.get(&url, &self.api_key, cancel).await;
            let response = match outcome {
                SendOutcome::Responded(response) => response,
                SendOutcome::Cancelled => return Err(PollError::Cancelled),
                SendOutcome::Failed { error, .. } => {
                    // Safe GET: transient transport classes may retry.
                    let retryable = matches!(&error, AnalysisError::Transport(_));
                    if !retryable || attempt >= policy.max_attempts {
                        return Err(PollError::Failed(Box::new(transport_poll_error(&error))));
                    }
                    let delay = http::backoff_delay(&policy, previous_delay, attempt + 1);
                    let delay =
                        http::clamp_retry_sleep(&policy, delay, spent, clock.now(), deadline);
                    spent += delay;
                    previous_delay = delay;
                    attempt += 1;
                    match sleep_or_cancel(clock, delay, cancel, deadline).await {
                        RetryWake::Slept => {}
                        RetryWake::Cancelled => return Err(PollError::Cancelled),
                        RetryWake::DeadlineReached => return Err(PollError::DeadlineExceeded),
                    }
                    continue;
                }
            };

            let status = response.status();
            if (200..300).contains(&status) {
                return classify_poll(response, task_id)
                    .map_err(|error| PollError::Failed(Box::new(contract_symptom_error(&error))));
            }
            if (status == 429 || (500..600).contains(&status)) && attempt < policy.max_attempts {
                // Transient server-side pressure on a safe read: honor
                // Retry-After when present, else the computed backoff. Then
                // clamp the chosen sleep to the cumulative budget and the
                // caller's wait deadline so a hint can never stretch a chain
                // past it.
                let delay = match http::honor_retry_after(&policy, response.retry_after_ms()) {
                    Some(override_delay) => override_delay,
                    None => http::backoff_delay(&policy, previous_delay, attempt + 1),
                };
                let delay = http::clamp_retry_sleep(&policy, delay, spent, clock.now(), deadline);
                spent += delay;
                previous_delay = delay;
                attempt += 1;
                match sleep_or_cancel(clock, delay, cancel, deadline).await {
                    RetryWake::Slept => {}
                    RetryWake::Cancelled => return Err(PollError::Cancelled),
                    RetryWake::DeadlineReached => return Err(PollError::DeadlineExceeded),
                }
                continue;
            }
            if status == 404 {
                return Ok(TaskPoll::NotFound);
            }
            return Err(PollError::Failed(Box::new(classify_http_failure(
                &response,
                Some(task_id),
            ))));
        }
    }
}

/// The classified result of one poll that produced an HTTP response. Not
/// `Debug`able: the terminal variant carries response content.
pub enum TaskPoll {
    /// The provider accepted the task and it is still running.
    InProgress {
        /// The provider's current stage token, preserved for provenance.
        last_stage: String,
        /// The full consumed response, for additive inspection by callers.
        response: ResponseSummary,
    },
    /// A terminal success document; normalization happens in `normalize`.
    Terminal(Box<Response>),
    /// The task ID was not found upstream.
    NotFound,
    /// The provider reported a terminal task failure with a sanitized
    /// message (control sequences stripped, non-ASCII removed, length
    /// bounded; raw provider text never crosses this boundary).
    AnalysisFailed { message: String, stage: String },
}

/// A sanitized summary of an in-progress response body: identity fields
/// only, never text content.
#[derive(Debug)]
pub struct ResponseSummary {
    structural: String,
}

impl ResponseSummary {
    #[must_use]
    pub fn structural(&self) -> &str {
        &self.structural
    }
}

/// How a billable submission ended.
#[derive(Debug)]
pub enum SubmitOutcome {
    /// The request provably never reached the peer (connect/build class).
    /// The caller may surface a retryable network failure.
    Failed(Box<CanonicalError>),
    /// The server may have seen the request but produced no usable
    /// acceptance. Never retried automatically.
    Ambiguous(AnalysisError),
    /// The caller's local cancellation fired before conclusion.
    Cancelled,
}

/// The failure surface of one safe GET observation. `Failed` is the
/// canonical, already-classified error (HTTP mapping or unreadable
/// terminal state); `Cancelled` is the local stop signal; `DeadlineExceeded`
/// marks the caller's wait budget expiring inside a paced wait or retry
/// sleep (surfaced by the observe loop as the canonical wait timeout).
#[derive(Debug)]
pub enum PollError {
    Failed(Box<CanonicalError>),
    Cancelled,
    DeadlineExceeded,
}

/// A safe-read transport exhaustion mapped onto the canonical vocabulary:
/// timeouts are retryable because no billable body was sent.
fn transport_poll_error(error: &AnalysisError) -> CanonicalError {
    match error {
        AnalysisError::Transport(message) if message.contains("timed out") => CanonicalError::new(
            ErrorCode::NetworkTimeout,
            "Pangram did not answer a safe read before the request timeout.",
        )
        .and_then(|error| error.with_contextual_retryability(true))
        .expect("static template"),
        other => contract_symptom_error(other),
    }
}

fn contract_symptom_error(error: &AnalysisError) -> CanonicalError {
    let message = match error {
        AnalysisError::Transport(detail) => detail.clone(),
        other => other.to_string(),
    };
    let mut details = std::collections::BTreeMap::new();
    details.insert("symptom".to_owned(), serde_json::Value::from(message));
    CanonicalError::new(
        ErrorCode::UpstreamContractChanged,
        "Pangram returned a document outside the pinned Pangram 4 contract.",
    )
    .and_then(|error_value| error_value.with_details(details))
    .expect("static template")
}

/// Classifies a submit response. Only a 200-range JSON object with a
/// non-empty top-level `task_id` counts as acceptance; anything else is an
/// upstream contract or HTTP failure.
fn classify_submit(response: Response) -> Result<AcceptedTask, SubmitOutcome> {
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(SubmitOutcome::Failed(Box::new(classify_http_failure(
            &response, None,
        ))));
    }
    let wire: TaskCreatedWire = response.json().map_err(|error| {
        SubmitOutcome::Ambiguous(AnalysisError::MalformedBody(format!(
            "{error} (submit acceptance malformed; the task may exist remotely)"
        )))
    })?;
    let task_id = UpstreamTaskId::new(wire.task_id).map_err(|_| {
        SubmitOutcome::Ambiguous(AnalysisError::MalformedBody(
            "the submit acceptance carried an empty task_id; the task may exist remotely".into(),
        ))
    })?;
    Ok(AcceptedTask { task_id })
}

/// Classifies a 2xx poll response. Non-terminal stages stay in-progress;
/// terminal stages are handed to the caller untouched.
fn classify_poll(response: Response, _task_id: &UpstreamTaskId) -> Result<TaskPoll, AnalysisError> {
    // Peek only at the stage token: in-progress responses are not fully
    // schema-checked because terminal fields may legitimately be absent.
    let body = response.json_value()?;
    let stage = body
        .get("stage")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            AnalysisError::MalformedBody(format!(
                "a task poll response was missing `stage`; unknown upstream document shape; {}",
                response.structural_symptom()
            ))
        })?;
    match stage {
        "STAGE_SUCCESS" => Ok(TaskPoll::Terminal(Box::new(response))),
        "STAGE_FAILED" => {
            let message = body
                .get("error_message")
                .or_else(|| body.get("headline"))
                .or_else(|| body.get("detail"))
                .and_then(serde_json::Value::as_str)
                .map(super::normalize::sanitize_upstream_message)
                .unwrap_or_else(|| "Pangram reported a task failure without detail".to_owned());
            Ok(TaskPoll::AnalysisFailed {
                message,
                stage: stage.to_owned(),
            })
        }
        "STAGE_PREPROCESSING" | "STAGE_INFERENCE" => Ok(TaskPoll::InProgress {
            last_stage: stage.to_owned(),
            response: ResponseSummary {
                structural: response.structural_symptom(),
            },
        }),
        other => Err(AnalysisError::MalformedBody(format!(
            "unknown upstream stage token {other:?}; the upstream contract may have changed"
        ))),
    }
}

/// Maps an HTTP failure status onto the canonical error vocabulary once.
/// All messages are fixed sanitized templates; details carry only the status
/// integer and safe rate-limit metadata.
fn classify_http_failure(response: &Response, task_id: Option<&UpstreamTaskId>) -> CanonicalError {
    let status = response.status();
    let build = |code: ErrorCode, message: &str| -> Result<CanonicalError, OutputValidationError> {
        let mut details = std::collections::BTreeMap::new();
        details.insert(
            "http_status".to_owned(),
            serde_json::Value::from(u64::from(status)),
        );
        CanonicalError::new(code, message)?.with_details(details)
    };

    let error = match status {
        400 => build(
            ErrorCode::UpstreamError,
            "Pangram rejected the request as invalid.",
        )
        .and_then(|error| error.with_contextual_retryability(false)),
        401 => build(ErrorCode::InvalidApiKey, "Pangram rejected the API key."),
        402 => build(
            ErrorCode::PaymentRequired,
            "Pangram reports a billing requirement.",
        ),
        403 => build(
            ErrorCode::PermissionDenied,
            "Pangram denied this API key permission for the request.",
        ),
        404 => {
            let _ = task_id; // identity never enters the error payload
            build(
                ErrorCode::UpstreamNotFound,
                "Pangram does not recognize the given task.",
            )
        }
        413 => build(
            ErrorCode::UnsupportedInput,
            "Pangram rejected the submission as too large.",
        ),
        415 => build(
            ErrorCode::UpstreamContractChanged,
            "Pangram refused the request media type; the upstream contract may have changed.",
        ),
        422 => build(
            ErrorCode::UpstreamContractChanged,
            "Pangram could not process the request document; the upstream contract may have changed.",
        ),
        429 => build(ErrorCode::RateLimited, "Pangram is rate limiting requests.").map(|error| {
            response
                .retry_after_ms()
                .map_or(error.clone(), |ms| error.with_retry_after_ms(ms))
        }),
        500..=599 => build(
            ErrorCode::UpstreamError,
            "Pangram reported a server-side failure.",
        )
        .and_then(|error| error.with_contextual_retryability(status != 501)),
        _ => build(
            ErrorCode::UpstreamError,
            "Pangram returned an unexpected failure status.",
        )
        .and_then(|error| error.with_contextual_retryability(false)),
    };

    error.unwrap_or_else(|validation| {
        // The fixed templates above are compile-time valid; this branch only
        // protects a future template edit from panicking inside the client.
        CanonicalError::new(
            ErrorCode::UpstreamError,
            format!(
                "Pangram returned HTTP {status} and the error template was invalid: {validation}"
            ),
        )
        .expect("the fallback error template is statically valid")
    })
}

fn timed_out(error: &AnalysisError) -> bool {
    match error {
        AnalysisError::Transport(message) => message.contains("timed out"),
        _ => false,
    }
}

fn map_transport_failure(error: &AnalysisError) -> CanonicalError {
    let message = match error {
        AnalysisError::Transport(detail) => detail.clone(),
        other => other.to_string(),
    };
    if timed_out(error) {
        return CanonicalError::new(
            ErrorCode::NetworkTimeout,
            "Pangram did not answer the submission before the request timeout; no acceptance was observed.",
        )
        .expect("the submission timeout template is statically valid");
    }
    CanonicalError::new(
        ErrorCode::NetworkUnavailable,
        format!("Pangram could not be reached: {message}"),
    )
    .expect("the network-unavailable template is statically valid")
}

/// How one inter-retry wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryWake {
    /// The full planned sleep elapsed; the chain may issue the next attempt.
    Slept,
    /// The caller's cancellation token fired during the sleep.
    Cancelled,
    /// No cancellation fired, but the caller's wait deadline arrived first:
    /// the observe loop surfaces the canonical wait timeout, not a stop.
    DeadlineReached,
}

/// Sleeps for `delay` or until `cancel` (whichever first), but no later
/// than `deadline`. A deadline shorter than the planned delay cuts the wait
/// and reports `DeadlineReached` so the observe loop surfaces the canonical
/// wait timeout rather than starting another retry sleep or mislabeling a
/// time-out as a local cancellation.
async fn sleep_or_cancel(
    clock: impl Clock,
    delay: Duration,
    cancel: &CancellationToken,
    deadline: Option<Instant>,
) -> RetryWake {
    if delay.is_zero() {
        // Still yield once so a fully deterministic policy cannot starve a
        // concurrently cancelled token on a single-thread runtime.
        tokio::task::yield_now().await;
        return if cancel.is_cancelled() {
            RetryWake::Cancelled
        } else {
            RetryWake::Slept
        };
    }
    let natural = clock.now() + delay;
    let wake = deadline.map_or(natural, |deadline| natural.min(deadline));
    if !clock.sleep_until(wake, cancel).await {
        return RetryWake::Cancelled;
    }
    if wake < natural {
        return RetryWake::DeadlineReached;
    }
    RetryWake::Slept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_endpoints_match_the_documented_text_api() {
        let endpoints = UpstreamEndpoints::production();
        assert_eq!(
            endpoints.poll_url(&UpstreamTaskId::new("task-123").unwrap()),
            "https://text.external-api.pangram.com/task/task-123"
        );
        assert_eq!(
            endpoints.submit,
            "https://text.external-api.pangram.com/task"
        );
    }

    #[test]
    fn path_encoding_keeps_opaque_ids_structurally_safe() {
        assert_eq!(encode_path("task-123"), "task-123");
        assert_eq!(encode_path("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn status_mapping_uses_canonical_codes() {
        // Exercised through classify_http_failure only indirectly; targeted
        // behavior (429 retry_after, 404 upstream_not_found, 413 usage, and
        // the contextual retryability of 5xx) is proven end-to-end by the
        // loopback protocol tests.
        assert!(ErrorCode::RateLimited.default_retryable());
        assert!(!ErrorCode::UpstreamNotFound.default_retryable());
    }
}
