//! The bulk half of the loopback fixture: the four documented Pangram 4
//! bulk routes and their scripted queues (contracts section 9.1, official
//! source `eb214f4`). Handlers share request recording with the text fixture
//! through [`super::FixtureState`]; this module owns only the `/bulk` route
//! registration and handlers so the parent stays below the decomposition
//! threshold.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::response::Response;
use axum::routing::{get, post};

use super::{BulkQueues, FixtureState, RecordedRequest, Step, play, record, split};
use microck_pangram_cli::domain::{
    BulkItemsPage, BulkResultsPage, BulkStatusResponse, BulkSubmitResponse,
};
use secrecy::ExposeSecret as _;

/// Registers the four documented bulk routes on the fixture router.
pub fn routes() -> Router<Arc<Mutex<FixtureState>>> {
    Router::new()
        .route("/bulk", post(handle_bulk_submit))
        .route("/bulk/{id}", get(handle_bulk_status))
        .route("/bulk/{id}/items", get(handle_bulk_items))
        .route("/bulk/{id}/results", get(handle_bulk_results))
}

fn pop(
    state: &Arc<Mutex<FixtureState>>,
    pick: impl FnOnce(&mut BulkQueues) -> &mut VecDeque<Step>,
    what: &str,
) -> Step {
    // Take the step under the lock, release the guard, then decide. Panicking
    // while the guard is alive would poison the fixture mutex and hide the
    // original cause behind an unrelated poison panic on the next call.
    let step = {
        let mut guard = state.lock().expect("fixture state");
        pick(guard.bulk_queues()).pop_front()
    };
    step.unwrap_or_else(|| panic!("an unscripted {what} reached the fixture"))
}

async fn handle_bulk_submit(
    State(state): State<Arc<Mutex<FixtureState>>>,
    request: Request,
) -> Response {
    record(&state, split(request).await);
    let step = pop(&state, |queues| &mut queues.submit, "POST /bulk");
    play(step).await
}

async fn handle_bulk_status(
    State(state): State<Arc<Mutex<FixtureState>>>,
    Path(_id): Path<String>,
    request: Request,
) -> Response {
    record(&state, split(request).await);
    let step = pop(&state, |queues| &mut queues.status, "GET /bulk/{id}");
    play(step).await
}

async fn handle_bulk_items(
    State(state): State<Arc<Mutex<FixtureState>>>,
    Path(_id): Path<String>,
    request: Request,
) -> Response {
    record(&state, split(request).await);
    let step = pop(&state, |queues| &mut queues.items, "GET /bulk/{id}/items");
    play(step).await
}

async fn handle_bulk_results(
    State(state): State<Arc<Mutex<FixtureState>>>,
    Path(_id): Path<String>,
    request: Request,
) -> Response {
    record(&state, split(request).await);
    let step = pop(
        &state,
        |queues| &mut queues.results,
        "GET /bulk/{id}/results",
    );
    play(step).await
}

/// Read-only views over the recorded request log so tests assert the exact
/// per-route request grammar (method, path, query) without scanning every
/// recorded request.
pub struct BulkRequestView;

impl BulkRequestView {
    /// Recorded `POST /bulk` submissions.
    #[must_use]
    pub fn submits(recorded: &[RecordedRequest]) -> Vec<&RecordedRequest> {
        recorded
            .iter()
            .filter(|request| request.method == "POST" && request.path == "/bulk")
            .collect()
    }

    /// Recorded requests for one job sub-path: `""` for status, `"/items"`,
    /// or `"/results"`. The raw `query` is available on each request for
    /// offset/limit assertions.
    #[must_use]
    pub fn for_path<'a>(
        recorded: &'a [RecordedRequest],
        bulk_id: &str,
        suffix: &str,
    ) -> Vec<&'a RecordedRequest> {
        let prefix = format!("/bulk/{bulk_id}{suffix}");
        recorded
            .iter()
            .filter(|request| request.path == prefix)
            .collect()
    }
}

/// The outcome of one bulk probe call: the HTTP status plus the decoded
/// typed body for 2xx responses. Non-2xx responses carry the status so tests
/// assert the exact failure-mapping surface without a body contract.
#[derive(Debug)]
pub struct BulkProbeOutcome<T> {
    pub status: u16,
    pub body: Option<T>,
}

/// A loopback-only bulk probe: the real HTTP surface the future production
/// analysis client will consume. It issues actual requests through reqwest
/// against the fixture's `/bulk` routes and decodes 2xx bodies into the
/// documented domain wire types. It is deliberately thin: no retry, pacing,
/// or normalization lives here (those belong to the Phase 3 production
/// client); this only proves the fixture plays the documented contract.
///
/// The probe is constructed from the fixture [`UpstreamClient`]'s resolved
/// loopback endpoints and reuses its synthetic key, so request-auth and path
/// assertions mirror the production grammar exactly.
pub struct BulkProbeClient {
    http: reqwest::Client,
    api_key: secrecy::SecretString,
    submit_url: String,
    status_url: String,
}

