//! Phase 7 CLI planning and execution for binary detection, plagiarism, and
//! combined text analysis.
//!
//! This adapter owns local source selection, conservative billing preflight,
//! persistence, and rendering only. Every Pangram request and normalization
//! remains in the shared analysis module.

// These cold local-input boundaries return the canonical error type directly.
// Boxing it only to satisfy an ABI-size heuristic would add allocation and
// unwrap noise without reducing work on a billable or repeated path.
#![allow(clippy::result_large_err)]

use crate::analysis::{
    AnalysisRequest, Analyzer, FileAnalysisRequest, FileFormat, FileUpload, StopObserving,
    TextAnalysisMode, WaitOptions,
};
use crate::domain::{
    Analysis, Check, CheckState, OrderedChecks, Provenance, Provider, SaveState, SubmissionOutcome,
    UtcTimestamp, text_billable_units,
};
use crate::output::{CanonicalError, ErrorCode, ResolvedCommand};

use super::StreamTty;
use super::detect::{
    self, DetectArgs, DetectOutcome, Flow, GlobalFlags, ProgressMode, ProgressSink, ResolvedOutput,
    Source,
};

pub(crate) const fn command(mode: TextAnalysisMode) -> ResolvedCommand {
    match mode {
        TextAnalysisMode::Detection => ResolvedCommand::Detect,
        TextAnalysisMode::Plagiarism => ResolvedCommand::Plagiarism,
        TextAnalysisMode::Combined => ResolvedCommand::Analyze,
    }
}

enum Input {
    Text(detect::inputs::ResolvedInput),
    Binary { upload: FileUpload, path: String },
}

pub(crate) struct Plan {
    mode: TextAnalysisMode,
    arguments: DetectArgs,
    output: ResolvedOutput,
    progress: ProgressMode,
    inputs: Vec<Input>,
    history_gate: detect::SaveStoreGate,
}

impl Plan {
    pub(crate) const fn resolved_output(&self) -> ResolvedOutput {
        self.output
    }
}

pub(crate) fn has_binary_file(source: &Source) -> bool {
    matches!(source, Source::Files(files) if files.iter().any(|path| file_format(path).is_some()))
}

pub(crate) fn plan(
    mode: TextAnalysisMode,
    source: Source,
    arguments: DetectArgs,
    global: &GlobalFlags,
    streams: &dyn StreamTty,
    stdin_text: Option<String>,
) -> Result<Plan, DetectOutcome> {
    let started_at = UtcTimestamp::now();
    let output = detect::resolve_output(&arguments, &source, global, streams);
    let inputs = resolve_inputs(mode, source, streams, stdin_text)
        .map_err(|error| detect::failure_outcome(command(mode), output, started_at, error))?;

    if inputs
        .iter()
        .any(|input| matches!(input, Input::Binary { .. }))
    {
        let invalid = if arguments.detach {
            Some("--detach is available only for UTF-8 text detection")
        } else if arguments.public_link {
            Some("--public-link is not supported for binary file detection")
        } else if arguments.max_billable_units.is_some() {
            Some(
                "--max-billable-units is unavailable for binary files because Pangram publishes no pre-submission estimator",
            )
        } else {
            None
        };
        if let Some(message) = invalid {
            return Err(detect::failure_outcome(
                command(mode),
                output,
                started_at,
                detect::usage_error(ErrorCode::UnsupportedInput, message),
            ));
        }
    }

    if let Some(ceiling) = arguments.max_billable_units {
        let estimated = inputs
            .iter()
            .map(|input| match input {
                Input::Text(input) => mode.billable_units(text_billable_units(input.word_count)),
                Input::Binary { .. } => 0,
            })
            .fold(0_u64, u64::saturating_add);
        if estimated > ceiling {
            return Err(detect::failure_outcome(
                command(mode),
                output,
                started_at,
                detect::usage_error(
                    ErrorCode::UnsupportedInput,
                    &format!(
                        "estimated {estimated} billable unit(s) exceeds --max-billable-units {ceiling}"
                    ),
                ),
            ));
        }
    }

    let history_gate = detect::save::resolve_gate(arguments.save);
    let progress = detect::resolve_progress(arguments.progress, output, streams);
    Ok(Plan {
        mode,
        arguments,
        output,
        progress,
        inputs,
        history_gate,
    })
}

