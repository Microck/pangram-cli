//! Detection adapter: the thin CLI-owning layer over the shared analysis
//! module.
//!
//! The adapter owns only stream and argument decisions: input resolution
//! (literal, stdin, file), the canonical word count, billing preflight,
//! format/color/progress resolution, and the success/failure envelope plus
//! exit mapping. Every Pangram call, retry, timeout, cancellation, and
//! normalization stays inside [`crate::analysis`]. No raw protocol material
//! crosses this boundary.
//!
//! Privacy invariants enforced here:
//! - credentials enter the analysis module only through the endpoint-bearing
//!   client; they are never logged or rendered
//! - submitted text appears in stdout only when `--include-input` was given
//! - upstream text (segments, provider messages) is sanitized by the
//!   projection, never echoed raw to the terminal
//!
//! `CanonicalError` is 224 bytes wide because it is the adapter-facing
//! canonical object. Detection plumbing returns it directly (matching
//! `analysis::normalize`): these are cold input-validation and dispatch paths,
//! so boxing every intermediate would add churn for no measurable gain.
#![allow(clippy::result_large_err)]

pub(crate) mod client;
pub(crate) mod inputs;
mod render;
pub(crate) mod save;

pub(crate) use crate::analysis::config_error;
pub(crate) use client::{bridge_sigint, install_sigint_driver, reset_sigint_flag};
pub(crate) use render::{
    DetectOutcome, analysis_command_outcome, analysis_exit_code, early_failure, elapsed_ms,
    failure_outcome, identity_note, internal_error, interrupted_outcome, note_stderr,
    primary_outcome, sanitize_for_stderr, usage_error, warning_stderr,
};
pub(crate) use save::SaveStoreGate;

use clap::ArgMatches;

use crate::analysis::{Accepted, AnalysisRequest, Analyzer, StopObserving, WaitOptions};
use crate::output::{CanonicalError, ColorPolicy, ErrorCode, OutputFormat, ProgressEvent};

use super::StreamTty;

use inputs::{ResolvedInput, enforce_billable_ceiling, resolve_inputs};
use render::{DETACH_NOTE, success_outcome};

// The submodules hold the cohesive halves of the adapter:
// - `inputs`: source resolution, word counting, and billing preflight
// - `render`: envelope assembly, exit mapping, and projection handoff
// - `client`: credential resolution and SIGINT bridging
//
// This file keeps the argument-and-flow core: flag parsing, plan/execute,
// and per-observation progress emission.

/// The resolved rendering policy shared by primary output and errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedOutput {
    pub(crate) format: OutputFormat,
    pub(crate) color: ColorPolicy,
    /// How failure envelopes are surfaced: canonical JSON on stdout, or a
    /// sanitized text message on stderr.
    pub(crate) error: ErrorSurface,
}

/// Failure-envelope surface selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorSurface {
    /// Canonical JSON envelope printed to stdout (the noninteractive default).
    Json,
    /// A sanitized single message to stderr (the pretty default).
    Text,
}

/// The one source category for a detection request: literal text, stdin, or
/// one or more UTF-8 text files.
#[derive(Debug)]
pub(crate) enum Source {
    Literal(String),
    Stdin,
    Files(Vec<String>),
}

/// The detection-relevant flags resolved from one `detect` subcommand match.
/// Bare root text carries none of these; every field defaults per the
/// contract. Construction is the adapter's only Clap reads of subcommand-only
/// arguments, so a bare root parse (which has no such arguments) can never
/// panic.
#[derive(Debug, Clone)]
pub(crate) struct DetectArgs {
    detach: bool,
    include_input: bool,
    save: bool,
    public_link: bool,
    format: Option<OutputFormat>,
    progress: ProgressMode,
    timeout: Option<std::time::Duration>,
    max_billable_units: Option<u64>,
}

impl DetectArgs {
    /// The defaults for a bare-text invocation (no `detect` flags present):
    /// JSON output, waiting for terminal, no detach, progress auto.
    pub(crate) fn for_bare() -> Self {
        Self {
            detach: false,
            include_input: false,
            save: false,
            public_link: false,
            format: None,
            progress: ProgressMode::Auto,
            timeout: None,
            max_billable_units: None,
        }
    }

