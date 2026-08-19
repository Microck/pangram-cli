//! Synchronous Pangram operations and combined-analysis orchestration.

use tokio_util::sync::CancellationToken;

use crate::domain::{
    Analysis, Check, CheckState, OrderedChecks, Provenance, Provider, SaveState, SubmissionOutcome,
    UtcTimestamp,
};
use crate::output::{CanonicalError, ErrorCode};

use super::super::WaitOptions;
use super::super::task::{Accepted, AnalysisRequest, FileAnalysisRequest, TaskError};
use super::super::upstream::SubmitOutcome;
use super::{
    Analyzer, CombinedAnalysisObservation, InterruptedAnalysis, OperationIdentity, StopObserving,
    cancelled_error, contract_symptom, submission_unknown_for,
};

impl<C: super::super::config::Clock> Analyzer<C> {
    /// Runs one synchronous binary file detection through the shared
    /// protocol owner and returns its canonical terminal analysis.
    pub async fn detect_file(
        &self,
        request: FileAnalysisRequest,
        cancel: &CancellationToken,
    ) -> Result<Analysis<CanonicalError>, TaskError> {
        self.detect_file_retained(request, cancel)
            .await
            .map(|(analysis, _)| analysis)
    }

    /// The CLI save seam needs the exact extracted text even when primary
    /// output omits it. This crate-private variant returns that retained
    /// plaintext beside the redacted canonical analysis; adapters must pass
    /// it only to the explicit or configured history path.
    pub(crate) async fn detect_file_retained(
        &self,
        request: FileAnalysisRequest,
        cancel: &CancellationToken,
    ) -> Result<(Analysis<CanonicalError>, String), TaskError> {
        let submitted_at = UtcTimestamp::now();
        let normalized = match self
            .client
            .submit_files(std::slice::from_ref(request.upload()), cancel)
            .await
        {
            Ok(mut results) => results
                .pop()
                .expect("one submitted file yields one normalized result"),
            Err(SubmitOutcome::Failed(error)) => {
                return Err(TaskError::new(request.id(), *error));
            }
            Err(SubmitOutcome::Cancelled) => {
                return Err(TaskError::new(request.id(), cancelled_error()));
            }
            Err(SubmitOutcome::Ambiguous(_)) => {
                return Err(TaskError::new(
                    request.id(),
                    submission_unknown_for(
                        request.id(),
                        request.request_sha256(),
                        "file result unacknowledged",
                    ),
                ));
            }
        };
        let completed_at = UtcTimestamp::now();
        let check = Check::AiDetection(CheckState::Succeeded {
            upstream: None,
            result: normalized.result,
        });
        let checks = OrderedChecks::new([check]).expect("one check is valid");
        let provenance = Provenance {
            provider: Provider::Pangram,
            upstream_version: Some(normalized.version),
            upstream_task_ids: None,
            upstream_bulk_id: None,
            submitted_at: Some(submitted_at),
            completed_at: Some(completed_at),
        };
        let retained_text = normalized.extracted_text;
        let output_text = request.include_input().then(|| retained_text.clone());
        Analysis::new(
            request.id(),
            SubmissionOutcome::Terminal,
            request.input(output_text),
            checks,
            SaveState::Ephemeral,
            provenance,
            None,
            None,
            submitted_at,
            completed_at,
            Some(completed_at),
        )
        .map(|analysis| (analysis, retained_text))
        .map_err(|_| TaskError::new(request.id(), contract_symptom("analysis", "invalid")))
    }

    /// Runs one synchronous plagiarism check and returns its canonical
    /// terminal analysis. The request body contains only the documented text key.
    pub async fn plagiarism(
        &self,
        request: AnalysisRequest,
        cancel: &CancellationToken,
    ) -> Result<Analysis<CanonicalError>, TaskError> {
        let submitted_at = UtcTimestamp::now();
        let result = match self.client.submit_plagiarism(request.text(), cancel).await {
            Ok(result) => result,
            Err(SubmitOutcome::Failed(error)) => {
                return Err(TaskError::new(request.id(), *error));
            }
            Err(SubmitOutcome::Cancelled) => {
                return Err(TaskError::new(request.id(), cancelled_error()));
            }
            Err(SubmitOutcome::Ambiguous(_)) => {
                return Err(TaskError::new(
                    request.id(),
                    submission_unknown_for(
                        request.id(),
                        request.plagiarism_request_sha256(),
                        "plagiarism result unacknowledged",
                    ),
                ));
            }
        };
        let completed_at = UtcTimestamp::now();
        let check = Check::Plagiarism(CheckState::Succeeded {
            upstream: None,
            result,
        });
        let checks = OrderedChecks::new([check]).expect("one check is valid");
        let provenance = Provenance {
            provider: Provider::Pangram,
            upstream_version: None,
            upstream_task_ids: None,
            upstream_bulk_id: None,
            submitted_at: Some(submitted_at),
            completed_at: Some(completed_at),
        };
        Analysis::new(
            request.id(),
            SubmissionOutcome::Terminal,
            request.input(),
            checks,
            SaveState::Ephemeral,
            provenance,
            None,
            request.rerun_of(),
            submitted_at,
            completed_at,
            Some(completed_at),
        )
        .map_err(|_| TaskError::new(request.id(), contract_symptom("analysis", "invalid")))
    }

