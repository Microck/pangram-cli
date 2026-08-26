//! Read-only observation of upstream-authored Pangram tasks.

use tokio_util::sync::CancellationToken;

use crate::domain::{
    Analysis, AnalysisId, Check, CheckState, NonEmptyString, OrderedChecks, Provenance, Provider,
    SaveState, SubmissionOutcome, UpstreamIdentity, UpstreamTaskId, UpstreamTaskIds, UtcTimestamp,
};
use crate::output::{CanonicalError, ErrorCode};

use super::super::WaitOptions;
use super::super::normalize::{self, NormalizedTask, TaskState};
use super::super::task::TaskError;
use super::super::upstream::{PollError, TaskPoll};
use super::{
    AnalysisProgress, Analyzer, OperationIdentity, StopObserving, analysis_failed_error,
    cancelled_error, contract_symptom, wait_timeout_error,
};

impl<C: super::super::config::Clock> Analyzer<C> {
    /// Reads one observed task snapshot by upstream ID (`task status`).
    ///
    /// This reconciles a remote record the caller did not author
    /// (contracts.md 4.6): the canonical analysis carries the observed
    /// upstream identity and terminal state, marks `submission_outcome:
    /// accepted`, omits `provenance.submitted_at`, and derives the input
    /// descriptor only from the terminal document Pangram attested (or
    /// omits it until one exists). Progress events are not emitted on this
    /// one-shot read.
    pub async fn task_status(
        &self,
        task_id: &UpstreamTaskId,
        cancel: &CancellationToken,
    ) -> Result<Analysis<CanonicalError>, TaskError> {
        let local_id = AnalysisId::new();
        match self.client.poll_task(task_id, cancel, None).await {
            Ok(TaskPoll::InProgress { last_stage, .. }) => {
                Ok(observed_running_task(local_id, task_id, &last_stage))
            }
            Ok(TaskPoll::Terminal(response)) => {
                let body = response.json_value().map_err(|error| {
                    TaskError::new(local_id, contract_symptom("body", error.to_string()))
                })?;
                match normalize::normalize_task_state(&body) {
                    Ok(TaskState::Success(task)) => {
                        Ok(observed_terminal_task_success(local_id, task_id, *task))
                    }
                    Ok(TaskState::Failed { message, stage }) => Ok(observed_terminal_task_failed(
                        local_id, task_id, &stage, &message,
                    )),
                    // A terminal response classifying in-progress means the
                    // upstream stage surface drifted (see observe).
                    Ok(TaskState::InProgress { .. }) => Err(TaskError::new(
                        local_id,
                        contract_symptom("stage", "terminal-classified-in-progress"),
                    )),
                    Err(error) => Err(TaskError::new(local_id, error)),
                }
            }
            Ok(TaskPoll::NotFound) => Err(TaskError::new(
                local_id,
                CanonicalError::new(
                    ErrorCode::UpstreamNotFound,
                    "Pangram does not recognize the task.",
                )
                .expect("static template"),
            )),
            Ok(TaskPoll::AnalysisFailed { message, stage }) => Ok(observed_terminal_task_failed(
                local_id, task_id, &stage, &message,
            )),
            Err(PollError::Cancelled) => Err(TaskError::new(local_id, cancelled_error())),
            Err(PollError::DeadlineExceeded) => {
                unreachable!("task status passes no caller deadline")
            }
            Err(PollError::Failed(error)) => Err(TaskError::new(local_id, *error)),
        }
    }