    /// Reads the closed detect flags from a matched `detect` subcommand. A
    /// malformed `--timeout` or `--max-billable-units` value is a usage error
    /// reported before any billable work.
    pub(crate) fn from_matches(matches: &ArgMatches) -> Result<Self, CanonicalError> {
        let format = match matches.get_one::<String>("format").map(String::as_str) {
            Some("json") => Some(OutputFormat::Json),
            Some("jsonl") => Some(OutputFormat::Jsonl),
            Some("toon") => Some(OutputFormat::Toon),
            Some("markdown") => Some(OutputFormat::Markdown),
            Some("pretty") => Some(OutputFormat::Pretty),
            // Clap's value parser already rejected any other spelling.
            Some(_) | None => None,
        };
        let progress = match matches.get_one::<String>("progress").map(String::as_str) {
            Some("jsonl") => ProgressMode::Jsonl,
            Some("never") => ProgressMode::Quiet,
            // "auto" is the documented default.
            Some(_) | None => ProgressMode::Auto,
        };
        let timeout = matches
            .get_one::<String>("timeout")
            .map(|raw| {
                parse_duration(raw).ok_or_else(|| {
                    usage_error(
                        ErrorCode::UnsupportedInput,
                        "--timeout must be a number of seconds, optionally with an s, ms, m, or h suffix",
                    )
                })
            })
            .transpose()?;
        let max_billable_units = matches
            .get_one::<String>("max-billable-units")
            .map(|raw| {
                raw.trim().parse::<u64>().map_err(|_| {
                    usage_error(
                        ErrorCode::UnsupportedInput,
                        "--max-billable-units must be a non-negative integer",
                    )
                })
            })
            .transpose()?;
        Ok(Self {
            detach: matches.get_flag("detach"),
            include_input: matches.get_flag("include-input"),
            save: matches.get_flag("save"),
            public_link: matches.get_flag("public-link"),
            format,
            progress,
            timeout,
            max_billable_units,
        })
    }
}

/// A fully validated, priced detection plan: its output projection, progress
/// mode, resolved inputs, and the history gate. Constructing it performs no
/// billable work and never opens or creates history storage.
pub(crate) struct DetectionPlan {
    arguments: DetectArgs,
    output: ResolvedOutput,
    progress: ProgressMode,
    inputs: Vec<ResolvedInput>,
    history_gate: SaveStoreGate,
}

/// Parses arguments, validates input, and enforces the billable-unit ceiling
/// without any credential or network work. Local usage errors surface here so
/// a caller without a configured key still gets the canonical input feedback.
pub(crate) fn plan(
    source: Source,
    arguments: DetectArgs,
    global: &GlobalFlags,
    streams: &dyn StreamTty,
    stdin_text: Option<String>,
) -> Result<DetectionPlan, DetectOutcome> {
    let started_at = crate::domain::UtcTimestamp::now();
    let output = resolve_output(&arguments, &source, global, streams);

    let inputs = match resolve_inputs(source, streams, stdin_text) {
        Ok(inputs) => inputs,
        Err(error) => {
            return Err(failure_outcome(
                crate::output::ResolvedCommand::Detect,
                output,
                started_at,
                error,
            ));
        }
    };

    if let Err(error) = enforce_billable_ceiling(arguments.max_billable_units, &inputs) {
        return Err(failure_outcome(
            crate::output::ResolvedCommand::Detect,
            output,
            started_at,
            error,
        ));
    }

    // Resolve the history gate without touching storage: the configuration
    // read decides whether the automatic path is armed, and `--save` arms
    // the manual path. Opening the database waits until a completed analysis
    // exists to persist, so a disabled run never creates the directory.
    let history_gate = save::resolve_gate(arguments.save);
    let progress = resolve_progress(arguments.progress, output, streams);
    Ok(DetectionPlan {
        arguments,
        output,
        progress,
        inputs,
        history_gate,
    })
}

impl DetectionPlan {
    /// The resolved rendering/policy for this plan, handed to credential and
    /// client construction so prepare failures surface through the exact
    /// selected format and error surface.
    pub(crate) fn resolved_output(&self) -> ResolvedOutput {
        self.output
    }
}

