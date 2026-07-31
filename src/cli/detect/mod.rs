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
mod inputs;
mod render;

pub(crate) use client::{build_analyzer, credential_error, resolve_api_key};
pub(crate) use render::{DetectOutcome, early_failure};

use clap::ArgMatches;

use crate::analysis::{Accepted, AnalysisRequest, Analyzer, StopObserving, WaitOptions};
use crate::output::{CanonicalError, ColorPolicy, ErrorCode, OutputFormat, ProgressEvent};

use super::StreamTty;

/// The default AI-detection wait deadline when `--timeout` is absent.
const DEFAULT_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

// The submodules hold the cohesive halves of the adapter:
// - `inputs`: source resolution, word counting, and billing preflight
// - `render`: envelope assembly, exit mapping, and projection handoff
// - `client`: credential resolution, the endpoint-bearing client, SIGINT
//
// This file keeps the argument-and-flow core: flag parsing, plan/execute,
// and per-observation progress emission.
use client::set_active_cancel;
use inputs::{ResolvedInput, enforce_billable_ceiling, resolve_inputs};
use render::{
    DETACH_NOTE, failure_outcome, identity_note, internal_error, interrupted_outcome, note_stderr,
    success_outcome, usage_error,
};

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
    public_link: bool,
    save: bool,
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
            public_link: false,
            save: false,
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
            public_link: matches.get_flag("public-link"),
            save: matches.get_flag("save"),
            format,
            progress,
            timeout,
            max_billable_units,
        })
    }
}

/// A fully validated, priced detection plan: its output projection, progress
/// mode, and resolved inputs. Constructing it performs no billable work.
pub(crate) struct DetectionPlan {
    arguments: DetectArgs,
    output: ResolvedOutput,
    progress: ProgressMode,
    inputs: Vec<ResolvedInput>,
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

    if arguments.save {
        return Err(failure_outcome(
            output,
            started_at,
            usage_error(
                ErrorCode::UnsupportedCombination,
                "--save is unavailable: local history arrives in a later phase",
            ),
        ));
    }

    let inputs = match resolve_inputs(source, streams, stdin_text) {
        Ok(inputs) => inputs,
        Err(error) => return Err(failure_outcome(output, started_at, error)),
    };

    if let Err(error) = enforce_billable_ceiling(arguments.max_billable_units, &inputs) {
        return Err(failure_outcome(output, started_at, error));
    }

    let progress = resolve_progress(arguments.progress, output, streams);
    Ok(DetectionPlan {
        arguments,
        output,
        progress,
        inputs,
    })
}

/// Runs a priced plan against the analyzer. By this point credentials and the
/// client exist, so only submission, observation, and rendering remain.
pub(crate) fn execute(
    plan: &DetectionPlan,
    analyzer: Analyzer,
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
                output,
                started_at,
                internal_error("could not start the local async runtime"),
            );
        }
    };

    let stop = StopObserving::new();
    let _cancel_guard = set_active_cancel(stop.token());

    let analyses = runtime.block_on(async {
        let mut completed = Vec::with_capacity(plan.inputs.len());
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
                Ok(analysis) => completed.push(analysis),
                Err(error) => return Err(error),
            }
        }
        Ok(completed)
    });

    match analyses {
        Ok(analyses) => success_outcome(output, started_at, analyses),
        Err(Flow::Failed(error)) => failure_outcome(output, started_at, error),
        Err(Flow::Interrupted(note)) => interrupted_outcome(output, started_at, note),
    }
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

/// The stop condition of one detection flow.
enum Flow {
    /// A canonical failure to surface and map to an exit code.
    Failed(CanonicalError),
    /// Ctrl+C / SIGINT stopped local observation; identity is reported.
    Interrupted(String),
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
    let accepted = match analyzer.start(request, &cancel).await {
        Ok(accepted) => accepted,
        Err(error) => return Err(Flow::Failed(error.into_error())),
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

            let timeout = arguments.timeout.unwrap_or(DEFAULT_WAIT_TIMEOUT);
            let wait = WaitOptions::with_timeout(timeout);
            let progress_sink = ProgressSink::new(progress, analysis_id);
            match running
                .observe(wait, |event| progress_sink.on_progress(event), stop.clone())
                .await
            {
                Ok(Ok(analysis)) => Ok(analysis),
                Ok(Err(error)) => Err(Flow::Failed(error.into_error())),
                Err(interrupted_analysis) => Err(Flow::Interrupted(identity_note(
                    &interrupted_analysis.identity,
                ))),
            }
        }
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
                eprintln!(
                    "detect {}: running ({})",
                    event.analysis_id, event.last_stage
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
enum ProgressMode {
    Auto,
    Quiet,
    Human,
    Jsonl,
}

/// `--progress auto` emits human progress only when stderr is a TTY and the
/// primary format is pretty; `--progress jsonl` forces canonical events;
/// `--progress never` (or any nonqualifying auto case) emits nothing.
fn resolve_progress(
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

fn parse_duration(raw: &str) -> Option<std::time::Duration> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (number, multiplier) = if let Some(prefix) = raw.strip_suffix("ms") {
        (prefix, 1_f64)
    } else if let Some(prefix) = raw.strip_suffix('s') {
        (prefix, 1_000_f64)
    } else if let Some(prefix) = raw.strip_suffix('m') {
        (prefix, 60_000_f64)
    } else if let Some(prefix) = raw.strip_suffix('h') {
        (prefix, 3_600_000_f64)
    } else {
        (raw, 1_000_f64)
    };
    let value: f64 = number.trim().parse().ok()?;
    if !(value.is_finite() && value >= 0.0) {
        return None;
    }
    Some(std::time::Duration::from_secs_f64(
        value * multiplier / 1_000.0,
    ))
}

/// Chooses color only when pretty output is selected for a TTY, color is not
/// disabled by `--no-color`, and `NO_COLOR` is unset.
fn color_policy(
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
        assert!(parse_duration("--3").is_none());
        assert!(parse_duration("abc").is_none());
        assert!(parse_duration("").is_none());
    }
}
