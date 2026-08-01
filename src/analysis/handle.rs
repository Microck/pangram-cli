//! Submission, observation, timeout, and cancellation orchestration.
//!
//! `Analyzer::start` submits exactly once and returns a running handle (or,
//! when the provider answered synchronously, the terminal analysis). A POST
//! whose outcome is ambiguous is never replayed; the caller receives the
//! fixed `submission_outcome_unknown` failure with the local identity,
//! request hash, and last observed state.
//!
//! `RunningAnalysis::observe` reuses one private poll step until a terminal
//! state, the local wait timeout, or local cancellation. A wait timeout
//! exits through the canonical `wait_timeout` and carries the local ID,
//! upstream task ID, and last observed stage. Cancellation yields
//! [`InterruptedAnalysis`] with the same identity; no remote cancellation
//! request is ever sent.

use std::fmt;

use tokio_util::sync::CancellationToken;

use crate::domain::{
    Analysis, AnalysisId, Check, CheckState, LocalOperationId, NonEmptyString, OrderedChecks,
    Provenance, Provider, SaveState, SubmissionOutcome, SubmissionOutcomeUnknownDetails,
    UpstreamIdentity, UpstreamTaskId, UpstreamTaskIds, UtcTimestamp,
};
use crate::output::{CanonicalError, ErrorCode};

use super::WaitOptions;
use super::bulk::{
    BulkAnalysisError, BulkAnalysisRequest, BulkPageResult, BulkProgress, RunningBulk,
};
use super::normalize::{self, NormalizedTask, TaskState};
use super::task::{Accepted, AcceptedInput, AnalysisRequest, TaskError};
use super::upstream::{PollError, SubmitOutcome, TaskPoll, UpstreamClient};
use crate::domain::{BulkId, UpstreamBulkId};

/// The identity tuple reported on wait timeouts and interruptions. It is
/// the payload the final adapter output prints so the caller can reconcile
/// without a hidden task ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationIdentity {
    pub analysis_id: AnalysisId,
    pub task_id: Option<UpstreamTaskId>,
    pub last_stage: Option<NonEmptyString>,
}

/// The progress snapshot emitted on each non-terminal observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisProgress {
    pub analysis_id: AnalysisId,
    pub task_id: UpstreamTaskId,
    /// The provider's current stage token, preserved without interpretation.
    pub last_stage: NonEmptyString,
}

/// A token adapters pass to stop observing. Cancelling it aborts the local
/// wait at the next boundary; it cannot affect the upstream task.
#[derive(Debug, Clone, Default)]
pub struct StopObserving(CancellationToken);

impl StopObserving {
    #[must_use]
    pub fn new() -> Self {
        Self(CancellationToken::new())
    }

    pub fn stop(&self) {
        self.0.cancel();
    }

    #[must_use]
    pub const fn token(&self) -> &CancellationToken {
        &self.0
    }
}

/// A locally stopped observation. `Interrupted` always maps to exit 130.
#[derive(Debug)]
pub struct InterruptedAnalysis {
    pub identity: OperationIdentity,
}

/// The running operation returned by [`Analyzer::running`]. Its identity
/// exists before submission so every failure path can report it.
#[derive(Clone)]
pub struct RunningAnalysis<C = super::config::SystemClock> {
    client: UpstreamClient<C>,
    request: AnalysisRequest,
    task_id: UpstreamTaskId,
    accepted_at: UtcTimestamp,
    last_stage: Option<NonEmptyString>,
    created_at: UtcTimestamp,
}

impl<C: super::config::Clock> fmt::Debug for RunningAnalysis<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunningAnalysis")
            .field("analysis_id", &self.request.id())
            .field("task_id", &self.task_id)
            .field("accepted_at", &self.accepted_at)
            .field("last_stage", &self.last_stage)
            .finish_non_exhaustive()
    }
}

impl<C: super::config::Clock> RunningAnalysis<C> {
    #[must_use]
    pub const fn analysis_id(&self) -> AnalysisId {
        self.request.id()
    }

    #[must_use]
    pub const fn task_id(&self) -> &UpstreamTaskId {
        &self.task_id
    }

