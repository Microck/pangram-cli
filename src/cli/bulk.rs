//! The bulk and task command adapter. Every path plans local inputs,
//! resolves credentials and the shared analyzer, runs exactly one analysis
//! flow, and renders one canonical outcome. The adapter never reaches
//! Pangram directly: submission, observation, safe-GET paging, timeouts, and
//! cancellation all stay inside the shared [`crate::analysis`] module, and
//! every rendered envelope or failure goes through the single projection,
//! exit-code, and error-surface owners shared with detection.
//!
//! Local JSONL reading is a whole-file UTF-8 read (the Pangram request-body
//! and billable-unit ceilings keep files small). Item text surfaces only in
//! the validated plan; failures name the source shape, never the text.
//!
//! `CanonicalError` is the 224-byte adapter-facing object used end to end, so
//! this module inherits the render-level `result_large_err` rationale.

use std::io::Read as _;

use clap::ArgMatches;
use tokio_util::sync::CancellationToken;

use crate::analysis::{Analyzer, BulkAnalysisRequest, StopObserving, WaitOptions};
use crate::cli::StreamTty;
use crate::cli::detect::{self, DetectOutcome, ErrorSurface, GlobalFlags, ProgressMode};
use crate::domain::{
    Analysis, AnalysisStatus, BulkCollection, BulkJsonlError, BulkPage, BulkSubmissionPlan,
    Sha256Hash, SubmissionOutcome, UpstreamBulkId, UpstreamTaskId, UtcTimestamp, parse_bulk_jsonl,
};
use crate::output::{
    CanonicalError, CommandData, CommandEnvelope, EnvelopeMeta, ErrorCode, ExitCode, OutputFormat,
    ProgressEvent, ResolvedCommand,
};

/// One adapter entry point: dispatches the matched bulk or task subcommand
/// to its flow and returns the outcome the shared process layer renders.
pub(crate) fn execute(
    resolved: ResolvedCommand,
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
) -> DetectOutcome {
    let started = UtcTimestamp::now();
    match resolved {
        ResolvedCommand::BulkSubmit => bulk_submit(sub, root_matches, global, streams, started),
        ResolvedCommand::BulkStatus => bulk_status(sub, root_matches, global, streams, started),
        ResolvedCommand::BulkWait => bulk_wait(sub, root_matches, global, streams, started),
        ResolvedCommand::BulkResults => bulk_results(sub, root_matches, global, streams, started),
        ResolvedCommand::TaskStatus => task_status(sub, root_matches, global, streams, started),
        ResolvedCommand::TaskWait => task_wait(sub, root_matches, global, streams, started),
        _ => unreachable!("dispatch only routes bulk and task commands here"),
    }
}

/// The rendering policy for one bulk/task invocation: `--format` where the
/// grammar permits it, the JSON default elsewhere; the shared color policy;
/// and the shared global error surface.
fn resolve_policy(
    resolved: ResolvedCommand,
    sub: &ArgMatches,
    global: &GlobalFlags,
    streams: &dyn StreamTty,
) -> detect::ResolvedOutput {
    // `--format` exists only where the grammar carries it (bulk submit and
    // bulk results); reading an undefined Clap argument id panics, so every
    // other command resolves the JSON default without touching the match.
    let format = if matches!(
        resolved,
        ResolvedCommand::BulkSubmit | ResolvedCommand::BulkResults
    ) {
        match sub.get_one::<String>("format").map(String::as_str) {
            Some("jsonl") => OutputFormat::Jsonl,
            Some("toon") => OutputFormat::Toon,
            Some("markdown") => OutputFormat::Markdown,
            Some("pretty") => OutputFormat::Pretty,
            _ => OutputFormat::Json,
        }
    } else {
        OutputFormat::Json
    };
    detect::ResolvedOutput {
        format,
        color: detect::color_policy(global, format, streams),
        error: if global.error_format_text == Some(true) {
            ErrorSurface::Text
        } else {
            ErrorSurface::Json
        },
    }
}

/// The progress mode for `bulk wait` and `task wait` (the only bulk/task
/// commands whose grammar carries `--progress`); resolved against the
/// terminal and selected format exactly like detection.
fn resolve_wait_progress(
    sub: &ArgMatches,
    output: detect::ResolvedOutput,
    streams: &dyn StreamTty,
) -> ProgressMode {
    let selected = match sub.get_one::<String>("progress").map(String::as_str) {
        Some("never") => ProgressMode::Quiet,
        Some("jsonl") => ProgressMode::Jsonl,
        _ => ProgressMode::Auto,
    };
    detect::resolve_progress(selected, output, streams)
}

