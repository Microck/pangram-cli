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
        // read, but it leaves the entire collection-plus-members persistence
        // unit untouched and surfaces one sanitized automatic warning below.
        let children = if detect::save::automatic_history_armed(&service) && collection.is_ok() {
            // Upstream children of a job this process did not submit
            // carry no locally held source name.
            Some(
                analyzer
                    .bulk_observed_children(&observed_running, None, &cancel)
                    .await
                    .map_err(|_| ()),
            )
        } else {
            None
        };
        (collection, children)
    });
    let (result, children) = result;
    match result {
        Ok(collection) => {
            let exit = collection_exit(&collection);
            let collection = persist_observed_collection(
                collection,
                children,
                &service,
                &mut bulk_warning,
                streams,
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
                Some(
                    analyzer
                        .bulk_observed_children(&observed_running, None, &cancel)
                        .await
                        .map_err(|_| ()),
                )
            } else {
                None
            };
        bridge.abort();
        (outcome, children)
    });
    detect::reset_sigint_flag();
    let (result, children) = result;
    match result {
        Ok(Ok(collection)) => {
            let exit = collection_exit(&collection);
            let collection = persist_observed_collection(
                collection,
                children,
                &service,
                &mut bulk_warning,
                streams,
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

/// Persists one bulk observation only when its complete child window was
/// retrieved. A failed child read means the collection-plus-members unit is
/// uncertified, so opening history would create a misleading memberless row.
/// The primary status/wait result remains successful and the read phase owns
/// the invocation's one automatic-history warning.
fn persist_observed_collection(
    collection: BulkCollection,
    children: Option<Result<Vec<detect::save::BulkChild>, ()>>,
    service: &crate::config::ConfigService,
    warning: &mut detect::save::BulkSaveWarning,
    streams: &dyn StreamTty,
) -> BulkCollection {
    match children {
        Some(Ok(children)) => {
            detect::save::persist_bulk_collection(&collection, children, service, warning).0
        }
        Some(Err(())) => {
            *warning.latch() = true;
            detect::warning_stderr(
                streams,
                "automatic history save failed (the observed bulk children could not be read)",
            );
            collection
        }
        // The first gate read was off, so no complete child window exists.
        // A concurrent enable before the persistence-time gate check must
        // not turn that skipped read into a memberless collection save.
        None => collection,
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::config::{ConfigOverrides, ConfigService, Paths};
    use crate::domain::{BulkCounters, BulkId, SubmissionOutcome, UpstreamBulkId};

    #[test]
    fn skipped_child_read_stays_unpersisted_if_history_becomes_enabled() {
        let root = tempfile::tempdir().unwrap();
        let config_dir = root.path().join("config");
        let data_dir = root.path().join("data");
        let service = ConfigService::for_test(
            Paths::for_test(config_dir, data_dir.clone()),
            ConfigOverrides::default(),
        );
        let timestamp = UtcTimestamp::from_str("2026-08-09T00:00:00Z").unwrap();
        let collection = BulkCollection::new(
            BulkId::new(),
            Some(UpstreamBulkId::new("blk-gate-race").unwrap()),
            AnalysisStatus::Succeeded,
            SubmissionOutcome::Accepted,
            BulkCounters::new(1, 1, 1, 0).unwrap(),
            None,
            timestamp,
            timestamp,
            Some(timestamp),
        )
        .unwrap();

        // This enable occurs after the earlier gate read chose not to fetch
        // children. The later persistence gate must not reinterpret that
        // skipped read as a certified empty window.
        service.set("history.enabled", "true").unwrap();
        let persisted = persist_observed_collection(
            collection.clone(),
            None,
            &service,
            &mut detect::save::BulkSaveWarning::new(),
            &crate::cli::RealStreams,
        );

        assert_eq!(persisted, collection);
        assert!(
            !data_dir.exists(),
            "a skipped child read must never create history after a concurrent enable"
        );
    }
}
