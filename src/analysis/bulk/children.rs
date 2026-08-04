//! Canonical per-item child projections for the history save seam
//! (contracts.md 14.2): the honest HTTP 202 acceptance children of a
//! trusted local submission plan, and the observed children of one
//! documented results read. Input descriptors derive only from the trusted
//! plan (contracts.md 9.1); persisted local authorship is never claimed by
//! an observed read (contracts.md 4.6). The page-window assembly lives in
//! [`assemble`]; this module owns the child-level save projections.

use crate::domain::{
    Analysis, AnalysisId, AnalysisInput, BulkItem, BulkPage, BulkSubmissionPlan, Check, CheckState,
    Provenance, Provider, SaveState, Sha256Hash, TextInput, TextOrigin, UpstreamBulkId,
    UpstreamIdentity, UpstreamTaskIds, UtcTimestamp,
};
use crate::output::{CanonicalError, ErrorCode};

use crate::analysis::normalize::bulk::{NormalizedBulkItemOutcome, NormalizedBulkPlan};

use super::assemble::outcome_error;

/// The honest plan children of one validated HTTP 202 acceptance (contracts
/// section 9 + section 14.2): one canonical child per validated plan item,
/// in plan order, each mapped from the acceptance's per-item outcome rather
/// than fabricated all-queued. An accepted item the acceptance attests with
/// an upstream `task_id` becomes an `accepted` queued child whose check and
/// provenance carry that real remote identity; an item the acceptance
/// reports as failed through immediate upstream validation becomes a
/// terminal-failed child with the sanitized canonical check error; an item
/// the acceptance attests with no task identity at all stays
/// `not_submitted` and queued rather than fabricating an identity.
///
/// `observed_at` stamps the children's lifecycle times (acceptance
/// observation time), and `source_name` selects the `file` origin for a
/// JSONL-file submission (`stdin` otherwise). Input descriptors come only
/// from the trusted validated plan (contracts section 9.1), never from any
/// upstream body. The caller decides whether to persist (bulk carries no
/// `--save`; only the automatic history gate applies).
pub(super) fn build_acceptance_children(
    plan: &BulkSubmissionPlan,
    normalized: &NormalizedBulkPlan,
    upstream_bulk_id: &UpstreamBulkId,
    source_name: Option<&str>,
    observed_at: UtcTimestamp,
) -> Vec<(Analysis<CanonicalError>, Option<String>)> {
    let accepted: std::collections::HashMap<u64, &NormalizedBulkItemOutcome> = normalized
        .accepted
        .iter()
        .map(|outcome| (outcome.index, outcome))
        .collect();
    let failed: std::collections::HashMap<u64, &NormalizedBulkItemOutcome> = normalized
        .failed
        .iter()
        .map(|outcome| (outcome.index, outcome))
        .collect();
    plan.items()
        .iter()
        .enumerate()
        .filter_map(|(position, item)| {
            let index = u64::try_from(position).unwrap_or(u64::MAX);
            let caller_id = item.caller_id().map(|id| id.as_str().to_owned());
            let child = if let Some(outcome) = accepted.get(&index) {
                build_accepted_child(item, outcome, upstream_bulk_id, source_name, observed_at)?
            } else if let Some(outcome) = failed.get(&index) {
                build_acceptance_failed_child(
                    item,
                    outcome,
                    upstream_bulk_id,
                    source_name,
                    observed_at,
                )
            } else {
                // Only an unattested position (one the acceptance neither
                // attests a task identity nor reports failed, a defensive
                // gap the normalizer already rules out) stays queued with
                // `not_submitted` and no upstream evidence (never a
                // fabricated identity, contracts section 9 + 14.2). On a
                // valid acceptance the normalizer's exact-coverage rule
                // means every position attests, so this is the sole
                // queued/not_submitted child the 202 snapshot can carry.
                build_unattested_child(item, source_name, observed_at)?
            };
            Some((child, caller_id))
        })
        .collect()
}