fn resolve_inputs(
    mode: TextAnalysisMode,
    source: Source,
    streams: &dyn StreamTty,
    stdin_text: Option<String>,
) -> Result<Vec<Input>, CanonicalError> {
    let Source::Files(files) = source else {
        return detect::inputs::resolve_inputs(source, streams, stdin_text)
            .map(|inputs| inputs.into_iter().map(Input::Text).collect());
    };

    files
        .into_iter()
        .map(|path| {
            if let Some(format) = file_format(&path) {
                if mode != TextAnalysisMode::Detection {
                    return Err(detect::usage_error(
                        ErrorCode::UnsupportedInput,
                        "binary file plagiarism and combined analysis are not supported",
                    ));
                }
                read_binary(path, format)
            } else {
                detect::inputs::read_text_file(&path).map(Input::Text)
            }
        })
        .collect()
}

fn file_format(path: &str) -> Option<FileFormat> {
    let extension = std::path::Path::new(path).extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("pdf") {
        Some(FileFormat::Pdf)
    } else if extension.eq_ignore_ascii_case("docx") {
        Some(FileFormat::Docx)
    } else if extension.eq_ignore_ascii_case("rtf") {
        Some(FileFormat::Rtf)
    } else {
        None
    }
}

fn read_binary(path: String, format: FileFormat) -> Result<Input, CanonicalError> {
    let bytes = std::fs::read(&path).map_err(|error| {
        detect::usage_error(
            ErrorCode::InputRequired,
            &format!("cannot read {path}: {}", crate::cli::redact_io(&error)),
        )
    })?;
    let filename = std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            detect::usage_error(
                ErrorCode::UnsupportedInput,
                "binary file names must be valid UTF-8 basenames",
            )
        })?;
    let upload = FileUpload::new(filename, format, bytes).ok_or_else(|| {
        detect::usage_error(
            ErrorCode::UnsupportedInput,
            "binary files must be non-empty and have a valid basename",
        )
    })?;
    Ok(Input::Binary { upload, path })
}

pub(crate) fn execute(
    plan: &Plan,
    analyzer: Analyzer,
    service: &crate::config::ConfigService,
    streams: &dyn StreamTty,
) -> DetectOutcome {
    let started_at = UtcTimestamp::now();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            return detect::failure_outcome(
                command(plan.mode),
                plan.output,
                started_at,
                detect::internal_error("could not start the local async runtime"),
            );
        }
    };

    let stop = StopObserving::new();
    detect::install_sigint_driver();
    let (members, retained_inputs, terminal) = runtime.block_on(async {
        let bridge = tokio::spawn(detect::bridge_sigint(stop.token().clone()));
        let mut members = Vec::with_capacity(plan.inputs.len());
        let mut retained_inputs = Vec::with_capacity(plan.inputs.len());
        let mut terminal = None;
        for input in &plan.inputs {
            match execute_one(plan, &analyzer, input, &stop, streams).await {
                Ok((analysis, retained)) => {
                    members.push(analysis);
                    retained_inputs.push(retained);
                }
                Err(flow) => {
                    terminal = Some(flow);
                    break;
                }
            }
        }
        bridge.abort();
        (members, retained_inputs, terminal)
    });
    detect::reset_sigint_flag();
    finish(
        plan,
        service,
        started_at,
        members,
        retained_inputs,
        terminal,
    )
}