    #[must_use]
    pub fn last_stage(&self) -> Option<&NonEmptyString> {
        self.last_stage.as_ref()
    }

    #[must_use]
    pub fn identity(&self) -> OperationIdentity {
        OperationIdentity {
            analysis_id: self.request.id(),
            task_id: Some(self.task_id.clone()),
            last_stage: self.last_stage.clone(),
        }
    }

    /// The canonical input descriptor computed from the original request.
    #[must_use]
    pub fn input(&self) -> crate::domain::AnalysisInput {
        self.request.input()
    }

    fn provenance(&self, completed_at: Option<UtcTimestamp>) -> Provenance {
        let ids = UpstreamTaskIds::new(vec![self.task_id.clone()])
            .expect("one validated task ID cannot duplicate");
        Provenance {
            provider: Provider::Pangram,
            upstream_version: None,
            upstream_task_ids: Some(ids),
            upstream_bulk_id: None,
            submitted_at: Some(self.accepted_at),
            completed_at,
        }
    }

    fn identity_upstream(&self) -> UpstreamIdentity {
        UpstreamIdentity {
            task_id: Some(self.task_id.clone()),
            last_stage: self.last_stage.clone(),
        }
    }

    fn snapshot_analysis(&self, running: bool) -> Analysis<CanonicalError> {
        let upstream = running.then(|| self.identity_upstream());
        let check: CheckState<crate::domain::AiDetectionResult, CanonicalError> = if running {
            CheckState::Running { upstream }
        } else {
            CheckState::Queued { upstream: None }
        };
        let checks = OrderedChecks::new([Check::AiDetection(check)]).expect("one check is valid");
        let now = UtcTimestamp::now();
        Analysis::new(
            self.request.id(),
            SubmissionOutcome::Accepted,
            self.request.input(),
            checks,
            SaveState::Ephemeral,
            self.provenance(None),
            None,
            None,
            self.created_at,
            now,
            None,
        )
        .expect("a running analysis snapshot always satisfies invariants")
    }

    /// The current non-terminal canonical analysis. An accepted task that no
    /// poll has observed yet is reported `queued` with a `None` check-level
    /// upstream stage (no stage exists to report); the real upstream task id
    /// is already and honestly preserved in `provenance.upstream_task_ids`,
    /// so no identity is fabricated or lost. Once a stage is observed the
    /// snapshot becomes `running` with that stage.
    #[must_use]
    pub fn snapshot(&self) -> Analysis<CanonicalError> {
        self.snapshot_analysis(self.last_stage.is_some())
    }

    fn finish_success(&self, task: NormalizedTask) -> Analysis<CanonicalError> {
        let now = UtcTimestamp::now();
        let state = CheckState::Succeeded {
            upstream: Some(UpstreamIdentity {
                task_id: Some(self.task_id.clone()),
                last_stage: NonEmptyString::new(task.last_stage.clone()).ok(),
            }),
            result: task.result,
        };
        let checks = OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
        let mut provenance = self.provenance(Some(now));
        provenance.upstream_version = Some(task.version);
        Analysis::new(
            self.request.id(),
            SubmissionOutcome::Terminal,
            self.request.input(),
            checks,
            SaveState::Ephemeral,
            provenance,
            None,
            None,
            self.created_at,
            now,
            Some(now),
        )
        .expect("a terminal success always satisfies invariants")
    }

    fn finish_failed(&self, error: CanonicalError) -> Analysis<CanonicalError> {
        let now = UtcTimestamp::now();
        let state: CheckState<crate::domain::AiDetectionResult, CanonicalError> =
            CheckState::Failed {
                upstream: Some(self.identity_upstream()),
                error,
            };
        let checks = OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
        Analysis::new(
            self.request.id(),
            SubmissionOutcome::Terminal,
            self.request.input(),
            checks,
            SaveState::Ephemeral,
            self.provenance(Some(now)),
            None,
            None,
            self.created_at,
            now,
            Some(now),
        )
        .expect("a terminal failure always satisfies invariants")
    }

