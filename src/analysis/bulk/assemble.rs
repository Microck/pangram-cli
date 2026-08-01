//! Canonical-page and item assembly for the bulk pipeline.

use crate::domain::{
    Analysis, AnalysisId, AnalysisInput, AnalysisStatus, BulkCounters, BulkItem,
    BulkSubmissionPlan, Check, CheckState, Provenance, Provider, SaveState, Sha256Hash, TextInput,
    TextOrigin, UpstreamBulkId, UpstreamTaskIds, UtcTimestamp,
};
use crate::output::{CanonicalError, ErrorCode};

use crate::analysis::config::Clock;
use crate::analysis::normalize::bulk::{
    NormalizedBulkItemOutcome, NormalizedBulkPlan, NormalizedBulkSuccess, NormalizedItemResult,
    NormalizedItemsPage, NormalizedResultsPage,
};
use crate::analysis::{RunningBulk, normalize};

/// The canonical collection for one status snapshot at
/// `updated_at` (the observation time; the upstream `created_at`/
/// `completed_at` are authoritative).
pub(super) fn assemble_collection<C: Clock>(
    running: &RunningBulk<C>,
    status: &super::StatusSnapshot,
    updated_at: UtcTimestamp,
) -> crate::domain::BulkCollection {
    crate::domain::BulkCollection::new(
        running.bulk_id(),
        Some(running.upstream_bulk_id().clone()),
        status.status,
        crate::domain::SubmissionOutcome::Accepted,
        status.counters,
        running
            .plan()
            .map(BulkSubmissionPlan::estimated_billable_units),
        status.created_at,
        updated_at,
        status.completed_at,
    )
    .expect("a validated status snapshot always satisfies collection invariants")
}

/// The initial counters for a freshly accepted job: total from the plan,
/// immediate failures and not-yet-finished accepted work.
pub(super) fn initial_counters(
    plan: &BulkSubmissionPlan,
    normalized: &NormalizedBulkPlan,
) -> (BulkCounters, AnalysisStatus) {
    let accepted_count = u64::try_from(normalized.accepted.len()).unwrap_or(u64::MAX);
    let failed_count = u64::try_from(normalized.failed.len()).unwrap_or(u64::MAX);
    let total = u64::try_from(plan.items().len()).unwrap_or(u64::MAX);
    let counters = BulkCounters::new(total, accepted_count, 0, failed_count)
        .expect("validated acceptance counters satisfy bounds");
    let status = if counters.is_terminal() {
        if failed_count == total {
            AnalysisStatus::Failed
        } else if failed_count > 0 {
            AnalysisStatus::Partial
        } else {
            AnalysisStatus::Succeeded
        }
    } else {
        AnalysisStatus::Queued
    };
    (counters, status)
}

/// Builds canonical analyses for a results page: one per succeeded or
/// documented in-progress position, in page order; failed positions become
/// failed analyses carrying the sanitized upstream error. With a plan the
/// input descriptors come only from the trusted local plan; on a resumed
/// observation (`None`) they derive only from upstream-attested terminal
/// content (contracts.md 4.6) and running/failed children omit the input
/// descriptor rather than fabricating one. The observed job's upstream bulk
/// identity is carried on every resumed child analysis's provenance so a
/// text-less (or no-task-id) observed child still satisfies the `accepted`
/// evidence rule.
pub(super) fn assemble_results_page(
    plan: Option<&BulkSubmissionPlan>,
    upstream_bulk_id: &UpstreamBulkId,
    page: NormalizedResultsPage,
) -> Result<Vec<BulkItem<CanonicalError>>, CanonicalError> {
    let NormalizedResultsPage {
        mut succeeded,
        failed,
    } = page;
    let mut indexes: Vec<u64> = succeeded.keys().chain(failed.keys()).copied().collect();
    indexes.sort_unstable();
    indexes.dedup();
    let mut items = Vec::with_capacity(indexes.len());
    for index in indexes {
        if let Some(entry) = succeeded.remove(&index) {
            match entry {
                NormalizedItemResult::Succeeded(success) => {
                    let now = UtcTimestamp::now();
                    let task_id = success.task_id.clone();
                    let caller_id = caller_id_at(plan, index);
                    let analysis = match plan {
                        Some(plan) => build_terminal_success_analysis(
                            plan_item_at(plan, index)?,
                            success.task_id.as_ref(),
                            success.task,
                            now,
                        )?,
                        None => build_observed_success_analysis(success, upstream_bulk_id, now)?,
                    };
                    items.push(BulkItem {
                        index,
                        caller_id,
                        analysis_id: Some(analysis.id),
                        upstream_task_id: task_id,
                        state: crate::domain::BulkItemState::Succeeded {
                            analysis: Box::new(analysis),
                        },
                    });
                }
                NormalizedItemResult::Running { task_id } => {
                    // A documented `result: null` entry: a running child with
                    // no analysis content yet. With a plan the caller ID is
                    // preserved from the trusted local plan; on a resumed
                    // read the worker identity is the only provenance.
                    items.push(BulkItem {
                        index,
                        caller_id: caller_id_at(plan, index),
                        analysis_id: None,
                        upstream_task_id: task_id,
                        state: crate::domain::BulkItemState::Running,
                    });
                }
            }
        } else if let Some(outcome) = failed.get(&index) {
            let analysis = match plan {
                Some(plan) => build_terminal_failed_analysis(plan_item_at(plan, index)?, outcome)?,
                None => build_observed_failed_analysis(outcome, upstream_bulk_id)?,
            };
            items.push(BulkItem {
                index,
                caller_id: caller_id_at(plan, index),
                analysis_id: Some(analysis.id),
                upstream_task_id: outcome.task_id.clone(),
                state: crate::domain::BulkItemState::Failed {
                    error: outcome_error(outcome),
                },
            });
        }
    }
    Ok(items)
}

