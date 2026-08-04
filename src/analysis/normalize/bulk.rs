//! Bulk upstream normalization: raw documented bulk wire documents -> typed
//! values the analysis core assembles into canonical collections and pages.
//!
//! One disciplined seam (architecture section 6.2): unknown or impossible
//! required state (status tokens, counter equations, identity mismatches,
//! malformed epoch-second timestamps) fails fail-closed as
//! `upstream_contract_changed` with a sanitized `(field, token)` detail set.
//! Response content (item text, error strings, task results) never crosses
//! into error values; only structural field paths and bounded sanitized
//! tokens do.

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;

use crate::domain::{AnalysisStatus, BulkCounters, NonEmptyString, UpstreamBulkId, UpstreamTaskId};
use crate::output::CanonicalError;

use super::contract_changed;

/// The validated typed submit acceptance: the job identity and the ordered
/// accepted/failed item outcomes, cross-checked against the plan's validated
/// input count. Preserving index, caller ID, and task ID exactly is the
/// anti-fabrication guarantee.
#[derive(Clone)]
pub(in crate::analysis) struct NormalizedBulkPlan {
    pub(in crate::analysis) accepted: Vec<NormalizedBulkItemOutcome>,
    pub(in crate::analysis) failed: Vec<NormalizedBulkItemOutcome>,
}

/// One item outcome in the submit acceptance or a results page. `task_id` is
/// present only when the worker actually returned one (accepted items);
/// `error` is present only for a failed item. The canonical caller ID is
/// resolved from the validated local plan by source index (not echoed here),
/// so the worker cannot substitute identities.
#[derive(Clone)]
pub(in crate::analysis) struct NormalizedBulkItemOutcome {
    pub(in crate::analysis) index: u64,
    pub(in crate::analysis) task_id: Option<UpstreamTaskId>,
    pub(in crate::analysis) last_stage: Option<String>,
    pub(in crate::analysis) error: Option<String>,
}

/// The normalized job status: counters, the worker status token mapped onto
/// the section 9 precedence, and converted RFC 3339 timestamps. No upstream
/// content (item text or results) is present, so a Debug form is safe.
#[derive(Debug)]
pub(in crate::analysis) struct NormalizedBulkStatus {
    pub(in crate::analysis) status: AnalysisStatus,
    pub(in crate::analysis) counters: BulkCounters,
    pub(in crate::analysis) created_at: crate::domain::UtcTimestamp,
    pub(in crate::analysis) completed_at: Option<crate::domain::UtcTimestamp>,
}

/// The exact closed status token the client accepts on the 202 acceptance
/// body (contracts section 9.1). Between the acceptance and the first status
/// read the client has no network authority that could set a later value, so
/// `queued` is the only legal start-of-life marker; anything else fails
/// closed as contract drift.
const ACCEPTANCE_STATUS_TOKEN: &str = "queued";