/// One accepted child: the HTTP 202 acceptance attests its `task_id`, so the
/// child is `accepted` and queued, with the attested upstream task identity
/// on its check evidence. `submission_outcome: accepted` is honored under
/// the same fresh-read-identity rule as section 4.6 (the task ID is real
/// remote identity; only the stored-row reconciliation claims local
/// authorship).
fn build_accepted_child(
    item: &crate::domain::BulkSubmissionItem,
    outcome: &NormalizedBulkItemOutcome,
    upstream_bulk_id: &UpstreamBulkId,
    source_name: Option<&str>,
    observed_at: UtcTimestamp,
) -> Option<Analysis<CanonicalError>> {
    let task_id = outcome.task_id.clone()?;
    let input = accepted_child_input(item, source_name).ok()?;
    let state = CheckState::Queued {
        upstream: Some(UpstreamIdentity {
            task_id: Some(task_id.clone()),
            last_stage: None,
        }),
    };
    let checks =
        crate::domain::OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: None,
        upstream_task_ids: Some(UpstreamTaskIds::new(vec![task_id]).expect("one ID")),
        upstream_bulk_id: Some(upstream_bulk_id.clone()),
        submitted_at: None,
        completed_at: None,
    };
    Analysis::new(
        AnalysisId::new(),
        crate::domain::SubmissionOutcome::Accepted,
        input,
        checks,
        SaveState::SavedHistory,
        provenance,
        None,
        None,
        observed_at,
        observed_at,
        None,
    )
    .ok()
}

/// One acceptance-failed child: the acceptance reports the position failed
/// through immediate upstream validation. The child is terminal-failed with
/// the sanitized canonical check error; no upstream task identity exists
/// (immediate validation never reached a worker), so its `accepted`
/// outcome evidence is the observed upstream bulk identity on provenance
/// (real remote identity of the job that rejected it). The child claims no
/// local submission authorship. Like the accepted sibling, it keeps the
/// trusted local input descriptor (the file origin and source basename for
/// a JSONL-file submission, `stdin` otherwise, and the locally held
/// plaintext the persistence layer indexes): an immediate validation
/// failure loses no provenance the caller already holds (contracts section
/// 9.1 + 14.2).
fn build_acceptance_failed_child(
    item: &crate::domain::BulkSubmissionItem,
    outcome: &NormalizedBulkItemOutcome,
    upstream_bulk_id: &UpstreamBulkId,
    source_name: Option<&str>,
    observed_at: UtcTimestamp,
) -> Analysis<CanonicalError> {
    let error = outcome_error(outcome);
    let state = CheckState::Failed {
        upstream: None,
        error,
    };
    let checks =
        crate::domain::OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: None,
        upstream_task_ids: None,
        upstream_bulk_id: Some(upstream_bulk_id.clone()),
        submitted_at: None,
        completed_at: Some(observed_at),
    };
    let input = accepted_child_input(item, source_name)
        .unwrap_or_else(|_| panic!("a trusted local child input descriptor is total"));
    Analysis::new(
        AnalysisId::new(),
        crate::domain::SubmissionOutcome::Accepted,
        input,
        checks,
        SaveState::SavedHistory,
        provenance,
        None,
        None,
        observed_at,
        observed_at,
        Some(observed_at),
    )
    .expect("a terminal-failed acceptance child satisfies the analysis invariants")
}

/// One unattested child: the acceptance lists no evidence for this position,
/// so the child stays queued with `not_submitted` and no upstream evidence
/// (no fabricated identity, contracts section 9).
fn build_unattested_child(
    item: &crate::domain::BulkSubmissionItem,
    source_name: Option<&str>,
    observed_at: UtcTimestamp,
) -> Option<Analysis<CanonicalError>> {
    let input = accepted_child_input(item, source_name).ok()?;
    let state = CheckState::Queued { upstream: None };
    let checks =
        crate::domain::OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: None,
        upstream_task_ids: None,
        upstream_bulk_id: None,
        submitted_at: None,
        completed_at: None,
    };
    Analysis::new(
        AnalysisId::new(),
        crate::domain::SubmissionOutcome::NotSubmitted,
        input,
        checks,
        SaveState::SavedHistory,
        provenance,
        None,
        None,
        observed_at,
        observed_at,
        None,
    )
    .ok()
}