/// Builds canonical bulk items for an items-metadata page: one per reported
/// position, preserving the worker's in-flight or failed state with the
/// worker's own task identity. Item order is enforced upstream by the page
/// validator; each entry only composes the worker's reported outcome. On a
/// resumed observation (`None`) a failed child carries the upstream error
/// with no fabricated input descriptor, and its provenance carries the
/// observed upstream bulk identity (contracts.md 4.6).
pub(super) fn assemble_items_metadata_page(
    plan: Option<&BulkSubmissionPlan>,
    upstream_bulk_id: &UpstreamBulkId,
    page: NormalizedItemsPage,
) -> Result<Vec<BulkItem<CanonicalError>>, CanonicalError> {
    let NormalizedItemsPage { outcomes } = page;
    let mut items = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        let NormalizedBulkItemOutcome {
            index,
            task_id,
            error,
        } = outcome;
        let caller_id = caller_id_at(plan, index);
        match error {
            Some(_) => {
                let outcome = NormalizedBulkItemOutcome {
                    index,
                    task_id: task_id.clone(),
                    error,
                };
                let analysis = match plan {
                    Some(plan) => {
                        build_terminal_failed_analysis(plan_item_at(plan, index)?, &outcome)?
                    }
                    None => build_observed_failed_analysis(&outcome, upstream_bulk_id)?,
                };
                items.push(BulkItem {
                    index,
                    caller_id,
                    analysis_id: Some(analysis.id),
                    upstream_task_id: task_id,
                    state: crate::domain::BulkItemState::Failed {
                        error: outcome_error(&outcome),
                    },
                });
            }
            None => {
                // Accepted in-flight work: a running metadata entry whose
                // upstream task identity is the worker's. A missing task_id
                // on accepted work is tolerated here (the metadata page may
                // carry null task_id for very early items).
                items.push(BulkItem {
                    index,
                    caller_id,
                    analysis_id: None,
                    upstream_task_id: task_id,
                    state: crate::domain::BulkItemState::Running,
                });
            }
        }
    }
    Ok(items)
}

fn plan_item_at(
    plan: &BulkSubmissionPlan,
    index: u64,
) -> Result<&crate::domain::BulkSubmissionItem, CanonicalError> {
    plan.items()
        .get(usize::try_from(index).map_err(|_| domain_out_of_range(index))?)
        .ok_or_else(|| {
            super::contract_symptom(
                "index",
                format!("no local plan item for source position {index}"),
            )
        })
}

fn domain_out_of_range(index: u64) -> CanonicalError {
    super::contract_symptom(
        "index",
        format!("source position {index} does not fit usize"),
    )
}