/// Validates the documented 202 acceptance document against the plan's
/// validated input count. The `total_items`, accepted list, and failed list
/// MUST cover positions `0..total_items` exactly once in ascending order
/// (contracts section 9); any gap, duplicate, or out-of-range position is
/// contract drift, as is a `total_items` that disagrees with the validated
/// plan. The acceptance `status` token must equal the pinned
/// [`ACCEPTANCE_STATUS_TOKEN`]; anything else fails closed as drift.
pub(in crate::analysis) fn normalize_bulk_acceptance(
    acceptance: &crate::domain::BulkSubmitResponse,
    expected_total: u64,
) -> Result<NormalizedBulkPlan, CanonicalError> {
    if acceptance.status.as_str() != ACCEPTANCE_STATUS_TOKEN {
        return Err(contract_changed(
            "status",
            format!(
                "acceptance status {} is not the pinned {ACCEPTANCE_STATUS_TOKEN} token",
                acceptance.status
            ),
        ));
    }
    if acceptance.total_items != expected_total {
        return Err(contract_changed(
            "total_items",
            format!(
                "acceptance total {} disagrees with the validated plan count {}",
                acceptance.total_items, expected_total
            ),
        ));
    }
    let total = acceptance.total_items;
    // The upstream bulk identity is validated for non-emptiness here; the
    // caller already received it as the typed `AcceptedBulk::bulk_id`.
    UpstreamBulkId::new(acceptance.bulk_id.as_str())
        .map_err(|_| contract_changed("bulk_id", "empty"))?;

    let mut covered = HashSet::with_capacity(usize::try_from(total).unwrap_or(0));
    let mut accepted = Vec::with_capacity(acceptance.accepted_items.len());
    for item in &acceptance.accepted_items {
        if item.index >= total {
            return Err(contract_changed(
                "accepted_items.index",
                format!("index {} is at or above total_items {total}", item.index),
            ));
        }
        if !covered.insert(item.index) {
            return Err(contract_changed(
                "accepted_items.index",
                format!("source position {} is duplicated", item.index),
            ));
        }
        accepted.push(NormalizedBulkItemOutcome {
            index: item.index,
            task_id: Some(
                UpstreamTaskId::new(item.task_id.as_str())
                    .map_err(|_| contract_changed("accepted_items.task_id", "empty"))?,
            ),
            last_stage: None,
            error: None,
        });
    }
    let mut failed = Vec::with_capacity(acceptance.failed_items.len());
    for item in &acceptance.failed_items {
        if item.index >= total {
            return Err(contract_changed(
                "failed_items.index",
                format!("index {} is at or above total_items {total}", item.index),
            ));
        }
        if !covered.insert(item.index) {
            return Err(contract_changed(
                "failed_items.index",
                format!("source position {} is duplicated", item.index),
            ));
        }
        failed.push(NormalizedBulkItemOutcome {
            index: item.index,
            task_id: None,
            last_stage: item
                .stage
                .as_ref()
                .map(|stage| super::sanitize_upstream_message(stage.as_str())),
            error: Some(
                item.error
                    .clone()
                    .unwrap_or_else(|| "rejected by immediate upstream validation".to_owned()),
            ),
        });
    }
    if u64::try_from(covered.len()).unwrap_or(u64::MAX) != total {
        return Err(contract_changed(
            "accepted_items",
            format!(
                "the accepted and failed lists cover {} positions, not all {total}",
                covered.len()
            ),
        ));
    }
    Ok(NormalizedBulkPlan { accepted, failed })
}

/// Normalizes one job status document. `expected_total` is the validated
/// input count when known (accepted submissions); `None` accepts the
/// upstream `total_items` (status-only reads rehydrate the total but keep an
/// honest estimate). The status token maps onto the section 9 precedence and
/// MUST agree with the exact counters; a disagreeing or out-of-precedence
/// combination is contract drift.
pub(in crate::analysis) fn normalize_bulk_status(
    status: &crate::domain::BulkStatusResponse,
    expected_total: Option<u64>,
) -> Result<NormalizedBulkPlanStatus, CanonicalError> {
    let total = match expected_total {
        Some(total) => {
            if status.total_items != total {
                return Err(contract_changed(
                    "total_items",
                    format!(
                        "status total {} disagrees with the validated plan count {total}",
                        status.total_items
                    ),
                ));
            }
            total
        }
        None => validate_untrusted_total_items(status.total_items)?,
    };
    let counters = BulkCounters::new(total, status.accepted, status.succeeded, status.failed)
        .map_err(|_| {
            contract_changed(
                "counters",
                format!(
                    "total_items={total} accepted={} succeeded={} failed={}",
                    status.accepted, status.succeeded, status.failed
                ),
            )
        })?;
    let status_value = normalize_bulk_status_token(status.status.as_str(), &counters)?;
    let created_at = parse_epoch_seconds("created_at", status.created_at.as_str())?;
    let completed_at = match &status.completed_at {
        Some(raw) => Some(parse_epoch_seconds("completed_at", raw.as_str())?),
        None => None,
    };
    // A terminal state requires a completed_at; a non-terminal state must
    // not carry one (contracts section 9.1). This is a reciprocal relation
    // JSON Schema cannot express.
    match (counters.is_terminal(), completed_at.is_some()) {
        (true, false) => {
            return Err(contract_changed(
                "completed_at",
                "terminal without a completed_at",
            ));
        }
        (false, true) => {
            return Err(contract_changed(
                "completed_at",
                "a completed_at on a non-terminal job",
            ));
        }
        _ => {}
    }
    Ok(NormalizedBulkPlanStatus {
        status: NormalizedBulkStatus {
            status: status_value,
            counters,
            created_at,
            completed_at,
        },
        bulk_id: UpstreamBulkId::new(status.bulk_id.as_str())
            .map_err(|_| contract_changed("bulk_id", "empty"))?,
    })
}