/// Parses the shared `--timeout` duration; the locked grammar lives inside
/// `detect::parse_duration`, reused here one-to-one. An invalid token is a
/// usage error on this command.
fn resolve_timeout(
    resolved: ResolvedCommand,
    sub: &ArgMatches,
    started: UtcTimestamp,
    output: detect::ResolvedOutput,
) -> Result<Option<std::time::Duration>, DetectOutcome> {
    match sub.get_one::<String>("timeout") {
        Some(raw) => match detect::parse_duration(raw) {
            Some(duration) => Ok(Some(duration)),
            None => Err(detect::failure_outcome(
                resolved,
                output,
                started,
                detect::usage_error(
                    ErrorCode::UnsupportedInput,
                    "--timeout must be an ASCII decimal count with an optional s, ms, m, or h suffix",
                ),
            )),
        },
        None => Ok(None),
    }
}

/// Reads the bulk JSONL source text: a UTF-8 file, stdin for the literal
/// `-` marker, or stdin when the positional path is omitted (the piped
/// default). Failures name the channel, never item text.
#[allow(clippy::result_large_err)]
fn read_jsonl_source(sub: &ArgMatches) -> Result<String, CanonicalError> {
    match sub.get_one::<String>("JSONL_PATH").map(String::as_str) {
        Some("-") | None => {
            let mut text = String::new();
            let read = std::io::stdin().read_to_string(&mut text);
            match read {
                Ok(_) if !text.is_empty() => Ok(text),
                Ok(_) => Err(detect::usage_error(
                    ErrorCode::InputRequired,
                    "the bulk JSONL source on stdin was empty",
                )),
                Err(_) => Err(detect::usage_error(
                    ErrorCode::UnsupportedInput,
                    "the bulk JSONL source on stdin was not valid UTF-8",
                )),
            }
        }
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) if !text.is_empty() => Ok(text),
            Ok(_) => Err(detect::usage_error(
                ErrorCode::InputRequired,
                "the bulk JSONL file was empty",
            )),
            Err(_) => Err(detect::usage_error(
                ErrorCode::UnsupportedInput,
                "the bulk JSONL file could not be read as UTF-8 text",
            )),
        },
    }
}

/// The canonical word count shared with detection input summaries:
/// whitespace-split words.
fn word_count(text: &str) -> u64 {
    u64::try_from(text.split_whitespace().count()).unwrap_or(u64::MAX)
}

/// Whole-file JSONL validation into the shared domain contract. The error
/// carries the line number and structural reason only.
#[allow(clippy::result_large_err)]
fn plan_from_jsonl(
    text: &str,
    max_billable_units: u64,
) -> Result<BulkSubmissionPlan, CanonicalError> {
    let items = parse_bulk_jsonl(text, word_count).map_err(|error| match error {
        BulkJsonlError::EmptyFile => detect::usage_error(
            ErrorCode::InputRequired,
            "the bulk JSONL source contained no items",
        ),
        error @ BulkJsonlError::InvalidLine { .. } => {
            let message = error.to_string();
            detect::usage_error(ErrorCode::UnsupportedInput, &message)
        }
    })?;
    BulkSubmissionPlan::new(items, max_billable_units).map_err(|error| match error {
        crate::domain::DomainError::BulkLimitExceeded => detect::usage_error(
            ErrorCode::BulkLimitExceeded,
            "the bulk submission exceeds the billable-unit ceiling",
        ),
        crate::domain::DomainError::DuplicateBulkCallerId => detect::usage_error(
            ErrorCode::UnsupportedInput,
            "the bulk JSONL contains a duplicate caller id",
        ),
        other => {
            let message = other.to_string();
            detect::usage_error(ErrorCode::UnsupportedInput, &message)
        }
    })
}

/// Parses a caller-supplied ID string into the validated upstream identity
/// type. The ID itself is trusted terminal input, so a parse failure names
/// no content beyond the empty-shape contract.
#[allow(clippy::result_large_err)]
fn parse_upstream_bulk_id(raw: &str) -> Result<UpstreamBulkId, CanonicalError> {
    UpstreamBulkId::new(raw).map_err(|_| {
        detect::usage_error(ErrorCode::InputRequired, "a bulk job ID must not be empty")
    })
}

#[allow(clippy::result_large_err)]
fn parse_upstream_task_id(raw: &str) -> Result<UpstreamTaskId, CanonicalError> {
    UpstreamTaskId::new(raw)
        .map_err(|_| detect::usage_error(ErrorCode::InputRequired, "a task ID must not be empty"))
}

