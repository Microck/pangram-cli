//! The shared Pangram 4 bulk-analysis module: submit, observe/wait, and
//! typed page reads over one accepted job.
//!
//! This module owns the bulk protocol surface for every adapter (contracts
//! section 9.1); CLI/TUI/MCP call these typed operations and never construct
//! bulk HTTP themselves. One observation loop serves wait; one safe-GET
//! retry chain serves status/items/results reads. There is no aggregate
//! endpoint and no second polling implementation.
//!
//! The hub keeps the request/identity/progress shapes, the `RunningBulk`
//! observational state machine, and the `BulkAnalyzer` lifecycle (submit,
//! resume, observation). The read-only typed page reads (items, results, the
//! bounded fetch-all walk) and their page-only error mapping live in the
//! `fetch` submodule so each module stays cohesive and under the source-size
//! threshold; the shared operation-error helpers that both use
//! (`contract_symptom`, `running_wait_timeout`) stay here as `pub(super)`.
//!
//! Safety invariants, on top of the shared analyzer guarantees:
//!
//! - A bulk submission is issued exactly once from a validated
//!   [`crate::domain::BulkSubmissionPlan`]; the constructor has already run
//!   the caller-ceiling and 1,000-unit preflight before this module touches
//!   credentials or the network. An ambiguous send is never replayed and
//!   surfaces the canonical `submission_outcome_unknown` with the local bulk
//!   ID and the exact request SHA-256 (contracts section 12.1).
//! - Status, counters, timestamps, and page windows are validated during
//!   normalization; unknown/impossible state and page-window drift fail
//!   fail-closed as `upstream_contract_changed` (architecture section 6.2).
//! - Item order and caller IDs from the acceptance are preserved exactly;
//!   no task ID or acceptance certainty is ever fabricated.
//! - Each item's canonical input descriptor derives from the validated
//!   local plan, never from untrusted upstream result text (contracts
//!   section 9.1).
//! - Cancellation and wait deadlines stop local observation only; no remote
//!   cancellation request is ever sent, and successful child results are
//!   preserved on partial state.
//! - Credentials, auth headers, item text, and result content never enter
//!   errors, Debug output, or serialized details; upstream failure text is
//!   reduced before any canonical surface.
//!
//! `CanonicalError` is 224 bytes wide because it is the adapter-facing
//! canonical object: bulk paths return it directly rather than boxing every
//! intermediate, matching the rest of the analysis framework.
#![allow(clippy::result_large_err)]

mod assemble;
mod children;
mod fetch;

use std::fmt;

use tokio_util::sync::CancellationToken;

use crate::domain::{
    AnalysisStatus, BulkCollection, BulkId, BulkPage, BulkSubmissionPlan, LocalOperationId,
    NonEmptyString, Sha256Hash, SubmissionOutcomeUnknownDetails, UpstreamBulkId, UtcTimestamp,
};
use crate::output::{CanonicalError, ErrorCode};

use super::config::Clock;
use super::normalize::bulk::{self, NormalizedBulkStatus};
use super::upstream::{PollError, SubmitOutcome, UpstreamClient};
use super::{StopObserving, WaitOptions};

use assemble::{assemble_accepted_collection, assemble_collection, initial_counters};

/// One validated bulk-analysis request. The plan carries the constructor-
/// enforced ceiling preflight; the local bulk ID is generated here so it
/// exists before any network call and can be reported on every ambiguous or
/// interrupted outcome.
#[derive(Clone)]
pub struct BulkAnalysisRequest {
    id: BulkId,
    plan: BulkSubmissionPlan,
}

impl fmt::Debug for BulkAnalysisRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BulkAnalysisRequest")
            .field("id", &self.id)
            .field("item_count", &self.plan.items().len())
            .field(
                "estimated_billable_units",
                &self.plan.estimated_billable_units(),
            )
            .field("max_billable_units", &self.plan.max_billable_units())
            .finish_non_exhaustive()
    }
}

impl BulkAnalysisRequest {
    /// Wraps a validated plan. Construction is infallible: the plan
    /// constructor owns every preflight invariant, so no network or
    /// credential work can precede ceiling enforcement.
    #[must_use]
    pub fn new(plan: BulkSubmissionPlan) -> Self {
        Self {
            id: BulkId::new(),
            plan,
        }
    }