/// The bundle a successful status normalization yields: the typed status plus
/// the upstream identity. No upstream content is present, so Debug is safe.
#[derive(Debug)]
pub(in crate::analysis) struct NormalizedBulkPlanStatus {
    pub(in crate::analysis) status: NormalizedBulkStatus,
    pub(in crate::analysis) bulk_id: UpstreamBulkId,
}

/// The documented hard bound on an upstream-reported `total_items` when no
/// trusted local plan exists (a resumed remote handle). A job bills every
/// valid item at a minimum of one started-100-word unit and a request is
/// capped at 1,000 billable units, so no valid job exceeds 1,000 items.
/// A larger reported count fails closed as contract drift; the client never
/// allocates from an unchecked count (contracts section 9.1).
fn validate_untrusted_total_items(total: u64) -> Result<u64, CanonicalError> {
    // The item-count ceiling deliberately reuses the billable-unit limit: the
    // bound holds only while every item bills at least one unit (contracts
    // 9.1), so a future billing-rule change could silently change the item
    // ceiling. Assert the coupling in one place so the invariant is explicit
    // and any divergence fails a test rather than slipping through.
    const _: () = assert!(
        crate::domain::MIN_ITEM_BILLABLE_UNITS == 1,
        "the 1,000-item ceiling assumes every item bills at least one unit"
    );
    if total > crate::domain::BULK_BILLABLE_UNIT_LIMIT {
        return Err(contract_changed(
            "total_items",
            format!("reported total_items {total} exceeds the documented 1,000-unit job cap"),
        ));
    }
    Ok(total)
}

/// Maps the worker status token onto the section 9 precedence and rejects
/// drift. The worker uses `queued`/`running` for in-progress jobs and
/// `succeeded`/`failed`/`partial` for terminal jobs. When some but not all
/// accepted items have finished the collection is `running`, so a `partial`
/// token on a non-terminal counter set is contract drift; a terminal set
/// with a disagreeing token is contract drift.
fn normalize_bulk_status_token(
    token: &str,
    counters: &BulkCounters,
) -> Result<AnalysisStatus, CanonicalError> {
    let parsed = match token {
        "queued" => AnalysisStatus::Queued,
        "running" => AnalysisStatus::Running,
        "succeeded" => AnalysisStatus::Succeeded,
        "failed" => AnalysisStatus::Failed,
        "partial" => AnalysisStatus::Partial,
        other => return Err(contract_changed("status", other)),
    };
    // Enforce the precedence on the parsed token. Non-terminal counters only
    // ever map to queued/running; terminal counters only ever map to one of
    // the three terminal equations.
    let valid = match parsed {
        AnalysisStatus::Queued | AnalysisStatus::Running => !counters.is_terminal(),
        AnalysisStatus::Succeeded => {
            counters.succeeded() == counters.total_items() && counters.failed() == 0
        }
        AnalysisStatus::Failed => {
            counters.failed() == counters.total_items() && counters.succeeded() == 0
        }
        AnalysisStatus::Partial => {
            counters.is_terminal() && counters.succeeded() > 0 && counters.failed() > 0
        }
    };
    if !valid {
        return Err(contract_changed(
            "status",
            format!(
                "status {token} disagrees with counters total_items={} accepted={} succeeded={} failed={}",
                counters.total_items(),
                counters.accepted(),
                counters.succeeded(),
                counters.failed()
            ),
        ));
    }
    Ok(parsed)
}

/// Converts a documented Unix epoch-seconds string (`"1760000000.0"`) into a
/// canonical RFC 3339 UTC timestamp. A malformed or out-of-range value is
/// contract drift. The fractional part is tolerated (`.0`) because the
/// documented form is a stringified float; sub-second precision is dropped.
fn parse_epoch_seconds(
    field: &'static str,
    raw: &str,
) -> Result<crate::domain::UtcTimestamp, CanonicalError> {
    // Split off an optional single fractional suffix; the integer part is the
    // epoch second. Reject extra dots, signs in the fraction, and empties.
    let (integer, fraction) = match raw.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (raw, None),
    };
    if let Some(fraction) = fraction {
        if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return Err(contract_changed(field, raw));
        }
    }
    let seconds: i64 = integer.parse().map_err(|_| contract_changed(field, raw))?;
    let timestamp = Timestamp::from_second(seconds).map_err(|_| contract_changed(field, raw))?;
    Ok(crate::domain::UtcTimestamp::from_jiff(timestamp))
}

