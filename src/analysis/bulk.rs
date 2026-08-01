//! The shared Pangram 4 bulk-analysis module: submit, observe/wait, and
//! typed page reads over one accepted job.
//!
//! This module owns the bulk protocol surface for every adapter (contracts
//! section 9.1); CLI/TUI/MCP call these typed operations and never construct
//! bulk HTTP themselves. One observation loop serves wait; one safe-GET
//! retry chain serves status/items/results reads. There is no aggregate
//! endpoint and no second polling implementation.
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
//!
//! Architectural note (line budget): this module intentionally exceeds
//! 1,000 lines. The bulk surface is one cohesive pipeline — submit, observe
//! loop, terminal projection, then typed page reads — and splitting it into
//! submodules would force cross-file plumbing of the private `RunningBulk`
//! state (plan, accepted acceptance, live counters, paging scratch) purely
//! for the line count, obscuring the single-owner observation invariants
//! listed above. The normalization rules that grow independently already
//! live in [`crate::analysis::normalize::bulk`]; what remains here is the
//! protocol-adjacent orchestration. Splits stay cheap later because all
//! helpers are free functions with no shared mutable state.
#![allow(clippy::result_large_err)]

use std::fmt;

use tokio_util::sync::CancellationToken;

use crate::domain::{
    Analysis, AnalysisId, AnalysisInput, AnalysisStatus, BulkCollection, BulkCounters, BulkId,
    BulkItem, BulkItemState, BulkPage, BulkSubmissionPlan, Check, CheckState, LocalOperationId,
    NonEmptyString, OrderedChecks, Provenance, Provider, SaveState, Sha256Hash, SubmissionOutcome,
    SubmissionOutcomeUnknownDetails, TextInput, TextOrigin, UpstreamBulkId, UpstreamIdentity,
    UpstreamTaskId, UtcTimestamp,
};
use crate::output::{CanonicalError, ErrorCode};

use super::config::Clock;
use super::normalize::bulk::{
    self, NormalizedBulkItemOutcome, NormalizedBulkPlan, NormalizedBulkStatus, NormalizedItemsPage,
    NormalizedResultsPage,
};
use super::upstream::{PollError, SubmitOutcome, UpstreamClient};
use super::{StopObserving, WaitOptions};

/// The maximum `limit` the client requests on a page (the documented cap).
const BULK_FETCH_PAGE_LIMIT: u64 = crate::domain::BULK_PAGE_LIMIT_MAX;

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
    pub counters: BulkCounters,
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

/// A normalized status snapshot bundled for canonical-collection assembly.
/// The `created_at`/`completed_at` timestamps are authoritative from
/// upstream; `updated_at` is the local observation time.
#[derive(Debug)]
struct StatusBundle {
    status: NormalizedBulkStatus,
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
    plan: BulkSubmissionPlan,
    last: BulkCounters,
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

    #[must_use]
    pub fn identity(&self) -> BulkOperationIdentity {
        BulkOperationIdentity {
            bulk_id: self.bulk_id,
            upstream_bulk_id: Some(self.upstream_bulk_id.clone()),
        }
    }

    /// Assembles the canonical collection from a normalized status snapshot.
    /// `updated_at` is the observation time (the state change is local); the
    /// upstream `created_at`/`completed_at` are authoritative.
    fn assemble(&self, bundle: &StatusBundle, updated_at: UtcTimestamp) -> BulkCollection {
        BulkCollection::new(
            self.bulk_id,
            Some(self.upstream_bulk_id.clone()),
            bundle.status.status,
            SubmissionOutcome::Accepted,
            bundle.status.counters,
            self.plan.estimated_billable_units(),
            bundle.status.created_at,
            updated_at,
            bundle.status.completed_at,
        )
        .expect("a validated status snapshot always satisfies collection invariants")
    }