    #[must_use]
    pub const fn id(&self) -> BulkId {
        self.id
    }

    #[must_use]
    pub const fn plan(&self) -> &BulkSubmissionPlan {
        &self.plan
    }

    /// The exact JSON body sent upstream for this request: the single owner
    /// of the submit document that [`Self::request_sha256`] hashes and the
    /// client posts, so the reconciliation hash can never drift from the
    /// bytes on the wire.
    #[must_use]
    pub fn submit_body(&self) -> serde_json::Value {
        self.plan.submit_body()
    }

    /// The request SHA-256 used in `submission_outcome_unknown` details.
    /// Hashes the exact JSON document [`Self::submit_body`] returns.
    #[must_use]
    pub fn request_sha256(&self) -> Sha256Hash {
        Sha256Hash::digest(self.submit_body().to_string().as_bytes())
    }
}

/// The identity tuple reported on wait timeouts and interruptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkOperationIdentity {
    pub bulk_id: BulkId,
    pub upstream_bulk_id: Option<UpstreamBulkId>,
}

/// A locally stopped bulk observation. Always maps to exit 130.
#[derive(Debug)]
pub struct InterruptedBulk {
    pub identity: BulkOperationIdentity,
}

/// The non-terminal progress snapshot emitted after each observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkProgress {
    pub bulk_id: BulkId,
    pub status: AnalysisStatus,
    pub counters: crate::domain::BulkCounters,
}

/// A validation failure of one bulk operation: the canonical error plus the
/// local identity needed for the adapter to report it.
#[derive(Debug)]
pub struct BulkAnalysisError {
    bulk_id: BulkId,
    error: CanonicalError,
}

impl BulkAnalysisError {
    fn new(bulk_id: BulkId, error: CanonicalError) -> Self {
        Self { bulk_id, error }
    }

    #[must_use]
    pub const fn bulk_id(&self) -> BulkId {
        self.bulk_id
    }

    #[must_use]
    pub const fn canonical(&self) -> &CanonicalError {
        &self.error
    }

    #[must_use]
    pub fn into_error(self) -> CanonicalError {
        self.error
    }
}

/// The result of one bulk page read: the canonical page plus the job
/// metadata (identity, counters, timestamps) the adapter needs for a
/// `bulk_results`-style envelope. The items/results JSON surface serializes
/// only the page; the metadata supports provenance and consistency.
#[derive(Debug)]
pub struct BulkPageResult {
    pub page: BulkPage<CanonicalError>,
    pub total_items: u64,
    pub upstream_bulk_id: UpstreamBulkId,
}

/// One validated normalized status snapshot: the fields collection assembly
/// reads. Private to the pipeline; the normalized status is validated before
/// it reaches this shape.
#[derive(Debug)]
pub(super) struct StatusSnapshot {
    pub(super) status: AnalysisStatus,
    pub(super) counters: crate::domain::BulkCounters,
    pub(super) created_at: UtcTimestamp,
    pub(super) completed_at: Option<UtcTimestamp>,
}

impl StatusSnapshot {
    fn from_normalized(normalized: NormalizedBulkStatus) -> Self {
        Self {
            status: normalized.status,
            counters: normalized.counters,
            created_at: normalized.created_at,
            completed_at: normalized.completed_at,
        }
    }
}

/// The running bulk operation returned by [`BulkAnalyzer::submit_bulk`]. Its
/// identity exists before submission so every failure path can report it. The
/// validated plan is retained so per-item input descriptors are built from
/// trusted local data only.
#[derive(Clone)]
pub struct RunningBulk<C = super::config::SystemClock> {
    client: UpstreamClient<C>,
    bulk_id: BulkId,
    upstream_bulk_id: UpstreamBulkId,
    /// The validated local submission plan. `Some` for an operation this
    /// process submitted (or a same-process resume), so per-item input
    /// descriptors are built from trusted local data only. `None` for a
    /// resumed observation of a remotely authored job (contracts.md 4.6):
    /// page items then derive descriptors only from upstream-attested
    /// terminal content, and the collection omits the billing estimate.
    plan: Option<BulkSubmissionPlan>,
    /// The validated HTTP 202 per-item acceptance outcomes retained at
    /// submit time (accepted with attested task IDs, failed with their
    /// canonical errors). `Some` only for an operation this process
    /// submitted; `None` on any resumed handle. History persistence reads
    /// this through [`RunningBulk::acceptance_children`] so a `bulk submit`
    /// without `--wait` never fabricates all-queued children over an
    /// acceptance that already attests identities or immediate failures.
    acceptance: Option<super::normalize::bulk::NormalizedBulkPlan>,
    /// The local acceptance time when this process submitted or rehydrated
    /// the plan-backed operation. Remote-only observation keeps this absent,
    /// so child projection never invents submission authorship.
    submitted_at: Option<UtcTimestamp>,
    last: crate::domain::BulkCounters,
    last_status: AnalysisStatus,
}