/// A validated typed bulk items-metadata page. Page integrity (identity,
/// echoed page window, total_items consistency, ascending strict order,
/// in-window positions) is enforced by the page assembler; this type carries
/// the per-item typed outcome after per-item validation.
pub(in crate::analysis) struct NormalizedItemsPage {
    pub(in crate::analysis) outcomes: Vec<NormalizedBulkItemOutcome>,
}

/// A validated typed bulk results page: the per-index items-list outcomes
/// (succeeded or in-progress) and the per-index failed outcomes, kept
/// distinct so the assembler preserves the worker's own split.
pub(in crate::analysis) struct NormalizedResultsPage {
    pub(in crate::analysis) succeeded: HashMap<u64, NormalizedItemResult>,
    pub(in crate::analysis) failed: HashMap<u64, NormalizedBulkItemOutcome>,
}

/// One results-page items-list entry: a terminal success carrying the
/// normalized Pangram 4 task document, or an in-progress (`result: null`)
/// child with no content yet (content never surfaces in errors).
pub(in crate::analysis) enum NormalizedItemResult {
    /// A terminal success document and the worker's task identity.
    Succeeded(NormalizedBulkSuccess),
    /// A documented in-progress entry (`result: null`): a running child.
    Running {
        task_id: Option<UpstreamTaskId>,
        last_stage: Option<String>,
    },
}

/// One succeeded result item: the task identity and the normalized Pangram 4
/// task document (content never surfaces in errors).
pub(in crate::analysis) struct NormalizedBulkSuccess {
    pub(in crate::analysis) task_id: Option<UpstreamTaskId>,
    pub(in crate::analysis) task: Box<super::NormalizedTask>,
}

impl NormalizedBulkSuccess {
    /// The in-progress result-item outcome: no content, only the worker's
    /// optional task identity (which may be null for very early items).
    fn running(
        task_id: Option<UpstreamTaskId>,
        last_stage: Option<String>,
    ) -> NormalizedItemResult {
        NormalizedItemResult::Running {
            task_id,
            last_stage,
        }
    }
}

impl std::fmt::Debug for NormalizedBulkSuccess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NormalizedBulkSuccess")
            .field("task_id", &self.task_id)
            .finish_non_exhaustive()
    }
}

/// A parsed page envelope shared by the items and results decode paths.
/// Carries the echoed window and identity for the assembler's integrity
/// checks, plus `total_items` for cross-page consistency.
///
/// The cross-page `total_items` expectation is threaded through every page
/// normalization: `None` seeds it from the first page, `Some` enforces that
/// later pages agree; a disagreement is contract drift (contracts 9.1).
pub(in crate::analysis) struct RawPageHeader {
    pub(in crate::analysis) offset: u64,
    pub(in crate::analysis) limit: u64,
    pub(in crate::analysis) total_items: u64,
}

