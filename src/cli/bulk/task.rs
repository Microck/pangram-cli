//! The `task status` and `task wait` flows. Both observe one Pangram 4 task
//! by its explicit upstream ID, reconciling a remotely authored record
//! honestly (contracts.md 4.6). A status read is one safe snapshot; a wait
//! loops until terminal, the local timeout, or interruption. The analysis
//! exit derives from the canonical terminal check error's category, exactly
//! as detection (contracts.md 12).

use clap::ArgMatches;
use tokio_util::sync::CancellationToken;

use crate::analysis::{StopObserving, WaitOptions};
use crate::cli::StreamTty;
use crate::cli::detect::{self, DetectOutcome, GlobalFlags, ProgressMode};
use crate::domain::{Analysis, UtcTimestamp};
use crate::output::{
    CanonicalError, CommandData, ErrorCode, ExitCode, ProgressEvent, ResolvedCommand,
};

use super::plan::parse_upstream_task_id;
use super::policy::{resolve_policy, resolve_timeout, resolve_wait_progress};
use super::{new_runtime, prepare, succeed};

pub(super) fn task_status(
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

pub(super) fn task_wait(
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

/// The task surface exit (contracts.md 12 for task_status/task_wait): the
/// same category-derived precedence as detection. A partial analysis exits 3;
/// a failed analysis exits per its terminal check error's category (an
/// upstream `STAGE_FAILED` is `upstream_analysis_failed`, exit 6); every
/// other status stays 0.
fn task_exit(analysis: &Analysis<CanonicalError>) -> ExitCode {
    detect::analysis_exit_code(analysis)
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