    /// Polls until terminal, the local timeout, or local cancellation.
    /// Progress is reported through `on_progress` after each non-terminal
    /// observation. Cancel-safe: dropping the future leaves no orphans.
    pub async fn observe(
        mut self,
        options: WaitOptions,
        mut on_progress: impl FnMut(&AnalysisProgress),
        stop: StopObserving,
    ) -> Result<Result<Analysis<CanonicalError>, TaskError>, InterruptedAnalysis> {
        let cancel = stop.token().child_token();
        let clock = self.client.config().clock();
        let deadline = options.timeout.map(|timeout| clock.now() + timeout);

        loop {
            if cancel.is_cancelled() {
                return Err(InterruptedAnalysis {
                    identity: self.identity(),
                });
            }
            if let Some(deadline) = deadline {
                if clock.now() >= deadline {
                    return Ok(Err(TaskError::new(
                        self.request.id(),
                        wait_timeout_error(&self.identity()),
                    )));
                }
            }

            match self
                .client
                .poll_task(&self.task_id, &cancel, deadline)
                .await
            {
                Ok(TaskPoll::InProgress {
                    last_stage: raw_stage,
                    ..
                }) => {
                    let token = match NonEmptyString::new(raw_stage) {
                        Ok(token) => token,
                        Err(_) => {
                            return Ok(Err(TaskError::new(
                                self.request.id(),
                                contract_symptom("stage", "empty"),
                            )));
                        }
                    };
                    self.last_stage = Some(token.clone());
                    on_progress(&AnalysisProgress {
                        analysis_id: self.request.id(),
                        task_id: self.task_id.clone(),
                        last_stage: token,
                    });
                }
                Ok(TaskPoll::Terminal(response)) => {
                    let body = match response.json_value() {
                        Ok(body) => body,
                        Err(error) => {
                            return Ok(Err(TaskError::new(
                                self.request.id(),
                                contract_symptom("body", error.to_string()),
                            )));
                        }
                    };
                    return Ok(match normalize::normalize_task_state(&body) {
                        Ok(TaskState::Success(task)) => Ok(self.finish_success(*task)),
                        Ok(TaskState::Failed { message, stage }) => {
                            self.last_stage = NonEmptyString::new(stage).ok();
                            Ok(self.finish_failed(analysis_failed_error(&message)))
                        }
                        Ok(TaskState::InProgress { .. }) => {
                            // A terminal body must normalize to Success or
                            // Failed; an in-progress token here means the
                            // upstream stage surface drifted from the pinned
                            // contract, not a genuine in-progress state.
                            Err(TaskError::new(
                                self.request.id(),
                                contract_symptom("stage", "terminal-classified-in-progress"),
                            ))
                        }
                        Err(error) => Err(TaskError::new(self.request.id(), error)),
                    });
                }
                Ok(TaskPoll::NotFound) => {
                    return Ok(Err(TaskError::new(
                        self.request.id(),
                        CanonicalError::new(
                            ErrorCode::UpstreamNotFound,
                            "Pangram does not recognize the task.",
                        )
                        .expect("static template"),
                    )));
                }
                Ok(TaskPoll::AnalysisFailed { message, stage }) => {
                    self.last_stage = NonEmptyString::new(stage).ok();
                    return Ok(Ok(self.finish_failed(analysis_failed_error(&message))));
                }
                Err(PollError::Cancelled) => {
                    return Err(InterruptedAnalysis {
                        identity: self.identity(),
                    });
                }
                // A paced wait or retry sleep crossed the caller's wait
                // budget: surface the canonical wait timeout with identity.
                Err(PollError::DeadlineExceeded) => {
                    return Ok(Err(TaskError::new(
                        self.request.id(),
                        wait_timeout_error(&self.identity()),
                    )));
                }
                Err(PollError::Failed(error)) => {
                    return Ok(Err(TaskError::new(self.request.id(), *error)));
                }
            }

            let interval = self.client.config().polling().effective_interval();
            let wake = {
                let natural = clock.now() + interval;
                deadline.map_or(natural, |deadline| natural.min(deadline))
            };
            if !clock.sleep_until(wake, &cancel).await {
                return Err(InterruptedAnalysis {
                    identity: self.identity(),
                });
            }
        }
    }
}