/// Builds configuration, credentials, and the analyzer for a bulk/task
/// request. Shared with detection through the process-owned preparation.
fn prepare(
    resolved: ResolvedCommand,
    root_matches: &ArgMatches,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
) -> Result<Analyzer, DetectOutcome> {
    super::prepare_detection(resolved, root_matches, output, started)
}

/// A success outcome for one canonical data payload, through the one
/// projection handoff. The exit code derives from the canonical content
/// before rendering ever runs.
fn succeed(
    data: CommandData,
    exit_code: ExitCode,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
) -> DetectOutcome {
    let meta = EnvelopeMeta::default()
        .with_started_at(started)
        .with_duration_ms(detect::elapsed_ms(started));
    let envelope = CommandEnvelope::success(data, meta);
    detect::primary_outcome(&envelope, output, exit_code.as_u8(), started)
}

/// Emits one canonical JSON bulk progress line on stderr for `--progress
/// jsonl`; the adapter counter snapshot flows into the shared event type so
/// the schema stays output-owned.
fn emit_bulk_jsonl_progress(progress: &crate::analysis::BulkProgress) {
    let observed = UtcTimestamp::now();
    if let Ok(event) = ProgressEvent::bulk(progress.bulk_id, progress.status, observed)
        .with_counters(progress.counters)
    {
        if let Ok(line) = serde_json::to_string(&event) {
            eprintln!("{line}");
        }
    }
}

/// A textual human progress line for bulk observation; only IDs and
/// counters, never content.
fn emit_bulk_human_progress(progress: &crate::analysis::BulkProgress) {
    let counters = progress.counters;
    let status = match progress.status {
        AnalysisStatus::Queued => "queued",
        AnalysisStatus::Running => "running",
        AnalysisStatus::Succeeded => "succeeded",
        AnalysisStatus::Failed => "failed",
        AnalysisStatus::Partial => "partial",
    };
    eprintln!(
        "bulk {}: {} ({}/{} succeeded, {} failed)",
        progress.bulk_id,
        status,
        counters.succeeded(),
        counters.total_items(),
        counters.failed(),
    );
}

// ---------------------------------------------------------------------------
// bulk submit
// ---------------------------------------------------------------------------

fn bulk_submit(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
) -> DetectOutcome {
    let resolved = ResolvedCommand::BulkSubmit;
    let output = resolve_policy(resolved, sub, &global, streams);

    let max_billable_units = match sub.get_one::<String>("max-billable-units") {
        Some(raw) => match raw.parse::<u64>().ok().filter(|value| *value >= 1) {
            Some(value) => value,
            None => {
                return detect::failure_outcome(
                    resolved,
                    output,
                    started,
                    detect::usage_error(
                        ErrorCode::UnsupportedInput,
                        "--max-billable-units must be an integer of at least 1",
                    ),
                );
            }
        },
        None => {
            return detect::failure_outcome(
                resolved,
                output,
                started,
                detect::usage_error(ErrorCode::InputRequired, "--max-billable-units is required"),
            );
        }
    };

    let dry_run = sub.get_flag("dry-run");
    let wait = sub.get_flag("wait");
    if dry_run && wait {
        return detect::failure_outcome(
            resolved,
            output,
            started,
            detect::usage_error(
                ErrorCode::UnsupportedCombination,
                "--dry-run is unsupported alongside --wait",
            ),
        );
    }

    let source = match read_jsonl_source(sub) {
        Ok(source) => source,
        Err(error) => return detect::failure_outcome(resolved, output, started, error),
    };
    let plan = match plan_from_jsonl(&source, max_billable_units) {
        Ok(plan) => plan,
        Err(error) => return detect::failure_outcome(resolved, output, started, error),
    };
    let request = BulkAnalysisRequest::new(plan);
    let plan_sha256 = request.request_sha256();

    if dry_run {
        return dry_run_outcome(&request, plan_sha256, output, started);
    }

    // Actual submission: only accepted work (a 202) keeps the pipelines. A
    // local observation failure after acceptance changes the local status
    // and exits 1, never a top-level failure envelope.
    let analyzer = match prepare(resolved, root_matches, output, started) {
        Ok(analyzer) => analyzer,
        Err(outcome) => return outcome,
    };

    let runtime = match new_runtime(resolved, output, started) {
        Ok(runtime) => runtime,
        Err(outcome) => return outcome,
    };
    let stop = StopObserving::new();
    detect::install_sigint_driver();
    let progress = resolve_wait_progress(sub, output, streams);
    let wait_mode = wait;
    let result = runtime.block_on(async {
        let bridge = tokio::spawn(detect::bridge_sigint(stop.token().clone()));
        let cancel = stop.token().child_token();
        let outcome = match analyzer.submit_bulk(request, &cancel).await {
            Ok(running) => {
                if wait_mode {
                    let observed = running
                        .observe(
                            WaitOptions::UNBOUNDED,
                            |event| match progress {
                                ProgressMode::Jsonl => emit_bulk_jsonl_progress(event),
                                ProgressMode::Human => emit_bulk_human_progress(event),
                                ProgressMode::Auto | ProgressMode::Quiet => {}
                            },
                            stop.clone(),
                        )
                        .await;
                    Analyzed::Observed(observed)
                } else {
                    Analyzed::Accepted(running)
                }
            }
            Err(failure) => {
                if cancel.is_cancelled() {
                    let identity = bulk_error_identity(&failure);
                    Analyzed::Interrupted(failure.into_error(), identity)
                } else {
                    Analyzed::Failed(failure.into_error())
                }
            }
        };
        bridge.abort();
        outcome
    });
    detect::reset_sigint_flag();
    match result {
        Analyzed::Accepted(running) => submit_accepted_outcome(&running, output, started),
        Analyzed::Observed(Ok(Ok(collection))) => {
            let exit = collection_exit(&collection);
            succeed(CommandData::BulkWait(collection), exit, output, started)
        }
        Analyzed::Observed(Ok(Err(failure))) => {
            let error = failure.into_error();
            observed_failure_outcome(
                resolved,
                ResolvedCommand::BulkWait,
                "the bulk job was accepted but its local observation failed",
                error,
                global,
                streams,
                started,
            )
        }
        Analyzed::Observed(Err(interrupted)) => {
            let note = bulk_identity_note(&interrupted.identity);
            detect::interrupted_outcome(
                ResolvedCommand::BulkWait,
                output,
                started,
                stopped_observation_error(),
                note,
            )
        }
        Analyzed::Interrupted(error, note) => {
            detect::interrupted_outcome(resolved, output, started, error, note)
        }
        Analyzed::Failed(error) => detect::failure_outcome(resolved, output, started, error),
    }
}

