//! Bulk counters, item states, collections, and ordered result pages.

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use super::model::FieldPresence;
use super::{
    Analysis, AnalysisId, AnalysisStatus, BulkId, CheckStatus, DomainError, SubmissionOutcome,
    UpstreamBulkId, UpstreamTaskId, UtcTimestamp, deserialize_missing_only,
};

#[derive(Clone, Copy, Debug, JsonSchema, PartialEq, Eq, Serialize)]
pub struct BulkCounters {
    #[schemars(range(min = 1))]
    total_items: u64,
    accepted: u64,
    succeeded: u64,
    failed: u64,
}

impl BulkCounters {
    pub fn new(
        total_items: u64,
        accepted: u64,
        succeeded: u64,
        failed: u64,
    ) -> Result<Self, DomainError> {
        let finished = succeeded
            .checked_add(failed)
            .ok_or(DomainError::InvalidBulkCounters)?;
        if total_items == 0
            || accepted > total_items
            || succeeded > accepted
            || finished > total_items
        {
            return Err(DomainError::InvalidBulkCounters);
        }
        Ok(Self {
            total_items,
            accepted,
            succeeded,
            failed,
        })
    }

    #[must_use]
    pub fn is_terminal(&self) -> bool {
        self.succeeded + self.failed == self.total_items
    }

    #[must_use]
    pub const fn total_items(&self) -> u64 {
        self.total_items
    }

    #[must_use]
    pub const fn accepted(&self) -> u64 {
        self.accepted
    }

    #[must_use]
    pub const fn succeeded(&self) -> u64 {
        self.succeeded
    }

    #[must_use]
    pub const fn failed(&self) -> u64 {
        self.failed
    }
}

#[derive(Deserialize)]
struct BulkCountersWire {
    total_items: u64,
    accepted: u64,
    succeeded: u64,
    failed: u64,
}

impl<'de> Deserialize<'de> for BulkCounters {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BulkCountersWire::deserialize(deserializer)?;
        Self::new(wire.total_items, wire.accepted, wire.succeeded, wire.failed)
            .map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Eq, Serialize)]
pub struct BulkCollection {
    id: BulkId,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_bulk_id: Option<UpstreamBulkId>,
    status: AnalysisStatus,
    submission_outcome: SubmissionOutcome,
    #[serde(flatten)]
    counters: BulkCounters,
    #[schemars(range(min = 1))]
    estimated_billable_units: u64,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<UtcTimestamp>,
}

impl BulkCollection {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: BulkId,
        upstream_bulk_id: Option<UpstreamBulkId>,
        status: AnalysisStatus,
        submission_outcome: SubmissionOutcome,
        counters: BulkCounters,
        estimated_billable_units: u64,
        created_at: UtcTimestamp,
        updated_at: UtcTimestamp,
        completed_at: Option<UtcTimestamp>,
    ) -> Result<Self, DomainError> {
        if estimated_billable_units == 0 {
            return Err(DomainError::OutOfRange("estimated billable units"));
        }
        let status_is_valid = match status {
            AnalysisStatus::Queued | AnalysisStatus::Running => !counters.is_terminal(),
            AnalysisStatus::Succeeded => {
                counters.succeeded == counters.total_items && counters.failed == 0
            }
            AnalysisStatus::Failed => {
                counters.failed == counters.total_items && counters.succeeded == 0
            }
            AnalysisStatus::Partial => {
                counters.is_terminal() && counters.succeeded > 0 && counters.failed > 0
            }
        };
        if !status_is_valid {
            return Err(DomainError::InvalidBulkStatus);
        }
        let outcome_is_valid = match submission_outcome {
            SubmissionOutcome::Accepted => upstream_bulk_id.is_some(),
            SubmissionOutcome::Terminal => {
                !matches!(status, AnalysisStatus::Queued | AnalysisStatus::Running)
            }
            SubmissionOutcome::NotSubmitted => {
                upstream_bulk_id.is_none()
                    && counters.accepted == 0
                    && counters.succeeded == 0
                    && counters.failed == 0
            }
            SubmissionOutcome::AcceptanceUnknown => true,
        };
        if !outcome_is_valid {
            return Err(DomainError::InvalidSubmissionOutcome);
        }
        Ok(Self {
            id,
            upstream_bulk_id,
            status,
            submission_outcome,
            counters,
            estimated_billable_units,
            created_at,
            updated_at,
            completed_at,
        })
    }

    #[must_use]
    pub const fn status(&self) -> AnalysisStatus {
        self.status
    }

    #[must_use]
    pub const fn counters(&self) -> &BulkCounters {
        &self.counters
    }
}

