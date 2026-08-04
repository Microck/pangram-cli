//! The read-only typed page reads of one running bulk job: one validated
//! items-metadata page, one validated results page, and the bounded fetch-all
//! results walk. These are the observation-free `GET` operations: each takes
//! the running handle and issues its safe-GET through the shared client fetch
//! chain, normalizes and assembles the canonical page, and cross-checks the
//! echoed window and `total_items`. The page-only error mapping
//! (`page_poll_error`, `bulk_domain_error`) and the fetch-all page constant
//! live here next to their only callers; the status-fetch and wait-timeout
//! helpers shared with the hub live in the parent `mod.rs`.

use crate::domain::{BulkId, BulkItem, BulkPage};
use crate::output::{CanonicalError, ErrorCode};

use tokio_util::sync::CancellationToken;

use super::super::config::Clock;
use super::super::normalize::bulk;
use super::super::upstream::PollError;
use super::assemble::{
    assemble_items_metadata_page, assemble_results_page, next_offset, validate_page_request,
};
use super::{
    BulkAnalysisError, BulkAnalyzer, BulkPageResult, BulkProgress, RunningBulk, contract_symptom,
};

use super::super::upstream::BulkPageFetch;

/// The page size the fetch-all walk requests internally. This is the
/// conservative bounded fetch-all page size (contracts section 9.1): explicit
/// one-page requests may still use the documented `1..=1,000` window, but the
/// internal walk never requests the maximum page, so one received results
/// page stays well below the client's 16 MiB hard response cap.
const FETCH_ALL_PAGE_SIZE: u64 = crate::domain::BULK_FETCH_ALL_PAGE_SIZE;

impl<C: Clock> BulkAnalyzer<C> {
    /// Fetches one validated typed items-metadata page (read-only). The page
    /// identity, echoed window, `total_items` consistency, ordering, and
    /// per-item shape are enforced during normalization; the canonical page
    /// strictly ascends by source index.
    pub async fn bulk_items_page(
        &self,
        running: &RunningBulk<C>,
        offset: u64,
        limit: u64,
        cancel: &CancellationToken,
    ) -> Result<BulkPageResult, BulkAnalysisError> {
        validate_page_request(limit)
            .map_err(|error| BulkAnalysisError::new(running.bulk_id(), error))?;
        let url = self
            .client
            .bulk_items_url(running.upstream_bulk_id().as_str(), offset, limit);
        let fetch = self
            .client
            .fetch_bulk_page(url, cancel, None)
            .await
            .map_err(|error| page_poll_error(running, error))?;
        let BulkPageFetch::Page(response) = fetch else {
            return Err(BulkAnalysisError::new(
                running.bulk_id(),
                CanonicalError::new(
                    ErrorCode::UpstreamNotFound,
                    "Pangram does not recognize the bulk job.",
                )
                .expect("static template"),
            ));
        };
        let response = *response;
        let wire: crate::domain::BulkItemsPage = response.json().map_err(|error| {
            BulkAnalysisError::new(
                running.bulk_id(),
                contract_symptom("body", error.to_string()),
            )
        })?;
        let mut expected_total: Option<u64> = None;
        let (header, page) = bulk::normalize_items_page(
            &wire,
            running.upstream_bulk_id(),
            offset,
            limit,
            &mut expected_total,
        )
        .map_err(|error| BulkAnalysisError::new(running.bulk_id(), error))?;
        let total = header.total_items;
        let items = assemble_items_metadata_page(running.plan(), running.upstream_bulk_id(), page)
            .map_err(|error| BulkAnalysisError::new(running.bulk_id(), error))?;
        let next = next_offset(&items, total);
        Ok(BulkPageResult {
            page: BulkPage::new(items, offset, limit, next)
                .map_err(|error| bulk_domain_error(running.bulk_id(), error))?,
            total_items: total,
            upstream_bulk_id: running.upstream_bulk_id().clone(),
        })
    }