    /// Runs AI detection and plagiarism concurrently, exactly once each, then
    /// assembles their results in canonical check order. A concluded failure
    /// becomes a failed check so either successful result remains available as
    /// partial success.
    pub async fn analyze_combined(
        &self,
        request: AnalysisRequest,
        options: WaitOptions,
        mut on_observation: impl FnMut(CombinedAnalysisObservation<'_, C>),
        stop: StopObserving,
    ) -> Result<Result<Analysis<CanonicalError>, TaskError>, InterruptedAnalysis> {
        let started_at = UtcTimestamp::now();
        let detection = async {
            match self.start_full(request.clone(), stop.token()).await {
                Ok(Accepted::Terminal(analysis)) => Ok(Ok(*analysis)),
                Ok(Accepted::Task(accepted)) => {
                    let running = self.running(accepted);
                    on_observation(CombinedAnalysisObservation::Accepted(&running));
                    running
                        .observe(
                            options,
                            |progress| {
                                on_observation(CombinedAnalysisObservation::Progress(progress));
                            },
                            stop.clone(),
                        )
                        .await
                }
                Err(failure) => Ok(Err(failure.task_error)),
            }
        };
        let plagiarism = self.plagiarism(request.clone(), stop.token());
        let (detection, plagiarism) = tokio::join!(detection, plagiarism);
        let detection = match detection {
            Ok(result) => result,
            Err(interrupted) => return Err(interrupted),
        };

        if stop.token().is_cancelled() {
            return Err(match &detection {
                Ok(analysis) => interrupted_after_detection(analysis),
                Err(_) => InterruptedAnalysis {
                    identity: OperationIdentity {
                        analysis_id: request.id(),
                        task_id: None,
                        last_stage: None,
                    },
                },
            });
        }

        let mut submission_outcome = SubmissionOutcome::Terminal;
        let (ai_check, mut provenance, created_at) = match detection {
            Ok(analysis) => (
                analysis
                    .checks()
                    .first()
                    .cloned()
                    .expect("a detection analysis owns its AI check"),
                analysis.provenance().clone(),
                analysis.created_at,
            ),
            Err(error) => {
                if matches!(error.error().code(), ErrorCode::SubmissionOutcomeUnknown) {
                    submission_outcome = SubmissionOutcome::AcceptanceUnknown;
                }
                (
                    Check::AiDetection(CheckState::Failed {
                        upstream: None,
                        error: error.into_error(),
                    }),
                    Provenance {
                        provider: Provider::Pangram,
                        upstream_version: None,
                        upstream_task_ids: None,
                        upstream_bulk_id: None,
                        submitted_at: None,
                        completed_at: None,
                    },
                    started_at,
                )
            }
        };
        let plagiarism_check = match plagiarism {
            Ok(analysis) => {
                if provenance.submitted_at.is_none() {
                    provenance = analysis.provenance().clone();
                }
                analysis
                    .checks()
                    .first()
                    .cloned()
                    .expect("a plagiarism analysis owns its plagiarism check")
            }
            Err(error) => {
                if matches!(error.error().code(), ErrorCode::SubmissionOutcomeUnknown) {
                    submission_outcome = SubmissionOutcome::AcceptanceUnknown;
                }
                Check::Plagiarism(CheckState::Failed {
                    upstream: None,
                    error: error.into_error(),
                })
            }
        };
        let checks = OrderedChecks::new([ai_check, plagiarism_check])
            .expect("AI detection followed by plagiarism is canonical order");
        let completed_at = UtcTimestamp::now();
        provenance.completed_at = Some(completed_at);
        Ok(Analysis::new(
            request.id(),
            submission_outcome,
            request.input(),
            checks,
            SaveState::Ephemeral,
            provenance,
            None,
            request.rerun_of(),
            created_at,
            completed_at,
            Some(completed_at),
        )
        .map_err(|_| TaskError::new(request.id(), contract_symptom("analysis", "invalid"))))
    }
}

fn interrupted_after_detection(analysis: &Analysis<CanonicalError>) -> InterruptedAnalysis {
    let task_id = analysis
        .provenance()
        .upstream_task_ids
        .as_ref()
        .and_then(|ids| ids.as_slice().first())
        .cloned();
    InterruptedAnalysis {
        identity: OperationIdentity {
            analysis_id: analysis.id,
            task_id,
            last_stage: None,
        },
    }
}
