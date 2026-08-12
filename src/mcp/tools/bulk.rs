//! Bulk submission, observation, and explicit results-page MCP tools.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::analysis::{BulkAnalysisRequest, StopObserving, WaitOptions, canonical_text_word_count};
use crate::domain::{
    BULK_PAGE_LIMIT_MAX, BulkId, BulkSubmissionItem, BulkSubmissionPlan, NonEmptyString,
    UpstreamBulkId, UtcTimestamp,
};
use crate::history::{HistoryError, HistoryStore};
use crate::output::{CanonicalError, CommandData, ErrorCode, ResolvedCommand};

use super::{
    ToolCallContext, ToolCallOutcome, blocking_operation_error, canonical_error, failure,
    invalid_arguments, resolve_analyzer, success,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmitBulkArgs {
    items: Option<Vec<InlineItem>>,
    jsonl_path: Option<String>,
    max_billable_units: u64,
    #[serde(default)]
    save: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InlineItem {
    id: Option<NonEmptyString>,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BulkArgs {
    bulk_id: Option<BulkId>,
    upstream_bulk_id: Option<UpstreamBulkId>,
    timeout_ms: Option<u64>,
    offset: Option<u64>,
    limit: Option<u64>,
}

pub(super) async fn submit(
    context: &ToolCallContext<'_>,
    arguments: Map<String, Value>,
    started: UtcTimestamp,
) -> ToolCallOutcome {
    let Ok(arguments) = serde_json::from_value::<SubmitBulkArgs>(Value::Object(arguments)) else {
        return invalid_arguments();
    };
    if arguments.items.is_some() == arguments.jsonl_path.is_some()
        || arguments.max_billable_units == 0
        || arguments.max_billable_units > crate::domain::BULK_BILLABLE_UNIT_LIMIT
        || arguments
            .items
            .as_ref()
            .is_some_and(|items| items.is_empty() || items.len() > 1000)
    {
        return invalid_arguments();
    }
    if arguments.save && !context.history_mutations_enabled() {
        return failure(
            ResolvedCommand::BulkSubmit,
            canonical_error(
                ErrorCode::McpCapabilityRequired,
                "saving requires --history and --allow-history-mutations",
            ),
            started,
        );
    }

    let (plan, source_name) = match submission_plan(
        context,
        arguments.items,
        arguments.jsonl_path,
        arguments.max_billable_units,
    )
    .await
    {
        Ok(plan) => plan,
        Err(error) => return failure(ResolvedCommand::BulkSubmit, *error, started),
    };
    let analyzer = match resolve_analyzer(context).await {
        Ok(analyzer) => analyzer,
        Err(error) => return failure(ResolvedCommand::BulkSubmit, *error, started),
    };
    if context.cancellation().is_cancelled() {
        return ToolCallOutcome::Cancelled { diagnostic: None };
    }
    let running = match analyzer
        .submit_bulk(BulkAnalysisRequest::new(plan), context.cancellation())
        .await
    {
        Ok(running) => running,
        Err(error) if context.cancellation().is_cancelled() => {
            return ToolCallOutcome::Cancelled {
                diagnostic: Some(format!(
                    "cancelled local observation for bulk {}",
                    error.bulk_id()
                )),
            };
        }
        Err(error) => return failure(ResolvedCommand::BulkSubmit, error.into_error(), started),
    };

    let accepted_at = UtcTimestamp::now();
    let collection = running.accepted_collection(accepted_at);
    let collection = if arguments.save {
        let children = running.acceptance_children(source_name.as_deref(), accepted_at);
        let data_dir = context.service().paths().data_dir().to_path_buf();
        match tokio::task::spawn_blocking(move || {
            let mut store = HistoryStore::open(&data_dir)?;
            crate::history::save_bulk_snapshot(&mut store, &collection, &children)?;
            Ok::<_, HistoryError>(collection)
        })
        .await
        {
            Ok(Ok(collection)) => collection,
            Ok(Err(error)) => {
                return failure(ResolvedCommand::BulkSubmit, error.into_canonical(), started);
            }
            Err(_) => {
                return failure(
                    ResolvedCommand::BulkSubmit,
                    blocking_operation_error(),
                    started,
                );
            }
        }
    } else {
        collection
    };

    let id = collection.id().to_string();
    success(
        CommandData::BulkSubmit(crate::output::BulkSubmitOutput::collection(collection)),
        format!("bulk job {id} accepted"),
        started,
    )
}

#[derive(Clone, Copy)]
pub(super) enum BulkOperation {
    Get,
    Wait,
    Results,
}

impl BulkOperation {
    const fn command(self) -> ResolvedCommand {
        match self {
            Self::Get => ResolvedCommand::BulkStatus,
            Self::Wait => ResolvedCommand::BulkWait,
            Self::Results => ResolvedCommand::BulkResults,
        }
    }
}

pub(super) async fn observe(
    context: &ToolCallContext<'_>,
    arguments: Map<String, Value>,
    started: UtcTimestamp,
    operation: BulkOperation,
) -> ToolCallOutcome {
    let Ok(arguments) = serde_json::from_value::<BulkArgs>(Value::Object(arguments)) else {
        return invalid_arguments();
    };
    if arguments.bulk_id.is_some() == arguments.upstream_bulk_id.is_some()
        || (!matches!(operation, BulkOperation::Wait) && arguments.timeout_ms.is_some())
        || arguments.timeout_ms == Some(0)
        || (matches!(operation, BulkOperation::Results)
            && (arguments.offset.is_none()
                || !(1..=BULK_PAGE_LIMIT_MAX).contains(&arguments.limit.unwrap_or(0))))
        || (!matches!(operation, BulkOperation::Results)
            && (arguments.offset.is_some() || arguments.limit.is_some()))
    {
        return invalid_arguments();
    }
    let command = operation.command();
    let identity = match bulk_identity(context, arguments.bulk_id, arguments.upstream_bulk_id).await
    {
        Ok(identity) => identity,
        Err(error) => return failure(command, *error, started),
    };
    let analyzer = match resolve_analyzer(context).await {
        Ok(analyzer) => analyzer,
        Err(error) => return failure(command, *error, started),
    };
    if context.cancellation().is_cancelled() {
        return ToolCallOutcome::Cancelled { diagnostic: None };
    }
    let upstream = identity.upstream.clone();
    let running = match identity.local {
        Some(local) => analyzer.observe_bulk_as(local, upstream.clone()),
        None => analyzer.observe_bulk(upstream.clone()),
    };

    match operation {
        BulkOperation::Get => {
            let running_identity = running.identity();
            match running.snapshot(context.cancellation(), None).await {
                Ok(collection) => success(
                    CommandData::BulkStatus(collection),
                    format!("bulk job {} observed", upstream.as_str()),
                    started,
                ),
                Err(_) if context.cancellation().is_cancelled() => ToolCallOutcome::Cancelled {
                    diagnostic: Some(bulk_diagnostic(&running_identity)),
                },
                Err(error) => failure(command, error.into_error(), started),
            }
        }
        BulkOperation::Wait => {
            let timeout = arguments
                .timeout_ms
                .map(|timeout| WaitOptions::with_timeout(std::time::Duration::from_millis(timeout)))
                .unwrap_or(WaitOptions::UNBOUNDED);
            let identity = running.identity();
            let observation = running.observe(timeout, |_| {}, StopObserving::new());
            let observed = tokio::select! {
                result = observation => Some(result),
                () = context.cancellation().cancelled() => None,
            };
            match observed {
                None | Some(Err(_)) => ToolCallOutcome::Cancelled {
                    diagnostic: Some(bulk_diagnostic(&identity)),
                },
                Some(Ok(Ok(collection))) => success(
                    CommandData::BulkWait(collection),
                    format!("bulk job {} completed", upstream.as_str()),
                    started,
                ),
                Some(Ok(Err(error))) => failure(command, error.into_error(), started),
            }
        }
        BulkOperation::Results => match analyzer
            .bulk_results_page(
                &running,
                arguments.offset.expect("validated offset"),
                arguments.limit.expect("validated limit"),
                context.cancellation(),
            )
            .await
        {
            Ok(page) => success(
                CommandData::BulkResults(page.page),
                format!("bulk job {} results page read", upstream.as_str()),
                started,
            ),
            Err(_) if context.cancellation().is_cancelled() => cancelled_bulk(&running),
            Err(error) => failure(command, error.into_error(), started),
        },
    }
}

async fn submission_plan(
    context: &ToolCallContext<'_>,
    inline: Option<Vec<InlineItem>>,
    jsonl_path: Option<String>,
    max_billable_units: u64,
) -> Result<(BulkSubmissionPlan, Option<String>), Box<CanonicalError>> {
    if let Some(inline) = inline {
        let items = inline
            .into_iter()
            .map(|item| {
                let words = canonical_text_word_count(&item.text);
                BulkSubmissionItem::new(item.id, item.text, words)
                    .map_err(|_| Box::new(invalid_bulk_input()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        return finish_submission_plan(items, None, max_billable_units);
    }

    let path = PathBuf::from(jsonl_path.expect("one source was validated"));
    let approved_roots = context.approved_roots_handle();
    tokio::task::spawn_blocking(move || {
        let file = approved_roots
            .open(&path)
            .map_err(|error| Box::new(map_file_error(error)))?;
        let jsonl = std::io::read_to_string(file).map_err(|_| {
            Box::new(canonical_error(
                ErrorCode::UnsupportedInput,
                "the approved bulk file is not valid UTF-8",
            ))
        })?;
        let items = crate::domain::parse_bulk_jsonl(&jsonl, canonical_text_word_count).map_err(
            |error| {
                Box::new(canonical_error(
                    ErrorCode::UnsupportedInput,
                    &error.to_string(),
                ))
            },
        )?;
        let source_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned);
        finish_submission_plan(items, source_name, max_billable_units)
    })
    .await
    .map_err(|_| Box::new(blocking_operation_error()))?
}

fn finish_submission_plan(
    items: Vec<BulkSubmissionItem>,
    source_name: Option<String>,
    max_billable_units: u64,
) -> Result<(BulkSubmissionPlan, Option<String>), Box<CanonicalError>> {
    let plan = BulkSubmissionPlan::new(items, max_billable_units).map_err(|error| {
        Box::new(match error {
            crate::domain::DomainError::BulkLimitExceeded => canonical_error(
                ErrorCode::BulkLimitExceeded,
                "the bulk submission exceeds the billable-unit ceiling",
            ),
            _ => invalid_bulk_input(),
        })
    })?;
    Ok((plan, source_name))
}

async fn bulk_identity(
    context: &ToolCallContext<'_>,
    bulk_id: Option<BulkId>,
    upstream_bulk_id: Option<UpstreamBulkId>,
) -> Result<ResolvedBulkIdentity, Box<CanonicalError>> {
    if let Some(upstream) = upstream_bulk_id {
        return Ok(ResolvedBulkIdentity {
            local: None,
            upstream,
        });
    }
    if !context.options().history {
        return Err(Box::new(canonical_error(
            ErrorCode::McpCapabilityRequired,
            "local bulk IDs require --history",
        )));
    }
    let Some(bulk_id) = bulk_id else {
        return Err(Box::new(local_bulk_unresolvable()));
    };
    let data_dir = context.service().paths().data_dir().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let Some(store) = HistoryStore::open_existing(&data_dir)
            .map_err(|error| Box::new(error.into_canonical()))?
        else {
            return Err(Box::new(local_bulk_unresolvable()));
        };
        let stored = store
            .get_bulk_collection(&bulk_id)
            .map_err(|error| Box::new(error.into_canonical()))?;
        let upstream = stored
            .upstream_bulk_id
            .as_deref()
            .ok_or_else(|| Box::new(local_bulk_unresolvable()))
            .and_then(|id| {
                UpstreamBulkId::new(id).map_err(|_| Box::new(local_bulk_unresolvable()))
            })?;
        Ok(ResolvedBulkIdentity {
            local: Some(bulk_id),
            upstream,
        })
    })
    .await
    .map_err(|_| Box::new(blocking_operation_error()))?
}

struct ResolvedBulkIdentity {
    local: Option<BulkId>,
    upstream: UpstreamBulkId,
}

fn cancelled_bulk(running: &crate::analysis::RunningBulk) -> ToolCallOutcome {
    ToolCallOutcome::Cancelled {
        diagnostic: Some(bulk_diagnostic(&running.identity())),
    }
}

fn bulk_diagnostic(identity: &crate::analysis::BulkOperationIdentity) -> String {
    let mut diagnostic = format!("cancelled local observation for bulk {}", identity.bulk_id);
    if let Some(upstream) = &identity.upstream_bulk_id {
        diagnostic.push_str(" and upstream bulk ");
        diagnostic.push_str(upstream.as_str());
    }
    diagnostic
}

fn map_file_error(error: crate::mcp::files::ApprovedFileError) -> crate::output::CanonicalError {
    use crate::mcp::files::ApprovedFileError;
    let code = match error {
        ApprovedFileError::NoApprovedRoots => ErrorCode::McpRootRequired,
        ApprovedFileError::OutsideApprovedRoots
        | ApprovedFileError::PathNotAbsolute
        | ApprovedFileError::UnsafePath => ErrorCode::McpPathOutsideRoot,
        _ => ErrorCode::UnsupportedInput,
    };
    canonical_error(code, "the bulk JSONL path is not an approved regular file")
}

fn invalid_bulk_input() -> crate::output::CanonicalError {
    canonical_error(
        ErrorCode::UnsupportedInput,
        "the bulk submission contains an invalid item",
    )
}

fn local_bulk_unresolvable() -> crate::output::CanonicalError {
    canonical_error(
        ErrorCode::HistoryUnavailable,
        "the saved bulk collection does not resolve to an upstream job",
    )
}
