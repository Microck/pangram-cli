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
    ExitCode, ResolvedCommand,
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
    /// Whether the primary output honestly completed: the primary envelope(s)
    /// (or its text error surface) rendered and flushed successfully. A
    /// primary render failure degrades this to `false` with exit 1, and the
    /// post-primary failure attachment reads it so it never overwrites the
    /// general render-failure exit with its own category-derived exit
    /// (contracts.md 14.2 note).
    pub(crate) primary_ok: bool,
}

/// A byte sink the render paths write through. The process wires
/// `std::io::StdoutLock`/`StderrLock` into this seam; tests wire a
/// deterministic faulting sink so a first-write-fails/second-write-succeeds
/// surface can never pass accidentally (a `/dev/full`-only proof could
/// silently bypass the real write). Production paths never hold this type;
/// only the terminal write boundary uses it.
pub(crate) trait RenderWrite {
    fn write_all_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()>;
    fn flush_bytes(&mut self) -> std::io::Result<()>;
}

impl<T: std::io::Write> RenderWrite for &mut T {
    fn write_all_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.write_all(bytes)
    }
    fn flush_bytes(&mut self) -> std::io::Result<()> {
        self.flush()
    }
}

// The concrete process stream locks implement the seam directly so a
// multi-iteration call can reborrow its sink without relying on blanket
// reborrow coercion.
impl RenderWrite for std::io::StdoutLock<'_> {
    fn write_all_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.write_all(bytes)
    }
    fn flush_bytes(&mut self) -> std::io::Result<()> {
        self.flush()
    }
}

impl RenderWrite for std::io::StderrLock<'_> {
    fn write_all_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.write_all(bytes)
    }
    fn flush_bytes(&mut self) -> std::io::Result<()> {
        self.flush()
    }
}

/// The sinks a terminal write boundary emits through. Production locks the
/// real process streams; tests inject deterministic faulting sinks.
pub(crate) struct RenderSinks<'a> {
    pub(crate) stdout: &'a mut dyn RenderWrite,
    pub(crate) stderr: &'a mut dyn RenderWrite,
}

impl DetectOutcome {
    /// Attaches a post-primary canonical failure (contracts.md 14.2 note: an
    /// explicit `--save` that failed after the remote result already
    /// rendered). The primary envelope is already fixed with the honest
    /// `ephemeral` save state; this replaces the process result with the
    /// failure envelope and its category-derived exit (7 for local history).
    ///
    /// A rendered (streamed) primary stays streamed: for machine formats the
    /// failure envelope is emitted on stdout after the already-printed
    /// primary line, so both halves stay machine-readable in order; for the
    /// text surface it prints on stderr.
    ///
    /// A render failure always wins. When the primary render already failed
    /// (`primary_ok == false`, already exit 1), the attachment can never
    /// overwrite that general render-failure exit with the category-derived
    /// exit 7: a command whose own output surface could not render has not
    /// honestly reported either failure. Equally, when the failure envelope
    /// itself cannot be written (a closed stdout or stderr), the process
    /// reports the render failure at exit 1 and never masks it behind the
    /// category-derived exit (7).
    pub(crate) fn attach_failure(
        &mut self,
        command: ResolvedCommand,
        output: ResolvedOutput,
        started_at: crate::domain::UtcTimestamp,
        error: CanonicalError,
    ) {
        let mut stdout = std::io::stdout().lock();
        let mut stderr = std::io::stderr().lock();
        let mut sinks = RenderSinks {
            stdout: &mut stdout,
            stderr: &mut stderr,
        };
        self.attach_failure_with(command, output, started_at, error, &mut sinks);
    }