async fn execute_one(
    plan: &Plan,
    analyzer: &Analyzer,
    input: &Input,
    stop: &StopObserving,
    streams: &dyn StreamTty,
) -> Result<(Analysis<CanonicalError>, crate::history::RetainedInput), Flow> {
    match input {
        Input::Binary { upload, path } => {
            let request = FileAnalysisRequest::new(
                upload.clone(),
                Some(path.clone()),
                plan.arguments.include_input,
            );
            let retained_request = request.clone();
            let wait = plan
                .arguments
                .timeout
                .map_or(WaitOptions::UNBOUNDED, WaitOptions::with_timeout);
            match analyzer
                .detect_file_retained(request, wait, stop.token())
                .await
            {
                Ok((analysis, extracted_text)) => Ok((
                    analysis,
                    crate::history::RetainedInput::File {
                        path: path.clone(),
                        extracted_text: Some(extracted_text),
                    },
                )),
                Err(error) if stop.token().is_cancelled() => Err(Flow::Interrupted(
                    error.into_error(),
                    "interrupted during binary file submission".to_owned(),
                )),
                Err(error)
                    if matches!(error.error().code(), ErrorCode::SubmissionOutcomeUnknown) =>
                {
                    Ok((
                        failed_file_member(&retained_request, error.into_error()),
                        crate::history::RetainedInput::File {
                            path: path.clone(),
                            extracted_text: None,
                        },
                    ))
                }
                Err(error) => Err(Flow::Failed(error.into_error())),
            }
        }
        Input::Text(input) => {
            let retained = crate::history::RetainedInput::Text(input.text.clone());
            match plan.mode {
                TextAnalysisMode::Detection => detect::analyze_one(
                    analyzer,
                    &plan.arguments,
                    input,
                    stop,
                    plan.progress,
                    streams,
                )
                .await
                .map(|analysis| (analysis, retained)),
                TextAnalysisMode::Plagiarism => {
                    let request = text_request(plan, input, false);
                    let wait = plan
                        .arguments
                        .timeout
                        .map_or(WaitOptions::UNBOUNDED, WaitOptions::with_timeout);
                    match analyzer
                        .plagiarism(request.clone(), wait, stop.token())
                        .await
                    {
                        Ok(analysis) => Ok((analysis, retained)),
                        Err(error) if stop.token().is_cancelled() => Err(Flow::Interrupted(
                            error.into_error(),
                            "interrupted during plagiarism submission".to_owned(),
                        )),
                        Err(error)
                            if matches!(
                                error.error().code(),
                                ErrorCode::SubmissionOutcomeUnknown
                            ) =>
                        {
                            Ok((
                                failed_plagiarism_member(&request, error.into_error()),
                                retained,
                            ))
                        }
                        Err(error) => Err(Flow::Failed(error.into_error())),
                    }
                }
                TextAnalysisMode::Combined => {
                    let request = text_request(plan, input, plan.arguments.public_link);
                    let wait = plan
                        .arguments
                        .timeout
                        .map(WaitOptions::with_timeout)
                        .unwrap_or(WaitOptions::UNBOUNDED);
                    let progress = ProgressSink::new(plan.progress, request.id());
                    match analyzer
                        .analyze_combined(
                            request,
                            wait,
                            |observation| {
                                if let crate::analysis::CombinedAnalysisObservation::Progress(
                                    event,
                                ) = observation
                                {
                                    progress.on_progress(event);
                                }
                            },
                            stop.clone(),
                        )
                        .await
                    {
                        Ok(Ok(analysis)) => Ok((analysis, retained)),
                        Ok(Err(error)) if stop.token().is_cancelled() => Err(Flow::Interrupted(
                            error.into_error(),
                            detect::AMBIGUOUS_INTERRUPTION_NOTE.to_owned(),
                        )),
                        Ok(Err(error)) => Err(Flow::Failed(error.into_error())),
                        Err(interrupted) => Err(Flow::Interrupted(
                            detect::internal_error("combined analysis was interrupted locally"),
                            detect::identity_note(&interrupted.identity),
                        )),
                    }
                }
            }
        }
    }
}