/// The intermediate flow states for a bulk submission run. Accepted keeps
/// the running handle for the enqueue projection; Observed carries the
/// observe outcome; Failed/Interrupted carry the canonical error.
enum Analyzed {
    Accepted(crate::analysis::RunningBulk),
    Observed(
        Result<
            Result<BulkCollection, crate::analysis::BulkAnalysisError>,
            crate::analysis::InterruptedBulk,
        >,
    ),
    Interrupted(CanonicalError, String),
    Failed(CanonicalError),
}

/// The stderr identity note for an interrupted submission, derived from the
/// canonical error's reconciliation details where present.
fn bulk_error_identity(failure: &crate::analysis::BulkAnalysisError) -> String {
    match failure.canonical().details() {
        Some(crate::output::CanonicalErrorDetails::SubmissionOutcomeUnknown(details)) => {
            let mut note = "interrupted; local bulk id ".to_owned();
            note.push_str(&match details.operation_id() {
                crate::domain::LocalOperationId::AnalysisId(id) => id.to_string(),
                crate::domain::LocalOperationId::BulkId(id) => id.to_string(),
            });
            note.push_str(&format!("; request sha256 {}", details.request_sha256));
            note.push_str(&format!("; last status {}", details.last_status.as_str()));
            note
        }
        _ => "interrupted during bulk submission; no remote action was completed".to_owned(),
    }
}

/// The stderr identity note for an interrupted observation: local bulk ID,
/// upstream ID when accepted, and the last observed status.
fn bulk_identity_note(identity: &crate::analysis::BulkOperationIdentity) -> String {
    let mut note = format!("interrupted; local bulk id {}", identity.bulk_id);
    if let Some(upstream) = &identity.upstream_bulk_id {
        note.push_str(&format!(
            "; upstream bulk id {}",
            detect::sanitize_for_stderr(upstream.as_str())
        ));
    }
    note
}

/// The canonical local-stop error for a wait-phase cancellation: the job was
/// accepted; local observation stopped without any remote cancellation.
fn stopped_observation_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::NetworkUnavailable,
        "observation was interrupted locally; no remote cancellation was sent",
    )
    .expect("static template")
}

