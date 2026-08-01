//! Rendered-outcome assembly for detection: canonical envelopes, exit-code
//! mapping, and the single projection handoff. Every success or failure path
//! funnels through one outcome builder so the process layer sees exactly one
//! rendering policy per invocation. No protocol or parsing happens here; the
//! adapter feeds already-normalized analyses and errors in, and this module
//! turns them into printable/exit-able outcomes.
//!
//! `CanonicalError` is the 224-byte adapter-facing object used end to end, so
//! this module inherits the plan-level `result_large_err` rationale.

use std::io::Write;

use crate::output::{self};
use crate::output::{
    AnalysisOutput, CanonicalError, CommandData, CommandEnvelope, EnvelopeMeta, ErrorCode,
    ExitCode, Recovery, ResolvedCommand,
};

use super::ErrorSurface;
use super::ResolvedOutput;

/// A note that always goes to stderr for a non-fatal lifecycle event. The
/// only current one is `--detach`. Kept adjacent to render so the note copy
/// lives with the only place that emits it.
pub(super) const DETACH_NOTE: &str =
    "detached without waiting; the upstream task continues on Pangram";

/// One executed detect command before the process renders it. `None` means
/// output was already streamed (a consumed projection) or cannot honestly be
/// rendered; the process layer exits 1 in the unrenderable case.
pub(crate) struct DetectOutcome {
    pub(crate) exit_code: u8,
    pub(crate) envelopes: Vec<CommandEnvelope>,
    /// A single-envelope machine or human projection already emitted by the
    /// dispatch; the process layer must not re-print.
    pub(crate) rendered: bool,
}

pub(super) fn success_outcome(
    output: ResolvedOutput,
    started_at: crate::domain::UtcTimestamp,
    analyses: Vec<crate::domain::Analysis<CanonicalError>>,
) -> DetectOutcome {
    let duration_ms = elapsed_ms(started_at);
    let meta = EnvelopeMeta::default()
        .with_started_at(started_at)
        .with_duration_ms(duration_ms);

    // Repeated streamed work (JSONL) emits one canonical envelope per
    // analysis; every other case wraps the series through AnalysisOutput
    // (one, or an ordered array inside `data`). A raw multi-analysis series
    // is only ever produced for textual `--file` input in Phase 2, so the
    // streaming default is JSONL with no other format path to guard.
    match analyses.len() {
        1 => {
            let analysis = analyses.into_iter().next().expect("one analysis");
            let envelope =
                CommandEnvelope::success(CommandData::Detect(AnalysisOutput::one(analysis)), meta);
            let exit_code = output_exit_code(&envelope).as_u8();
            emit_primary(
                std::slice::from_ref(&envelope),
                output,
                exit_code,
                started_at,
            )
        }
        _ => {
            // The repeated-run exit is the single precedence owned by
            // `run_exit_code` (contracts.md 4.1 + 12), shared with
            // `output_exit_code` so both the JSONL and single-document paths
            // agree exactly.
            let exit_code = run_exit_code(&analyses).as_u8();
            // The ordered-series rule (contracts.md 3.1): JSONL streams one
            // ordered envelope per analyzed file; every single-document format
            // (JSON, TOON, Markdown, pretty) wraps the whole series in one
            // success envelope whose `data` is the ordered analysis array, so
            // an explicit format never performs billable work and then fails
            // rendering.
            match output.format {
                crate::output::OutputFormat::Jsonl => {
                    let envelopes: Vec<_> = analyses
                        .into_iter()
                        .map(|analysis| {
                            CommandEnvelope::success(
                                CommandData::Detect(AnalysisOutput::one(analysis)),
                                meta.clone(),
                            )
                        })
                        .collect();
                    emit_primary(&envelopes, output, exit_code, started_at)
                }
                _ => {
                    let data = match AnalysisOutput::from_analyses(analyses) {
                        Ok(data) => data,
                        Err(_) => {
                            return internal_outcome(output, started_at);
                        }
                    };
                    let envelope = CommandEnvelope::success(CommandData::Detect(data), meta);
                    emit_primary(
                        std::slice::from_ref(&envelope),
                        output,
                        exit_code,
                        started_at,
                    )
                }
            }
        }
    }
}

/// The process exit for a success envelope derives from its canonical
/// content exactly as the contract maps it (contracts.md 12): a partial
/// result exits 3; a failed analysis exits per its terminal check error's
/// category (an upstream `STAGE_FAILED` is `upstream_analysis_failed`, exit
/// 6); an accepted (`--detach`) run stays 0.
fn output_exit_code(envelope: &CommandEnvelope) -> ExitCode {
    match envelope.data() {
        Some(CommandData::Detect(AnalysisOutput::One(analysis))) => analysis_exit_code(analysis),
        Some(CommandData::Detect(AnalysisOutput::Many(analyses))) => {
            run_exit_code(analyses.as_slice())
        }
        _ => ExitCode::Success,
    }
}