/// Owns construction of running operations. Clones share the connection
/// pool and the time-based pacing gate. This is the single adapter-facing
/// analysis owner: text and bulk surfaces both enter through it over one
/// shared pacemaker/HTTP stack, so CLI/TUI/MCP never own a second top-level
/// protocol client.
#[derive(Clone)]
pub struct Analyzer<C = super::config::SystemClock> {
    client: UpstreamClient<C>,
}

impl<C: super::config::Clock> Analyzer<C> {
    #[must_use]
    pub fn from_client(client: UpstreamClient<C>) -> Self {
        Self { client }
    }

    /// The internal bulk owner sharing this analyzer's client. `pub(crate)`
    /// so the facade methods (and only they) build it; adapters never
    /// construct a `BulkAnalyzer` directly.
    pub(crate) fn bulk(&self) -> super::bulk::BulkAnalyzer<C> {
        super::bulk::BulkAnalyzer::from_client(self.client.clone())
    }

    /// Submits one validated bulk plan exactly once and returns its running
    /// handle, preserving the same pacing, billing, and ambiguity rules as
    /// the text surface. This is the adapter-facing bulk submit entry.
    pub async fn submit_bulk(
        &self,
        request: BulkAnalysisRequest,
        cancel: &CancellationToken,
    ) -> Result<RunningBulk<C>, BulkAnalysisError> {
        self.bulk().submit_bulk(request, cancel).await
    }

    /// Rehydrates a running handle for an already-accepted job (a
    /// `bulk_status`-style read of a job submitted earlier). The caller's
    /// validated plan keeps per-item input descriptors trusted-local.
    #[must_use]
    pub fn resume_bulk(
        &self,
        bulk_id: BulkId,
        upstream_bulk_id: UpstreamBulkId,
        plan: crate::domain::BulkSubmissionPlan,
    ) -> RunningBulk<C> {
        self.bulk().resume(bulk_id, upstream_bulk_id, plan)
    }

    /// Fetches one validated typed items-metadata page through this
    /// analyzer's shared safe-GET chain.
    pub async fn bulk_items_page(
        &self,
        running: &RunningBulk<C>,
        offset: u64,
        limit: u64,
        cancel: &CancellationToken,
    ) -> Result<BulkPageResult, BulkAnalysisError> {
        self.bulk()
            .bulk_items_page(running, offset, limit, cancel)
            .await
    }

    /// Fetches one validated typed results page through this analyzer's
    /// shared safe-GET chain.
    pub async fn bulk_results_page(
        &self,
        running: &RunningBulk<C>,
        offset: u64,
        limit: u64,
        cancel: &CancellationToken,
    ) -> Result<BulkPageResult, BulkAnalysisError> {
        self.bulk()
            .bulk_results_page(running, offset, limit, cancel)
            .await
    }

    /// Iterates documented results pages until the set is exhausted, over
    /// this analyzer's shared safe-GET chain and the bounded fetch-all page
    /// size.
    pub async fn bulk_results_all(
        &self,
        running: &RunningBulk<C>,
        max_reads: u64,
        cancel: &CancellationToken,
        on_progress: impl FnMut(&BulkProgress),
    ) -> Result<BulkPageResult, BulkAnalysisError> {
        self.bulk()
            .bulk_results_all(running, max_reads, cancel, on_progress)
            .await
    }

    /// Submits one text-analysis request exactly once.
    ///
    /// Outcome rules:
    /// - acceptance returns `Accepted::Task` (the running input)
    /// - a provably unreached send returns the retryable network error
    /// - an ambiguous send returns `submission_outcome_unknown`
    /// - local cancellation yields the canonical interrupted message
    pub async fn start(
        &self,
        request: AnalysisRequest,
        cancel: &CancellationToken,
    ) -> Result<Accepted, TaskError> {
        match self.start_full(request, cancel).await {
            Ok(accepted) => Ok(accepted),
            Err(super::upstream::SubmissionFailure { task_error, .. }) => Err(task_error),
        }
    }