impl RawPageHeader {
    /// Validates the echoed page window (offset/limit) against the exact
    /// request, then checks `total_items` for cross-page consistency.
    /// `expected_total`: `None` seeds the expectation from this page; `Some`
    /// enforces agreement. The window echo is always validated.
    pub(in crate::analysis) fn validate_window(
        &self,
        expected_offset: u64,
        expected_limit: u64,
        expected_total: &mut Option<u64>,
    ) -> Result<(), CanonicalError> {
        // Never build page structures from an unchecked count: a reported
        // total that exceeds the documented job cap is contract drift before
        // any coverage or allocation uses it.
        validate_untrusted_total_items(self.total_items)?;
        if self.offset != expected_offset {
            return Err(contract_changed(
                "offset",
                format!(
                    "page offset {} does not echo request {expected_offset}",
                    self.offset
                ),
            ));
        }
        if self.limit != expected_limit {
            return Err(contract_changed(
                "limit",
                format!(
                    "page limit {} does not echo request {expected_limit}",
                    self.limit
                ),
            ));
        }
        match expected_total {
            None => *expected_total = Some(self.total_items),
            Some(expected) if *expected != self.total_items => {
                return Err(contract_changed(
                    "total_items",
                    format!("page total {} disagrees across pages", self.total_items),
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

/// Parses and validates one items-metadata page. The identity, window, order,
/// and per-item shape checks all run here so the assembler only composes.
pub(in crate::analysis) fn normalize_items_page(
    response: &crate::domain::BulkItemsPage,
    expected_bulk_id: &UpstreamBulkId,
    expected_offset: u64,
    expected_limit: u64,
    expected_total: &mut Option<u64>,
) -> Result<(RawPageHeader, NormalizedItemsPage), CanonicalError> {
    validate_page_identity(&response.bulk_id, expected_bulk_id)?;
    let header = RawPageHeader {
        offset: response.offset,
        limit: response.limit,
        total_items: response.total_items,
    };
    header.validate_window(expected_offset, expected_limit, expected_total)?;
    // Never index the caller's plan from an unchecked count: bound the
    // reported total before any in-window position reaches `plan_item_at`.
    validate_total_within_window(header.total_items)?;
    validate_ascending(response.items.iter().map(|item| item.index))?;
    let mut outcomes = Vec::with_capacity(response.items.len());
    for item in &response.items {
        validate_index_in_window(item.index, header.offset, header.limit, header.total_items)?;
        outcomes.push(item_outcome(
            item.index,
            item.task_id.as_ref(),
            item.stage.as_ref(),
            item.error.clone(),
            item.error.is_some(),
        )?);
    }
    Ok((header, NormalizedItemsPage { outcomes }))
}

/// Parses and validates one results page: the `items` list (succeeded or
/// in-progress) and the separate `failed_items` list. An in-progress entry
/// carrying the documented `result: null` is a valid `running` child, not
/// contract drift. Each succeeded entry's `result` is normalized through the
/// shared Pangram 4 success validator (which enforces the `STAGE_SUCCESS`
/// stage, the `4.0` version, and humanizer evidence). Each of the two lists
/// is strictly ascending by source `index` on its own; cross-list integrity
/// is disjointness plus coverage, not a chained ordering across the lists.
pub(in crate::analysis) fn normalize_results_page(
    response: &crate::domain::BulkResultsPage,
    expected_bulk_id: &UpstreamBulkId,
    expected_offset: u64,
    expected_limit: u64,
    expected_total: &mut Option<u64>,
) -> Result<(RawPageHeader, NormalizedResultsPage), CanonicalError> {
    validate_page_identity(&response.bulk_id, expected_bulk_id)?;
    let header = RawPageHeader {
        offset: response.offset,
        limit: response.limit,
        total_items: response.total_items,
    };
    header.validate_window(expected_offset, expected_limit, expected_total)?;
    // Never index the caller's plan from an unchecked count: bound the
    // reported total before any in-window position reaches `plan_item_at`.
    validate_total_within_window(header.total_items)?;
    // Each list is strictly ascending on its own; cross-list integrity is
    // disjointness (checked below), not a chained ordering across the lists.
    validate_ascending(response.items.iter().map(|item| item.index))?;
    validate_ascending(response.failed_items.iter().map(|item| item.index))?;
    for index in response.items.iter().map(|item| item.index) {
        validate_index_in_window(index, header.offset, header.limit, header.total_items)?;
    }
    for index in response.failed_items.iter().map(|item| item.index) {
        validate_index_in_window(index, header.offset, header.limit, header.total_items)?;
    }

    let mut succeeded = HashMap::with_capacity(response.items.len());
    for item in &response.items {
        let task_id = item
            .task_id
            .as_ref()
            .map(|task| {
                UpstreamTaskId::new(task.as_str())
                    .map_err(|_| contract_changed("items.task_id", "empty"))
            })
            .transpose()?;
        match &item.result {
            Some(document) => {
                // The raw document still carries its stage token; the shared
                // validator enforces STAGE_SUCCESS and the full Pangram 4
                // success shape.
                let state = super::normalize_task_state(document)?;
                let task = match state {
                    super::TaskState::Success(task) => task,
                    // A present result document must be a terminal success.
                    // Anything else on an items-list entry is drift.
                    _ => {
                        return Err(contract_changed(
                            "items.result",
                            "a present result document that is not a terminal success",
                        ));
                    }
                };
                if succeeded
                    .insert(
                        item.index,
                        NormalizedItemResult::Succeeded(NormalizedBulkSuccess { task_id, task }),
                    )
                    .is_some()
                {
                    return Err(contract_changed(
                        "items.index",
                        format!("duplicate succeeded source position {}", item.index),
                    ));
                }
            }
            None => {
                // A documented `result: null` entry is an in-progress
                // (running) child; the items list may report it mixed with
                // completed siblings. Preserve it as an in-flight outcome
                // rather than treating the null as a missing result.
                if succeeded
                    .insert(
                        item.index,
                        NormalizedBulkSuccess::running(
                            task_id,
                            item.stage
                                .as_ref()
                                .map(|stage| super::sanitize_upstream_message(stage.as_str())),
                        ),
                    )
                    .is_some()
                {
                    return Err(contract_changed(
                        "items.index",
                        format!("duplicate source position {}", item.index),
                    ));
                }
            }
        }
    }

    let mut failed = HashMap::with_capacity(response.failed_items.len());
    for item in &response.failed_items {
        let outcome = item_outcome(
            item.index,
            item.task_id.as_ref(),
            item.stage.as_ref(),
            item.error.clone(),
            true,
        )?;
        if failed.insert(item.index, outcome).is_some() {
            return Err(contract_changed(
                "failed_items.index",
                format!("duplicate failed source position {}", item.index),
            ));
        }
    }
    // A position must not appear in both the items and failed lists.
    for index in succeeded.keys() {
        if failed.contains_key(index) {
            return Err(contract_changed(
                "index",
                format!("source position {index} appears as both succeeded and failed"),
            ));
        }
    }
    Ok((header, NormalizedResultsPage { succeeded, failed }))
}

/// Never index the caller's validated plan from an unchecked page total.
/// The normalizer bounds `total_items` in [`RawPageHeader::validate_window`]
/// against the documented job cap only when no trusted plan count is known;
/// this second check is a defense-in-depth ceiling applied to every page so
/// a window item can never drive `plan_item_at` out of a trusted plan.
fn validate_total_within_window(total_items: u64) -> Result<(), CanonicalError> {
    validate_untrusted_total_items(total_items).map(|_| ())
}

/// Every reported position must lie inside the requested window
/// (`offset..min(offset + limit, total_items)`) and below the validated
/// `total_items`. An out-of-window or at-or-above-total position is contract
/// drift; without this bound a page read of a remotely authored job could
/// feed an unchecked index (including `u64::MAX`) into the assembler's
/// `next_offset` (which computes `max + 1`) and into the caller's reassembled
/// set. Applied to every items entry and to both results lists so all page
/// reads fail closed (contracts 9.1).
fn validate_index_in_window(
    index: u64,
    offset: u64,
    limit: u64,
    total_items: u64,
) -> Result<(), CanonicalError> {
    let end = offset.saturating_add(limit).min(total_items);
    if index < offset || index >= end {
        return Err(contract_changed(
            "index",
            format!("source position {index} is outside the requested window {offset}..{end}"),
        ));
    }
    Ok(())
}

/// One items-page entry or failed entry shared outcome. `task_id` is
/// required absent for a failed entry and required present for an accepted
/// one only in the submit acceptance; item metadata pages tolerate a null
/// task_id on in-flight work, so the caller passes `failed` to decide.
fn item_outcome(
    index: u64,
    task_id: Option<&NonEmptyString>,
    stage: Option<&NonEmptyString>,
    error: Option<String>,
    failed: bool,
) -> Result<NormalizedBulkItemOutcome, CanonicalError> {
    let task_id = task_id
        .map(|task| {
            UpstreamTaskId::new(task.as_str()).map_err(|_| contract_changed("task_id", "empty"))
        })
        .transpose()?;
    if failed && error.is_none() {
        return Err(contract_changed(
            "error",
            format!("a failed item at index {index} carries no error"),
        ));
    }
    Ok(NormalizedBulkItemOutcome {
        index,
        task_id,
        last_stage: stage.map(|stage| super::sanitize_upstream_message(stage.as_str())),
        error: if failed { error } else { None },
    })
}

fn validate_page_identity(
    page_bulk_id: &NonEmptyString,
    expected: &UpstreamBulkId,
) -> Result<(), CanonicalError> {
    if page_bulk_id.as_str() != expected.as_str() {
        return Err(contract_changed(
            "bulk_id",
            "page identity does not match the queried job",
        ));
    }
    Ok(())
}

/// Page entries MUST be strictly ascending by source index (duplicates and
/// out-of-order positions are drift). `BulkPage::new` enforces the same on
/// the canonical side; this is the upstream-facing pre-check.
fn validate_ascending(indexes: impl Iterator<Item = u64>) -> Result<(), CanonicalError> {
    let mut previous: Option<u64> = None;
    for index in indexes {
        if let Some(previous) = previous {
            if index <= previous {
                return Err(contract_changed(
                    "index",
                    format!(
                        "source positions are not strictly ascending ({previous} then {index})"
                    ),
                ));
            }
        }
        previous = Some(index);
    }
    Ok(())
}