impl<C: Clock> fmt::Debug for RunningBulk<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunningBulk")
            .field("bulk_id", &self.bulk_id)
            .field("upstream_bulk_id", &self.upstream_bulk_id)
            .field("last_status", &self.last_status)
            .field("last", &self.last)
            .finish_non_exhaustive()
    }
}

impl<C: Clock> RunningBulk<C> {
    #[must_use]
    pub const fn bulk_id(&self) -> BulkId {
        self.bulk_id
    }

    #[must_use]
    pub const fn upstream_bulk_id(&self) -> &UpstreamBulkId {
        &self.upstream_bulk_id
    }

    /// The validated local plan when this handle describes a locally
    /// submitted (or planned) operation; `None` on a resumed observation.
    #[must_use]
    pub const fn plan(&self) -> Option<&BulkSubmissionPlan> {
        self.plan.as_ref()
    }

    /// The most recent observed status/counter state (for fetch-all
    /// progress reporting).
    #[must_use]
    pub(super) const fn last_state(&self) -> (AnalysisStatus, crate::domain::BulkCounters) {
        (self.last_status, self.last)
    }

    /// Projects the canonical collection certified by the validated HTTP 202
    /// acceptance. The retained plan and normalized counters make this
    /// infallible: mixed immediate failures remain non-terminal while
    /// accepted work is outstanding, and an all-failed acceptance completes
    /// at `accepted_at`.
    #[must_use]
    pub fn accepted_collection(&self, accepted_at: UtcTimestamp) -> BulkCollection {
        assemble_accepted_collection(self, accepted_at)
    }

    /// The honest plan children of the validated HTTP 202 acceptance
    /// (contracts section 9 + 14.2): accepted positions persist as `accepted`
    /// queued children with their attested task identities; failed positions
    /// become terminal-failed children with the canonical check error; only
    /// unattested positions stay `not_submitted` and queued. Returns an
    /// empty series on a resumed observation (no local plan). `source_name`
    /// is the JSONL source basename for a file submission (`stdin` when
    /// `None`).
    #[must_use]
    pub fn acceptance_children(
        &self,
        source_name: Option<&str>,
        observed_at: UtcTimestamp,
    ) -> Vec<(crate::domain::Analysis<CanonicalError>, Option<String>)> {
        match (self.plan.as_ref(), self.acceptance.as_ref()) {
            (Some(plan), Some(normalized)) => children::build_acceptance_children(
                plan,
                normalized,
                &self.upstream_bulk_id,
                source_name,
                observed_at,
            ),
            _ => Vec::new(),
        }
    }

    /// Projects the observed children of one fetched results window for the
    /// history save seam (contracts.md 14.2). The caller furnishes the
    /// already-validated `window` (a [`crate::domain::BulkPage`] from any
    /// safe-GET read); this retains same-process authorship when the handle
    /// owns a plan and otherwise assembles remote-only `accepted` children.
    /// It issues no further network work, so projection cannot diverge from
    /// the read the caller already performed.
    #[must_use]
    pub fn project_observed_children(
        &self,
        window: crate::domain::BulkPage<CanonicalError>,
        source_name: Option<&str>,
    ) -> Vec<(crate::domain::Analysis<CanonicalError>, Option<String>)> {
        children::build_observed_children(
            window,
            self.plan.as_ref(),
            self.submitted_at,
            self.upstream_bulk_id(),
            &children::ChildInputContext::new(source_name.map(str::to_owned)),
            UtcTimestamp::now(),
        )
    }

    #[must_use]
    pub fn identity(&self) -> BulkOperationIdentity {
        BulkOperationIdentity {
            bulk_id: self.bulk_id,
            upstream_bulk_id: Some(self.upstream_bulk_id.clone()),
        }
    }