/// A local observation failure after acceptance reports through an accepted
/// status-changed envelope (exit 1), never a top-level failure: the billable
/// acceptance is real, so the caller sees the running collection with the
/// note that local observation degraded (contracts.md 4.8).
fn observed_failure_outcome(
    resolved: ResolvedCommand,
    collection_command: ResolvedCommand,
    note: &str,
    error: CanonicalError,
    global: GlobalFlags,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
) -> DetectOutcome {
    let _ = resolved;
    let _ = collection_command;
    detect::note_stderr(streams, note);
    detect::note_stderr(
        streams,
        &format!(
            "reconcile manually with pangram bulk status; error: {}",
            detect::sanitize_for_stderr(error.message())
        ),
    );
    let _ = global;
    failure_status_envelope(error, streams, started)
}

/// The accepted status-changed envelope: a canonical failure envelope at the
/// collection command with the observation error, exit 1. Success-style
/// envelope assembly cannot fabricate a collection, so the canonical failure
/// envelope is the honest surface for the local observation failure.
fn failure_status_envelope(
    error: CanonicalError,
    _streams: &dyn StreamTty,
    started: UtcTimestamp,
) -> DetectOutcome {
    let exit_code = 1_u8;
    let envelope = CommandEnvelope::failure(
        ResolvedCommand::BulkWait,
        error,
        EnvelopeMeta::default()
            .with_started_at(started)
            .with_failed_at(UtcTimestamp::now()),
    );
    DetectOutcome {
        exit_code,
        envelopes: vec![envelope],
        rendered: false,
    }
}

/// The accepted (enqueued) submission outcome: the running collection at the
/// observed snapshot resolution. With no `--wait`, the adapter records the
/// accept state without observing remotely.
fn submit_accepted_outcome(
    running: &crate::analysis::RunningBulk,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
) -> DetectOutcome {
    let identity = running.identity();
    let plan = running.plan();
    let estimated = plan.map(|plan| plan.estimated_billable_units());
    let total = plan.map(|plan| plan.items().len()).unwrap_or(1);
    let counters =
        crate::domain::BulkCounters::new(u64::try_from(total).unwrap_or(u64::MAX), 0, 0, 0)
            .expect("an all-queued counter set is valid");
    let now = UtcTimestamp::now();
    let collection = match BulkCollection::new(
        identity.bulk_id,
        identity.upstream_bulk_id.clone(),
        AnalysisStatus::Queued,
        SubmissionOutcome::Accepted,
        counters,
        estimated,
        now,
        now,
        None,
    ) {
        Ok(collection) => collection,
        Err(_) => {
            return detect::failure_outcome(
                ResolvedCommand::BulkSubmit,
                output,
                started,
                detect::internal_error("the accepted bulk state could not be projected"),
            );
        }
    };
    succeed(
        CommandData::BulkSubmit(collection),
        ExitCode::Success,
        output,
        started,
    )
}

/// The dry-run outcome: the canonical reconciliation tuple at exit 0 with
/// credentials and network skipped (δ). The JSON default and explicit JSON
/// select the machine shape; any other format declines the composition.
fn dry_run_outcome(
    request: &BulkAnalysisRequest,
    plan_sha256: Sha256Hash,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
) -> DetectOutcome {
    if output.format != OutputFormat::Json {
        return detect::failure_outcome(
            ResolvedCommand::BulkSubmit,
            output,
            started,
            detect::usage_error(
                ErrorCode::UnsupportedCombination,
                "--dry-run renders only the default JSON reconciliation shape",
            ),
        );
    }
    let meta = EnvelopeMeta::default()
        .with_started_at(started)
        .with_duration_ms(detect::elapsed_ms(started));
    let data = serde_json::json!({
        "dry": { "noop": true, "observed": false },
        "bulk_id": request.id().to_string(),
        "plan_sha256": plan_sha256.to_string(),
        "estimated_billable_units": request.plan().estimated_billable_units(),
        "item_count": request.plan().items().len(),
    });
    let envelope = serde_json::json!({
        "schema_version": "1",
        "command": "bulk_submit",
        "data": data,
        "meta": meta,
    });
    let mut stdout = std::io::stdout().lock();
    use std::io::Write as _;
    match serde_json::to_string(&envelope)
        .map_err(std::io::Error::other)
        .and_then(|line| writeln!(stdout, "{line}").and_then(|()| stdout.flush()))
    {
        Ok(()) => DetectOutcome {
            exit_code: ExitCode::Success.as_u8(),
            envelopes: vec![],
            rendered: true,
        },
        Err(_) => detect::failure_outcome(
            ResolvedCommand::BulkSubmit,
            output,
            started,
            detect::internal_error("the dry-run plan could not be rendered honestly"),
        ),
    }
}

// ---------------------------------------------------------------------------
// bulk status / wait
// ---------------------------------------------------------------------------