/// Runs a priced plan against the analyzer. By this point credentials and the
/// client exist, so only submission, observation, persistence, and rendering
/// remain.
pub(crate) fn execute(
    plan: &DetectionPlan,
    analyzer: Analyzer,
    service: &crate::config::ConfigService,
    streams: &dyn StreamTty,
) -> DetectOutcome {
    let started_at = crate::domain::UtcTimestamp::now();
    let output = plan.output;

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            return failure_outcome(
                crate::output::ResolvedCommand::Detect,
                output,
                started_at,
                internal_error("could not start the local async runtime"),
            );
        }
    };

    let stop = StopObserving::new();
    // Install the SIGINT driver once; the child task that cancels this run's
    // token from the recorded flag is spawned inside the runtime below.
    client::install_sigint_driver();

    let (members, terminal) = runtime.block_on(async {
        // A lock-free SIGINT bridge: the low-level handler only sets an
        // atomic flag (async-signal-safe); this task translates it into the
        // shared observation token cancel outside signal context.
        let bridge = tokio::spawn(client::bridge_sigint(stop.token().clone()));
        let mut members: Vec<crate::domain::Analysis<CanonicalError>> =
            Vec::with_capacity(plan.inputs.len());
        let mut terminal = None;
        for input in &plan.inputs {
            match analyze_one(
                &analyzer,
                &plan.arguments,
                input,
                &stop,
                plan.progress,
                streams,
            )
            .await
            {
                // A completed (succeeded or honestly failed) member is kept in
                // order; the run continues with the remaining files so one
                // failed file never discards the billable work already done.
                Ok(analysis) => members.push(analysis),
                // The current member did not complete, but the ordered prefix
                // did. Carry both parts out so the prefix can persist and
                // render before the terminal failure or interruption.
                Err(flow) => {
                    terminal = Some(flow);
                    break;
                }
            }
        }
        bridge.abort();
        (members, terminal)
    });
    // A finished flow must not leak its interrupt into the next run.
    client::reset_sigint_flag();

    let command = crate::output::ResolvedCommand::Detect;
    if members.is_empty() {
        return match terminal.expect("an empty run has a terminal flow") {
            Flow::Failed(error) => failure_outcome(command, output, started_at, error),
            Flow::Interrupted(error, note) => {
                interrupted_outcome(command, output, started_at, error, note)
            }
        };
    }

    // A terminal flow can only stop at the next input, so the completed
    // analyses map exactly to the same-length input prefix.
    let retained_texts = plan
        .inputs
        .iter()
        .take(members.len())
        .map(|input| input.text.clone())
        .collect::<Vec<_>>();
    let (members, save_failure) =
        save::persist_analyses(members, &retained_texts, plan.history_gate, service);
    let mut outcome = success_outcome(output, started_at, members);
    let required_save_exit = save_failure.and_then(|error| {
        outcome.attach_failure(command, output, started_at, error);
        // A render failure owns exit 1. Otherwise retain the exact manual-save
        // category so a later terminal flow can remain visible without
        // replacing the caller's unfulfilled explicit save requirement.
        outcome.primary_ok.then_some(outcome.exit_code)
    });
    if let Some(flow) = terminal {
        match flow {
            Flow::Failed(error) => {
                outcome.attach_failure(command, output, started_at, error);
                if outcome.primary_ok {
                    if let Some(exit_code) = required_save_exit {
                        outcome.exit_code = exit_code;
                    }
                }
            }
            Flow::Interrupted(error, note) => {
                render::note_stderr_raw(&note);
                outcome.attach_failure(command, output, started_at, error);
                if outcome.primary_ok {
                    outcome.exit_code = crate::output::ExitCode::Interrupted.as_u8();
                }
            }
        }
    }
    outcome
}

/// The global (root-level) flags that affect detection rendering. Read once
/// at the root so a subcommand-only match never has to supply them.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GlobalFlags {
    pub(crate) no_color: bool,
    pub(crate) error_format_text: Option<bool>,
}

impl GlobalFlags {
    /// Reads the two global rendering flags. Both are defined global clap
    /// arguments, so typed reads are safe on any matched level.
    pub(crate) fn from_matches(matches: &ArgMatches) -> Self {
        Self {
            no_color: matches
                .get_one::<bool>("no-color")
                .copied()
                .unwrap_or(false),
            error_format_text: matches
                .get_one::<String>("error-format")
                .map(|value| value.as_str() == "text"),
        }
    }
}