    /// Issues the shared safe-GET status read and normalizes the document,
    /// cross-checking `total_items` against the validated plan count. Safe
    /// GET retry, pacing, deadline, and the cumulative budget are all shared
    /// through the client's one fetch chain.
    async fn fetch_status(
        &self,
        cancel: &CancellationToken,
        deadline: Option<super::Instant>,
    ) -> Result<StatusBundle, PollError> {
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
        let plan_total = u64::try_from(self.plan.items().len()).unwrap_or(u64::MAX);
        let normalized = bulk::normalize_bulk_status(&wire, Some(plan_total))
            .map_err(|error| PollError::Failed(Box::new(error)))?;
        if normalized.bulk_id.as_str() != self.upstream_bulk_id.as_str() {
            return Err(PollError::Failed(Box::new(contract_symptom(
                "bulk_id",
                "status identity does not match the queried job",
            ))));
        }
        Ok(StatusBundle {
            status: normalized.status,
        })
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
            if let Some(deadline) = deadline {
                if clock.now() >= deadline {
                    return Ok(Err(BulkAnalysisError::new(
                        self.bulk_id,
                        self.wait_timeout_error(),
                    )));
                }
            }

            match self.fetch_status(&cancel, deadline).await {
                Ok(bundle) => {
                    self.last = bundle.status.counters;
                    self.last_status = bundle.status.status;
                    let collection = self.assemble(&bundle, UtcTimestamp::now());
                    match bundle.status.status {
                        AnalysisStatus::Queued | AnalysisStatus::Running => {
                            on_progress(&BulkProgress {
                                bulk_id: self.bulk_id,
                                status: bundle.status.status,
                                counters: bundle.status.counters,
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
            Ok(bundle) => {
                self.last = bundle.status.counters;
                self.last_status = bundle.status.status;
                Ok(self.assemble(&bundle, UtcTimestamp::now()))
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
/// shares the connection pool and the time-based pacing gate.
#[derive(Clone)]
pub struct BulkAnalyzer<C = super::config::SystemClock> {
    client: UpstreamClient<C>,
}

impl<C: Clock> BulkAnalyzer<C> {
    #[must_use]
    pub fn from_client(client: UpstreamClient<C>) -> Self {
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
        normalized: &NormalizedBulkPlan,
    ) -> RunningBulk<C> {
        let accepted_count = u64::try_from(normalized.accepted.len()).unwrap_or(u64::MAX);
        let failed_count = u64::try_from(normalized.failed.len()).unwrap_or(u64::MAX);
        let total = u64::try_from(request.plan().items().len()).unwrap_or(u64::MAX);
        // The acceptance already validated counters; the initial last-state
        // reflects immediate failures and any not-yet-finished accepted work.
        let counters = BulkCounters::new(total, accepted_count, 0, failed_count)
            .expect("validated acceptance counters satisfy bounds");
        let initial_status = if counters.is_terminal() {
            if failed_count == total {
                AnalysisStatus::Failed
            } else if failed_count > 0 {
                AnalysisStatus::Partial
            } else {
                AnalysisStatus::Succeeded
            }
        } else {
            AnalysisStatus::Queued
        };
        RunningBulk {
            client: self.client.clone(),
            bulk_id: request.id(),
            upstream_bulk_id,
            plan: request.plan().clone(),
            last: counters,
            last_status: initial_status,
        }
    }

    /// Rehydrates a running handle for an already-accepted job (for example a
    /// `bulk_status`-style read of a job submitted earlier). `total_items`
    /// and `estimated_billable_units` are the caller's honest values; the
    /// initial state re-polls on observation.
    #[must_use]
    pub fn resume(
        &self,
        bulk_id: BulkId,
        upstream_bulk_id: UpstreamBulkId,
        plan: BulkSubmissionPlan,
    ) -> RunningBulk<C> {
        let total = u64::try_from(plan.items().len()).unwrap_or(u64::MAX);
        let counters =
            BulkCounters::new(total, 0, 0, 0).expect("a total-only counter set is valid");
        RunningBulk {
            client: self.client.clone(),
            bulk_id,
            upstream_bulk_id,
            plan,
            last: counters,
            last_status: AnalysisStatus::Queued,
        }
    }

    /// Fetches one validated typed items-metadata page (read-only). The page
    /// identity, echoed window, `total_items` consistency, ordering, and
    /// per-item shape are enforced during normalization; the canonical page
    /// strictly ascends by source index.
    pub async fn bulk_items_page(
        &self,
        running: &RunningBulk<C>,
        offset: u64,
        limit: u64,
        cancel: &CancellationToken,
    ) -> Result<BulkPageResult, BulkAnalysisError> {
        validate_page_request(offset, limit)
            .map_err(|error| BulkAnalysisError::new(running.bulk_id, error))?;
        let url = self
            .client
            .bulk_items_url(running.upstream_bulk_id.as_str(), offset, limit);
        let fetch = self
            .client
            .fetch_bulk_page(url, cancel, None)
            .await
            .map_err(|error| page_poll_error(running, error))?;
        let super::upstream::BulkPageFetch::Page(response) = fetch else {
            return Err(BulkAnalysisError::new(
                running.bulk_id,
                CanonicalError::new(
                    ErrorCode::UpstreamNotFound,
                    "Pangram does not recognize the bulk job.",
                )
                .expect("static template"),
            ));
        };
        let response = *response;
        let wire: crate::domain::BulkItemsPage = response.json().map_err(|error| {
            BulkAnalysisError::new(running.bulk_id, contract_symptom("body", error.to_string()))
        })?;
        let mut expected_total: Option<u64> = None;
        let (header, page) = bulk::normalize_items_page(
            &wire,
            running.upstream_bulk_id(),
            offset,
            limit,
            &mut expected_total,
        )
        .map_err(|error| BulkAnalysisError::new(running.bulk_id, error))?;
        let total = header.total_items;
        let items = assemble_items_metadata_page(&running_plan(running), page)
            .map_err(|error| BulkAnalysisError::new(running.bulk_id, error))?;
        let next = next_offset(offset, &items, total);
        Ok(BulkPageResult {
            page: BulkPage::new(items, offset, limit, next)
                .map_err(|error| bulk_domain_error(running.bulk_id, error))?,
            total_items: total,
            upstream_bulk_id: running.upstream_bulk_id.clone(),
        })
    }

    /// Fetches one validated typed results page (read-only). Succeeded items
    /// carry a canonical analysis built from the local plan's trusted input;
    /// failed items carry the sanitized upstream error. The canonical page
    /// strictly ascends by source index.
    pub async fn bulk_results_page(
        &self,
        running: &RunningBulk<C>,
        offset: u64,
        limit: u64,
        cancel: &CancellationToken,
    ) -> Result<BulkPageResult, BulkAnalysisError> {
        validate_page_request(offset, limit)
            .map_err(|error| BulkAnalysisError::new(running.bulk_id, error))?;
        let url = self
            .client
            .bulk_results_url(running.upstream_bulk_id.as_str(), offset, limit);
        let fetch = self
            .client
            .fetch_bulk_page(url, cancel, None)
            .await
            .map_err(|error| page_poll_error(running, error))?;
        let super::upstream::BulkPageFetch::Page(response) = fetch else {
            return Err(BulkAnalysisError::new(
                running.bulk_id,
                CanonicalError::new(
                    ErrorCode::UpstreamNotFound,
                    "Pangram does not recognize the bulk job.",
                )
                .expect("static template"),
            ));
        };
        let response = *response;
        let wire: crate::domain::BulkResultsPage = response.json().map_err(|error| {
            BulkAnalysisError::new(running.bulk_id, contract_symptom("body", error.to_string()))
        })?;
        let mut expected_total: Option<u64> = None;
        let (header, page) = bulk::normalize_results_page(
            &wire,
            running.upstream_bulk_id(),
            offset,
            limit,
            &mut expected_total,
        )
        .map_err(|error| BulkAnalysisError::new(running.bulk_id, error))?;
        let total = header.total_items;
        let items = assemble_results_page(&running_plan(running), page)
            .map_err(|error| BulkAnalysisError::new(running.bulk_id, error))?;
        let next = next_offset(offset, &items, total);
        Ok(BulkPageResult {
            page: BulkPage::new(items, offset, limit, next)
                .map_err(|error| bulk_domain_error(running.bulk_id, error))?,
            total_items: total,
            upstream_bulk_id: running.upstream_bulk_id.clone(),
        })
    }

    /// Iterates documented results pages from offset 0 until the set is
    /// exhausted, with per-position duplicate/out-of-order/non-advancing
    /// protection. Reads are bounded by the caller's `max_reads`; progress is
    /// reported per page through `on_progress`. Returns the strictly ordered
    /// assembled page over the whole covered set.
    ///
    /// There is no aggregate endpoint; this is iteration over documented
    /// pages only (contracts section 9.1). Cancellation stops the walk
    /// between page reads.
    pub async fn bulk_results_all(
        &self,
        running: &RunningBulk<C>,
        max_reads: u64,
        cancel: &CancellationToken,
        mut on_progress: impl FnMut(&BulkProgress),
    ) -> Result<BulkPageResult, BulkAnalysisError> {
        if max_reads == 0 {
            return Err(BulkAnalysisError::new(
                running.bulk_id,
                CanonicalError::new(
                    ErrorCode::InputRequired,
                    "fetch-all requires a positive read bound.",
                )
                .expect("static template"),
            ));
        }
        let mut all_items: Vec<BulkItem<CanonicalError>> = Vec::new();
        let mut covered: Vec<bool> = Vec::new();
        let mut total: Option<u64> = None;
        let mut offset = 0_u64;
        let mut reads = 0_u64;

        loop {
            if cancel.is_cancelled() {
                return Err(BulkAnalysisError::new(
                    running.bulk_id,
                    CanonicalError::new(
                        ErrorCode::NetworkUnavailable,
                        "The bulk fetch-all was cancelled locally; no remote action was taken.",
                    )
                    .expect("static template"),
                ));
            }
            if reads >= max_reads {
                return Err(BulkAnalysisError::new(
                    running.bulk_id,
                    CanonicalError::new(
                        ErrorCode::NetworkUnavailable,
                        "The bulk results fetch-all exceeded its read bound.",
                    )
                    .expect("static template"),
                ));
            }
            let page = self
                .bulk_results_page(running, offset, BULK_FETCH_PAGE_LIMIT, cancel)
                .await?;
            reads += 1;

            let page_total = page.total_items;
            match total {
                None => {
                    total = Some(page_total);
                    covered = vec![false; usize::try_from(page_total).unwrap_or(0)];
                }
                Some(total) if total != page_total => {
                    return Err(BulkAnalysisError::new(
                        running.bulk_id,
                        contract_symptom("total_items", "results pages disagree on the job total"),
                    ));
                }
                _ => {}
            }
            let total_value = total.expect("seeded above");

            let had_any = !page.page.items().is_empty();
            let mut page_max: Option<u64> = None;
            for item in page.page.items() {
                if item.index >= total_value {
                    return Err(BulkAnalysisError::new(
                        running.bulk_id,
                        contract_symptom(
                            "index",
                            format!(
                                "source position {} is at or above total {total_value}",
                                item.index
                            ),
                        ),
                    ));
                }
                let slot = &mut covered[usize::try_from(item.index).unwrap_or(usize::MAX)];
                if *slot {
                    return Err(BulkAnalysisError::new(
                        running.bulk_id,
                        contract_symptom(
                            "index",
                            format!("duplicate source position {} across pages", item.index),
                        ),
                    ));
                }
                *slot = true;
                page_max = Some(item.index);
                all_items.push(clone_item(item));
            }

            on_progress(&BulkProgress {
                bulk_id: running.bulk_id,
                status: running.last_status,
                counters: running.last,
            });

            // Exhaustion: every position covered, or this page was empty.
            let covered_count = covered.iter().filter(|covered| **covered).count();
            if u64::try_from(covered_count).unwrap_or(u64::MAX) >= total_value || !had_any {
                break;
            }
            // Non-advancing protection: without covered positions the walk
            // cannot progress, and the next offset must strictly advance.
            let next = match page_max {
                Some(max) => max + 1,
                None => offset,
            };
            if next <= offset {
                return Err(BulkAnalysisError::new(
                    running.bulk_id,
                    contract_symptom("offset", "the results walk did not advance"),
                ));
            }
            offset = next;
        }

        let total_value = total.unwrap_or(0);
        Ok(BulkPageResult {
            page: BulkPage::new(all_items, 0, BULK_FETCH_PAGE_LIMIT, None)
                .map_err(|error| bulk_domain_error(running.bulk_id, error))?,
            total_items: total_value,
            upstream_bulk_id: running.upstream_bulk_id.clone(),
        })
    }
}

fn running_plan<C: Clock>(running: &RunningBulk<C>) -> BulkSubmissionPlan {
    running.plan.clone()
}

/// Builds canonical analyses for a results page: one per succeeded position,
/// in page order; failed positions become failed analyses carrying the
/// sanitized upstream error. Input descriptors come only from the local plan.
fn assemble_results_page(
    plan: &BulkSubmissionPlan,
    page: NormalizedResultsPage,
) -> Result<Vec<BulkItem<CanonicalError>>, CanonicalError> {
    let NormalizedResultsPage {
        mut succeeded,
        failed,
    } = page;
    let mut indexes: Vec<u64> = succeeded.keys().chain(failed.keys()).copied().collect();
    indexes.sort_unstable();
    indexes.dedup();
    let mut items = Vec::with_capacity(indexes.len());
    for index in indexes {
        let plan_item = plan_item_at(plan, index)?;
        if let Some(success) = succeeded.remove(&index) {
            let now = UtcTimestamp::now();
            let caller_id = plan_item.caller_id().map(|id| id.as_str().to_owned());
            let task_id = success.task_id.clone();
            let analysis = build_terminal_success_analysis(
                plan_item,
                index,
                success.task_id.as_ref(),
                success.task,
                now,
            )?;
            items.push(BulkItem {
                index,
                caller_id,
                analysis_id: Some(analysis.id),
                upstream_task_id: task_id,
                state: BulkItemState::Succeeded {
                    analysis: Box::new(analysis),
                },
            });
        } else if let Some(outcome) = failed.get(&index) {
            let analysis = build_terminal_failed_analysis(plan_item, index, outcome)?;
            items.push(BulkItem {
                index,
                caller_id: plan_item.caller_id().map(|id| id.as_str().to_owned()),
                analysis_id: Some(analysis.id),
                upstream_task_id: outcome.task_id.clone(),
                state: BulkItemState::Failed {
                    error: outcome_error(outcome),
                },
            });
        }
    }
    Ok(items)
}

/// Builds canonical bulk items for an items-metadata page: one per reported
/// position, preserving the worker's in-flight or failed state with the
/// worker's own task identity. Item order is enforced upstream by the page
/// validator; each entry only composes the worker's reported outcome.
fn assemble_items_metadata_page(
    plan: &BulkSubmissionPlan,
    page: NormalizedItemsPage,
) -> Result<Vec<BulkItem<CanonicalError>>, CanonicalError> {
    let NormalizedItemsPage { outcomes } = page;
    let mut items = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        let plan_item = plan_item_at(plan, outcome.index)?;
        let NormalizedBulkItemOutcome {
            index,
            task_id,
            error,
        } = outcome;
        match error {
            Some(_) => {
                let outcome = NormalizedBulkItemOutcome {
                    index,
                    task_id: task_id.clone(),
                    error,
                };
                let analysis = build_terminal_failed_analysis(plan_item, index, &outcome)?;
                items.push(BulkItem {
                    index,
                    caller_id: plan_item.caller_id().map(|id| id.as_str().to_owned()),
                    analysis_id: Some(analysis.id),
                    upstream_task_id: task_id,
                    state: BulkItemState::Failed {
                        error: outcome_error(&outcome),
                    },
                });
            }
            None => {
                // Accepted in-flight work: a running metadata entry whose
                // upstream task identity is the worker's. A missing task_id
                // on accepted work is not asserted here (the metadata page
                // may carry null task_id for very early items).
                items.push(BulkItem {
                    index,
                    caller_id: plan_item.caller_id().map(|id| id.as_str().to_owned()),
                    analysis_id: None,
                    upstream_task_id: task_id,
                    state: BulkItemState::Running,
                });
            }
        }
    }
    Ok(items)
}

fn plan_item_at(
    plan: &BulkSubmissionPlan,
    index: u64,
) -> Result<&crate::domain::BulkSubmissionItem, CanonicalError> {
    plan.items()
        .get(usize::try_from(index).map_err(|_| domain_out_of_range(index))?)
        .ok_or_else(|| {
            contract_symptom(
                "index",
                format!("no local plan item for source position {index}"),
            )
        })
}

fn domain_out_of_range(index: u64) -> CanonicalError {
    contract_symptom(
        "index",
        format!("source position {index} does not fit usize"),
    )
}

/// A canonical terminal-success bulk child analysis. Identity derives from
/// the local plan (source position is stable); input is the trusted local
/// descriptor; provenance carries the worker's task identity and version.
fn build_terminal_success_analysis(
    plan_item: &crate::domain::BulkSubmissionItem,
    index: u64,
    task_id: Option<&UpstreamTaskId>,
    task: Box<super::normalize::NormalizedTask>,
    now: UtcTimestamp,
) -> Result<Analysis<CanonicalError>, CanonicalError> {
    let input = trusted_text_input(plan_item);
    let state = CheckState::Succeeded {
        upstream: Some(UpstreamIdentity {
            task_id: task_id.cloned(),
            last_stage: NonEmptyString::new(task.last_stage.clone()).ok(),
        }),
        result: task.result,
    };
    let checks = OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: Some(task.version.clone()),
        upstream_task_ids: task_id
            .map(|id| crate::domain::UpstreamTaskIds::new(vec![id.clone()]).expect("one ID")),
        upstream_bulk_id: None,
        submitted_at: Some(now),
        completed_at: Some(now),
    };
    let _ = index;
    Analysis::new(
        AnalysisId::new(),
        SubmissionOutcome::Terminal,
        AnalysisInput::Text(input),
        checks,
        SaveState::Ephemeral,
        provenance,
        None,
        None,
        now,
        now,
        Some(now),
    )
    .map_err(|error| {
        CanonicalError::new(ErrorCode::UpstreamContractChanged, error.to_string())
            .expect("static template")
    })
}

/// A canonical terminal-failure bulk child analysis carrying the sanitized
/// upstream error.
fn build_terminal_failed_analysis(
    plan_item: &crate::domain::BulkSubmissionItem,
    _index: u64,
    outcome: &NormalizedBulkItemOutcome,
) -> Result<Analysis<CanonicalError>, CanonicalError> {
    let input = trusted_text_input(plan_item);
    let error = outcome_error(outcome);
    let state = CheckState::Failed {
        upstream: Some(UpstreamIdentity {
            task_id: outcome.task_id.clone(),
            last_stage: None,
        }),
        error,
    };
    let checks = OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let now = UtcTimestamp::now();
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: None,
        upstream_task_ids: outcome
            .task_id
            .as_ref()
            .map(|id| crate::domain::UpstreamTaskIds::new(vec![id.clone()]).expect("one ID")),
        upstream_bulk_id: None,
        submitted_at: Some(now),
        completed_at: Some(now),
    };
    Analysis::new(
        AnalysisId::new(),
        SubmissionOutcome::Terminal,
        AnalysisInput::Text(input),
        checks,
        SaveState::Ephemeral,
        provenance,
        None,
        None,
        now,
        now,
        Some(now),
    )
    .map_err(|error| {
        CanonicalError::new(ErrorCode::UpstreamContractChanged, error.to_string())
            .expect("static template")
    })
}

/// The canonical input descriptor for one item, derived ONLY from the
/// validated local plan: origin, SHA-256, byte/word counts. Upstream result
/// text is never reparsed as the item's input (contracts section 9.1).
fn trusted_text_input(plan_item: &crate::domain::BulkSubmissionItem) -> TextInput {
    let text = plan_item.text();
    let byte_count = u64::try_from(text.len()).unwrap_or(u64::MAX);
    TextInput::new(
        TextOrigin::Literal,
        None,
        Sha256Hash::digest(text.as_bytes()),
        byte_count,
        plan_item.word_count(),
        None,
    )
    .expect("a local text input descriptor is total")
}

fn outcome_error(outcome: &NormalizedBulkItemOutcome) -> CanonicalError {
    let message = outcome
        .error
        .as_deref()
        .unwrap_or("Pangram rejected the item.");
    let reduced = super::normalize::sanitize_upstream_message(message);
    let mut details = std::collections::BTreeMap::new();
    details.insert(
        "upstream_message".to_owned(),
        serde_json::Value::from(reduced),
    );
    CanonicalError::new(
        ErrorCode::UpstreamAnalysisFailed,
        "Pangram could not analyze the submitted text.",
    )
    .and_then(|error| error.with_contextual_retryability(false))
    .and_then(|error| error.with_details(details))
    .expect("static template")
}

fn validate_page_request(offset: u64, limit: u64) -> Result<(), CanonicalError> {
    // A malformed caller page request is a local usage error raised before
    // any network read. The caller supplies its own operation identity, so
    // this returns the bare canonical error.
    if !(1..=BULK_FETCH_PAGE_LIMIT).contains(&limit) {
        let mut details = std::collections::BTreeMap::new();
        details.insert("field".to_owned(), serde_json::Value::from("limit"));
        details.insert("limit".to_owned(), serde_json::Value::from(limit));
        let error = CanonicalError::new(
            ErrorCode::InputRequired,
            "the bulk page limit must be in 1..=1000.",
        )
        .and_then(|error| error.with_details(details))
        .expect("static template");
        return Err(error);
    }
    let _ = offset;
    Ok(())
}

fn next_offset(_offset: u64, items: &[BulkItem<CanonicalError>], total: u64) -> Option<u64> {
    let max = items.iter().map(|item| item.index).max()?;
    let next = max + 1;
    if next >= total { None } else { Some(next) }
}

fn clone_item(item: &BulkItem<CanonicalError>) -> BulkItem<CanonicalError> {
    item.clone()
}

fn page_poll_error<C: Clock>(running: &RunningBulk<C>, error: PollError) -> BulkAnalysisError {
    match error {
        PollError::Failed(error) => BulkAnalysisError::new(running.bulk_id, *error),
        PollError::Cancelled => BulkAnalysisError::new(
            running.bulk_id,
            CanonicalError::new(
                ErrorCode::NetworkUnavailable,
                "The bulk page read was cancelled locally; no remote action was taken.",
            )
            .expect("static template"),
        ),
        PollError::DeadlineExceeded => {
            BulkAnalysisError::new(running.bulk_id, running_wait_timeout(running))
        }
    }
}

fn running_wait_timeout<C: Clock>(running: &RunningBulk<C>) -> CanonicalError {
    let mut details = std::collections::BTreeMap::new();
    details.insert(
        "bulk_id".to_owned(),
        serde_json::Value::from(running.bulk_id.to_string()),
    );
    details.insert(
        "upstream_bulk_id".to_owned(),
        serde_json::Value::from(running.upstream_bulk_id.as_str()),
    );
    CanonicalError::new(
        ErrorCode::WaitTimeout,
        "Pangram did not finish the bulk job before the local wait timeout.",
    )
    .and_then(|error| error.with_details(details))
    .expect("static template")
}

fn contract_symptom(field: &'static str, token: impl Into<String>) -> CanonicalError {
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

fn bulk_domain_error(bulk_id: BulkId, error: crate::domain::DomainError) -> BulkAnalysisError {
    let mut details = std::collections::BTreeMap::new();
    details.insert(
        "conflict".to_owned(),
        serde_json::Value::from(error.to_string()),
    );
    BulkAnalysisError::new(
        bulk_id,
        CanonicalError::new(
            ErrorCode::UpstreamContractChanged,
            "Pangram returned a bulk document outside the pinned contract.",
        )
        .and_then(|error| error.with_details(details))
        .expect("static template"),
    )
}