fn text_request(
    plan: &Plan,
    input: &detect::inputs::ResolvedInput,
    public_link: bool,
) -> AnalysisRequest {
    AnalysisRequest::new(
        input.text.clone(),
        input.origin,
        input.name.clone(),
        input.word_count,
        plan.arguments.include_input,
        public_link,
    )
}

fn finish(
    plan: &Plan,
    service: &crate::config::ConfigService,
    started_at: UtcTimestamp,
    members: Vec<Analysis<CanonicalError>>,
    retained_inputs: Vec<crate::history::RetainedInput>,
    terminal: Option<Flow>,
) -> DetectOutcome {
    let command = command(plan.mode);
    if members.is_empty() {
        return match terminal.expect("an empty run has a terminal flow") {
            Flow::Failed(error) => detect::failure_outcome(command, plan.output, started_at, error),
            Flow::Interrupted(error, note) => {
                detect::interrupted_outcome(command, plan.output, started_at, error, note)
            }
        };
    }

    let (members, save_failure) =
        detect::save::persist_analyses(members, &retained_inputs, plan.history_gate, service);
    let mut outcome = detect::success_outcome_for(command, plan.output, started_at, members);
    let required_save_exit = save_failure.and_then(|error| {
        outcome.attach_failure(command, plan.output, started_at, error);
        outcome.primary_ok.then_some(outcome.exit_code)
    });
    if let Some(flow) = terminal {
        match flow {
            Flow::Failed(error) => {
                outcome.attach_failure(command, plan.output, started_at, error);
                if outcome.primary_ok
                    && let Some(exit_code) = required_save_exit
                {
                    outcome.exit_code = exit_code;
                }
            }
            Flow::Interrupted(error, note) => {
                detect::note_stderr_raw(&note);
                outcome.attach_failure(command, plan.output, started_at, error);
                if outcome.primary_ok {
                    outcome.exit_code = crate::output::ExitCode::Interrupted.as_u8();
                }
            }
        }
    }
    outcome
}

fn failed_file_member(
    request: &FileAnalysisRequest,
    error: CanonicalError,
) -> Analysis<CanonicalError> {
    failed_member(
        request.id(),
        request.input(None),
        Check::AiDetection(CheckState::Failed {
            upstream: None,
            error,
        }),
    )
}

pub(crate) fn failed_plagiarism_member(
    request: &AnalysisRequest,
    error: CanonicalError,
) -> Analysis<CanonicalError> {
    failed_member(
        request.id(),
        request.input(),
        Check::Plagiarism(CheckState::Failed {
            upstream: None,
            error,
        }),
    )
}

fn failed_member(
    id: crate::domain::AnalysisId,
    input: crate::domain::AnalysisInput,
    check: Check<CanonicalError>,
) -> Analysis<CanonicalError> {
    let unknown = match &check {
        Check::AiDetection(CheckState::Failed { error, .. })
        | Check::Plagiarism(CheckState::Failed { error, .. }) => {
            matches!(error.code(), ErrorCode::SubmissionOutcomeUnknown)
        }
        _ => false,
    };
    let now = UtcTimestamp::now();
    Analysis::new(
        id,
        if unknown {
            SubmissionOutcome::AcceptanceUnknown
        } else {
            SubmissionOutcome::Terminal
        },
        input,
        OrderedChecks::new([check]).expect("one check is valid"),
        SaveState::Ephemeral,
        Provenance {
            provider: Provider::Pangram,
            upstream_version: None,
            upstream_task_ids: None,
            upstream_bulk_id: None,
            submitted_at: None,
            completed_at: None,
        },
        None,
        None,
        now,
        now,
        None,
    )
    .expect("a failed synchronous member satisfies canonical invariants")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_extensions_are_case_insensitive_and_closed() {
        assert_eq!(file_format("a.PDF"), Some(FileFormat::Pdf));
        assert_eq!(file_format("a.DocX"), Some(FileFormat::Docx));
        assert_eq!(file_format("a.rTf"), Some(FileFormat::Rtf));
        assert_eq!(file_format("a.doc"), None);
        assert_eq!(file_format("pdf"), None);
    }
}
