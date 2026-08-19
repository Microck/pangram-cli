//! Billable history rerun flow, including shared SIGINT interruption.

use crate::domain::Analysis;
use crate::history::{HistoryErrorCode, HistoryStore};
use crate::output::{CanonicalError, ErrorCode, ResolvedCommand};

use super::super::StreamTty;
use super::super::detect::{self, DetectOutcome, ResolvedOutput};
use super::Request;

enum RerunFlow {
    Completed(Analysis<CanonicalError>),
    Failed(CanonicalError),
    Interrupted(CanonicalError, String),
}

pub(super) fn execute(
    request: Request,
    store: Option<HistoryStore>,
    service: &crate::config::ConfigService,
    output: ResolvedOutput,
    started: crate::domain::UtcTimestamp,
    streams: &dyn StreamTty,
    analyzer_source: &crate::analysis::AnalyzerSource,
) -> DetectOutcome {
    let original_id = request.id.expect("rerun requires ID");
    let original = match store {
        None => return failed(output, started, super::unresolvable()),
        Some(store) => match store.canonical_analysis(&original_id, true) {
            Ok(analysis) => analysis,
            Err(error) if error.code() == HistoryErrorCode::NotFound => {
                return failed(output, started, super::unresolvable());
            }
            Err(error) => return failed(output, started, error.into_canonical()),
        },
    };
    // The shared analysis seam resolves every retained-input eligibility and
    // integrity rule before credential lookup or network work. Keep the text
    // only because automatic history persistence needs the submitted content
    // after the analyzer consumes the private request.
    let (analysis_request, mode) =
        match crate::analysis::AnalysisRequest::from_saved_rerun(&original) {
            Some(prepared) => prepared,
            None => return failed(output, started, super::unresolvable()),
        };
    let text = analysis_request.text().to_owned();

    let analyzer = match analyzer_source.resolve(service) {
        Ok(analyzer) => analyzer,
        Err(error) => return failed(output, started, error),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            return failed(
                output,
                started,
                detect::internal_error("could not start the local async runtime"),
            );
        }
    };
    let progress = detect::resolve_progress(request.progress, output, streams);
    let stop = crate::analysis::StopObserving::new();
    detect::install_sigint_driver();
    let result = runtime.block_on(async {
        let bridge = tokio::spawn(detect::bridge_sigint(stop.token().clone()));
        let outcome = match mode {
            crate::analysis::TextAnalysisMode::Detection => {
                run_detection_rerun(&analyzer, analysis_request, stop, progress).await
            }
            crate::analysis::TextAnalysisMode::Plagiarism => {
                let retained = analysis_request.clone();
                match analyzer.plagiarism(analysis_request, stop.token()).await {
                    Ok(analysis) => RerunFlow::Completed(analysis),
                    Err(error) if stop.token().is_cancelled() => RerunFlow::Interrupted(
                        error.into_error(),
                        "interrupted during plagiarism submission".to_owned(),
                    ),
                    Err(error)
                        if matches!(error.error().code(), ErrorCode::SubmissionOutcomeUnknown) =>
                    {
                        RerunFlow::Completed(crate::cli::phase7::failed_plagiarism_member(
                            &retained,
                            error.into_error(),
                        ))
                    }
                    Err(error) => RerunFlow::Failed(error.into_error()),
                }
            }
            crate::analysis::TextAnalysisMode::Combined => match analyzer
                .analyze_combined(
                    analysis_request,
                    crate::analysis::WaitOptions::UNBOUNDED,
                    |observation| {
                        if let crate::analysis::CombinedAnalysisObservation::Progress(event) =
                            observation
                        {
                            emit_progress(progress, event);
                        }
                    },
                    stop,
                )
                .await
            {
                Ok(Ok(analysis)) => RerunFlow::Completed(analysis),
                Ok(Err(error)) => RerunFlow::Failed(error.into_error()),
                Err(interrupted) => RerunFlow::Interrupted(
                    stopped_observation_error(),
                    detect::identity_note(&interrupted.identity),
                ),
            },
        };
        bridge.abort();
        outcome
    });
    detect::reset_sigint_flag();

    let analysis = match result {
        RerunFlow::Completed(analysis) => analysis,
        RerunFlow::Failed(error) => return failed(output, started, error),
        RerunFlow::Interrupted(error, note) => {
            return detect::interrupted_outcome(
                ResolvedCommand::HistoryRerun,
                output,
                started,
                error,
                note,
            );
        }
    };
    let retained_input = crate::history::RetainedInput::Text(text);
    let (mut analyses, _) = detect::save::persist_analyses(
        vec![analysis],
        std::slice::from_ref(&retained_input),
        detect::SaveStoreGate::Automatic,
        service,
    );
    detect::analysis_command_outcome(
        ResolvedCommand::HistoryRerun,
        output,
        started,
        analyses.pop().expect("one rerun analysis"),
    )
}