    /// Fetches one validated typed results page (read-only). Succeeded items
    /// carry a canonical analysis built from the local plan's trusted input;
    /// in-progress (`result: null`) items surface as running children; failed
    /// items carry the sanitized upstream error. The canonical page strictly
    /// ascends by source index.
    pub async fn bulk_results_page(
        &self,
        running: &RunningBulk<C>,
        offset: u64,
        limit: u64,
        cancel: &CancellationToken,
    ) -> Result<BulkPageResult, BulkAnalysisError> {
        validate_page_request(limit)
            .map_err(|error| BulkAnalysisError::new(running.bulk_id(), error))?;
        let url = self
            .client
            .bulk_results_url(running.upstream_bulk_id().as_str(), offset, limit);
        let fetch = self
            .client
            .fetch_bulk_page(url, cancel, None)
            .await
            .map_err(|error| page_poll_error(running, error))?;
        let BulkPageFetch::Page(response) = fetch else {
            return Err(BulkAnalysisError::new(
                running.bulk_id(),
                CanonicalError::new(
                    ErrorCode::UpstreamNotFound,
                    "Pangram does not recognize the bulk job.",
                )
                .expect("static template"),
            ));
        };
        let response = *response;
        let wire: crate::domain::BulkResultsPage = response.json().map_err(|error| {
            BulkAnalysisError::new(
                running.bulk_id(),
                contract_symptom("body", error.to_string()),
            )
        })?;
        let mut expected_total: Option<u64> = None;
        let (header, page) = bulk::normalize_results_page(
            &wire,
            running.upstream_bulk_id(),
            offset,
            limit,
            &mut expected_total,
        )
        .map_err(|error| BulkAnalysisError::new(running.bulk_id(), error))?;
        let total = header.total_items;
        let items = assemble_results_page(running.plan(), running.upstream_bulk_id(), page)
            .map_err(|error| BulkAnalysisError::new(running.bulk_id(), error))?;
        let next = next_offset(&items, total);
        Ok(BulkPageResult {
            page: BulkPage::new(items, offset, limit, next)
                .map_err(|error| bulk_domain_error(running.bulk_id(), error))?,
            total_items: total,
            upstream_bulk_id: running.upstream_bulk_id().clone(),
        })
    }

    /// Iterates documented results pages from offset 0 until the set is
    /// exhausted, with per-position duplicate/out-of-order/non-advancing
    /// protection. Reads are bounded by the caller's `max_reads`; progress is
    /// reported per page through `on_progress`. Returns the strictly ordered
    /// assembled page over the whole covered set.
    ///
    /// There is no aggregate endpoint; this is iteration over documented
    /// pages only (contracts section 9.1). The walk requests the conservative
    /// bounded fetch-all page size (never the 1,000 maximum), so one received
    /// page stays well below the 16 MiB hard response cap. Completion
    /// requires exact coverage of `0..total_items`; an empty page while
    /// positions remain uncovered is non-advancing drift. Cancellation stops
    /// the walk between page reads.
    pub async fn bulk_results_all(
        &self,
        running: &RunningBulk<C>,
        max_reads: u64,
        cancel: &CancellationToken,
        mut on_progress: impl FnMut(&BulkProgress),
    ) -> Result<BulkPageResult, BulkAnalysisError> {
        if max_reads == 0 {
            return Err(BulkAnalysisError::new(
                running.bulk_id(),
                CanonicalError::new(
                    ErrorCode::InputRequired,
                    "fetch-all requires a positive read bound.",
                )
                .expect("static template"),
            ));
        }
        let mut all_items: Vec<BulkItem<CanonicalError>> = Vec::new();
        let mut covered: Vec<bool> = Vec::new();
        let mut total: Option<u64> = None;
        let mut offset = 0_u64;
        let mut reads = 0_u64;
        // Report one truthful observed state on fetch-all progress: capture
        // the handle's current counters once before the walk and reuse that
        // for every emitted event. `running.last_state()` only advances when
        // `observe`/`snapshot` refresh it, so per-page reads through
        // `bulk_results_page` never move it; emitting it per page would claim
        // the handle's initial (possibly placeholder) state was newly observed
        // work. A single captured snapshot stays honest without paying an
        // extra status round-trip on every page (CodeRabbit finding).
        let (status, counters) = running.last_state();

        loop {
            if cancel.is_cancelled() {
                return Err(BulkAnalysisError::new(
                    running.bulk_id(),
                    CanonicalError::new(
                        ErrorCode::NetworkUnavailable,
                        "The bulk fetch-all was cancelled locally; no remote action was taken.",
                    )
                    .expect("static template"),
                ));
            }
            if reads >= max_reads {
                // The read bound is a local policy limit, not a transport
                // problem: the network was available and every page read
                // succeeded. Report the usage-scoped code consistently with
                // the `max_reads == 0` caller-validation case above.
                return Err(BulkAnalysisError::new(
                    running.bulk_id(),
                    CanonicalError::new(
                        ErrorCode::InputRequired,
                        "The bulk results fetch-all exceeded its read bound.",
                    )
                    .expect("static template"),
                ));
            }
            let page = self
                .bulk_results_page(running, offset, FETCH_ALL_PAGE_SIZE, cancel)
                .await?;
            reads += 1;

            let page_total = page.total_items;
            match total {
                None => {
                    total = Some(page_total);
                    // Bound the coverage bitmap by the validated page total.
                    // `validate_window` already rejected any count above the
                    // documented job cap, so this allocation never grows from
                    // an unchecked u64.
                    covered = vec![false; usize::try_from(page_total).unwrap_or(0)];
                }
                Some(total) if total != page_total => {
                    return Err(BulkAnalysisError::new(
                        running.bulk_id(),
                        contract_symptom("total_items", "results pages disagree on the job total"),
                    ));
                }
                _ => {}
            }
            let total_value = total.expect("seeded above");

            let had_any = !page.page.items().is_empty();
            let mut page_max: Option<u64> = None;
            for item in page.page.items() {
                if item.index >= total_value {
                    return Err(BulkAnalysisError::new(
                        running.bulk_id(),
                        contract_symptom(
                            "index",
                            format!(
                                "source position {} is at or above total {total_value}",
                                item.index
                            ),
                        ),
                    ));
                }
                let slot = &mut covered[usize::try_from(item.index).unwrap_or(usize::MAX)];
                if *slot {
                    return Err(BulkAnalysisError::new(
                        running.bulk_id(),
                        contract_symptom(
                            "index",
                            format!("duplicate source position {} across pages", item.index),
                        ),
                    ));
                }
                *slot = true;
                page_max = Some(item.index);
                all_items.push(item.clone());
            }

            on_progress(&BulkProgress {
                bulk_id: running.bulk_id(),
                status,
                counters,
            });

            // Completion requires exact coverage of 0..total_items; an empty
            // page is never a completion signal while positions remain
            // uncovered, and it is non-advancing drift when it is.
            let covered_count = covered.iter().filter(|covered| **covered).count();
            if u64::try_from(covered_count).unwrap_or(u64::MAX) >= total_value {
                break;
            }
            if !had_any {
                return Err(BulkAnalysisError::new(
                    running.bulk_id(),
                    contract_symptom(
                        "offset",
                        format!(
                            "an empty results page at offset {offset} with {covered_count} of {total_value} positions still uncovered"
                        ),
                    ),
                ));
            }
            // Non-advancing protection: the next offset must strictly exceed
            // the request that produced this page.
            let next = page_max.expect("had_any implies a covered position") + 1;
            if next <= offset {
                return Err(BulkAnalysisError::new(
                    running.bulk_id(),
                    contract_symptom("offset", "the results walk did not advance"),
                ));
            }
            offset = next;
        }

        let total_value = total.unwrap_or(0);
        // The fetch-all aggregate is one canonical page whose synthetic
        // window metadata reports the whole reassembled set, not the
        // 100-item walk granularity: `offset` is 0 and `limit` is
        // `max(1, total_items)` bounded by the documented 1,000-item page
        // cap, so consumers can tell "one complete aggregate" from "one
        // bounded upstream page" (contracts.md 9.1/14.3). `next_offset` is
        // absent (the end-of-set marker).
        let aggregate_limit = total_value.clamp(1, crate::domain::BULK_PAGE_LIMIT_MAX);
        Ok(BulkPageResult {
            page: BulkPage::new(all_items, 0, aggregate_limit, None)
                .map_err(|error| bulk_domain_error(running.bulk_id(), error))?,
            total_items: total_value,
            upstream_bulk_id: running.upstream_bulk_id().clone(),
        })
    }
}