/// The local-input context for the observed-children projector: when the
/// operator's own validated plan source is a JSONL file, its basename
/// becomes every child's `file`-origin display name and the child input
/// carries its locally held text; a stdin source (or a resumed read with
/// nothing local) carries no name, and a remote-only child omits the
/// descriptor rather than inventing one (contracts.md 4.6).
pub(super) struct ChildInputContext {
    /// The JSONL source basename when this process holds a file source.
    source_name: Option<String>,
}

impl ChildInputContext {
    /// The context for one observation run: a file source when the caller
    /// handed one to `bulk submit`, `None` otherwise (`stdin`, or a
    /// `bulk status`/`bulk wait` of a job this process did not submit).
    #[must_use]
    pub(super) fn new(source_name: Option<String>) -> Self {
        Self { source_name }
    }
}

/// Projects one observed `results`-window item onto the canonical child
/// that persists through the history save seam (contracts.md 14.2). Every
/// observed child is `accepted` (never `terminal`: only the submission-flow
/// projection claims local authorship) and `saved_history`-state; identity
/// is fresh (`anl_` minted per read) because persistence reconciles on the
/// `(bulk_id, bulk_index)` membership, never on a fabricated stable id.
///
/// - A succeeded item becomes a terminal-success child carrying its
///   canonical result; the input descriptor comes from the trusted plan
///   descendants when the process holds the plan (file source: submitted
///   text kept locally; stdin source: counts only), else none.
/// - A failed item becomes a terminal-failed child carrying the canonical
///   check error (never a result).
/// - A queued or running item becomes a non-terminal child with the
///   item's upstream task identity as its check evidence (no fabricated
///   result or error).
///
/// With held provenance the observed upstream bulk identity is recorded on
/// the child; without it (`status`/`wait` of a job this process did not
/// submit) the provenance stays honest about what the read actually holds.
pub(super) fn build_observed_children(
    page: BulkPage<CanonicalError>,
    plan: Option<&BulkSubmissionPlan>,
    upstream_bulk_id: &UpstreamBulkId,
    context: &ChildInputContext,
    observed_at: UtcTimestamp,
) -> Vec<(Analysis<CanonicalError>, Option<String>)> {
    page.items()
        .iter()
        .map(|item| match &item.state {
            crate::domain::BulkItemState::Succeeded { analysis } => {
                let child = project_succeeded_child(
                    item,
                    analysis,
                    plan,
                    upstream_bulk_id,
                    context,
                    observed_at,
                );
                (child, item.caller_id.clone())
            }
            crate::domain::BulkItemState::Failed { error } => {
                let child =
                    project_failed_child(item, error, plan, upstream_bulk_id, context, observed_at);
                (child, item.caller_id.clone())
            }
            crate::domain::BulkItemState::Queued => {
                let child = project_inflight_child(
                    item,
                    crate::domain::CheckState::Queued {
                        upstream: upstream_identity(item),
                    },
                    upstream_bulk_id,
                    plan,
                    context,
                    observed_at,
                );
                (child, item.caller_id.clone())
            }
            crate::domain::BulkItemState::Running => {
                let child = project_inflight_child(
                    item,
                    crate::domain::CheckState::Running {
                        upstream: upstream_identity(item),
                    },
                    upstream_bulk_id,
                    plan,
                    context,
                    observed_at,
                );
                (child, item.caller_id.clone())
            }
        })
        .collect()
}

/// The check-state upstream identity for an observed child: the item's
/// attested upstream task id, never fabricated.
fn upstream_identity(item: &BulkItem<CanonicalError>) -> Option<UpstreamIdentity> {
    item.upstream_task_id.as_ref().map(|task| UpstreamIdentity {
        task_id: Some(task.clone()),
        last_stage: item
            .last_stage()
            .and_then(|stage| crate::domain::NonEmptyString::new(stage.to_owned()).ok()),
    })
}