    /// The sink-injectable core of [`Self::attach_failure`]. Split for the
    /// deterministic render-failure proof: a test wires a faulting sink and
    /// asserts the general render-failure exit 1 is preserved, while the
    /// second write genuinely succeeds.
    pub(crate) fn attach_failure_with(
        &mut self,
        command: ResolvedCommand,
        output: ResolvedOutput,
        started_at: crate::domain::UtcTimestamp,
        error: CanonicalError,
        sinks: &mut RenderSinks<'_>,
    ) {
        // Render precedence: a primary that already failed to render owns
        // the exit. The history failure can never upgrade or replace the
        // general render-failure exit 1 (contracts.md 14.2 note).
        if !self.primary_ok {
            self.exit_code = 1;
            return;
        }
        let envelope = CommandEnvelope::failure(
            command,
            error,
            EnvelopeMeta::default()
                .with_started_at(started_at)
                .with_failed_at(crate::domain::UtcTimestamp::now()),
        );
        let exit_code = envelope
            .error()
            .map_or(1, |error| ExitCode::for_error(error.category()).as_u8());
        match output.error {
            ErrorSurface::Json => {
                // A streamed primary was already printed: append the failure
                // envelope as its own JSON line so the sequence stays
                // machine-readable (success then failure, in emit order).
                // A queued (non-rendered) primary is dropped: its content is
                // fully reproducible from the already-persisted state or the
                // caller's rerun, and the failure envelope is the honest
                // final surface.
                if self.rendered {
                    let emitted = serde_json::to_string(&envelope)
                        .map_err(|_| ())
                        .and_then(|line| {
                            sinks
                                .stdout
                                .write_all_bytes(format!("{line}\n").as_bytes())
                                .map_err(|_| ())
                        })
                        .and_then(|()| sinks.stdout.flush_bytes().map_err(|_| ()));
                    if emitted.is_err() {
                        self.exit_code = 1;
                        self.primary_ok = false;
                        return;
                    }
                }
                self.exit_code = exit_code;
            }
            ErrorSurface::Text => {
                let rendered = emit_error_text_with(&envelope, sinks);
                self.exit_code = if rendered { exit_code } else { 1 };
                if !rendered {
                    self.primary_ok = false;
                }
            }
        }
    }
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
                ResolvedCommand::Detect,
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
                    emit_primary(
                        ResolvedCommand::Detect,
                        &envelopes,
                        output,
                        exit_code,
                        started_at,
                    )
                }
                _ => {
                    let data = match AnalysisOutput::from_analyses(analyses) {
                        Ok(data) => data,
                        Err(_) => {
                            return internal_outcome(ResolvedCommand::Detect, output, started_at);
                        }
                    };
                    let envelope = CommandEnvelope::success(CommandData::Detect(data), meta);
                    emit_primary(
                        ResolvedCommand::Detect,
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

/// Renders one canonical analysis for a non-detect command while preserving
/// the shared format, error, flush, and remote-outcome exit rules.
pub(crate) fn analysis_command_outcome(
    command: ResolvedCommand,
    output: ResolvedOutput,
    started_at: crate::domain::UtcTimestamp,
    analysis: crate::domain::Analysis<CanonicalError>,
) -> DetectOutcome {
    let data = match command {
        ResolvedCommand::HistoryRerun => CommandData::HistoryRerun(analysis),
        _ => return internal_outcome(command, output, started_at),
    };
    let envelope = CommandEnvelope::success(
        data,
        EnvelopeMeta::default()
            .with_started_at(started_at)
            .with_duration_ms(elapsed_ms(started_at)),
    );
    let exit_code = analysis_exit_code(match envelope.data() {
        Some(CommandData::HistoryRerun(analysis)) => analysis,
        _ => unreachable!("history rerun envelope"),
    })
    .as_u8();
    emit_primary(
        command,
        std::slice::from_ref(&envelope),
        output,
        exit_code,
        started_at,
    )
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
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let mut sinks = RenderSinks {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    failure_outcome_with(command, output, started_at, error, &mut sinks)
}

pub(crate) fn failure_outcome_with(
    command: ResolvedCommand,
    output: ResolvedOutput,
    started_at: crate::domain::UtcTimestamp,
    error: CanonicalError,
    sinks: &mut RenderSinks<'_>,
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
            primary_ok: true,
        },
        ErrorSurface::Text => {
            let rendered = emit_error_text_with(&envelope, sinks);
            DetectOutcome {
                exit_code: if rendered { exit_code } else { 1 },
                envelopes: vec![],
                rendered: true,
                primary_ok: rendered,
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
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let mut sinks = RenderSinks {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    interrupted_outcome_with(command, output, started_at, error, &mut sinks)
}

pub(crate) fn interrupted_outcome_with(
    command: ResolvedCommand,
    output: ResolvedOutput,
    started_at: crate::domain::UtcTimestamp,
    error: CanonicalError,
    sinks: &mut RenderSinks<'_>,
) -> DetectOutcome {
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
            primary_ok: true,
        },
        ErrorSurface::Text => {
            let rendered = emit_error_text_with(&envelope, sinks);
            DetectOutcome {
                exit_code: if rendered {
                    ExitCode::Interrupted.as_u8()
                } else {
                    1
                },
                envelopes: vec![],
                rendered: true,
                primary_ok: rendered,
            }
        }
    }
}

/// Renders a single success envelope as the primary output through the one
/// projection owner. Shared by detection, bulk, and task commands so every
/// projection boundary stays identical.
pub(crate) fn primary_outcome(
    command: ResolvedCommand,
    envelope: &CommandEnvelope,
    output: ResolvedOutput,
    exit_code: u8,
    started_at: crate::domain::UtcTimestamp,
) -> DetectOutcome {
    emit_primary(
        command,
        std::slice::from_ref(envelope),
        output,
        exit_code,
        started_at,
    )
}

/// Renders the primary envelope to stdout through the single projection
/// owner. Machine formats (JSON/JSONL/TOON) go to stdout; Markdown and pretty
/// also go to stdout because they are the requested primary output. A write
/// or flush failure degrades the exit to 1 and records `primary_ok = false`
/// so a later post-primary failure attachment can never overwrite the
/// general render-failure exit with a category-derived exit (contracts.md
/// 14.2 note).
fn emit_primary(
    command: ResolvedCommand,
    envelopes: &[CommandEnvelope],
    output: ResolvedOutput,
    exit_code: u8,
    started_at: crate::domain::UtcTimestamp,
) -> DetectOutcome {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let mut sinks = RenderSinks {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    emit_primary_with(
        command, envelopes, output, exit_code, started_at, &mut sinks,
    )
}

/// The sink-injectable core of [`emit_primary`], split for the deterministic
/// render-failure proof.
fn emit_primary_with(
    command: ResolvedCommand,
    envelopes: &[CommandEnvelope],
    output: ResolvedOutput,
    exit_code: u8,
    started_at: crate::domain::UtcTimestamp,
    sinks: &mut RenderSinks<'_>,
) -> DetectOutcome {
    let result = output::render(
        output.format,
        output.color,
        envelopes,
        &mut SinkAdapter(sinks),
    )
    .and_then(|()| sinks.stdout.flush_bytes());
    match result {
        Ok(()) => DetectOutcome {
            exit_code,
            envelopes: vec![],
            rendered: true,
            primary_ok: true,
        },
        Err(_) => internal_outcome_with(command, output, started_at, sinks),
    }
}

/// Brings the injected sinks into the `std::io::Write` shape the projection
/// owner writes through, without changing the projection call sites.
struct SinkAdapter<'a, 'b>(&'a mut RenderSinks<'b>);

impl std::io::Write for SinkAdapter<'_, '_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.stdout.write_all_bytes(buffer)?;
        Ok(buffer.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.stdout.flush_bytes()
    }
    fn write_all(&mut self, buffer: &[u8]) -> std::io::Result<()> {
        self.0.stdout.write_all_bytes(buffer)
    }
    fn write_fmt(&mut self, arguments: std::fmt::Arguments<'_>) -> std::io::Result<()> {
        let rendered = arguments.to_string();
        self.0.stdout.write_all_bytes(rendered.as_bytes())
    }
}

/// Renders a failure envelope as one sanitized text message on stderr
/// through the injected sinks. Returns false when the write fails so the
/// caller degrades to exit 1.
fn emit_error_text_with(envelope: &CommandEnvelope, sinks: &mut RenderSinks<'_>) -> bool {
    let Some(error) = envelope.error() else {
        return false;
    };
    let mut ok = writeln!(
        SinkWriter(sinks.stderr),
        "error: {}",
        sanitize_for_stderr(error.message())
    )
    .is_ok();
    if ok && let Some(recovery) = error.recovery() {
        ok = writeln!(
            SinkWriter(sinks.stderr),
            "help: {}",
            sanitize_for_stderr(recovery.message())
        )
        .is_ok();
        if ok && let Some(command) = recovery.command() {
            ok = writeln!(
                SinkWriter(sinks.stderr),
                "  try: {}",
                sanitize_for_stderr(command)
            )
            .is_ok();
        }
    }
    ok && sinks.stderr.flush_bytes().is_ok()
}

/// Brings one injected sink into the `std::io::Write` shape used by the
/// text-error rendering without changing its line structure.
struct SinkWriter<'a>(&'a mut dyn RenderWrite);

impl std::io::Write for SinkWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.write_all_bytes(buffer)?;
        Ok(buffer.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush_bytes()
    }
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

/// A direct advisory warning on stderr, TTY or not, per the same shell
/// contract as [`note_stderr`]. The body is reported verbatim (never
/// re-prefixed with `note:`): the shared CLI stderr owner emits exactly one
/// `warning:` line per emission, so an automatic history failure surfaces one
/// sanitized `warning: ...` with no doubling (contracts.md 14.2 note). The
/// caller supplies the sanitized body; every automatic-history call site
/// reduces upstream/detail text before it reaches this seam.
pub(crate) fn warning_stderr(_streams: &dyn crate::cli::StreamTty, body: &str) {
    warning_stderr_raw(body);
}

pub(crate) fn warning_stderr_raw(body: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "warning: {body}");
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
    command: ResolvedCommand,
    output: ResolvedOutput,
    started_at: crate::domain::UtcTimestamp,
) -> DetectOutcome {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let mut sinks = RenderSinks {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    internal_outcome_with(command, output, started_at, &mut sinks)
}

/// The sink-injectable render-failure outcome. The primary write already
/// failed: the internal failure is reported at the general render-failure
/// exit 1 and `primary_ok` is always `false`, so a later post-primary
/// failure attachment (a history save failure at category 7) can never
/// overwrite the render failure (contracts.md 14.2 note). On the JSON
/// surface the failure envelope defers to the process layer's write; on
/// the text surface it renders to stderr through the injected sinks here.
fn internal_outcome_with(
    command: ResolvedCommand,
    output: ResolvedOutput,
    started_at: crate::domain::UtcTimestamp,
    sinks: &mut RenderSinks<'_>,
) -> DetectOutcome {
    let envelope = CommandEnvelope::failure(
        command,
        internal_error("the result could not be rendered honestly"),
        EnvelopeMeta::default()
            .with_started_at(started_at)
            .with_failed_at(crate::domain::UtcTimestamp::now()),
    );
    match output.error {
        ErrorSurface::Json => DetectOutcome {
            exit_code: 1,
            envelopes: vec![envelope],
            rendered: false,
            primary_ok: false,
        },
        ErrorSurface::Text => {
            // The secondary stderr write is attempted once; its own failure
            // is still exit 1 with `primary_ok == false` either way.
            let _ = emit_error_text_with(&envelope, sinks);
            DetectOutcome {
                exit_code: 1,
                envelopes: vec![],
                rendered: true,
                primary_ok: false,
            }
        }
    }
}

pub(crate) fn internal_error(message: &str) -> CanonicalError {
    CanonicalError::new(ErrorCode::UpstreamError, message)
        .and_then(|error| error.with_contextual_retryability(false))
        .expect("the internal-error template is statically valid")
}

pub(crate) fn usage_error(code: ErrorCode, message: &str) -> CanonicalError {
    CanonicalError::new(code, message).expect("usage messages are non-empty")
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

#[cfg(test)]
mod tests;