fn bulk_status(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
) -> DetectOutcome {
    let resolved = ResolvedCommand::BulkStatus;
    let output = resolve_policy(resolved, sub, &global, streams);
    let raw = sub
        .get_one::<String>("ID")
        .map(String::as_str)
        .unwrap_or_default();
    let upstream_id = match parse_upstream_bulk_id(raw) {
        Ok(id) => id,
        Err(error) => return detect::failure_outcome(resolved, output, started, error),
    };
    let analyzer = match prepare(resolved, root_matches, output, started) {
        Ok(analyzer) => analyzer,
        Err(outcome) => return outcome,
    };
    let runtime = match new_runtime(resolved, output, started) {
        Ok(runtime) => runtime,
        Err(outcome) => return outcome,
    };
    let result = runtime.block_on(async {
        let cancel = CancellationToken::new();
        analyzer
            .observe_bulk(upstream_id)
            .snapshot(&cancel, None)
            .await
    });
    match result {
        Ok(collection) => {
            let exit = collection_exit(&collection);
            succeed(CommandData::BulkStatus(collection), exit, output, started)
        }
        Err(failure) => detect::failure_outcome(resolved, output, started, failure.into_error()),
    }
}

fn bulk_wait(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
) -> DetectOutcome {
    let resolved = ResolvedCommand::BulkWait;
    let output = resolve_policy(resolved, sub, &global, streams);
    let raw = sub
        .get_one::<String>("ID")
        .map(String::as_str)
        .unwrap_or_default();
    let upstream_id = match parse_upstream_bulk_id(raw) {
        Ok(id) => id,
        Err(error) => return detect::failure_outcome(resolved, output, started, error),
    };
    let timeout = match resolve_timeout(resolved, sub, started, output) {
        Ok(timeout) => timeout,
        Err(outcome) => return outcome,
    };
    let analyzer = match prepare(resolved, root_matches, output, started) {
        Ok(analyzer) => analyzer,
        Err(outcome) => return outcome,
    };
    let runtime = match new_runtime(resolved, output, started) {
        Ok(runtime) => runtime,
        Err(outcome) => return outcome,
    };
    let progress = resolve_wait_progress(sub, output, streams);
    let options = timeout
        .map(WaitOptions::with_timeout)
        .unwrap_or(WaitOptions::UNBOUNDED);
    let stop = StopObserving::new();
    detect::install_sigint_driver();
    let result = runtime.block_on(async {
        let bridge = tokio::spawn(detect::bridge_sigint(stop.token().clone()));
        let outcome = analyzer
            .observe_bulk(upstream_id)
            .observe(
                options,
                |event| match progress {
                    ProgressMode::Jsonl => emit_bulk_jsonl_progress(event),
                    ProgressMode::Human => emit_bulk_human_progress(event),
                    ProgressMode::Auto | ProgressMode::Quiet => {}
                },
                stop.clone(),
            )
            .await;
        bridge.abort();
        outcome
    });
    detect::reset_sigint_flag();
    match result {
        Ok(Ok(collection)) => {
            let exit = collection_exit(&collection);
            succeed(CommandData::BulkWait(collection), exit, output, started)
        }
        Ok(Err(failure)) => {
            detect::failure_outcome(resolved, output, started, failure.into_error())
        }
        Err(interrupted) => detect::interrupted_outcome(
            resolved,
            output,
            started,
            stopped_observation_error(),
            bulk_identity_note(&interrupted.identity),
        ),
    }
}

/// The shared bulk-wait exit precedence (contracts.md 12): a partial
/// collection exits 3; a failed collection exits 1.
fn collection_exit(collection: &BulkCollection) -> ExitCode {
    ExitCode::for_status(collection.status())
}

// ---------------------------------------------------------------------------
// bulk results
// ---------------------------------------------------------------------------

const BULK_RESULTS_DEFAULT_LIMIT: u64 = 100;
const BULK_RESULTS_MAX_LIMIT: u64 = 1000;
const BULK_RESULTS_FETCH_ALL_MIN_ITEMS: u64 = 10;

