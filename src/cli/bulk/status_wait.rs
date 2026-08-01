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
                    ProgressMode::Jsonl => super::emit_bulk_jsonl_progress(event),
                    ProgressMode::Human => super::emit_bulk_human_progress(event),
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