impl BulkProbeClient {
    /// Builds a probe bound to the fixture client's loopback bulk endpoints.
    /// Panics on a non-loopback endpoint set: construction is only valid for
    /// the `dev-tools` fixture client.
    pub fn from_fixture(client: &microck_pangram_cli::analysis::UpstreamClient) -> Self {
        let endpoints = client.endpoints();
        let base = endpoints.bulk_base();
        assert!(
            base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:"),
            "the bulk probe only binds to a loopback fixture, got {base}"
        );
        // The fixture key is a compile-time synthetic constant; resolve it
        // from the fixture's key constant rather than the (private) client
        // field so the probe never touches real material.
        Self {
            http: reqwest::Client::new(),
            api_key: secrecy::SecretString::from(super::SYNTHETIC_KEY.to_owned()),
            submit_url: base.clone(),
            status_url: base,
        }
    }

    fn bulk_url(&self, bulk_id: &str, suffix: &str) -> String {
        format!("{}/{bulk_id}{suffix}", self.status_url)
    }

    /// `POST /bulk` with one body. Returns the raw status and the decoded
    /// acceptance for 2xx, so the test asserts both the grammar and the
    /// documented response contract (and 413/422 bodies carry no acceptance).
    pub async fn submit(&self, body: &serde_json::Value) -> BulkProbeOutcome<BulkSubmitResponse> {
        let response = self
            .http
            .post(&self.submit_url)
            .header("x-api-key", self.api_key.expose_secret())
            .json(body)
            .send()
            .await
            .expect("bulk submit reaches the fixture");
        let status = response.status().as_u16();
        let body = if (200..300).contains(&status) {
            Some(
                response
                    .json::<BulkSubmitResponse>()
                    .await
                    .expect("a 2xx bulk submit decodes into the documented acceptance"),
            )
        } else {
            None
        };
        BulkProbeOutcome { status, body }
    }

    /// `GET /bulk/{bulk_id}`.
    pub async fn status(&self, bulk_id: &str) -> BulkProbeOutcome<BulkStatusResponse> {
        let url = self.bulk_url(bulk_id, "");
        let response = self
            .http
            .get(&url)
            .header("x-api-key", self.api_key.expose_secret())
            .send()
            .await
            .expect("bulk status reaches the fixture");
        let status = response.status().as_u16();
        let body = if (200..300).contains(&status) {
            Some(
                response
                    .json::<BulkStatusResponse>()
                    .await
                    .expect("a 2xx bulk status decodes into the documented counters"),
            )
        } else {
            None
        };
        BulkProbeOutcome { status, body }
    }

    /// `GET /bulk/{bulk_id}/items?offset=&limit=`.
    pub async fn items(
        &self,
        bulk_id: &str,
        offset: u64,
        limit: u64,
    ) -> BulkProbeOutcome<BulkItemsPage> {
        let url = format!(
            "{}?offset={offset}&limit={limit}",
            self.bulk_url(bulk_id, "/items")
        );
        let response = self
            .http
            .get(&url)
            .header("x-api-key", self.api_key.expose_secret())
            .send()
            .await
            .expect("bulk items reaches the fixture");
        let status = response.status().as_u16();
        let body = if (200..300).contains(&status) {
            Some(
                response
                    .json::<BulkItemsPage>()
                    .await
                    .expect("a 2xx bulk items page decodes into the documented metadata"),
            )
        } else {
            None
        };
        BulkProbeOutcome { status, body }
    }

    /// `GET /bulk/{bulk_id}/results?offset=&limit=`.
    pub async fn results(
        &self,
        bulk_id: &str,
        offset: u64,
        limit: u64,
    ) -> BulkProbeOutcome<BulkResultsPage> {
        let url = format!(
            "{}?offset={offset}&limit={limit}",
            self.bulk_url(bulk_id, "/results")
        );
        let response = self
            .http
            .get(&url)
            .header("x-api-key", self.api_key.expose_secret())
            .send()
            .await
            .expect("bulk results reaches the fixture");
        let status = response.status().as_u16();
        let body = if (200..300).contains(&status) {
            Some(
                response
                    .json::<BulkResultsPage>()
                    .await
                    .expect("a 2xx bulk results page decodes into the documented lists"),
            )
        } else {
            None
        };
        BulkProbeOutcome { status, body }
    }
}