fn bulk_results(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
) -> DetectOutcome {
    let resolved = ResolvedCommand::BulkResults;
    let output = resolve_policy(resolved, sub, &global, streams);
    let raw = sub
        .get_one::<String>("ID")
        .map(String::as_str)
        .unwrap_or_default();
    let upstream_id = match parse_upstream_bulk_id(raw) {
        Ok(id) => id,
        Err(error) => return detect::failure_outcome(resolved, output, started, error),
    };
    let offset = match parse_u64_arg(sub, "offset", 0, resolved, output, started) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    let explicit_limit = sub.get_one::<String>("limit").is_some();
    let limit = match parse_u64_arg(
        sub,
        "limit",
        BULK_RESULTS_DEFAULT_LIMIT,
        resolved,
        output,
        started,
    ) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    if !(1..=BULK_RESULTS_MAX_LIMIT).contains(&limit) {
        return detect::failure_outcome(
            resolved,
            output,
            started,
            detect::usage_error(
                ErrorCode::UnsupportedInput,
                "--limit must be within 1..=1000",
            ),
        );
    }

    let analyzer = match prepare(resolved, root_matches, output, started) {
        Ok(analyzer) => analyzer,
        Err(outcome) => return outcome,
    };
    let runtime = match new_runtime(resolved, output, started) {
        Ok(runtime) => runtime,
        Err(outcome) => return outcome,
    };
    let stop = StopObserving::new();
    detect::install_sigint_driver();
    let fetch_all = !explicit_limit && offset == 0;
    let result = runtime.block_on(async {
        let bridge = tokio::spawn(detect::bridge_sigint(stop.token().clone()));
        let cancel = stop.token().child_token();
        let running = analyzer.observe_bulk(upstream_id);
        let outcome = if fetch_all {
            analyzer
                .bulk_results_all(&running, BULK_RESULTS_FETCH_ALL_MIN_ITEMS, &cancel, |_| {})
                .await
        } else {
            analyzer
                .bulk_results_page(&running, offset, limit, &cancel)
                .await
        };
        bridge.abort();
        outcome
    });
    detect::reset_sigint_flag();
    match result {
        Ok(page) => succeed_page(page, fetch_all, offset, limit, output, started, resolved),
        Err(failure) => {
            let error = failure.into_error();
            if matches!(error.code(), ErrorCode::BulkLimitExceeded)
                && error.message().contains("unavailable until terminal")
            {
                detect::note_stderr(
                    streams,
                    "bulk results become readable only after the job is fully collected",
                );
            }
            if matches!(error.code(), ErrorCode::UpstreamNotFound) {
                detect::note_stderr(
                    streams,
                    "Pangram does not recognize the bulk job; check the ID",
                );
            }
            detect::failure_outcome(resolved, output, started, error)
        }
    }
}

/// Projects one bulk results page: a terminal read returns the canonical
/// page (exit 0 for a single page; the fetch-all composition reassembles one
/// canonical page shape through the domain owner).
fn succeed_page(
    page_result: crate::analysis::BulkPageResult,
    fetch_all: bool,
    offset: u64,
    limit: u64,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
    resolved: ResolvedCommand,
) -> DetectOutcome {
    let page = if fetch_all {
        let count = page_result.page.items().len();
        match BulkPage::new(
            page_result.page.items().to_vec(),
            0,
            u64::try_from(count.max(1)).unwrap_or(1),
            None,
        ) {
            Ok(page) => page,
            Err(_) => {
                return detect::failure_outcome(
                    resolved,
                    output,
                    started,
                    detect::internal_error("the fetched bulk results could not be reassembled"),
                );
            }
        }
    } else {
        page_result.page
    };
    let _ = (offset, limit);
    succeed(
        CommandData::BulkResults(page),
        ExitCode::Success,
        output,
        started,
    )
}

/// A numeric flag with the shared usage-error surface; `default` applies
/// when the flag is absent.
fn parse_u64_arg(
    sub: &ArgMatches,
    name: &str,
    default: u64,
    resolved: ResolvedCommand,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
) -> Result<u64, DetectOutcome> {
    match sub.get_one::<String>(name) {
        Some(raw) => raw.parse::<u64>().map_err(|_| {
            detect::failure_outcome(
                resolved,
                output,
                started,
                detect::usage_error(
                    ErrorCode::UnsupportedInput,
                    &format!("--{name} must be a decimal integer"),
                ),
            )
        }),
        None => Ok(default),
    }
}

// ---------------------------------------------------------------------------
// task status / wait
// ---------------------------------------------------------------------------

