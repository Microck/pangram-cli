//! The `bulk status` and `bulk wait` observation flows. A status read takes
//! one safe snapshot; a wait loops the shared observation until terminal,
//! the local timeout, or interruption. The collection exit derives from the
//! canonical collection state (contracts.md 12), never a blanket default.

use clap::ArgMatches;
use tokio_util::sync::CancellationToken;

use crate::analysis::{StopObserving, WaitOptions};
use crate::cli::StreamTty;
use crate::cli::detect::{self, DetectOutcome, GlobalFlags, ProgressMode};
use crate::domain::{AnalysisStatus, BulkCollection, UtcTimestamp};
use crate::output::{CommandData, ExitCode, ResolvedCommand};

use super::plan::parse_upstream_bulk_id;
use super::policy::{resolve_policy, resolve_timeout, resolve_wait_progress};
use super::{new_runtime, prepare, succeed};

pub(super) fn bulk_status(
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
    let (analyzer, service) = match prepare(resolved, root_matches, output, started) {
        Ok(prepared) => prepared,
        Err(outcome) => return outcome,
    };
    let runtime = match new_runtime(resolved, output, started) {
        Ok(runtime) => runtime,
        Err(outcome) => return outcome,
    };
    // The one invocation warning latch, shared across the observed-children
    // read phase and the persist phase below (contracts.md 14.2 note).
    let mut bulk_warning = detect::save::BulkSaveWarning::new();
    let result = runtime.block_on(async {
        let cancel = CancellationToken::new();
        let running = analyzer.observe_bulk(upstream_id);
        let observed_running = running.clone();
        let collection = running.snapshot(&cancel, None).await;
        // Refresh the same stored children (contracts.md 14.2): the status
        // read fetched the counters; the children come from the documented
        // results window. A save-read failure never degrades the status
        // read: the children best-effort fallback keeps the refresh honest
        // (an empty window refreshes counters without child rows), and its
        // one sanitized automatic warning surfaces below.
        let children = if detect::save::automatic_history_armed(&service) && collection.is_ok() {
            // Upstream children of a job this process did not submit
            // carry no locally held source name.
            analyzer
                .bulk_observed_children(&observed_running, None, &cancel)
                .await
                .map_err(|_| ())
        } else {
            Ok(Vec::new())
        };
        (collection, children)
    });
    let (result, children) = result;
    let (children, children_read_failed) = match children {
        Ok(children) => (children, false),
        Err(()) => (Vec::new(), true),
    };
    match result {
        Ok(collection) => {
            let exit = collection_exit(&collection);
            if children_read_failed {
                // The read phase failed first: it owns the one invocation
                // warning; the persist phase below flows through the same
                // latch and stays silent on its own failure.
                *bulk_warning.latch() = true;
                detect::warning_stderr(
                    streams,
                    "automatic history save failed (the observed bulk children could not be read)",
                );
            }
            let (collection, _) = detect::save::persist_bulk_collection(
                &collection,
                children,
                &service,
                &mut bulk_warning,
            );
            succeed(
                resolved,
                CommandData::BulkStatus(collection),
                exit,
                output,
                started,
            )
        }
        Err(failure) => detect::failure_outcome(resolved, output, started, failure.into_error()),
    }
}

pub(super) fn bulk_wait(
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
    let (analyzer, service) = match prepare(resolved, root_matches, output, started) {
        Ok(prepared) => prepared,
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
    // The one invocation warning latch, shared across phases (contracts.md
    // 14.2 note).
    let mut bulk_warning = detect::save::BulkSaveWarning::new();
    let result = runtime.block_on(async {
        let bridge = tokio::spawn(detect::bridge_sigint(stop.token().clone()));
        let running = analyzer.observe_bulk(upstream_id);
        let observed_running = running.clone();
        let outcome = running
            .observe(
                options,
                |event| match progress {
                    ProgressMode::Jsonl => super::emit_bulk_jsonl_progress(event),
                    ProgressMode::Human => super::emit_bulk_human_progress(event),
                    ProgressMode::Auto | ProgressMode::Quiet => {}
                },
                stop.clone(),
            )
            .await;
        let cancel = stop.token().child_token();
        let children =
            if detect::save::automatic_history_armed(&service) && matches!(outcome, Ok(Ok(_))) {
                analyzer
                    .bulk_observed_children(&observed_running, None, &cancel)
                    .await
                    .map_err(|_| ())
            } else {
                Ok(Vec::new())
            };
        bridge.abort();
        (outcome, children)
    });
    detect::reset_sigint_flag();
    let (result, children) = result;
    let (children, children_read_failed) = match children {
        Ok(children) => (children, false),
        Err(()) => (Vec::new(), true),
    };
    match result {
        Ok(Ok(collection)) => {
            let exit = collection_exit(&collection);
            if children_read_failed {
                // The read phase failed first: it owns the one invocation
                // warning; the persist phase below flows through the same
                // latch and stays silent on its own failure.
                *bulk_warning.latch() = true;
                detect::warning_stderr(
                    streams,
                    "automatic history save failed (the observed bulk children could not be read)",
                );
            }
            let (collection, _) = detect::save::persist_bulk_collection(
                &collection,
                children,
                &service,
                &mut bulk_warning,
            );
            succeed(
                resolved,
                CommandData::BulkWait(collection),
                exit,
                output,
                started,
            )
        }
        Ok(Err(failure)) => {
            detect::failure_outcome(resolved, output, started, failure.into_error())
        }
        Err(interrupted) => detect::interrupted_outcome(
            resolved,
            output,
            started,
            super::submit::stopped_observation_error(),
            super::submit::bulk_identity_note(&interrupted.identity),
        ),
    }
}

/// The shared bulk collection exit precedence (contracts.md 12): a partial
/// collection exits 3; a terminal failed collection failed every item through
/// an upstream terminal analysis failure and exits 6 (the upstream category);
/// every other state exits 0.
pub(super) fn collection_exit(collection: &BulkCollection) -> ExitCode {
    match collection.status() {
        AnalysisStatus::Partial => ExitCode::Partial,
        AnalysisStatus::Failed => ExitCode::NetworkOrUpstream,
        AnalysisStatus::Queued | AnalysisStatus::Running | AnalysisStatus::Succeeded => {
            ExitCode::Success
        }
    }
}