    /// Submits one text-analysis request exactly once, preserving the full
    /// [`AnalysisRequest`] on failure so an adapter can reconcile or render a
    /// failed series member with the original identity and input. Success is
    /// identical to [`Analyzer::start`]; a completed submission that failed to
    /// reach an acceptance carries the request through.
    pub async fn start_full(
        &self,
        request: AnalysisRequest,
        cancel: &CancellationToken,
    ) -> Result<Accepted, super::upstream::SubmissionFailure> {
        let body = request.submit_body();
        match self.client.submit_text(&body, cancel).await {
            Ok(accepted) => Ok(Accepted::Task(AcceptedInput {
                task_id: accepted.task_id,
                request,
            })),
            Err(SubmitOutcome::Failed(error)) => Err(super::upstream::SubmissionFailure {
                task_error: TaskError::new(request.id(), *error),
                request: Some(request),
            }),
            Err(SubmitOutcome::Cancelled) => Err(super::upstream::SubmissionFailure {
                task_error: TaskError::new(request.id(), cancelled_error()),
                request: Some(request),
            }),
            Err(SubmitOutcome::Ambiguous(error)) => Err(super::upstream::SubmissionFailure {
                task_error: TaskError::new(
                    request.id(),
                    submission_unknown_error(&request, &error),
                ),
                request: Some(request),
            }),
        }
    }

    /// Builds a running handle from an accepted input. Kept separate from
    /// `start` so task-style flows can rebuild observation state from a
    /// known identity in a later phase.
    #[must_use]
    pub fn running(&self, accepted: AcceptedInput) -> RunningAnalysis<C> {
        RunningAnalysis {
            client: self.client.clone(),
            request: accepted.request,
            task_id: accepted.task_id,
            accepted_at: UtcTimestamp::now(),
            last_stage: None,
            created_at: UtcTimestamp::now(),
        }
    }
}

fn contract_symptom(field: &'static str, token: impl Into<String>) -> CanonicalError {
    let mut details = std::collections::BTreeMap::new();
    details.insert("field".to_owned(), serde_json::Value::from(field));
    details.insert("token".to_owned(), serde_json::Value::from(token.into()));
    CanonicalError::new(
        ErrorCode::UpstreamContractChanged,
        "Pangram returned a document outside the pinned Pangram 4 contract.",
    )
    .and_then(|error| error.with_details(details))
    .expect("static template")
}

fn analysis_failed_error(message: &str) -> CanonicalError {
    let mut details = std::collections::BTreeMap::new();
    details.insert(
        "upstream_message".to_owned(),
        serde_json::Value::from(message),
    );
    CanonicalError::new(
        ErrorCode::UpstreamAnalysisFailed,
        "Pangram could not analyze the submitted text.",
    )
    .and_then(|error| error.with_contextual_retryability(false))
    .and_then(|error| error.with_details(details))
    .expect("static template")
}

fn cancelled_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::NetworkUnavailable,
        "The submission was cancelled locally before an upstream acceptance; no remote action was taken.",
    )
    .expect("static template")
}

fn wait_timeout_error(identity: &OperationIdentity) -> CanonicalError {
    let mut details = std::collections::BTreeMap::new();
    details.insert(
        "analysis_id".to_owned(),
        serde_json::Value::from(identity.analysis_id.to_string()),
    );
    if let Some(task_id) = &identity.task_id {
        details.insert(
            "upstream_task_id".to_owned(),
            serde_json::Value::from(task_id.as_str()),
        );
    }
    if let Some(stage) = &identity.last_stage {
        details.insert(
            "last_stage".to_owned(),
            serde_json::Value::from(stage.as_str()),
        );
    }
    CanonicalError::new(
        ErrorCode::WaitTimeout,
        "Pangram did not finish the task before the local wait timeout.",
    )
    .and_then(|error| error.with_details(details))
    .expect("static template")
}

fn submission_unknown_error(
    request: &AnalysisRequest,
    _error: &super::upstream::AnalysisError,
) -> CanonicalError {
    let details = SubmissionOutcomeUnknownDetails::new(
        LocalOperationId::AnalysisId(request.id()),
        request.request_sha256(),
        None,
        None,
        NonEmptyString::new("task creation unacknowledged".to_owned())
            .expect("fixed label is non-empty"),
    );
    CanonicalError::submission_outcome_unknown(
        "The submission may have reached Pangram, but no acceptance was obtained.",
        details,
    )
    .expect("submission-unknown construction is statically valid")
}