/// A canonical terminal-success bulk child analysis. Identity derives from
/// the local plan (source position is stable); input is the trusted local
/// descriptor; provenance carries the worker's task identity and version.
fn build_terminal_success_analysis(
    plan_item: &crate::domain::BulkSubmissionItem,
    task_id: Option<&crate::domain::UpstreamTaskId>,
    task: Box<normalize::NormalizedTask>,
    now: UtcTimestamp,
) -> Result<Analysis<CanonicalError>, CanonicalError> {
    let input = trusted_text_input(plan_item);
    let state = CheckState::Succeeded {
        upstream: Some(crate::domain::UpstreamIdentity {
            task_id: task_id.cloned(),
            last_stage: crate::domain::NonEmptyString::new(task.last_stage.clone()).ok(),
        }),
        result: task.result,
    };
    let checks =
        crate::domain::OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: Some(task.version.clone()),
        upstream_task_ids: task_id
            .map(|id| UpstreamTaskIds::new(vec![id.clone()]).expect("one ID")),
        upstream_bulk_id: None,
        submitted_at: Some(now),
        completed_at: Some(now),
    };
    Analysis::new(
        AnalysisId::new(),
        crate::domain::SubmissionOutcome::Terminal,
        AnalysisInput::Text(input),
        checks,
        SaveState::Ephemeral,
        provenance,
        None,
        None,
        now,
        now,
        Some(now),
    )
    .map_err(|error| {
        CanonicalError::new(ErrorCode::UpstreamContractChanged, error.to_string())
            .expect("static template")
    })
}

/// A canonical terminal-failure bulk child analysis carrying the sanitized
/// upstream error.
fn build_terminal_failed_analysis(
    plan_item: &crate::domain::BulkSubmissionItem,
    outcome: &NormalizedBulkItemOutcome,
) -> Result<Analysis<CanonicalError>, CanonicalError> {
    let input = trusted_text_input(plan_item);
    let error = outcome_error(outcome);
    let state = CheckState::Failed {
        upstream: Some(crate::domain::UpstreamIdentity {
            task_id: outcome.task_id.clone(),
            last_stage: None,
        }),
        error,
    };
    let checks =
        crate::domain::OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let now = UtcTimestamp::now();
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: None,
        upstream_task_ids: outcome
            .task_id
            .as_ref()
            .map(|id| UpstreamTaskIds::new(vec![id.clone()]).expect("one ID")),
        upstream_bulk_id: None,
        submitted_at: Some(now),
        completed_at: Some(now),
    };
    Analysis::new(
        AnalysisId::new(),
        crate::domain::SubmissionOutcome::Terminal,
        AnalysisInput::Text(input),
        checks,
        SaveState::Ephemeral,
        provenance,
        None,
        None,
        now,
        now,
        Some(now),
    )
    .map_err(|error| {
        CanonicalError::new(ErrorCode::UpstreamContractChanged, error.to_string())
            .expect("static template")
    })
}

/// A canonical terminal-success bulk child analysis on a resumed observation:
/// the input descriptor derives only from the upstream-attested terminal text
/// (`origin: unknown`, SHA-256, byte and word counts), never echoed back.
/// `submission_outcome` is `accepted` (never `terminal`) because the read
/// reconciles a remotely authored job; `terminal` is reserved for an
/// operation the caller itself submitted (contracts.md 4.6).
/// `provenance.submitted_at` is omitted because the caller did not submit
/// the operation.
fn build_observed_success_analysis(
    success: NormalizedBulkSuccess,
    upstream_bulk_id: &UpstreamBulkId,
    now: UtcTimestamp,
) -> Result<Analysis<CanonicalError>, CanonicalError> {
    let NormalizedBulkSuccess { task_id, task } = success;
    let last_stage = task.last_stage.clone();
    let version = task.version.clone();
    let input = task.normalized_text.as_deref().map(|text| {
        let byte_count = u64::try_from(text.len()).unwrap_or(u64::MAX);
        let word_count = u64::try_from(text.split_whitespace().count()).unwrap_or(u64::MAX);
        AnalysisInput::Text(
            TextInput::new(
                TextOrigin::Unknown,
                None,
                Sha256Hash::digest(text.as_bytes()),
                byte_count,
                word_count,
                None,
            )
            .expect("attested terminal text always yields a valid descriptor"),
        )
    });
    let state = CheckState::Succeeded {
        upstream: Some(crate::domain::UpstreamIdentity {
            task_id: task_id.clone(),
            last_stage: crate::domain::NonEmptyString::new(last_stage).ok(),
        }),
        result: task.result,
    };
    let checks =
        crate::domain::OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: Some(version),
        upstream_task_ids: task_id.map(|id| UpstreamTaskIds::new(vec![id]).expect("one ID")),
        upstream_bulk_id: Some(upstream_bulk_id.clone()),
        submitted_at: None,
        completed_at: Some(now),
    };
    Analysis::with_optional_input(
        AnalysisId::new(),
        crate::domain::SubmissionOutcome::Accepted,
        input,
        checks,
        SaveState::Ephemeral,
        provenance,
        None,
        None,
        now,
        now,
        Some(now),
    )
    .map_err(|error| {
        CanonicalError::new(ErrorCode::UpstreamContractChanged, error.to_string())
            .expect("static template")
    })
}