#[derive(Deserialize)]
struct BulkCollectionWire {
    id: BulkId,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    upstream_bulk_id: Option<UpstreamBulkId>,
    status: AnalysisStatus,
    submission_outcome: SubmissionOutcome,
    #[serde(flatten)]
    counters: BulkCounters,
    estimated_billable_units: u64,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    completed_at: Option<UtcTimestamp>,
}

impl<'de> Deserialize<'de> for BulkCollection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BulkCollectionWire::deserialize(deserializer)?;
        Self::new(
            wire.id,
            wire.upstream_bulk_id,
            wire.status,
            wire.submission_outcome,
            wire.counters,
            wire.estimated_billable_units,
            wire.created_at,
            wire.updated_at,
            wire.completed_at,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BulkItemState<E> {
    Queued,
    Running,
    Succeeded { analysis: Box<Analysis<E>> },
    Failed { error: E },
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "E: Deserialize<'de>"))]
struct BulkItemStateWire<E> {
    status: CheckStatus,
    #[serde(default)]
    analysis: FieldPresence<Analysis<E>>,
    #[serde(default)]
    error: FieldPresence<E>,
}

impl<'de, E> Deserialize<'de> for BulkItemState<E>
where
    E: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BulkItemStateWire::deserialize(deserializer)?;
        match (wire.status, wire.analysis, wire.error) {
            (CheckStatus::Queued, FieldPresence::Missing, FieldPresence::Missing) => {
                Ok(Self::Queued)
            }
            (CheckStatus::Running, FieldPresence::Missing, FieldPresence::Missing) => {
                Ok(Self::Running)
            }
            (CheckStatus::Succeeded, FieldPresence::Present(analysis), FieldPresence::Missing) => {
                Ok(Self::Succeeded {
                    analysis: Box::new(analysis),
                })
            }
            (CheckStatus::Failed, FieldPresence::Missing, FieldPresence::Present(error)) => {
                Ok(Self::Failed { error })
            }
            _ => Err(D::Error::custom(DomainError::InvalidState("bulk item"))),
        }
    }
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize, Deserialize)]
pub struct BulkItem<E> {
    pub index: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub caller_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub analysis_id: Option<AnalysisId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub upstream_task_id: Option<UpstreamTaskId>,
    #[serde(flatten)]
    pub state: BulkItemState<E>,
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize)]
pub struct BulkPage<E> {
    items: Vec<BulkItem<E>>,
    offset: u64,
    #[schemars(range(min = 1, max = 1000))]
    limit: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u64>,
}

impl<E> BulkPage<E> {
    pub fn new(
        items: Vec<BulkItem<E>>,
        offset: u64,
        limit: u64,
        next_offset: Option<u64>,
    ) -> Result<Self, DomainError> {
        if !(1..=1000).contains(&limit) {
            return Err(DomainError::OutOfRange("bulk page limit"));
        }
        if items.windows(2).any(|pair| pair[0].index >= pair[1].index) {
            return Err(DomainError::UnorderedBulkItems);
        }
        Ok(Self {
            items,
            offset,
            limit,
            next_offset,
        })
    }

    #[must_use]
    pub fn items(&self) -> &[BulkItem<E>] {
        &self.items
    }
}

#[derive(Deserialize)]
struct BulkPageWire<E> {
    items: Vec<BulkItem<E>>,
    offset: u64,
    limit: u64,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    next_offset: Option<u64>,
}

impl<'de, E> Deserialize<'de> for BulkPage<E>
where
    E: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BulkPageWire::deserialize(deserializer)?;
        Self::new(wire.items, wire.offset, wire.limit, wire.next_offset).map_err(D::Error::custom)
    }
}
