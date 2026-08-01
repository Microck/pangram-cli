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
//!
//! The cohesive halves own separate seams: [`policy`] resolves rendering,
//! timeout, and numeric flags; [`plan`] reads and validates the JSONL source
//! and upstream IDs; [`submit`] runs the submit/dry-run flow;
//! [`status_wait`] runs the bulk status/wait observation; [`results`] runs
//! the paged read; and [`task`] runs the task status/wait flow. This module
//! keeps the dispatch, the one success-envelope handoff, the shared async
//! runtime, the analyzer preparation, and the wait progress emitters.

use clap::ArgMatches;

use crate::analysis::Analyzer;
use crate::cli::StreamTty;
use crate::cli::detect::{self, DetectOutcome, GlobalFlags};
use crate::domain::{AnalysisStatus, UtcTimestamp};
use crate::output::{
    CommandData, CommandEnvelope, EnvelopeMeta, ExitCode, ProgressEvent, ResolvedCommand,
};

mod plan;
mod policy;
mod results;
mod status_wait;
mod submit;
mod task;

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
        ResolvedCommand::BulkSubmit => {
            submit::bulk_submit(sub, root_matches, global, streams, started)
        }
        ResolvedCommand::BulkStatus => {
            status_wait::bulk_status(sub, root_matches, global, streams, started)
        }
        ResolvedCommand::BulkWait => {
            status_wait::bulk_wait(sub, root_matches, global, streams, started)
        }
        ResolvedCommand::BulkResults => {
            results::bulk_results(sub, root_matches, global, streams, started)
        }
        ResolvedCommand::TaskStatus => {
            task::task_status(sub, root_matches, global, streams, started)
        }
        ResolvedCommand::TaskWait => task::task_wait(sub, root_matches, global, streams, started),
        _ => unreachable!("dispatch only routes bulk and task commands here"),
    }
}

/// Builds configuration, credentials, and the analyzer for a bulk/task
/// request. Shared with detection through the process-owned preparation.
pub(super) fn prepare(
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
pub(super) fn succeed(
    command: ResolvedCommand,
    data: CommandData,
    exit_code: ExitCode,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
) -> DetectOutcome {
    let meta = EnvelopeMeta::default()
        .with_started_at(started)
        .with_duration_ms(detect::elapsed_ms(started));
    let envelope = CommandEnvelope::success(data, meta);
    detect::primary_outcome(command, &envelope, output, exit_code.as_u8(), started)
}

/// Builds the single-threaded async runtime one bulk/task invocation blocks
/// on. Construction failure is an honest internal failure, never a panic.
pub(super) fn new_runtime(
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

/// Emits one canonical JSON bulk progress line on stderr for `--progress
/// jsonl`; the adapter counter snapshot flows into the shared event type so
/// the schema stays output-owned.
pub(super) fn emit_bulk_jsonl_progress(progress: &crate::analysis::BulkProgress) {
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
pub(super) fn emit_bulk_human_progress(progress: &crate::analysis::BulkProgress) {
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