    /// Issues the shared safe-GET status read and normalizes the document,
    /// cross-checking `total_items` against the validated plan count. Safe
    /// GET retry, pacing, deadline, and the cumulative budget are all shared
    /// through the client's one fetch chain.
    async fn fetch_status(
        &self,
        cancel: &CancellationToken,
        deadline: Option<super::Instant>,
    ) -> Result<StatusSnapshot, PollError> {
        let url = self.client.bulk_status_url(self.upstream_bulk_id.as_str());
        let fetch = self.client.fetch_bulk_page(url, cancel, deadline).await?;
        let super::upstream::BulkPageFetch::Page(response) = fetch else {
            return Err(PollError::Failed(Box::new(
                CanonicalError::new(
                    ErrorCode::UpstreamNotFound,
                    "Pangram does not recognize the bulk job.",
                )
                .expect("static template"),
            )));
        };
        let response = *response;
        let wire: crate::domain::BulkStatusResponse = response.json().map_err(|error| {
            PollError::Failed(Box::new(contract_symptom("body", error.to_string())))
        })?;
        let plan_total = self
            .plan
            .as_ref()
            .map(|plan| u64::try_from(plan.items().len()).unwrap_or(u64::MAX));
        let normalized = bulk::normalize_bulk_status(&wire, plan_total)
            .map_err(|error| PollError::Failed(Box::new(error)))?;
        if normalized.bulk_id.as_str() != self.upstream_bulk_id.as_str() {
            return Err(PollError::Failed(Box::new(contract_symptom(
                "bulk_id",
                "status identity does not match the queried job",
            ))));
        }
        Ok(StatusSnapshot::from_normalized(normalized.status))
    }

    /// Polls until terminal, the local timeout, or local cancellation. This
    /// is the one bulk observation loop: progress is reported through
    /// `on_progress` after each non-terminal snapshot, and a terminal or
    /// partial collection preserves the exact counters. Cancel-safe: dropping
    /// the future leaves no orphans.
    pub async fn observe(
        mut self,
        options: WaitOptions,
        mut on_progress: impl FnMut(&BulkProgress),
        stop: StopObserving,
    ) -> Result<Result<BulkCollection, BulkAnalysisError>, InterruptedBulk> {
        let cancel = stop.token().child_token();
        let clock = self.client.config().clock();
        let deadline = options.timeout.map(|timeout| clock.now() + timeout);

        loop {
            if cancel.is_cancelled() {
                return Err(InterruptedBulk {
                    identity: self.identity(),
                });
            }
            if let Some(deadline) = deadline
                && clock.now() >= deadline
            {
                return Ok(Err(BulkAnalysisError::new(
                    self.bulk_id,
                    self.wait_timeout_error(),
                )));
            }

            match self.fetch_status(&cancel, deadline).await {
                Ok(snapshot) => {
                    self.last = snapshot.counters;
                    self.last_status = snapshot.status;
                    let collection = assemble_collection(&self, &snapshot, UtcTimestamp::now());
                    match snapshot.status {
                        AnalysisStatus::Queued | AnalysisStatus::Running => {
                            on_progress(&BulkProgress {
                                bulk_id: self.bulk_id,
                                status: snapshot.status,
                                counters: snapshot.counters,
                            });
                        }
                        // Terminal (succeeded/failed/partial): return the
                        // collection with successful child results preserved
                        // in its exact counters.
                        _ => return Ok(Ok(collection)),
                    }
                }
                Err(PollError::Cancelled) => {
                    return Err(InterruptedBulk {
                        identity: self.identity(),
                    });
                }
                Err(PollError::DeadlineExceeded) => {
                    return Ok(Err(BulkAnalysisError::new(
                        self.bulk_id,
                        self.wait_timeout_error(),
                    )));
                }
                Err(PollError::Failed(error)) => {
                    return Ok(Err(BulkAnalysisError::new(self.bulk_id, *error)));
                }
            }

            let interval = self.client.config().polling().effective_interval();
            let wake = {
                let natural = clock.now() + interval;
                deadline.map_or(natural, |deadline| natural.min(deadline))
            };
            if !clock.sleep_until(wake, &cancel).await {
                return Err(InterruptedBulk {
                    identity: self.identity(),
                });
            }
        }
    }