/// The stop condition of one detection flow. A failed file submission is
/// preserved as a failed series member (Option A) rather than aborting; only
/// an accepted task's local observation failure and a genuine SIGINT stop the
/// run through these variants.
enum Flow {
    /// An accepted task's local observation failure (wait timeout, contract
    /// drift, transport): surfaces as the canonical top-level error envelope.
    Failed(CanonicalError),
    /// Ctrl+C / SIGINT stopped the flow. The carried error is the honest
    /// canonical outcome: a pre-issue local stop (`network_unavailable`) when
    /// no billable send was issued, or `submission_outcome_unknown` when the
    /// billable POST was already issued and the send became ambiguous (F3).
    /// The process still exits 130 as locked; only the reported outcome
    /// differs.
    Interrupted(CanonicalError, String),
}

/// Runs one input end to end: request construction, submit, optional wait,
/// and progress emission. This is the only place that awaits the analyzer.
async fn analyze_one(
    analyzer: &Analyzer,
    arguments: &DetectArgs,
    input: &ResolvedInput,
    stop: &StopObserving,
    progress: ProgressMode,
    streams: &dyn StreamTty,
) -> Result<crate::domain::Analysis<CanonicalError>, Flow> {
    let request = AnalysisRequest::new(
        input.text.clone(),
        input.origin,
        input.name.clone(),
        input.word_count,
        arguments.include_input,
        arguments.public_link,
    );

    let cancel = stop.token().child_token();
    let accepted = match analyzer.start_full(request, &cancel).await {
        Ok(accepted) => accepted,
        Err(failure) => {
            let crate::analysis::SubmissionFailure {
                task_error,
                request,
            } = failure;
            let error = task_error.into_error();
            // A SIGINT landing during submission still exits 130 for the
            // whole run; the carried error is the honest canonical outcome
            // the analysis module already classified (pre-issue local stop,
            // or submission_outcome_unknown once the POST was issued).
            if cancel.is_cancelled() {
                let note = identity_note_for_error(&error);
                return Err(Flow::Interrupted(error, note));
            }
            // A deterministic pre-billing submission rejection (auth,
            // payment, usage) aborts the run with its canonical top-level
            // error; the request provably produced no ambiguous billable
            // state worth preserving a member for. Only an ambiguous issued
            // POST preserves this file as a failed series member so the
            // completed billable work around it is not discarded, and the
            // run never replays the ambiguous send (contracts.md 3.3).
            if !matches!(error.code(), ErrorCode::SubmissionOutcomeUnknown) {
                return Err(Flow::Failed(error));
            }
            let request = request.expect("a completed submission carries its request");
            return Ok(failed_member(&request, error));
        }
    };

    match accepted {
        Accepted::Terminal(analysis) => Ok(*analysis),
        Accepted::Task(accepted_input) => {
            let running = analyzer.running(accepted_input);
            let analysis_id = running.analysis_id();
            if arguments.detach {
                // Accepted but not awaited: emit the running snapshot with its
                // upstream identity. Exit is still 0 for accepted async work.
                note_stderr(streams, DETACH_NOTE);
                return Ok(running.snapshot());
            }

            // No `--timeout` is the documented unbounded wait: the caller
            // supplies no local ceiling and the observation runs until the
            // task is terminal or locally cancelled.
            let wait = arguments
                .timeout
                .map(WaitOptions::with_timeout)
                .unwrap_or(WaitOptions::UNBOUNDED);
            let progress_sink = ProgressSink::new(progress, analysis_id);
            match running
                .observe(wait, |event| progress_sink.on_progress(event), stop.clone())
                .await
            {
                // Both a terminal success and a terminal upstream analysis
                // failure (`finish_failed`) arrive as a committed analysis.
                Ok(Ok(analysis)) => Ok(analysis),
                // A local observation failure (wait timeout, contract drift,
                // transport) is a canonical wait/observation error envelope.
                Ok(Err(task_error)) => Err(Flow::Failed(task_error.into_error())),
                Err(interrupted_analysis) => Err(Flow::Interrupted(
                    stopped_observation_error(),
                    identity_note(&interrupted_analysis.identity),
                )),
            }
        }
    }
}

