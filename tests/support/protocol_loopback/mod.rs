//! The real loopback Pangram 4 text fixture (dev-tools only).
//!
//! Axum serves scripted Pangram 4 text responses on an ephemeral loopback
//! port. Handlers record every request that reaches them (method, path,
//! header presence, raw body) and play queued [`Step`]s. Synthetic keys and
//! content only: the fixture and its failure messages never print header
//! values or auth material.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::fmt;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use microck_pangram_cli::analysis::{
    AnalysisConfig, Duration, PollPolicy, RetryPolicy, UpstreamClient, UpstreamEndpoints,
};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// The synthetic fixture key. It identifies nothing and grants nothing.
pub const SYNTHETIC_KEY: &str = "pg_fixture_synthetic_key_00000000000000000000";
pub const TASK_ID: &str = "task-123";
pub const SYNTHETIC_TEXT: &str =
    "A synthetic paragraph authored for loopback protocol verification only.";

/// One full request as seen by the fixture server. Header names plus raw
/// values are recorded; test assertions compare exact values against the
/// synthetic constants and never print them (assertion helpers return
/// booleans only).
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    #[must_use]
    pub fn header_present(&self, name: &str) -> bool {
        self.headers
            .iter()
            .any(|(header, _)| header.eq_ignore_ascii_case(name))
    }

    /// Exact-value comparison returning a bare boolean (no value echoed).
    #[must_use]
    pub fn header_equals(&self, name: &str, expected: &str) -> bool {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .is_some_and(|(_, value)| value == expected)
    }

    #[must_use]
    pub fn body_json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("fixture request body is JSON")
    }
}

/// The scripted behavior for one route invocation. `Debug` is implemented
/// manually below so scripted bodies never print content.
#[derive(Clone)]
pub enum Step {
    /// 200 with a JSON body.
    Json(serde_json::Value),
    /// A status with an optional Retry-After (seconds) and optional JSON.
    Status(u16, Option<u64>, Option<serde_json::Value>),
    /// Hold the request without ever responding (client-timeout trigger).
    Hang,
}

#[derive(Default)]
struct FixtureState {
    requests: Vec<RecordedRequest>,
    submits: VecDeque<Step>,
    polls: VecDeque<Step>,
}

/// The fixture handle. `stop` shuts the server down; dropping without
/// stopping leaks the task until test end (acceptable, but stop is tidier).
pub struct ProtocolFixture {
    base_url: String,
    state: Arc<Mutex<FixtureState>>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ProtocolFixture {
    /// Starts the server on an ephemeral loopback port.
    pub async fn start() -> Self {
        let state = Arc::new(Mutex::new(FixtureState::default()));
        let app = Router::new()
            .route("/task", post(handle_submit))
            .route("/task/{id}", get(handle_poll))
            .fallback(handle_unexpected)
            .with_state(state.clone());

        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .expect("bind the loopback fixture");
        let address = listener.local_addr().expect("the fixture local address");
        let (shutdown, shutdown_rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
            let _ = server.await;
        });

        Self {
            base_url: format!("http://{address}"),
            state,
            shutdown: Some(shutdown),
        }
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn on_submit(&self, step: Step) {
        self.state
            .lock()
            .expect("fixture state")
            .submits
            .push_back(step);
    }

    pub fn on_poll(&self, step: Step) {
        self.state
            .lock()
            .expect("fixture state")
            .polls
            .push_back(step);
    }

    #[must_use]
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.state.lock().expect("fixture state").requests.clone()
    }

    #[must_use]
    pub fn post_count(&self) -> usize {
        self.requests()
            .iter()
            .filter(|request| request.method == "POST")
            .count()
    }

    #[must_use]
    pub fn get_count(&self) -> usize {
        self.requests()
            .iter()
            .filter(|request| request.method == "GET")
            .count()
    }