    /// One safe status observation: fetch once and assemble the current
    /// canonical collection without looping. Wait deadlines apply to this
    /// single observation through the shared fetch chain.
    pub async fn snapshot(
        mut self,
        cancel: &CancellationToken,
        deadline: Option<super::Instant>,
    ) -> Result<BulkCollection, BulkAnalysisError> {
        match self.fetch_status(cancel, deadline).await {
            Ok(snapshot) => {
                self.last = snapshot.counters;
                self.last_status = snapshot.status;
                Ok(assemble_collection(&self, &snapshot, UtcTimestamp::now()))
            }
            Err(PollError::Cancelled) => Err(BulkAnalysisError::new(
                self.bulk_id,
                CanonicalError::new(
                    ErrorCode::NetworkUnavailable,
                    "The bulk status read was cancelled locally; no remote action was taken.",
                )
                .expect("static template"),
            )),
            Err(PollError::DeadlineExceeded) => Err(BulkAnalysisError::new(
                self.bulk_id,
                self.wait_timeout_error(),
            )),
            Err(PollError::Failed(error)) => Err(BulkAnalysisError::new(self.bulk_id, *error)),
        }
    }

    fn wait_timeout_error(&self) -> CanonicalError {
        running_wait_timeout(self)
    }
}

/// Owns construction of bulk operations over one shared client. Cloning
/// shares the connection pool and the time-based pacing gate. Internal to
/// the analysis module: adapters enter through the `Analyzer` facade, which
/// exposes the bulk surface over this one shared client.
#[derive(Clone)]
pub struct BulkAnalyzer<C = super::config::SystemClock> {
    client: UpstreamClient<C>,
}

impl<C: Clock> BulkAnalyzer<C> {
    #[must_use]
    pub(super) fn from_client(client: UpstreamClient<C>) -> Self {
        Self { client }
    }

    /// Submits one validated bulk plan exactly once from the plan's
    /// constructor-enforced ceiling preflight. On acceptance the running
    /// handle is returned; an ambiguous send surfaces the canonical
    /// `submission_outcome_unknown` with the local bulk ID and request hash;
    /// a deterministic pre-billing failure carries the mapped canonical code.
    pub async fn submit_bulk(
        &self,
        request: BulkAnalysisRequest,
        cancel: &CancellationToken,
    ) -> Result<RunningBulk<C>, BulkAnalysisError> {
        let plan = request.plan().clone();
        match self.client.submit_bulk(&plan, cancel).await {
            Ok(accepted) => {
                let normalized = bulk::normalize_bulk_acceptance(
                    &accepted.acceptance,
                    u64::try_from(plan.items().len()).unwrap_or(u64::MAX),
                )
                .map_err(|error| BulkAnalysisError::new(request.id(), error))?;
                let running = self.build_running(request, accepted.bulk_id, &normalized);
                Ok(running)
            }
            Err(SubmitOutcome::Failed(error)) => {
                Err(BulkAnalysisError::new(request.id(), *error))
            }
            Err(SubmitOutcome::Cancelled) => Err(BulkAnalysisError::new(
                request.id(),
                CanonicalError::new(
                    ErrorCode::NetworkUnavailable,
                    "The bulk submission was cancelled locally before an upstream acceptance; no remote action was taken.",
                )
                .expect("static template"),
            )),
            Err(SubmitOutcome::Ambiguous(_)) => {
                let details = SubmissionOutcomeUnknownDetails::new(
                    LocalOperationId::BulkId(request.id()),
                    request.request_sha256(),
                    None,
                    None,
                    NonEmptyString::new("bulk creation unacknowledged".to_owned())
                        .expect("fixed label is non-empty"),
                );
                Err(BulkAnalysisError::new(
                    request.id(),
                    CanonicalError::submission_outcome_unknown(
                        "The bulk submission may have reached Pangram, but no acceptance was obtained.",
                        details,
                    )
                    .expect("submission-unknown construction is statically valid"),
                ))
            }
        }
    }

    fn build_running(
        &self,
        request: BulkAnalysisRequest,
        upstream_bulk_id: UpstreamBulkId,
        normalized: &super::normalize::bulk::NormalizedBulkPlan,
    ) -> RunningBulk<C> {
        let (counters, initial_status) = initial_counters(request.plan(), normalized);
        RunningBulk {
            client: self.client.clone(),
            bulk_id: request.id(),
            upstream_bulk_id,
            plan: Some(request.plan().clone()),
            acceptance: Some(normalized.clone()),
            submitted_at: Some(UtcTimestamp::now()),
            last: counters,
            last_status: initial_status,
        }
    }

