//! The `task status` and `task wait` flows. Both observe one Pangram 4 task
//! by its explicit upstream ID or by a saved local `anl_` ID resolved through
//! canonical history evidence before credentials/network (contracts.md 4.6).
//! A status read is one safe snapshot; a wait loops until terminal, the local
//! timeout, or interruption. The analysis exit derives from the canonical
//! terminal check error's category, exactly as detection (contracts.md 12).

use clap::ArgMatches;
use tokio_util::sync::CancellationToken;

use crate::analysis::{StopObserving, WaitOptions};
use crate::cli::StreamTty;
use crate::cli::detect::{self, DetectOutcome, GlobalFlags, ProgressMode};
use crate::domain::{Analysis, AnalysisId, UpstreamTaskId, UtcTimestamp};
use crate::history::{HistoryErrorCode, HistoryStore};
use crate::output::{
    CanonicalError, CommandData, ErrorCode, ExitCode, ProgressEvent, ResolvedCommand,
};

use super::plan::parse_upstream_task_id;
use super::policy::{resolve_policy, resolve_timeout, resolve_wait_progress};
use super::{new_runtime, prepare, prepare_from_service, prepare_service, succeed};

pub(super) fn task_status(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
    analyzer_source: &crate::analysis::AnalyzerSource,
) -> DetectOutcome {
    let resolved = ResolvedCommand::TaskStatus;
    let output = resolve_policy(resolved, sub, &global, streams);
    let raw = sub
        .get_one::<String>("ID")
        .map(String::as_str)
        .unwrap_or_default();
    let (task_id, analyzer, service) = match prepare_task_identity(
        raw,
        resolved,
        root_matches,
        output,
        started,
        analyzer_source,
    ) {
        Ok(prepared) => prepared,
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
            let analysis = persist_observed_task(analysis, &service);
            succeed(
                resolved,
                CommandData::TaskStatus(analysis),
                exit,
                output,
                started,
            )
        }
        Err(failure) => detect::failure_outcome(resolved, output, started, failure.into_error()),
    }
}

pub(super) fn task_wait(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
    analyzer_source: &crate::analysis::AnalyzerSource,
) -> DetectOutcome {
    let resolved = ResolvedCommand::TaskWait;
    let output = resolve_policy(resolved, sub, &global, streams);
    let raw = sub
        .get_one::<String>("ID")
        .map(String::as_str)
        .unwrap_or_default();
    let timeout = match resolve_timeout(resolved, sub, started, output) {
        Ok(timeout) => timeout,
        Err(outcome) => return outcome,
    };
    let (task_id, analyzer, service) = match prepare_task_identity(
        raw,
        resolved,
        root_matches,
        output,
        started,
        analyzer_source,
    ) {
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
            let analysis = persist_observed_task(analysis, &service);
            succeed(
                resolved,
                CommandData::TaskWait(analysis),
                exit,
                output,
                started,
            )
        }
        Err(failure) => {
            let error = failure.into_error();
            if matches!(error.code(), ErrorCode::NetworkUnavailable) && stop.token().is_cancelled()
            {
                let note = format!(
                    "interrupted; upstream task id {}",
                    detect::sanitize_for_stderr(task_id.as_str())
                );
                return detect::interrupted_outcome(resolved, output, started, error, note);
            }
            detect::failure_outcome(resolved, output, started, error)
        }
    }
}

/// Resolves an opaque upstream task ID unchanged, or a canonical local
/// analysis ID through an existing validated history database. The local
/// branch deliberately splits service construction from credential/analyzer
/// preparation so every unresolvable/corrupt record fails before auth or
/// network work and a missing database is never created.
fn prepare_task_identity(
    raw: &str,
    resolved: ResolvedCommand,
    root_matches: &ArgMatches,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
    analyzer_source: &crate::analysis::AnalyzerSource,
) -> Result<
    (
        UpstreamTaskId,
        crate::analysis::Analyzer,
        crate::config::ConfigService,
    ),
    DetectOutcome,
> {
    let Ok(local_id) = raw.parse::<AnalysisId>() else {
        let task_id = parse_upstream_task_id(raw)
            .map_err(|error| detect::failure_outcome(resolved, output, started, error))?;
        let (analyzer, service) =
            prepare(resolved, root_matches, output, started, analyzer_source)?;
        return Ok((task_id, analyzer, service));
    };

    let service = prepare_service(resolved, root_matches, output, started)?;
    let store = HistoryStore::open_existing(service.paths().data_dir())
        .map_err(|error| {
            detect::failure_outcome(resolved, output, started, error.into_canonical())
        })?
        .ok_or_else(|| {
            detect::failure_outcome(resolved, output, started, local_task_unresolvable())
        })?;
    let task_id = store
        .resolve_analysis_task(&local_id)
        .map_err(|error| {
            let error = if error.code() == HistoryErrorCode::NotFound {
                local_task_unresolvable()
            } else {
                error.into_canonical()
            };
            detect::failure_outcome(resolved, output, started, error)
        })?
        .ok_or_else(|| {
            detect::failure_outcome(resolved, output, started, local_task_unresolvable())
        })?;
    drop(store);
    let (analyzer, service) =
        prepare_from_service(resolved, service, output, started, analyzer_source)?;
    Ok((task_id, analyzer, service))
}

fn local_task_unresolvable() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::LocalTaskUnresolvable,
        "The saved analysis does not resolve to exactly one upstream task.",
    )
    .expect("static error")
}

/// The task surface exit (contracts.md 12 for task_status/task_wait): the
/// same category-derived precedence as detection. A partial analysis exits 3;
/// a failed analysis exits per its terminal check error's category (an
/// upstream `STAGE_FAILED` is `upstream_analysis_failed`, exit 6); every
/// other status stays 0.
fn task_exit(analysis: &Analysis<CanonicalError>) -> ExitCode {
    detect::analysis_exit_code(analysis)
}

/// Persists one observed task analysis under the automatic history gate,
/// then hands the analysis back for rendering with its honest save state.
/// Only the contracted `history.enabled = true` path applies (the task
/// surface carries no `--save`); a failure warns once and never degrades
/// the read, and a repeated observation of the same remote task refreshes
/// its one row rather than duplicating it (contracts.md 14.2 note).
fn persist_observed_task(
    analysis: Analysis<CanonicalError>,
    service: &crate::config::ConfigService,
) -> Analysis<CanonicalError> {
    detect::save::persist_observed_analysis(analysis, service)
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
        && let Ok(line) = serde_json::to_string(&event)
    {
        eprintln!("{line}");
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