    /// Observes an upstream task until terminal or the caller's local wait
    /// budget expires (`task wait`). Progress events flow through
    /// `on_progress` only when the caller selected progress. Cancellation
    /// stops the local observation only; no remote cancellation is sent.
    pub async fn task_wait(
        &self,
        task_id: &UpstreamTaskId,
        options: WaitOptions,
        mut on_progress: impl FnMut(&AnalysisProgress),
        stop: StopObserving,
        cancel: &CancellationToken,
    ) -> Result<Analysis<CanonicalError>, TaskError> {
        let local_id = AnalysisId::new();
        let clock = self.client.config().clock();
        let deadline = options.timeout.map(|timeout| clock.now() + timeout);
        // Track the last observed upstream stage so a local wait timeout
        // preserves it for reconciliation, matching the shared running
        // observation path (Greptile P2).
        let mut observed_stage: Option<NonEmptyString> = None;
        loop {
            if cancel.is_cancelled() || stop.token().is_cancelled() {
                return Err(TaskError::new(local_id, cancelled_error()));
            }
            if let Some(deadline) = deadline
                && clock.now() >= deadline
            {
                return Err(TaskError::new(
                    local_id,
                    wait_timeout_error(&OperationIdentity {
                        analysis_id: local_id,
                        task_id: Some(task_id.clone()),
                        last_stage: observed_stage.clone(),
                    }),
                ));
            }

            match self.client.poll_task(task_id, cancel, deadline).await {
                Ok(TaskPoll::InProgress { last_stage, .. }) => {
                    let stage = match NonEmptyString::new(last_stage.clone()) {
                        Ok(stage) => stage,
                        Err(_) => {
                            return Err(TaskError::new(
                                local_id,
                                contract_symptom("stage", "empty"),
                            ));
                        }
                    };
                    observed_stage = Some(stage.clone());
                    on_progress(&AnalysisProgress {
                        analysis_id: local_id,
                        task_id: task_id.clone(),
                        last_stage: stage,
                    });
                }
                Ok(TaskPoll::Terminal(response)) => {
                    let body = response.json_value().map_err(|error| {
                        TaskError::new(local_id, contract_symptom("body", error.to_string()))
                    })?;
                    return match normalize::normalize_task_state(&body) {
                        Ok(TaskState::Success(task)) => {
                            Ok(observed_terminal_task_success(local_id, task_id, *task))
                        }
                        Ok(TaskState::Failed { message, stage }) => Ok(
                            observed_terminal_task_failed(local_id, task_id, &stage, &message),
                        ),
                        Ok(TaskState::InProgress { .. }) => Err(TaskError::new(
                            local_id,
                            contract_symptom("stage", "terminal-classified-in-progress"),
                        )),
                        Err(error) => Err(TaskError::new(local_id, error)),
                    };
                }
                Ok(TaskPoll::NotFound) => {
                    return Err(TaskError::new(
                        local_id,
                        CanonicalError::new(
                            ErrorCode::UpstreamNotFound,
                            "Pangram does not recognize the task.",
                        )
                        .expect("static template"),
                    ));
                }
                Ok(TaskPoll::AnalysisFailed { message, stage }) => {
                    return Ok(observed_terminal_task_failed(
                        local_id, task_id, &stage, &message,
                    ));
                }
                Err(PollError::Cancelled) => {
                    return Err(TaskError::new(local_id, cancelled_error()));
                }
                Err(PollError::DeadlineExceeded) => {
                    return Err(TaskError::new(
                        local_id,
                        wait_timeout_error(&OperationIdentity {
                            analysis_id: local_id,
                            task_id: Some(task_id.clone()),
                            last_stage: observed_stage.clone(),
                        }),
                    ));
                }
                Err(PollError::Failed(error)) => return Err(TaskError::new(local_id, *error)),
            }

            let interval = self.client.config().polling().effective_interval();
            let wake = {
                let natural = clock.now() + interval;
                deadline.map_or(natural, |deadline| natural.min(deadline))
            };
            if !clock.sleep_until(wake, cancel).await {
                return Err(TaskError::new(local_id, cancelled_error()));
            }
        }
    }
}