    /// Rehydrates a running handle for an already-accepted job (for example a
    /// `bulk_status`-style read of a job submitted earlier). The caller's
    /// plan is retained so per-item input descriptors stay trusted-local.
    #[must_use]
    pub fn resume(
        &self,
        bulk_id: BulkId,
        upstream_bulk_id: UpstreamBulkId,
        plan: BulkSubmissionPlan,
    ) -> RunningBulk<C> {
        let total = u64::try_from(plan.items().len()).unwrap_or(u64::MAX);
        let counters = crate::domain::BulkCounters::new(total, 0, 0, 0)
            .expect("a total-only counter set is valid");
        RunningBulk {
            client: self.client.clone(),
            bulk_id,
            upstream_bulk_id,
            plan: Some(plan),
            acceptance: None,
            submitted_at: Some(UtcTimestamp::now()),
            last: counters,
            last_status: AnalysisStatus::Queued,
        }
    }

    /// Rehydrates a running handle to observe a remotely authored job by its
    /// explicit upstream ID (`bulk status`, `bulk wait`, `bulk results` of a
    /// job not submitted in this process). No local plan exists, so per-item
    /// descriptors derive only from upstream-attested terminal content and
    /// the collection omits the billing estimate (contracts.md 4.6). Initial
    /// counters are the minimal non-terminal placeholder; the first status
    /// observation rehydrates the real counters.
    #[must_use]
    pub fn resume_observed(&self, upstream_bulk_id: UpstreamBulkId) -> RunningBulk<C> {
        self.resume_observed_as(BulkId::new(), upstream_bulk_id)
    }

    /// Rehydrates a remote-only observation under a caller-resolved local
    /// identity. History-backed adapters use this after resolving a saved
    /// `bulk_` ID; no plan or hidden transient ledger is introduced.
    #[must_use]
    pub fn resume_observed_as(
        &self,
        bulk_id: BulkId,
        upstream_bulk_id: UpstreamBulkId,
    ) -> RunningBulk<C> {
        let counters = crate::domain::BulkCounters::new(1, 0, 0, 0)
            .expect("a total-only counter set is valid");
        RunningBulk {
            client: self.client.clone(),
            bulk_id,
            upstream_bulk_id,
            plan: None,
            acceptance: None,
            submitted_at: None,
            last: counters,
            last_status: AnalysisStatus::Queued,
        }
    }
}

/// The operation-error helpers shared by this hub module and the
/// `fetch` submodule's typed page reads. `contract_symptom` and
/// `running_wait_timeout` are used by both (the status fetch path here and
/// the page/fetch-all reads in `fetch`), so they stay in the parent as
/// `pub(super)`; the page-only `page_poll_error`/`bulk_domain_error` and the
/// fetch-all page constant live in `fetch` next to their only callers.
pub(super) fn running_wait_timeout<C: Clock>(running: &RunningBulk<C>) -> CanonicalError {
    let mut details = std::collections::BTreeMap::new();
    details.insert(
        "bulk_id".to_owned(),
        serde_json::Value::from(running.bulk_id().to_string()),
    );
    details.insert(
        "upstream_bulk_id".to_owned(),
        serde_json::Value::from(running.upstream_bulk_id().as_str()),
    );
    CanonicalError::new(
        ErrorCode::WaitTimeout,
        "Pangram did not finish the bulk job before the local wait timeout.",
    )
    .and_then(|error| error.with_details(details))
    .expect("static template")
}

pub(super) fn contract_symptom(field: &'static str, token: impl Into<String>) -> CanonicalError {
    let mut details = std::collections::BTreeMap::new();
    details.insert("field".to_owned(), serde_json::Value::from(field));
    details.insert(
        "token".to_owned(),
        serde_json::Value::from(super::normalize::sanitize_upstream_message(&token.into())),
    );
    CanonicalError::new(
        ErrorCode::UpstreamContractChanged,
        "Pangram returned a document outside the pinned Pangram 4 contract.",
    )
    .and_then(|error| error.with_details(details))
    .expect("static template")
}