/// The single owner of the repeated-run exit precedence (contracts.md 4.1 +
/// 12). The run's parent status derives across members exactly like one
/// analysis derives across checks: a `partial` parent (mixed member outcomes,
/// or an individually `partial` member) exits 3; a run whose members are all
/// `failed` exits per the first failed member's check-error category (the
/// identical category mapping in both envelope positions), so an all-failed
/// upstream STAGE_FAILED run exits 6; otherwise the run exits 0.
fn run_exit_code(analyses: &[crate::domain::Analysis<CanonicalError>]) -> ExitCode {
    use crate::domain::AnalysisStatus as S;
    let any_partial = analyses.iter().any(|a| matches!(a.status(), S::Partial));
    let any_succeeded = analyses.iter().any(|a| matches!(a.status(), S::Succeeded));
    let any_failed = analyses.iter().any(|a| matches!(a.status(), S::Failed));
    if any_partial || (any_succeeded && any_failed) {
        ExitCode::Partial
    } else if any_failed {
        analyses
            .iter()
            .find(|a| matches!(a.status(), S::Failed))
            .map_or(ExitCode::GeneralFailure, analysis_exit_code)
    } else {
        ExitCode::Success
    }
}

/// One analysis: a terminal failure exits per its check error's category
/// (locked for the upstream `STAGE_FAILED` case at exit 6); every other
/// status exits 0. Shared with the task adapter so `task status`/`task wait`
/// derive the identical category-mapped exit (contracts.md 12).
pub(crate) fn analysis_exit_code(analysis: &crate::domain::Analysis<CanonicalError>) -> ExitCode {
    match analysis.status() {
        crate::domain::AnalysisStatus::Failed => analysis
            .checks()
            .iter()
            .find_map(|check| match check {
                crate::domain::Check::AiDetection(crate::domain::CheckState::Failed {
                    error,
                    ..
                })
                | crate::domain::Check::Plagiarism(crate::domain::CheckState::Failed {
                    error,
                    ..
                }) => Some(ExitCode::for_error(error.category())),
                _ => None,
            })
            .unwrap_or(ExitCode::GeneralFailure),
        status => ExitCode::for_status(status),
    }
}

/// Builds a failure outcome before input resolution exists: the rendering
/// policy is resolved from the global flags alone, defaulting a pre-flag
/// failure to the contract's machine-JSON default.
pub(crate) fn early_failure(
    command: ResolvedCommand,
    global: super::GlobalFlags,
    streams: &dyn crate::cli::StreamTty,
    started_at: crate::domain::UtcTimestamp,
    error: CanonicalError,
) -> DetectOutcome {
    // With no `--format` resolved yet, the contract's noninteractive default
    // applies: JSON primary output with JSON errors unless the caller passed
    // an explicit pretty selection (unavailable this early) or
    // `--error-format text`.
    let color = super::color_policy(&global, crate::output::OutputFormat::Json, streams);
    let surface = if global.error_format_text == Some(true) {
        ErrorSurface::Text
    } else {
        ErrorSurface::Json
    };
    let output = ResolvedOutput {
        format: crate::output::OutputFormat::Json,
        color,
        error: surface,
    };
    failure_outcome(command, output, started_at, error)
}

pub(crate) fn failure_outcome(
    command: ResolvedCommand,
    output: ResolvedOutput,
    started_at: crate::domain::UtcTimestamp,
    error: CanonicalError,
) -> DetectOutcome {
    let exit_code = ExitCode::for_error(error.category()).as_u8();
    let envelope = CommandEnvelope::failure(
        command,
        error,
        EnvelopeMeta::default()
            .with_started_at(started_at)
            .with_failed_at(crate::domain::UtcTimestamp::now()),
    );
    match output.error {
        ErrorSurface::Json => DetectOutcome {
            exit_code,
            envelopes: vec![envelope],
            rendered: false,
        },
        ErrorSurface::Text => {
            let rendered = emit_error_text(&envelope);
            DetectOutcome {
                exit_code: if rendered { exit_code } else { 1 },
                envelopes: vec![],
                rendered: true,
            }
        }
    }
}

pub(crate) fn interrupted_outcome(
    command: ResolvedCommand,
    output: ResolvedOutput,
    started_at: crate::domain::UtcTimestamp,
    error: CanonicalError,
    note: String,
) -> DetectOutcome {
    note_stderr_raw(&note);
    let envelope = CommandEnvelope::failure(
        command,
        error,
        EnvelopeMeta::default()
            .with_started_at(started_at)
            .with_failed_at(crate::domain::UtcTimestamp::now()),
    );
    match output.error {
        ErrorSurface::Json => DetectOutcome {
            exit_code: ExitCode::Interrupted.as_u8(),
            envelopes: vec![envelope],
            rendered: false,
        },
        ErrorSurface::Text => {
            let rendered = emit_error_text(&envelope);
            DetectOutcome {
                exit_code: if rendered {
                    ExitCode::Interrupted.as_u8()
                } else {
                    1
                },
                envelopes: vec![],
                rendered: true,
            }
        }
    }
}