/// Builds the ordered-series member for a file whose submission completed
/// without an acceptance (a proven-no-send network failure, or an ambiguous
/// issued POST). The member carries the file's real input descriptor and the
/// canonical error, never a fabricated result or upstream identity.
/// `submission_outcome` is the most precise honest value the error implies:
/// `acceptance_unknown` for an ambiguous issued send, `terminal` otherwise.
/// The member never carries upstream identity: an unaccepted submission has
/// no real task id or result, so none is fabricated.
pub(crate) fn failed_member(
    request: &AnalysisRequest,
    error: CanonicalError,
) -> crate::domain::Analysis<CanonicalError> {
    let outcome = submission_outcome_for_error(&error);
    let check: crate::domain::CheckState<crate::domain::AiDetectionResult, CanonicalError> =
        crate::domain::CheckState::Failed {
            upstream: None,
            error,
        };
    let checks = crate::domain::OrderedChecks::new([crate::domain::Check::AiDetection(check)])
        .expect("one check is valid");
    let now = crate::domain::UtcTimestamp::now();
    let provenance = crate::domain::Provenance {
        provider: crate::domain::Provider::Pangram,
        upstream_version: None,
        upstream_task_ids: None,
        upstream_bulk_id: None,
        submitted_at: None,
        completed_at: None,
    };
    crate::domain::Analysis::new(
        request.id(),
        outcome,
        request.input(),
        checks,
        crate::domain::SaveState::Ephemeral,
        provenance,
        None,
        request.rerun_of(),
        now,
        now,
        None,
    )
    .expect("a synthesized failed member satisfies the analysis invariants")
}

/// The most precise honest submission outcome an error implies. An ambiguous
/// issued send is `acceptance_unknown`; every other completed local or
/// network failure is `terminal` (the run reached a definitive local result
/// with no remote acceptance to reconcile).
fn submission_outcome_for_error(error: &CanonicalError) -> crate::domain::SubmissionOutcome {
    if matches!(
        error.code(),
        crate::output::ErrorCode::SubmissionOutcomeUnknown
    ) {
        crate::domain::SubmissionOutcome::AcceptanceUnknown
    } else {
        crate::domain::SubmissionOutcome::Terminal
    }
}

/// The canonical local-stop error for a wait-phase cancellation: the task
/// was accepted (the billable send is concluded), but local observation was
/// stopped without any remote cancellation, so the honest outcome is the
/// network-unavailable local stop.
fn stopped_observation_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::NetworkUnavailable,
        "observation was interrupted locally; no remote cancellation was sent",
    )
    .expect("static template")
}

/// The stderr identity note for a submission-phase interruption, derived from
/// the canonical error's own reconciliation details (local analysis ID,
/// request hash, last observed state) where present.
fn identity_note_for_error(error: &CanonicalError) -> String {
    match error.details() {
        Some(crate::output::CanonicalErrorDetails::SubmissionOutcomeUnknown(details)) => {
            let mut note = "interrupted; local analysis id ".to_owned();
            note.push_str(&match details.operation_id() {
                crate::domain::LocalOperationId::AnalysisId(id) => id.to_string(),
                crate::domain::LocalOperationId::BulkId(id) => id.to_string(),
            });
            note.push_str(&format!("; request sha256 {}", details.request_sha256));
            if let Some(task) = &details.upstream_task_id {
                note.push_str(&format!("; upstream task id {task}"));
            }
            note.push_str(&format!("; last status {}", details.last_status.as_str()));
            note
        }
        _ => "interrupted during submission; no remote action was completed".to_owned(),
    }
}

/// Emits progress events to stderr in the resolved mode. Human progress is
/// textual; `jsonl` progress emits canonical `ProgressEvent` lines.
struct ProgressSink {
    mode: ProgressMode,
    analysis_id: crate::domain::AnalysisId,
}

impl ProgressSink {
    fn new(mode: ProgressMode, analysis_id: crate::domain::AnalysisId) -> Self {
        Self { mode, analysis_id }
    }