/// One observed terminal-success child: the canonical result moves with the
/// child's own check; the input descriptor comes from the trusted plan only.
fn project_succeeded_child(
    item: &BulkItem<CanonicalError>,
    analysis: &Analysis<CanonicalError>,
    plan: Option<&BulkSubmissionPlan>,
    upstream_bulk_id: &UpstreamBulkId,
    context: &ChildInputContext,
    observed_at: UtcTimestamp,
) -> Analysis<CanonicalError> {
    let succeeded = analysis.checks().iter().find_map(|check| match check {
        Check::AiDetection(CheckState::Succeeded { result, upstream }) => {
            Some((result.clone(), upstream.clone()))
        }
        _ => None,
    });
    let input = merge_observed_and_local_input(
        analysis.input().cloned(),
        descriptor_for_index(plan, item.index, context),
    );
    build_terminal_child(
        item,
        input,
        succeeded
            .map(|(result, upstream)| TerminalBranch::Succeeded { result, upstream })
            .unwrap_or(TerminalBranch::Failed {
                error: plundered_success_error(),
            }),
        Some(analysis.provenance()),
        upstream_bulk_id,
        observed_at,
    )
}

/// The canonical error for a malformed observed success carrying no result
/// (defense-in-depth; the normalizer guarantees one).
fn plundered_success_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::UpstreamContractChanged,
        "a succeeded bulk item carried no result document",
    )
    .expect("static template")
}

/// One observed terminal-failure child: the canonical check error moves
/// with the child; the input descriptor comes from the trusted plan only.
fn project_failed_child(
    item: &BulkItem<CanonicalError>,
    error: &CanonicalError,
    plan: Option<&BulkSubmissionPlan>,
    upstream_bulk_id: &UpstreamBulkId,
    context: &ChildInputContext,
    observed_at: UtcTimestamp,
) -> Analysis<CanonicalError> {
    let input = descriptor_for_index(plan, item.index, context);
    build_terminal_child(
        item,
        input,
        TerminalBranch::Failed {
            error: error.clone(),
        },
        None,
        upstream_bulk_id,
        observed_at,
    )
}

enum TerminalBranch {
    Succeeded {
        result: crate::domain::AiDetectionResult,
        upstream: Option<UpstreamIdentity>,
    },
    Failed {
        error: CanonicalError,
    },
}

/// Assembles one terminal observed child under the honest-`accepted`
/// outcome, with the observed upstream bulk identity as its outcome
/// evidence and a terminal-observed `completed_at`.
fn build_terminal_child(
    item: &BulkItem<CanonicalError>,
    input: Option<AnalysisInput>,
    branch: TerminalBranch,
    observed_provenance: Option<&Provenance>,
    upstream_bulk_id: &UpstreamBulkId,
    observed_at: UtcTimestamp,
) -> Analysis<CanonicalError> {
    let state = match branch {
        TerminalBranch::Succeeded { result, upstream } => CheckState::Succeeded {
            upstream: merge_upstream_identity(upstream, item),
            result,
        },
        TerminalBranch::Failed { error } => CheckState::Failed {
            upstream: item.upstream_task_id.as_ref().map(|task| UpstreamIdentity {
                task_id: Some(task.clone()),
                last_stage: item
                    .last_stage()
                    .and_then(|stage| crate::domain::NonEmptyString::new(stage.to_owned()).ok()),
            }),
            error,
        },
    };
    let checks =
        crate::domain::OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: observed_provenance
            .and_then(|provenance| provenance.upstream_version.clone()),
        upstream_task_ids: observed_provenance
            .and_then(|provenance| provenance.upstream_task_ids.clone())
            .or_else(|| {
                item.upstream_task_id
                    .as_ref()
                    .map(|task| UpstreamTaskIds::new(vec![task.clone()]).expect("one ID"))
            }),
        upstream_bulk_id: Some(upstream_bulk_id.clone()),
        submitted_at: None,
        completed_at: observed_provenance
            .and_then(|provenance| provenance.completed_at)
            .or(Some(observed_at)),
    };
    Analysis::with_optional_input(
        AnalysisId::new(),
        crate::domain::SubmissionOutcome::Accepted,
        input,
        checks,
        SaveState::SavedHistory,
        provenance,
        None,
        None,
        observed_at,
        observed_at,
        Some(observed_at),
    )
    .expect("an observed terminal child satisfies the analysis invariants")
}