/// Builds a running snapshot for a task the caller did not submit.
fn observed_running_task(
    local_id: AnalysisId,
    task_id: &UpstreamTaskId,
    last_stage: &str,
) -> Analysis<CanonicalError> {
    let state: CheckState<crate::domain::AiDetectionResult, CanonicalError> = CheckState::Running {
        upstream: Some(UpstreamIdentity {
            task_id: Some(task_id.clone()),
            last_stage: NonEmptyString::new(last_stage.to_owned()).ok(),
        }),
    };
    let checks = OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let now = UtcTimestamp::now();
    let ids = UpstreamTaskIds::new(vec![task_id.clone()]).expect("one validated task ID");
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: None,
        upstream_task_ids: Some(ids),
        upstream_bulk_id: None,
        submitted_at: None,
        completed_at: None,
    };
    Analysis::with_optional_input(
        local_id,
        SubmissionOutcome::Accepted,
        None,
        checks,
        SaveState::Ephemeral,
        provenance,
        None,
        None,
        now,
        now,
        None,
    )
    .expect("a running observed snapshot satisfies the analysis invariants")
}

/// Builds terminal success without claiming authorship of the task.
fn observed_terminal_task_success(
    local_id: AnalysisId,
    task_id: &UpstreamTaskId,
    task: NormalizedTask,
) -> Analysis<CanonicalError> {
    let last_stage = task.last_stage.clone();
    let now = UtcTimestamp::now();
    let input = task.normalized_text.as_deref().map(|text| {
        let byte_count = u64::try_from(text.len()).unwrap_or(u64::MAX);
        let word_count = super::super::canonical_text_word_count(text);
        crate::domain::AnalysisInput::Text(
            crate::domain::TextInput::new(
                crate::domain::TextOrigin::Unknown,
                None,
                crate::domain::Sha256Hash::digest(text.as_bytes()),
                byte_count,
                word_count,
                None,
            )
            .expect("attested terminal text always yields a valid descriptor"),
        )
    });
    let state: CheckState<crate::domain::AiDetectionResult, CanonicalError> =
        CheckState::Succeeded {
            upstream: Some(UpstreamIdentity {
                task_id: Some(task_id.clone()),
                last_stage: NonEmptyString::new(last_stage).ok(),
            }),
            result: task.result,
        };
    let checks = OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let ids = UpstreamTaskIds::new(vec![task_id.clone()]).expect("one validated task ID");
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: Some(task.version),
        upstream_task_ids: Some(ids),
        upstream_bulk_id: None,
        submitted_at: None,
        completed_at: Some(now),
    };
    Analysis::with_optional_input(
        local_id,
        SubmissionOutcome::Accepted,
        input,
        checks,
        SaveState::Ephemeral,
        provenance,
        None,
        None,
        now,
        now,
        Some(now),
    )
    .expect("a terminal observed snapshot satisfies the analysis invariants")
}

/// Builds terminal failure without claiming authorship of the task.
fn observed_terminal_task_failed(
    local_id: AnalysisId,
    task_id: &UpstreamTaskId,
    stage: &str,
    message: &str,
) -> Analysis<CanonicalError> {
    let now = UtcTimestamp::now();
    let state: CheckState<crate::domain::AiDetectionResult, CanonicalError> = CheckState::Failed {
        upstream: Some(UpstreamIdentity {
            task_id: Some(task_id.clone()),
            last_stage: NonEmptyString::new(stage.to_owned()).ok(),
        }),
        error: analysis_failed_error(message),
    };
    let checks = OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let ids = UpstreamTaskIds::new(vec![task_id.clone()]).expect("one validated task ID");
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: None,
        upstream_task_ids: Some(ids),
        upstream_bulk_id: None,
        submitted_at: None,
        completed_at: Some(now),
    };
    Analysis::with_optional_input(
        local_id,
        SubmissionOutcome::Accepted,
        None,
        checks,
        SaveState::Ephemeral,
        provenance,
        None,
        None,
        now,
        now,
        Some(now),
    )
    .expect("a terminal observed failure satisfies the analysis invariants")
}
