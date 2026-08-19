//! History and restricted configuration MCP tools.
//!
//! Capability filtering controls discovery, but this module repeats the gate
//! at execution time so authorization cannot depend on an inventory lookup.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::analysis::{Accepted, AnalysisRequest, StopObserving, WaitOptions};
use crate::domain::{AnalysisId, AnalysisStatus, CheckKind, SaveState, UtcTimestamp};
use crate::history::{HistoryError, HistoryErrorCode, HistoryStore};
use crate::output::{
    CanonicalError, CommandData, ErrorCode, MutationAcknowledgement, ResolvedCommand,
};

use super::{
    ToolCallContext, ToolCallOutcome, canonical_error, failure, invalid_arguments,
    resolve_analyzer, success,
};

const DEFAULT_LIMIT: u32 = 50;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryQuery {
    query: Option<String>,
    status: Option<AnalysisStatus>,
    check: Option<CheckKind>,
    #[serde(default = "default_limit")]
    limit: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryGet {
    analysis_id: AnalysisId,
    #[serde(default)]
    include_content: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryIdentity {
    analysis_id: AnalysisId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryRerun {
    analysis_id: AnalysisId,
    max_billable_units: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateConfig {
    key: String,
    value: String,
}

pub(super) async fn history_query(
    context: &ToolCallContext<'_>,
    arguments: Map<String, Value>,
    search: bool,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    let command = if search {
        ResolvedCommand::HistorySearch
    } else {
        ResolvedCommand::HistoryList
    };
    if !context.options().history {
        return capability_failure(command, "local history reads", started);
    }
    let Ok(arguments) = serde_json::from_value::<HistoryQuery>(Value::Object(arguments)) else {
        return invalid_arguments();
    };
    if !(1..=1000).contains(&arguments.limit)
        || search && arguments.query.as_deref().is_none_or(str::is_empty)
        || !search && arguments.query.is_some()
    {
        return invalid_arguments();
    }

    let data_dir = context.service().paths().data_dir().to_path_buf();
    let operation = tokio::task::spawn_blocking(move || {
        let store = HistoryStore::open_existing(&data_dir)?;
        let hits = match (store, search) {
            (Some(store), false) => {
                store.list_filtered(arguments.status, arguments.check, arguments.limit, 0)?
            }
            (Some(store), true) => store.search_filtered(
                arguments
                    .query
                    .as_deref()
                    .expect("search query was validated"),
                arguments.status,
                arguments.check,
                arguments.limit,
            )?,
            (None, _) => Vec::new(),
        };
        crate::history::summary_page(hits)
    })
    .await;
    match operation {
        Ok(Ok(page)) if search => success(
            CommandData::HistorySearch(page),
            "Searched saved Pangram analyses.",
            started,
        ),
        Ok(Ok(page)) => success(
            CommandData::HistoryList(page),
            "Listed saved Pangram analyses.",
            started,
        ),
        Ok(Err(error)) => history_failure(command, error, started),
        Err(_) => internal_failure(command, started),
    }
}

pub(super) async fn history_get(
    context: &ToolCallContext<'_>,
    arguments: Map<String, Value>,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    let command = ResolvedCommand::HistoryShow;
    if !context.options().history {
        return capability_failure(command, "local history reads", started);
    }
    let Ok(arguments) = serde_json::from_value::<HistoryGet>(Value::Object(arguments)) else {
        return invalid_arguments();
    };
    let id = arguments.analysis_id;
    let data_dir = context.service().paths().data_dir().to_path_buf();
    let operation = tokio::task::spawn_blocking(move || {
        let store = HistoryStore::open_existing(&data_dir)?.ok_or_else(missing_record)?;
        store.canonical_analysis(&id, arguments.include_content)
    })
    .await;
    match operation {
        Ok(Ok(analysis)) => success(
            CommandData::HistoryShow(analysis),
            "Loaded one saved Pangram analysis.",
            started,
        ),
        Ok(Err(error)) => history_failure(command, error, started),
        Err(_) => internal_failure(command, started),
    }
}

pub(super) async fn history_delete(
    context: &ToolCallContext<'_>,
    arguments: Map<String, Value>,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    let command = ResolvedCommand::HistoryDelete;
    if !context.history_mutations_enabled() {
        return capability_failure(command, "local history mutations", started);
    }
    let Ok(arguments) = serde_json::from_value::<HistoryIdentity>(Value::Object(arguments)) else {
        return invalid_arguments();
    };
    let id = arguments.analysis_id;
    let data_dir = context.service().paths().data_dir().to_path_buf();
    let operation = tokio::task::spawn_blocking(move || {
        let mut store = HistoryStore::open_existing(&data_dir)?.ok_or_else(missing_record)?;
        store.delete_analysis(&id)
    })
    .await;
    match operation {
        Ok(Ok(())) => success(
            CommandData::HistoryDelete(MutationAcknowledgement::new()),
            "Deleted one saved Pangram analysis.",
            started,
        ),
        Ok(Err(error)) => history_failure(command, error, started),
        Err(_) => internal_failure(command, started),
    }
}

pub(super) async fn history_clear(
    context: &ToolCallContext<'_>,
    arguments: Map<String, Value>,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    let command = ResolvedCommand::HistoryClear;
    if !context.history_mutations_enabled() {
        return capability_failure(command, "local history mutations", started);
    }
    if !arguments.is_empty() {
        return invalid_arguments();
    }
    let data_dir = context.service().paths().data_dir().to_path_buf();
    let operation = tokio::task::spawn_blocking(move || {
        if let Some(mut store) = HistoryStore::open_existing(&data_dir)? {
            store.clear()?;
        }
        Ok::<_, HistoryError>(())
    })
    .await;
    match operation {
        Ok(Ok(())) => success(
            CommandData::HistoryClear(MutationAcknowledgement::new()),
            "Cleared saved Pangram analyses.",
            started,
        ),
        Ok(Err(error)) => history_failure(command, error, started),
        Err(_) => internal_failure(command, started),
    }
}

pub(super) async fn update_config(
    context: &ToolCallContext<'_>,
    arguments: Map<String, Value>,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    let command = ResolvedCommand::ConfigSet;
    if !context.options().allow_config_mutations {
        return capability_failure(command, "configuration mutations", started);
    }
    let Ok(arguments) = serde_json::from_value::<UpdateConfig>(Value::Object(arguments)) else {
        return invalid_arguments();
    };
    let Ok(key) = crate::config::ConfigKey::parse(&arguments.key) else {
        return invalid_arguments();
    };
    // Parse against the same closed key owner before any filesystem write.
    if key.parse_value(&arguments.value).is_err() {
        return invalid_arguments();
    }
    let service = context.service().clone();
    let operation =
        tokio::task::spawn_blocking(move || service.set(key.as_str(), &arguments.value)).await;
    match operation {
        Ok(Ok(_)) => success(
            CommandData::ConfigSet(MutationAcknowledgement::new()),
            "Updated one Pangram configuration value.",
            started,
        ),
        Ok(Err(error)) => failure(command, crate::analysis::config_error(error), started),
        Err(_) => internal_failure(command, started),
    }
}

pub(super) async fn history_rerun(
    context: &ToolCallContext<'_>,
    arguments: Map<String, Value>,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    let command = ResolvedCommand::HistoryRerun;
    if !context.history_mutations_enabled() {
        return capability_failure(command, "local history mutations", started);
    }
    let Ok(arguments) = serde_json::from_value::<HistoryRerun>(Value::Object(arguments)) else {
        return invalid_arguments();
    };
    if arguments.max_billable_units == 0 {
        return invalid_arguments();
    }
    let id = arguments.analysis_id;
    let data_dir = context.service().paths().data_dir().to_path_buf();
    let original = match tokio::task::spawn_blocking(move || {
        let store = HistoryStore::open_existing(&data_dir)?.ok_or_else(missing_record)?;
        store.canonical_analysis(&id, true)
    })
    .await
    {
        Ok(Ok(original)) => original,
        Ok(Err(error)) if error.code() == HistoryErrorCode::NotFound => {
            return failure(command, unresolvable(), started);
        }
        Ok(Err(error)) => return history_failure(command, error, started),
        Err(_) => return internal_failure(command, started),
    };
    let Some((request, mode)) = AnalysisRequest::from_saved_rerun(&original) else {
        return failure(command, unresolvable(), started);
    };
    let estimated = mode.billable_units(crate::domain::text_billable_units(request.word_count()));
    if estimated > arguments.max_billable_units {
        return failure(
            command,
            canonical_error(
                ErrorCode::UnsupportedInput,
                &format!(
                    "estimated {estimated} billable unit(s) exceeds max_billable_units {}",
                    arguments.max_billable_units
                ),
            ),
            started,
        );
    }
    let retained_text = request.text().to_owned();
    let analyzer = match resolve_analyzer(context).await {
        Ok(analyzer) => analyzer,
        Err(error) => return failure(command, *error, started),
    };
    if context.cancellation().is_cancelled() {
        return ToolCallOutcome::Cancelled { diagnostic: None };
    }

    let analysis = match run_rerun_analysis(context, &analyzer, request, mode).await {
        Ok(Some(analysis)) => analysis,
        Ok(None) => return ToolCallOutcome::Cancelled { diagnostic: None },
        Err(error) => return failure(command, error, started),
    };

    // An MCP history rerun is an explicit mutation. Persist the terminal
    // analysis with retained plaintext before reporting it as saved.
    let saved = analysis.with_save_state(SaveState::SavedManual);
    let retained_input = crate::history::RetainedInput::Text(retained_text);
    let data_dir = context.service().paths().data_dir().to_path_buf();
    let persisted = tokio::task::spawn_blocking(move || {
        let mut store = HistoryStore::open(&data_dir)?;
        crate::history::save_complete_analysis(
            &mut store,
            &saved,
            SaveState::SavedManual,
            Some(&retained_input),
        )?;
        Ok::<_, HistoryError>(saved)
    })
    .await;
    match persisted {
        Ok(Ok(saved)) => success(
            CommandData::HistoryRerun(saved),
            "Reran and saved one Pangram analysis.",
            started,
        ),
        Ok(Err(error)) => history_failure(command, error, started),
        Err(_) => internal_failure(command, started),
    }
}

async fn run_rerun_analysis(
    context: &ToolCallContext<'_>,
    analyzer: &crate::analysis::Analyzer,
    request: AnalysisRequest,
    mode: crate::analysis::TextAnalysisMode,
) -> Result<Option<crate::domain::Analysis<CanonicalError>>, CanonicalError> {
    match mode {
        crate::analysis::TextAnalysisMode::Detection => {
            let accepted = analyzer
                .start_full(request, context.cancellation())
                .await
                .map_err(|failure| failure.task_error.into_error())?;
            match accepted {
                Accepted::Terminal(analysis) => Ok(Some(*analysis)),
                Accepted::Task(accepted) => {
                    let running = analyzer.running(accepted);
                    let observation =
                        running.observe(WaitOptions::UNBOUNDED, |_| {}, StopObserving::new());
                    tokio::select! {
                        result = observation => match result {
                            Ok(Ok(analysis)) => Ok(Some(analysis)),
                            Ok(Err(error)) => Err(error.into_error()),
                            Err(_) => Ok(None),
                        },
                        () = context.cancellation().cancelled() => Ok(None),
                    }
                }
            }
        }
        crate::analysis::TextAnalysisMode::Plagiarism => tokio::select! {
            result = analyzer.plagiarism(request, context.cancellation()) => {
                result.map(Some).map_err(crate::analysis::TaskError::into_error)
            }
            () = context.cancellation().cancelled() => Ok(None),
        },
        crate::analysis::TextAnalysisMode::Combined => {
            let stop = StopObserving::new();
            let operation =
                analyzer.analyze_combined(request, WaitOptions::UNBOUNDED, |_| {}, stop.clone());
            tokio::select! {
                result = operation => match result {
                    Ok(Ok(analysis)) => Ok(Some(analysis)),
                    Ok(Err(error)) => Err(error.into_error()),
                    Err(_) => Ok(None),
                },
                () = context.cancellation().cancelled() => {
                    stop.stop();
                    Ok(None)
                },
            }
        }
    }
}

const fn default_limit() -> u32 {
    DEFAULT_LIMIT
}

fn missing_record() -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::NotFound,
        "no analysis with that identity is recorded",
    )
}

fn history_failure(
    command: ResolvedCommand,
    error: HistoryError,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    failure(command, error.into_canonical(), started)
}

fn capability_failure(
    command: ResolvedCommand,
    capability: &str,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    failure(
        command,
        canonical_error(
            ErrorCode::McpCapabilityRequired,
            &format!("this MCP server did not enable {capability}"),
        ),
        started,
    )
}

fn unresolvable() -> CanonicalError {
    canonical_error(
        ErrorCode::LocalTaskUnresolvable,
        "The saved analysis does not retain exact text that can be rerun.",
    )
}

fn internal_failure(command: ResolvedCommand, started: UtcTimestamp) -> ToolCallOutcome {
    failure(
        command,
        canonical_error(
            ErrorCode::InvalidConfig,
            "the MCP tool operation did not complete",
        ),
        started,
    )
}