    fn on_progress(&self, event: &crate::analysis::AnalysisProgress) {
        match self.mode {
            // `Auto` is always resolved away by `resolve_progress` before a
            // sink exists, so it is unreachable here.
            ProgressMode::Auto | ProgressMode::Quiet => {}
            ProgressMode::Human => {
                // The provider-supplied stage token is untrusted; sanitize it
                // for the terminal at the write boundary.
                eprintln!(
                    "detect {}: running ({})",
                    event.analysis_id,
                    sanitize_for_stderr(event.last_stage.as_str())
                );
            }
            ProgressMode::Jsonl => {
                let observed = crate::domain::UtcTimestamp::now();
                let progress = ProgressEvent::analysis(
                    self.analysis_id,
                    crate::domain::CheckKind::AiDetection,
                    crate::domain::CheckStatus::Running,
                    observed,
                )
                .with_upstream_stage(event.last_stage.as_str());
                if let Ok(progress) = progress {
                    if let Ok(line) = serde_json::to_string(&progress) {
                        eprintln!("{line}");
                    }
                }
            }
        }
    }
}

/// The closed set of stderr progress behaviors resolved from `--progress`.
/// `Auto` is resolved against the terminal and format before use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressMode {
    Auto,
    Quiet,
    Human,
    Jsonl,
}

/// `--progress auto` emits human progress only when stderr is a TTY and the
/// primary format is pretty; `--progress jsonl` forces canonical events;
/// `--progress never` (or any nonqualifying auto case) emits nothing.
pub(crate) fn resolve_progress(
    selected: ProgressMode,
    output: ResolvedOutput,
    streams: &dyn StreamTty,
) -> ProgressMode {
    match selected {
        ProgressMode::Jsonl => ProgressMode::Jsonl,
        ProgressMode::Quiet | ProgressMode::Human => ProgressMode::Quiet,
        ProgressMode::Auto => {
            if streams.stderr() && output.format == OutputFormat::Pretty {
                ProgressMode::Human
            } else {
                ProgressMode::Quiet
            }
        }
    }
}