async fn run_detection_rerun(
    analyzer: &crate::analysis::Analyzer,
    analysis_request: crate::analysis::AnalysisRequest,
    stop: crate::analysis::StopObserving,
    progress: detect::ProgressMode,
) -> RerunFlow {
    match analyzer.start_full(analysis_request, stop.token()).await {
        Ok(crate::analysis::Accepted::Terminal(analysis)) => RerunFlow::Completed(*analysis),
        Ok(crate::analysis::Accepted::Task(accepted)) => {
            let running = analyzer.running(accepted);
            match running
                .observe(
                    crate::analysis::WaitOptions::UNBOUNDED,
                    |event| emit_progress(progress, event),
                    stop,
                )
                .await
            {
                Ok(Ok(analysis)) => RerunFlow::Completed(analysis),
                Ok(Err(error)) => RerunFlow::Failed(error.into_error()),
                Err(interrupted) => RerunFlow::Interrupted(
                    stopped_observation_error(),
                    detect::identity_note(&interrupted.identity),
                ),
            }
        }
        Err(failure) => {
            let crate::analysis::SubmissionFailure {
                task_error,
                request,
            } = failure;
            let error = task_error.into_error();
            if stop.token().is_cancelled() {
                RerunFlow::Interrupted(
                    error,
                    "interrupted; reconcile using the canonical error identity".to_owned(),
                )
            } else if matches!(error.code(), ErrorCode::SubmissionOutcomeUnknown) {
                RerunFlow::Completed(detect::failed_member(
                    &request.expect("an ambiguous submission retains its request"),
                    error,
                ))
            } else {
                RerunFlow::Failed(error)
            }
        }
    }
}

fn failed(
    output: ResolvedOutput,
    started: crate::domain::UtcTimestamp,
    error: CanonicalError,
) -> DetectOutcome {
    detect::failure_outcome(ResolvedCommand::HistoryRerun, output, started, error)
}

fn stopped_observation_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::NetworkUnavailable,
        "observation was interrupted locally; no remote cancellation was sent",
    )
    .expect("static error")
}

fn emit_progress(progress: detect::ProgressMode, event: &crate::analysis::AnalysisProgress) {
    match progress {
        detect::ProgressMode::Jsonl => {
            let value = crate::output::ProgressEvent::analysis(
                event.analysis_id,
                crate::domain::CheckKind::AiDetection,
                crate::domain::CheckStatus::Running,
                crate::domain::UtcTimestamp::now(),
            )
            .with_upstream_stage(event.last_stage.as_str());
            if let Ok(value) = value
                && let Ok(line) = serde_json::to_string(&value)
            {
                eprintln!("{line}");
            }
        }
        detect::ProgressMode::Human => {
            eprintln!(
                "history rerun {}: running ({})",
                event.analysis_id,
                detect::sanitize_for_stderr(event.last_stage.as_str())
            );
        }
        detect::ProgressMode::Auto | detect::ProgressMode::Quiet => {}
    }
}
