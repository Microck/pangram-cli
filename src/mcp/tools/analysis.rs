//! Text submission and task observation MCP tools.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::analysis::{Accepted, AnalysisRequest, StopObserving, WaitOptions};
use crate::domain::{AnalysisId, SaveState, TextOrigin, UpstreamTaskId, UtcTimestamp};
use crate::history::{HistoryError, HistoryErrorCode, HistoryStore};
use crate::output::{CommandData, ErrorCode, ResolvedCommand};

use super::{
    ToolCallContext, ToolCallOutcome, blocking_operation_error, canonical_error, failure,
    invalid_arguments, resolve_analyzer, success,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DetectTextArgs {
    text: String,
    max_billable_units: u64,
    #[serde(default)]
    save: bool,
    #[serde(default)]
    public_link: bool,
    #[serde(default)]
    include_input: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskArgs {
    analysis_id: Option<AnalysisId>,
    upstream_task_id: Option<UpstreamTaskId>,
    timeout_ms: Option<u64>,
}

pub(super) async fn detect_text(
    context: &ToolCallContext<'_>,
    arguments: Map<String, Value>,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    let Ok(arguments) = serde_json::from_value::<DetectTextArgs>(Value::Object(arguments)) else {
        return invalid_arguments();
    };
    let Some(word_count) = AnalysisRequest::eligible_text_word_count(&arguments.text) else {
        return invalid_arguments();
    };
    if arguments.max_billable_units == 0 {
        return invalid_arguments();
    }
    if crate::domain::text_billable_units(word_count) > arguments.max_billable_units {
        return failure(
            ResolvedCommand::Detect,
            canonical_error(
                ErrorCode::UnsupportedInput,
                "the estimated billable units exceed max_billable_units",
            ),
            started,
        );
    }
    if arguments.public_link && !context.options().allow_public_links {
        return failure(
            ResolvedCommand::Detect,
            canonical_error(
                ErrorCode::McpCapabilityRequired,
                "public links require --allow-public-links",
            ),
            started,
        );
    }
    if arguments.save && !context.history_mutations_enabled() {
        return failure(
            ResolvedCommand::Detect,
            canonical_error(
                ErrorCode::McpCapabilityRequired,
                "saving requires --history and --allow-history-mutations",
            ),
            started,
        );
    }

    let analyzer = match resolve_analyzer(context).await {
        Ok(analyzer) => analyzer,
        Err(error) => return failure(ResolvedCommand::Detect, *error, started),
    };
    if context.cancellation().is_cancelled() {
        return ToolCallOutcome::Cancelled { diagnostic: None };
    }
    let retained_text = arguments.save.then(|| arguments.text.clone());
    let request = AnalysisRequest::new(
        arguments.text,
        TextOrigin::Literal,
        None,
        word_count,
        arguments.include_input,
        arguments.public_link,
    );
    let accepted = match analyzer.start_full(request, context.cancellation()).await {
        Ok(accepted) => accepted,
        Err(submission) => {
            if context.cancellation().is_cancelled() {
                return ToolCallOutcome::Cancelled {
                    diagnostic: Some(format!(
                        "cancelled local observation for analysis {}",
                        submission.task_error.analysis_id()
                    )),
                };
            }
            return failure(
                ResolvedCommand::Detect,
                submission.task_error.into_error(),
                started,
            );
        }
    };

    let analysis = match accepted {
        Accepted::Terminal(analysis) => *analysis,
        Accepted::Task(accepted) => {
            let running = analyzer.running(accepted);
            let identity = running.identity();
            let observation = running.observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new());
            let observed = tokio::select! {
                result = observation => Some(result),
                () = context.cancellation().cancelled() => None,
            };
            match observed {
                None => {
                    return ToolCallOutcome::Cancelled {
                        diagnostic: Some(task_diagnostic("analysis", &identity)),
                    };
                }
                Some(Ok(Ok(analysis))) => analysis,
                Some(Ok(Err(error))) => {
                    return failure(ResolvedCommand::Detect, error.into_error(), started);
                }
                Some(Err(_)) => {
                    return ToolCallOutcome::Cancelled {
                        diagnostic: Some(task_diagnostic("analysis", &identity)),
                    };
                }
            }
        }
    };

    let analysis = if arguments.save {
        let data_dir = context.service().paths().data_dir().to_path_buf();
        match tokio::task::spawn_blocking(move || {
            let mut store = HistoryStore::open(&data_dir)?;
            crate::history::save_complete_analysis(
                &mut store,
                &analysis,
                SaveState::SavedManual,
                retained_text.as_deref(),
            )?;
            Ok::<_, HistoryError>(analysis.with_save_state(SaveState::SavedManual))
        })
        .await
        {
            Ok(Ok(analysis)) => analysis,
            Ok(Err(error)) => {
                return failure(ResolvedCommand::Detect, error.into_canonical(), started);
            }
            Err(_) => {
                return failure(ResolvedCommand::Detect, blocking_operation_error(), started);
            }
        }
    } else {
        analysis
    };

    let id = analysis.id.to_string();
    success(
        CommandData::Detect(crate::output::AnalysisOutput::one(analysis)),
        format!("analysis {id} completed"),
        started,
    )
}

#[derive(Clone, Copy)]
pub(super) enum TaskOperation {
    Get,
    Wait,
}

impl TaskOperation {
    const fn command(self) -> ResolvedCommand {
        match self {
            Self::Get => ResolvedCommand::TaskStatus,
            Self::Wait => ResolvedCommand::TaskWait,
        }
    }
}

pub(super) async fn task(
    context: &ToolCallContext<'_>,
    arguments: Map<String, Value>,
    started: UtcTimestamp,
    operation: TaskOperation,
) -> ToolCallOutcome {
    let Ok(arguments) = serde_json::from_value::<TaskArgs>(Value::Object(arguments)) else {
        return invalid_arguments();
    };
    if arguments.analysis_id.is_some() == arguments.upstream_task_id.is_some()
        || (matches!(operation, TaskOperation::Get) && arguments.timeout_ms.is_some())
        || arguments.timeout_ms == Some(0)
    {
        return invalid_arguments();
    }
    let command = operation.command();
    let task_id =
        match task_identity(context, arguments.analysis_id, arguments.upstream_task_id).await {
            Ok(task_id) => task_id,
            Err(error) => return failure(command, *error, started),
        };
    let analyzer = match resolve_analyzer(context).await {
        Ok(analyzer) => analyzer,
        Err(error) => return failure(command, *error, started),
    };
    if context.cancellation().is_cancelled() {
        return ToolCallOutcome::Cancelled { diagnostic: None };
    }
    let result = if matches!(operation, TaskOperation::Wait) {
        let options = arguments
            .timeout_ms
            .map(|timeout| WaitOptions::with_timeout(std::time::Duration::from_millis(timeout)))
            .unwrap_or(WaitOptions::UNBOUNDED);
        analyzer
            .task_wait(
                &task_id,
                options,
                |_| {},
                StopObserving::new(),
                context.cancellation(),
            )
            .await
    } else {
        analyzer.task_status(&task_id, context.cancellation()).await
    };

    let analysis = match result {
        Ok(analysis) => analysis,
        Err(_) if context.cancellation().is_cancelled() => {
            return ToolCallOutcome::Cancelled {
                diagnostic: Some(format!(
                    "cancelled local observation for upstream task {}",
                    task_id.as_str()
                )),
            };
        }
        Err(error) => return failure(command, error.into_error(), started),
    };
    let id = analysis.id.to_string();
    let data = match operation {
        TaskOperation::Get => CommandData::TaskStatus(analysis),
        TaskOperation::Wait => CommandData::TaskWait(analysis),
    };
    success(data, format!("task observation {id} completed"), started)
}

async fn task_identity(
    context: &ToolCallContext<'_>,
    analysis_id: Option<AnalysisId>,
    upstream_task_id: Option<UpstreamTaskId>,
) -> Result<UpstreamTaskId, Box<crate::output::CanonicalError>> {
    if let Some(task_id) = upstream_task_id {
        return Ok(task_id);
    }
    if !context.options().history {
        return Err(Box::new(canonical_error(
            ErrorCode::McpCapabilityRequired,
            "local analysis IDs require --history",
        )));
    }
    let Some(analysis_id) = analysis_id else {
        return Err(Box::new(canonical_error(
            ErrorCode::LocalTaskUnresolvable,
            "the saved analysis does not resolve to one upstream task",
        )));
    };
    let data_dir = context.service().paths().data_dir().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let Some(store) = HistoryStore::open_existing(&data_dir)
            .map_err(|error| Box::new(error.into_canonical()))?
        else {
            return Err(Box::new(local_task_unresolvable()));
        };
        store
            .resolve_analysis_task(&analysis_id)
            .map_err(|error| {
                Box::new(match error.code() {
                    HistoryErrorCode::NotFound => local_task_unresolvable(),
                    _ => error.into_canonical(),
                })
            })?
            .ok_or_else(|| Box::new(local_task_unresolvable()))
    })
    .await
    .map_err(|_| Box::new(blocking_operation_error()))?
}

fn local_task_unresolvable() -> crate::output::CanonicalError {
    canonical_error(
        ErrorCode::LocalTaskUnresolvable,
        "the saved analysis does not resolve to one upstream task",
    )
}

fn task_diagnostic(kind: &str, identity: &crate::analysis::OperationIdentity) -> String {
    let mut diagnostic = format!(
        "cancelled local observation for {kind} {}",
        identity.analysis_id
    );
    if let Some(task_id) = &identity.task_id {
        diagnostic.push_str(" and upstream task ");
        diagnostic.push_str(task_id.as_str());
    }
    diagnostic
}