/// Parses one `--timeout` value against the locked grammar (contracts.md
/// 14.2): a non-negative ASCII decimal count, optionally followed by exactly
/// one ASCII unit suffix `s`, `ms`, `m`, or `h`; a missing suffix means
/// seconds. No whitespace is allowed anywhere in the token (so there is no
/// "space before the suffix" form). Exponent and non-finite forms, signed
/// counts, zero (and any value truncating to zero), unknown suffixes, and
/// out-of-range scaled values are all rejected.
pub(crate) fn parse_duration(raw: &str) -> Option<std::time::Duration> {
    if raw.is_empty() || !raw.is_ascii() || raw.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return None;
    }
    // Split the strictly-numeric ASCII decimal count from an optional single
    // unit suffix. Only ASCII digits and at most one `.` form the count.
    let split_at = raw
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .unwrap_or(raw.len());
    let (number, suffix) = raw.split_at(split_at);
    let multiplier_ms: u64 = match suffix {
        "" | "s" => 1_000,
        "ms" => 1,
        "m" => 60_000,
        "h" => 3_600_000,
        _ => return None,
    };
    // The count is digits with at most one dot, no sign, no exponent.
    let mut dots = 0_u8;
    if number.is_empty()
        || !number.bytes().all(|byte| {
            if byte == b'.' {
                dots += 1;
                dots == 1
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    if !(value.is_finite() && value > 0.0) {
        return None;
    }
    // Compute the bounded whole-millisecond duration without overflow.
    let millis = value * multiplier_ms as f64;
    if !(millis.is_finite() && millis >= 1.0 && millis <= u64::MAX as f64) {
        return None;
    }
    Some(std::time::Duration::from_millis(millis as u64))
}

/// Chooses color only when pretty output is selected for a TTY, color is not
/// disabled by `--no-color`, and `NO_COLOR` is unset.
pub(crate) fn color_policy(
    global: &GlobalFlags,
    format: OutputFormat,
    streams: &dyn StreamTty,
) -> ColorPolicy {
    let disabled = global.no_color || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    if !disabled && format == OutputFormat::Pretty && streams.stdout() {
        ColorPolicy::Color
    } else {
        ColorPolicy::Plain
    }
}

/// Resolves the primary format, color, and failure surface per the contract:
/// noninteractive default is JSON with JSON errors; explicit pretty defaults
/// to human errors; `--format` selects the projection; repeated files default
/// to JSONL.
pub(crate) fn resolve_output(
    arguments: &DetectArgs,
    source: &Source,
    global: &GlobalFlags,
    streams: &dyn StreamTty,
) -> ResolvedOutput {
    let repeated = matches!(source, Source::Files(files) if files.len() > 1);
    let format = arguments.format.unwrap_or(if repeated {
        OutputFormat::Jsonl
    } else {
        OutputFormat::Json
    });
    let color = color_policy(global, format, streams);
    let error = match global.error_format_text {
        Some(true) => ErrorSurface::Text,
        Some(false) => ErrorSurface::Json,
        None => {
            if format == OutputFormat::Pretty {
                ErrorSurface::Text
            } else {
                ErrorSurface::Json
            }
        }
    };
    ResolvedOutput {
        format,
        color,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_timeout_is_the_unbounded_wait_not_a_hidden_ceiling() {
        // F2: there is no hidden default wait timeout. The plan/option
        // construction proves it without sleeping: a bare invocation and a
        // `detect` invocation without `--timeout` both carry `timeout: None`,
        // which `analyze_one` maps to `WaitOptions::UNBOUNDED`.
        assert!(DetectArgs::for_bare().timeout.is_none());
        let matches = crate::cli::runtime_command()
            .try_get_matches_from(["pangram", "detect", "some words"])
            .expect("parses");
        let (_, sub) = matches.subcommand().expect("a subcommand was given");
        let args = DetectArgs::from_matches(sub).expect("valid args");
        assert!(args.timeout.is_none());
        let wait = args
            .timeout
            .map(WaitOptions::with_timeout)
            .unwrap_or(WaitOptions::UNBOUNDED);
        assert_eq!(wait, WaitOptions::UNBOUNDED);
        assert_eq!(wait.timeout, None);
    }

    #[test]
    fn duration_parsing_accepts_the_locked_grammar() {
        assert_eq!(
            parse_duration("30").unwrap(),
            std::time::Duration::from_secs(30)
        );
        assert_eq!(
            parse_duration("500ms").unwrap(),
            std::time::Duration::from_millis(500)
        );
        assert_eq!(
            parse_duration("2m").unwrap(),
            std::time::Duration::from_secs(120)
        );
        assert_eq!(
            parse_duration("1h").unwrap(),
            std::time::Duration::from_secs(3600)
        );
        assert_eq!(
            parse_duration("0.5").unwrap(),
            std::time::Duration::from_millis(500)
        );
        assert_eq!(
            parse_duration("1.5h").unwrap(),
            std::time::Duration::from_secs(5400)
        );
        // Bare fractional seconds and a leading-dot decimal are valid counts.
        assert_eq!(
            parse_duration(".5").unwrap(),
            std::time::Duration::from_millis(500)
        );
    }

    #[test]
    fn duration_parsing_rejects_out_of_grammar_forms() {
        // No whitespace anywhere in the token, including before the suffix.
        for rejected in ["1 s", "1s ", " 1s", "1 ms", "1\ts", "1\ts", "1\u{00a0}s"] {
            assert!(
                parse_duration(rejected).is_none(),
                "whitespace: {rejected:?}"
            );
        }
        // Exponent and non-finite forms.
        for rejected in [
            "1e2", "1E2", "1e-1", "inf", "-inf", "nan", "NaN", "Infinity",
        ] {
            assert!(
                parse_duration(rejected).is_none(),
                "exponent/non-finite: {rejected:?}"
            );
        }
        // Negative, empty, pure-suffix, and sign-prefixed counts.
        for rejected in ["-1", "-1s", "+1", "--3", "s", "ms", "m", "h", "", "."] {
            assert!(
                parse_duration(rejected).is_none(),
                "bad count: {rejected:?}"
            );
        }
        // Zero (and any value truncating to zero) bounds no wait.
        for rejected in ["0", "0s", "0ms", "0m", "0h", "0.0", "0.0001ms", ".0"] {
            assert!(parse_duration(rejected).is_none(), "zero: {rejected:?}");
        }
        // Unknown or doubled suffixes.
        for rejected in ["1d", "1w", "1ss", "1sms", "1mhz", "1mss"] {
            assert!(
                parse_duration(rejected).is_none(),
                "bad suffix: {rejected:?}"
            );
        }
        // Out-of-range: a count that scaled overflows the duration range.
        assert!(parse_duration("1e30").is_none(), "exponent overflow");
        assert!(parse_duration("999999999999999999999999h").is_none());
    }
}
