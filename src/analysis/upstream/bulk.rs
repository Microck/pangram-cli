//! The bulk half of the upstream client: the billable `POST /bulk` submit
//! classification and the typed status/items/results page reads, all over
//! the one shared client, pacing gate, and safe-GET retry chain owned by the
//! parent `upstream` module. No second HTTP stack or polling loop exists
//! here; the text and bulk surfaces differ only in their endpoint, status
//! matrix, and acceptance classification.

use tokio_util::sync::CancellationToken;

use super::http::{Response, SendOutcome};
use super::{AnalysisError, PollError, SubmitOutcome, UpstreamClient};
use crate::analysis::config::{Clock, Instant};
use crate::output::{CanonicalError, ErrorCode};

/// A validated bulk acceptance: the upstream job identity plus the raw
/// documented 202 acceptance document. `total_items` and the accepted/failed
/// item lists are cross-checked against the plan's validated count by the
/// normalizer; the analysis core never fabricates task IDs or acceptance
/// certainty beyond this document.
#[derive(Debug, Clone, PartialEq)]
pub struct AcceptedBulk {
    pub bulk_id: crate::domain::UpstreamBulkId,
    /// The documented 202 acceptance body, preserved exactly as decoded.
    pub acceptance: crate::domain::BulkSubmitResponse,
}

/// The result of one bulk status/items/results page fetch. `Page` is the
/// consumed 2xx response handed to the bulk normalizer; `NotFound` is the
/// documented 404 for an unknown job.
pub enum BulkPageFetch {
    Page(Box<Response>),
    NotFound,
}

impl<C: Clock> UpstreamClient<C> {
    /// Submits one validated Pangram 4 bulk request exactly once. This is
    /// billable: on any ambiguous outcome the caller produces
    /// `submission_outcome_unknown` rather than replaying (contracts section
    /// 12.1). The plan is pre-validated by its constructor, so the ceiling
    /// preflight has already run before this call issues any request. Only
    /// HTTP 202 is an acceptance; any other 2xx is ambiguous.
    pub async fn submit_bulk(
        &self,
        plan: &crate::domain::BulkSubmissionPlan,
        cancel: &CancellationToken,
    ) -> Result<AcceptedBulk, SubmitOutcome> {
        let body = plan.submit_body();
        match self.pace(cancel, None).await {
            super::PaceGate::Released => {}
            super::PaceGate::Cancelled | super::PaceGate::DeadlinePassed => {
                return Err(SubmitOutcome::Cancelled);
            }
        }
        let outcome = self.post(&self.bulk_submit_url(), &body, cancel).await;
        match outcome {
            SendOutcome::Responded(response) => classify_bulk_submit(response),
            SendOutcome::Cancelled { issued } => {
                if issued {
                    Err(SubmitOutcome::Ambiguous(AnalysisError::Cancelled))
                } else {
                    Err(SubmitOutcome::Cancelled)
                }
            }
            SendOutcome::Failed {
                delivered_may_have_occurred,
                error,
            } => {
                if delivered_may_have_occurred {
                    Err(SubmitOutcome::Ambiguous(error))
                } else {
                    Err(SubmitOutcome::Failed(Box::new(
                        super::map_transport_failure(&error),
                    )))
                }
            }
        }
    }

    /// Fetches one bulk status, items, or results page through the shared
    /// safe-GET retry chain (pacing, bounded backoff, `Retry-After`, caller
    /// deadline, and the cumulative retry budget). The returned response is
    /// validated by the bulk normalizer. A 404 maps to `BulkPage::NotFound`;
    /// failure statuses use the bulk matrix (413 -> `bulk_limit_exceeded`).
    pub async fn fetch_bulk_page(
        &self,
        url: String,
        cancel: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<BulkPageFetch, PollError> {
        match self
            .get(url.as_str(), cancel, deadline, super::StatusMap::Bulk)
            .await?
        {
            super::SafeGet::TwoHundred(response) => Ok(BulkPageFetch::Page(Box::new(response))),
            super::SafeGet::NotFound => Ok(BulkPageFetch::NotFound),
        }
    }
}

/// Classifies one bulk submit response. Exactly an HTTP `202 Accepted` is
/// an acceptance; every other status is examined. A non-`2xx` status maps
/// through the bulk HTTP matrix; a `2xx` other than 202 is ambiguous because
/// the job may exist remotely and the send must never be replayed; a 202 body
/// that cannot decode into the documented acceptance is ambiguous for the
/// same reason.
fn classify_bulk_submit(response: Response) -> Result<AcceptedBulk, SubmitOutcome> {
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(SubmitOutcome::Failed(Box::new(bulk_http_failure(
            &response,
        ))));
    }
    if status != 202 {
        // A non-202 2xx is not the documented acceptance. The job may exist
        // remotely, so this is ambiguous rather than a failed send or a
        // synthesized acceptance: never replay, surface
        // `submission_outcome_unknown`.
        return Err(SubmitOutcome::Ambiguous(AnalysisError::MalformedBody(
            format!(
                "the bulk submit returned HTTP {status}, not the documented 202; the job may exist remotely"
            ),
        )));
    }
    let wire: crate::domain::BulkSubmitResponse = response.json().map_err(|error| {
        SubmitOutcome::Ambiguous(AnalysisError::MalformedBody(format!(
            "{error} (bulk acceptance malformed; the job may exist remotely)"
        )))
    })?;
    let bulk_id = crate::domain::UpstreamBulkId::new(wire.bulk_id.as_str()).map_err(|_| {
        SubmitOutcome::Ambiguous(AnalysisError::MalformedBody(
            "the bulk acceptance carried an empty bulk_id; the job may exist remotely".into(),
        ))
    })?;
    Ok(AcceptedBulk {
        bulk_id,
        acceptance: wire,
    })
}

/// The bulk failure matrix shares [`super::classify_http_failure`] except for
/// 413, which is the documented bulk over-limit status and must surface the
/// canonical `bulk_limit_exceeded` (contracts section 9.1) rather than the
/// single-text usage code. Every bulk submit/page error funnels through this
/// one override so the code is never double-mapped.
pub(super) fn bulk_http_failure(response: &Response) -> CanonicalError {
    if response.status() == 413 {
        let mut details = std::collections::BTreeMap::new();
        details.insert(
            "http_status".to_owned(),
            serde_json::Value::from(u64::from(response.status())),
        );
        return CanonicalError::new(
            ErrorCode::BulkLimitExceeded,
            "Pangram rejected the bulk submission as over the billable-unit or request-size limit.",
        )
        .and_then(|error| error.with_details(details))
        .expect("the bulk-limit template is statically valid");
    }
    super::classify_http_failure(response, None)
}