/// Renders a single success envelope as the primary output through the one
/// projection owner. Shared by detection, bulk, and task commands so every
/// projection boundary stays identical.
pub(crate) fn primary_outcome(
    envelope: &CommandEnvelope,
    output: ResolvedOutput,
    exit_code: u8,
    started_at: crate::domain::UtcTimestamp,
) -> DetectOutcome {
    emit_primary(
        std::slice::from_ref(envelope),
        output,
        exit_code,
        started_at,
    )
}

/// Renders the primary envelope to stdout through the single projection
/// owner. Machine formats (JSON/JSONL/TOON) go to stdout; Markdown and pretty
/// also go to stdout because they are the requested primary output. A write
/// or flush failure degrades the exit to 1 rather than reporting success.
fn emit_primary(
    envelopes: &[CommandEnvelope],
    output: ResolvedOutput,
    exit_code: u8,
    started_at: crate::domain::UtcTimestamp,
) -> DetectOutcome {
    let mut stdout = std::io::stdout().lock();
    let result = output::render(output.format, output.color, envelopes, &mut stdout)
        .and_then(|()| stdout.flush());
    match result {
        Ok(()) => DetectOutcome {
            exit_code,
            envelopes: vec![],
            rendered: true,
        },
        Err(_) => internal_outcome(output, started_at),
    }
}

/// Renders a failure envelope as one sanitized text message on stderr.
/// Returns false when the write fails so the caller degrades to exit 1.
fn emit_error_text(envelope: &CommandEnvelope) -> bool {
    let Some(error) = envelope.error() else {
        return false;
    };
    let mut stderr = std::io::stderr().lock();
    let mut ok = writeln!(stderr, "error: {}", sanitize_for_stderr(error.message())).is_ok();
    if ok {
        if let Some(recovery) = error.recovery() {
            ok = writeln!(stderr, "help: {}", sanitize_for_stderr(recovery.message())).is_ok();
            if ok {
                if let Some(command) = recovery.command() {
                    ok = writeln!(stderr, "  try: {}", sanitize_for_stderr(command)).is_ok();
                }
            }
        }
    }
    ok && stderr.flush().is_ok()
}

/// Strips control characters from a message before it reaches stderr. One
/// owner: this delegates to the projection-owned terminal sanitizer so both
/// write boundaries strip exactly the same scalars and can never diverge.
pub(crate) fn sanitize_for_stderr(text: &str) -> String {
    crate::output::sanitize_terminal(text)
}

/// An advisory diagnostic note always goes to stderr, TTY or not, per the
/// shell contract; it carries no content and no ANSI.
pub(crate) fn note_stderr(_streams: &dyn crate::cli::StreamTty, note: &str) {
    note_stderr_raw(note);
}

pub(crate) fn note_stderr_raw(note: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "note: {note}");
    let _ = stderr.flush();
}

/// The identity tuple printed on interruption so the caller can reconcile
/// without a hidden task ledger. Carries only IDs and the last stage. The
/// upstream-supplied task id and stage token are untrusted, so each is
/// sanitized for the terminal at the write boundary (control sequences
/// stripped) before it is assembled into the note.
pub(crate) fn identity_note(identity: &crate::analysis::OperationIdentity) -> String {
    let mut note = format!("interrupted; local analysis id {}", identity.analysis_id);
    if let Some(task_id) = &identity.task_id {
        note.push_str(&format!(
            "; upstream task id {}",
            sanitize_for_stderr(task_id.as_str())
        ));
    }
    if let Some(stage) = &identity.last_stage {
        note.push_str(&format!(
            "; last stage {}",
            sanitize_for_stderr(stage.as_str())
        ));
    }
    note
}

fn internal_outcome(
    output: ResolvedOutput,
    started_at: crate::domain::UtcTimestamp,
) -> DetectOutcome {
    failure_outcome(
        ResolvedCommand::Detect,
        output,
        started_at,
        internal_error("the result could not be rendered honestly"),
    )
}

pub(crate) fn internal_error(message: &str) -> CanonicalError {
    CanonicalError::new(ErrorCode::UpstreamError, message)
        .and_then(|error| error.with_contextual_retryability(false))
        .expect("the internal-error template is statically valid")
}

pub(crate) fn usage_error(code: ErrorCode, message: &str) -> CanonicalError {
    CanonicalError::new(code, message).expect("usage messages are non-empty")
}

/// The missing-credential failure with its canonical recovery guidance.
pub(crate) fn missing_api_key_error() -> CanonicalError {
    let recovery = Recovery::new("Configure a persistent key or set PANGRAM_API_KEY.")
        .and_then(|recovery| recovery.with_command("pangram auth"))
        .expect("fixed recovery text is non-empty");
    CanonicalError::new(
        ErrorCode::MissingApiKey,
        "No Pangram API key is configured.",
    )
    .and_then(|error| error.with_recovery(recovery))
    .expect("recovery is valid for missing_api_key")
}

/// Milliseconds between `started_at` and now, for canonical `duration_ms`.
/// A clock skew clamps at zero rather than wrapping.
pub(crate) fn elapsed_ms(started: crate::domain::UtcTimestamp) -> u64 {
    let now = crate::domain::UtcTimestamp::now().get();
    match now.duration_since(started.get()) {
        duration if duration.is_negative() => 0,
        duration => u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
    }
}