    /// Waits until at least `n` GET requests have reached the fixture, or
    /// the (real, short) bound elapses. This gives cancellation and pacing
    /// tests a deterministic synchronization point: they must know a poll
    /// actually fired (and its scripted response was consumed, placing the
    /// client inside a retry sleep) before they act, rather than cancel on
    /// a wall-clock guess.
    pub async fn wait_for_gets(&self, n: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.get_count() < n {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {n} GET(s); saw {}",
                self.get_count()
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    /// Waits until at least `n` POST requests have reached the fixture. This
    /// is the deterministic "the billable send was issued" signal a
    /// cancellation test needs before it interrupts: the request bytes are at
    /// the peer, so the outcome is genuinely ambiguous.
    pub async fn wait_for_posts(&self, n: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.post_count() < n {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {n} POST(s); saw {}",
                self.post_count()
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    /// Builds a loopback client pinned to this fixture with deterministic
    /// policy (no backoff waits). Per-request timeout is 400 ms so `Hang`
    /// scenarios stay quick and virtual-time-friendly.
    #[must_use]
    pub fn client(&self) -> UpstreamClient {
        self.client_with_policy(
            RetryPolicy::OFF,
            PollPolicy::new(Duration::ZERO, Duration::ZERO),
            Duration::from_millis(400),
        )
    }

    /// Builds a loopback client with injected retry, poll, and timeout
    /// policy for the bounded- and honored-wait scenarios.
    #[must_use]
    pub fn client_with_policy(
        &self,
        retry: RetryPolicy,
        polling: PollPolicy,
        per_request_timeout: Duration,
    ) -> UpstreamClient {
        let config = AnalysisConfig::for_test(retry, polling, per_request_timeout, 5.0);
        let endpoints =
            UpstreamEndpoints::loopback(&self.base_url).expect("the fixture address is loopback");
        UpstreamClient::for_loopback(
            SecretString::from(SYNTHETIC_KEY.to_owned()),
            config,
            endpoints,
        )
        .expect("loopback client construction")
    }

    pub async fn shutdown(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

async fn split(request: Request) -> (String, String, Vec<(String, String)>, Vec<u8>) {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 4 * 1024 * 1024)
        .await
        .expect("fixture request bodies are small");
    let headers = parts
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or("[non-UTF-8]").to_owned(),
            )
        })
        .collect();
    (
        parts.method.as_str().to_owned(),
        parts.uri.path().to_owned(),
        headers,
        bytes.to_vec(),
    )
}

fn record(
    state: &Arc<Mutex<FixtureState>>,
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) {
    state
        .lock()
        .expect("fixture state")
        .requests
        .push(RecordedRequest {
            method,
            path,
            headers,
            body,
        });
}

async fn handle_submit(
    State(state): State<Arc<Mutex<FixtureState>>>,
    request: Request,
) -> Response {
    let (method, path, headers, body) = split(request).await;
    record(&state, method, path, headers, body);
    let step = state
        .lock()
        .expect("fixture state")
        .submits
        .pop_front()
        .unwrap_or_else(|| panic!("an unscripted POST /task reached the fixture"));
    play(step).await
}

async fn handle_poll(
    State(state): State<Arc<Mutex<FixtureState>>>,
    Path(_id): Path<String>,
    request: Request,
) -> Response {
    let (method, path, headers, body) = split(request).await;
    record(&state, method, path, headers, body);
    let step = state
        .lock()
        .expect("fixture state")
        .polls
        .pop_front()
        .unwrap_or_else(|| panic!("an unscripted GET /task/{{id}} reached the fixture"));
    play(step).await
}

async fn handle_unexpected(
    State(state): State<Arc<Mutex<FixtureState>>>,
    request: Request,
) -> Response {
    let (method, path, headers, body) = split(request).await;
    record(&state, method, path.clone(), headers, body);
    panic!("an unexpected request path reached the fixture: {path}");
}

async fn play(step: Step) -> Response {
    match step {
        Step::Json(value) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(value.to_string()))
            .expect("fixture response"),
        Step::Status(status, retry_after_seconds, body) => {
            let mut builder = Response::builder().status(status);
            if let Some(seconds) = retry_after_seconds {
                builder = builder.header("retry-after", seconds.to_string());
            }
            let payload = body.map_or_else(String::new, |value| value.to_string());
            builder
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload))
                .expect("fixture response")
        }
        Step::Hang => std::future::pending().await,
    }
}

impl fmt::Debug for Step {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(_) => formatter.write_str("Step::Json(<scripted>)"),
            Self::Status(status, retry_after, _) => formatter
                .debug_tuple("Step::Status")
                .field(status)
                .field(retry_after)
                .finish(),
            Self::Hang => formatter.write_str("Step::Hang"),
        }
    }
}

/// A canonical Pangram 4 success document for one text.
#[must_use]
pub fn pangram4_success(text: &str) -> serde_json::Value {
    serde_json::json!({
        "stage": "STAGE_SUCCESS",
        "text": text,
        "version": "4.0",
        "headline": "Human-written",
        "prediction": "The document appears to be human-written.",
        "prediction_short": "Human",
        "fraction_ai": 0.0,
        "fraction_ai_assisted": 0.0,
        "fraction_human": 1.0,
        "num_ai_segments": 0,
        "num_ai_assisted_segments": 0,
        "num_human_segments": 1,
        "windows": [
            {
                "text": text,
                "label": "Human Written",
                "ai_assistance_score": 0.0,
                "confidence": "High",
                "start_index": 0,
                "end_index": text.chars().count(),
                "word_count": 8,
                "token_length": 8,
                "is_humanized": false,
                "humanizer_score": 0.0
            }
        ]
    })
}

/// A canonical terminal-failure document.
#[must_use]
pub fn pangram4_failure(message: &str) -> serde_json::Value {
    serde_json::json!({
        "stage": "STAGE_FAILED",
        "error_message": message,
    })
}