/// Keeps the validated result analysis as the evidence owner, then enriches
/// its text descriptor with only the local source facts the plan can attest:
/// origin, basename, and locally held plaintext. In particular, the local
/// presentation never replaces the result document's hash or counts.
fn merge_observed_and_local_input(
    observed: Option<AnalysisInput>,
    local: Option<AnalysisInput>,
) -> Option<AnalysisInput> {
    match (observed, local) {
        (Some(AnalysisInput::Text(observed)), Some(AnalysisInput::Text(local))) => TextInput::new(
            local.origin(),
            local.name().map(str::to_owned),
            observed.sha256,
            observed.byte_count,
            observed.word_count,
            local.text,
        )
        .ok()
        .map(AnalysisInput::Text),
        (Some(observed), _) => Some(observed),
        (None, local) => local,
    }
}

/// The result analysis owns stage-bearing upstream evidence. The page item's
/// task ID fills only a missing task field and never clears the last stage.
fn merge_upstream_identity(
    upstream: Option<UpstreamIdentity>,
    item: &BulkItem<CanonicalError>,
) -> Option<UpstreamIdentity> {
    upstream
        .map(|identity| UpstreamIdentity {
            task_id: identity.task_id.or_else(|| item.upstream_task_id.clone()),
            last_stage: identity.last_stage,
        })
        .or_else(|| upstream_identity(item))
}

/// The trusted input descriptor for an acceptance child, honoring its
/// source: a JSONL-file submission reports the `file` origin with the
/// source basename and the child's own submitted text (locally held
/// plaintext, contracts section 9.1); a stdin submission reports `stdin`.
fn accepted_child_input(
    item: &crate::domain::BulkSubmissionItem,
    source_name: Option<&str>,
) -> Result<AnalysisInput, crate::domain::DomainError> {
    let text = item.text();
    let byte_count = u64::try_from(text.len()).unwrap_or(u64::MAX);
    let sha = Sha256Hash::digest(text.as_bytes());
    let (origin, name) = match source_name {
        Some(name) => (TextOrigin::File, Some(name.to_owned())),
        None => (TextOrigin::Stdin, None),
    };
    let input = TextInput::new(
        origin,
        name,
        sha,
        byte_count,
        item.word_count(),
        Some(text.to_owned()),
    )?;
    Ok(AnalysisInput::Text(input))
}

/// One observed in-flight child: queued or running with the observed
/// upstream bulk identity as its accepted-outcome evidence, no completed
/// stamp, and no fabricated result or error.
fn project_inflight_child(
    item: &BulkItem<CanonicalError>,
    state: CheckState<crate::domain::AiDetectionResult, CanonicalError>,
    upstream_bulk_id: &UpstreamBulkId,
    plan: Option<&BulkSubmissionPlan>,
    context: &ChildInputContext,
    observed_at: UtcTimestamp,
) -> Analysis<CanonicalError> {
    let checks =
        crate::domain::OrderedChecks::new([Check::AiDetection(state)]).expect("one check is valid");
    let provenance = Provenance {
        provider: Provider::Pangram,
        upstream_version: None,
        upstream_task_ids: None,
        upstream_bulk_id: Some(upstream_bulk_id.clone()),
        submitted_at: None,
        completed_at: None,
    };
    Analysis::with_optional_input(
        AnalysisId::new(),
        crate::domain::SubmissionOutcome::Accepted,
        descriptor_for_index(plan, item.index, context),
        checks,
        SaveState::SavedHistory,
        provenance,
        None,
        None,
        observed_at,
        observed_at,
        None,
    )
    .expect("an observed in-flight child satisfies the analysis invariants")
}

/// The input descriptor the read actually holds for one position: the
/// trusted plan descendant with its own text (file source) or counts only
/// (stdin source); `None` when the process holds no local descriptor at
/// all (a `bulk status`/`bulk wait` of a remotely authored job, contracts
/// 4.6). Upstream result text is never reparsed as the item's input.
fn descriptor_for_index(
    plan: Option<&BulkSubmissionPlan>,
    index: u64,
    context: &ChildInputContext,
) -> Option<AnalysisInput> {
    let item = plan.and_then(|plan| plan.items().get(usize::try_from(index).ok()?))?;
    accepted_child_input(item, context.source_name.as_deref()).ok()
}