/// A canonical terminal-failure bulk child analysis on a resumed observation:
/// no local plan and no attested item text exist, so the input descriptor is
/// omitted rather than fabricated (contracts.md 4.6). `submission_outcome` is
/// `accepted` (never `terminal`): the read reconciles a remotely authored
/// job, and `terminal` is reserved for an operation the caller itself
/// submitted (contracts.md 4.6). `provenance.submitted_at` is omitted.
fn build_observed_failed_analysis(
    outcome: &NormalizedBulkItemOutcome,
    upstream_bulk_id: &UpstreamBulkId,
) -> Result<Analysis<CanonicalError>, CanonicalError> {
    let error = outcome_error(outcome);
    let state = CheckState::Failed {
        upstream: Some(crate::domain::UpstreamIdentity {
            task_id: outcome.task_id.clone(),
            last_stage: None,
        }),
        error,
    };
    let checks =
        crate::domain::OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let now = UtcTimestamp::now();
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: None,
        upstream_task_ids: outcome
            .task_id
            .as_ref()
            .map(|id| UpstreamTaskIds::new(vec![id.clone()]).expect("one ID")),
        upstream_bulk_id: Some(upstream_bulk_id.clone()),
        submitted_at: None,
        completed_at: Some(now),
    };
    Analysis::with_optional_input(
        AnalysisId::new(),
        crate::domain::SubmissionOutcome::Accepted,
        None,
        checks,
        SaveState::Ephemeral,
        provenance,
        None,
        None,
        now,
        now,
        Some(now),
    )
    .map_err(|error| {
        CanonicalError::new(ErrorCode::UpstreamContractChanged, error.to_string())
            .expect("static template")
    })
}

/// The canonical input descriptor for one item, derived ONLY from the
/// validated local plan: origin, SHA-256, byte/word counts. Upstream result
/// text is never reparsed as the item's input (contracts section 9.1).
fn trusted_text_input(plan_item: &crate::domain::BulkSubmissionItem) -> TextInput {
    let text = plan_item.text();
    let byte_count = u64::try_from(text.len()).unwrap_or(u64::MAX);
    TextInput::new(
        TextOrigin::Literal,
        None,
        Sha256Hash::digest(text.as_bytes()),
        byte_count,
        plan_item.word_count(),
        None,
    )
    .expect("a local text input descriptor is total")
}

fn outcome_error(outcome: &NormalizedBulkItemOutcome) -> CanonicalError {
    let message = outcome
        .error
        .as_deref()
        .unwrap_or("Pangram rejected the item.");
    let reduced = normalize::sanitize_upstream_message(message);
    let mut details = std::collections::BTreeMap::new();
    details.insert(
        "upstream_message".to_owned(),
        serde_json::Value::from(reduced),
    );
    CanonicalError::new(
        ErrorCode::UpstreamAnalysisFailed,
        "Pangram could not analyze the submitted text.",
    )
    .and_then(|error| error.with_contextual_retryability(false))
    .and_then(|error| error.with_details(details))
    .expect("static template")
}

/// Resolves one item's caller ID from the trusted local plan. A resumed
/// observation has no plan, so no caller ID is available; an unchecked index
/// that cannot address the plan also resolves to `None` (contract drift
/// handling elsewhere never builds from it).
pub(super) fn caller_id_at(plan: Option<&BulkSubmissionPlan>, index: u64) -> Option<String> {
    plan.and_then(|plan| {
        plan.items()
            .get(usize::try_from(index).ok()?)
            .and_then(|item| item.caller_id())
    })
    .map(|id| id.as_str().to_owned())
}

/// Validates one explicit caller page request's `limit`. A malformed page
/// request is a local usage error raised before any network read.
pub(super) fn validate_page_request(limit: u64) -> Result<(), CanonicalError> {
    if !(1..=crate::domain::BULK_PAGE_LIMIT_MAX).contains(&limit) {
        let mut details = std::collections::BTreeMap::new();
        details.insert("field".to_owned(), serde_json::Value::from("limit"));
        details.insert("limit".to_owned(), serde_json::Value::from(limit));
        let error = CanonicalError::new(
            ErrorCode::InputRequired,
            "the bulk page limit must be in 1..=1000.",
        )
        .and_then(|error| error.with_details(details))
        .expect("static template");
        return Err(error);
    }
    Ok(())
}

/// The next page offset after one typed page: the maximum covered source
/// position plus one, or `None` when the page reached the end of the set.
pub(super) fn next_offset(items: &[BulkItem<CanonicalError>], total: u64) -> Option<u64> {
    let max = items.iter().map(|item| item.index).max()?;
    let next = max + 1;
    if next >= total { None } else { Some(next) }
}