/// Maps one page-read poll error onto the canonical bulk error: a failed
/// fetch copies its canonical code; a cancellation reports local
/// non-network action; a deadline surfaces the wait-timeout identity.
fn page_poll_error<C: Clock>(running: &RunningBulk<C>, error: PollError) -> BulkAnalysisError {
    match error {
        PollError::Failed(error) => BulkAnalysisError::new(running.bulk_id(), *error),
        PollError::Cancelled => BulkAnalysisError::new(
            running.bulk_id(),
            CanonicalError::new(
                ErrorCode::NetworkUnavailable,
                "The bulk page read was cancelled locally; no remote action was taken.",
            )
            .expect("static template"),
        ),
        PollError::DeadlineExceeded => {
            BulkAnalysisError::new(running.bulk_id(), super::running_wait_timeout(running))
        }
    }
}

/// A domain-validation failure during canonical page assembly surfaces as
/// contract drift (the validated page is assembled by the domain owner).
fn bulk_domain_error(bulk_id: BulkId, error: crate::domain::DomainError) -> BulkAnalysisError {
    let mut details = std::collections::BTreeMap::new();
    details.insert(
        "conflict".to_owned(),
        serde_json::Value::from(error.to_string()),
    );
    BulkAnalysisError::new(
        bulk_id,
        CanonicalError::new(
            ErrorCode::UpstreamContractChanged,
            "Pangram returned a bulk document outside the pinned contract.",
        )
        .and_then(|error| error.with_details(details))
        .expect("static template"),
    )
}