fn task_status(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
) -> DetectOutcome {
    let resolved = ResolvedCommand::TaskStatus;
    let output = resolve_policy(resolved, sub, &global, streams);
    let raw = sub
        .get_one::<String>("ID")
        .map(String::as_str)
        .unwrap_or_default();
    let task_id = match parse_upstream_task_id(raw) {
        Ok(id) => id,
        Err(error) => return detect::failure_outcome(resolved, output, started, error),
    };
    let analyzer = match prepare(resolved, root_matches, output, started) {
        Ok(analyzer) => analyzer,
        Err(outcome) => return outcome,
    };
    let runtime = match new_runtime(resolved, output, started) {
        Ok(runtime) => runtime,
        Err(outcome) => return outcome,
    };
    let result = runtime.block_on(async {
        let cancel = CancellationToken::new();
        analyzer.task_status(&task_id, &cancel).await
    });
    match result {
        Ok(analysis) => {
            let exit = task_exit(&analysis);
            succeed(CommandData::TaskStatus(analysis), exit, output, started)
        }
        Err(failure) => detect::failure_outcome(resolved, output, started, failure.into_error()),
    }
}

fn task_wait(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
) -> DetectOutcome {
    let resolved = ResolvedCommand::TaskWait;
    let output = resolve_policy(resolved, sub, &global, streams);
    let raw = sub
        .get_one::<String>("ID")
        .map(String::as_str)
        .unwrap_or_default();
    let task_id = match parse_upstream_task_id(raw) {
        Ok(id) => id,
        Err(error) => return detect::failure_outcome(resolved, output, started, error),
    };
    let timeout = match resolve_timeout(resolved, sub, started, output) {
        Ok(timeout) => timeout,
        Err(outcome) => return outcome,
    };
    let analyzer = match prepare(resolved, root_matches, output, started) {
        Ok(analyzer) => analyzer,
        Err(outcome) => return outcome,
    };
    let runtime = match new_runtime(resolved, output, started) {
        Ok(runtime) => runtime,
        Err(outcome) => return outcome,
    };
    let progress = resolve_wait_progress(sub, output, streams);
    let options = timeout
        .map(WaitOptions::with_timeout)
        .unwrap_or(WaitOptions::UNBOUNDED);
    let stop = StopObserving::new();
    detect::install_sigint_driver();
    let result = runtime.block_on(async {
        let bridge = tokio::spawn(detect::bridge_sigint(stop.token().clone()));
        let cancel = stop.token().child_token();
        let outcome = analyzer
            .task_wait(
                &task_id,
                options,
                |event| match progress {
                    ProgressMode::Jsonl => emit_task_jsonl_progress(event),
                    ProgressMode::Human => emit_task_human_progress(event),
                    ProgressMode::Auto | ProgressMode::Quiet => {}
                },
                stop.clone(),
                &cancel,
            )
            .await;
        bridge.abort();
        outcome
    });
    detect::reset_sigint_flag();
    match result {
        Ok(analysis) => {
            let exit = task_exit(&analysis);
            succeed(CommandData::TaskWait(analysis), exit, output, started)
        }
        Err(failure) => {
            let error = failure.into_error();
            if matches!(error.code(), ErrorCode::NetworkUnavailable) && stop.token().is_cancelled()
            {
                let note = format!("interrupted; upstream task id {}", task_id);
                return detect::interrupted_outcome(resolved, output, started, error, note);
            }
            detect::failure_outcome(resolved, output, started, error)
        }
    }
}

/// The task surface exit (contracts.md 12 for task_status/task_wait): a
/// partial analysis exits 3; a failed analysis exits 1; every other status
/// stays 0.
fn task_exit(analysis: &Analysis<CanonicalError>) -> ExitCode {
    ExitCode::for_status(analysis.status())
}

/// One canonical JSONL analysis progress event on stderr.
fn emit_task_jsonl_progress(progress: &crate::analysis::AnalysisProgress) {
    let observed = UtcTimestamp::now();
    if let Ok(event) = ProgressEvent::analysis(
        progress.analysis_id,
        crate::domain::CheckKind::AiDetection,
        crate::domain::CheckStatus::Running,
        observed,
    )
    .with_upstream_stage(progress.last_stage.as_str())
    {
        if let Ok(line) = serde_json::to_string(&event) {
            eprintln!("{line}");
        }
    }
}

/// One sanitized human task-progress line on stderr: IDs and the upstream
/// stage token only.
fn emit_task_human_progress(progress: &crate::analysis::AnalysisProgress) {
    eprintln!(
        "task {}: running ({})",
        detect::sanitize_for_stderr(progress.task_id.as_str()),
        detect::sanitize_for_stderr(progress.last_stage.as_str()),
    );
}

// ---------------------------------------------------------------------------
// shared runtime
// ---------------------------------------------------------------------------

fn new_runtime(
    resolved: ResolvedCommand,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
) -> Result<tokio::runtime::Runtime, DetectOutcome> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            detect::failure_outcome(
                resolved,
                output,
                started,
                detect::internal_error("could not start the local async runtime"),
            )
        })
}
