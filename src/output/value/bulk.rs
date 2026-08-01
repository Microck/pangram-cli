//! The bulk submit/dry-run success payloads and the repeated-file analysis
//! union. The closed `bulk_submit` union, its canonical dry-run
//! reconciliation record, and the one-or-many analysis projection own their
//! serialization feedback, Schemars metadata, and the structural
//! discrimination that keeps the untagged unions from coercing one shape
//! into another. Envelope assembly stays in the parent `value` module.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use schemars::JsonSchema;

use crate::domain::{
    Analysis, AnalysisStatus, BulkCollection, BulkId, Sha256Hash, SubmissionOutcome,
};

use super::{CanonicalError, NonEmptyAnalyses, OutputValidationError};

/// One analysis produced by a single-document command.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum AnalysisOutput {
    One(Box<Analysis<CanonicalError>>),
    Many(NonEmptyAnalyses),
}

impl AnalysisOutput {
    pub fn one(analysis: Analysis<CanonicalError>) -> Self {
        Self::One(Box::new(analysis))
    }

    pub fn many(analyses: Vec<Analysis<CanonicalError>>) -> Result<Self, OutputValidationError> {
        NonEmptyAnalyses::new(analyses).map(Self::Many)
    }

    /// One analysis becomes `One`; a non-empty series becomes `Many` in
    /// submission order. An empty series is the caller's bug and rejected.
    pub fn from_analyses(
        analyses: Vec<Analysis<CanonicalError>>,
    ) -> Result<Self, OutputValidationError> {
        match analyses.len() {
            0 => Err(OutputValidationError::EmptyValue("analysis output")),
            1 => Ok(Self::one(
                analyses.into_iter().next().expect("one analysis"),
            )),
            _ => Self::many(analyses),
        }
    }
}

impl<'de> Deserialize<'de> for AnalysisOutput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        if value.is_array() {
            serde_json::from_value(value)
                .map(Self::Many)
                .map_err(D::Error::custom)
        } else {
            serde_json::from_value(value)
                .map(Box::new)
                .map(Self::One)
                .map_err(D::Error::custom)
        }
    }
}

/// The dry-run marker object that distinguishes a local preflight from a real
/// queued collection for machine consumers. Fixed sentinel values only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BulkDryRunMarker {
    #[schemars(extend("const" = true))]
    noop: bool,
    #[schemars(extend("const" = false))]
    observed: bool,
}

/// The canonical `bulk_submit --dry-run` reconciliation shape (contracts.md
/// 9.2): the validated plan's identity and pricing, reported without
/// credentials, network, or any remote identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BulkDryRun {
    dry: BulkDryRunMarker,
    id: BulkId,
    status: AnalysisStatus,
    submission_outcome: SubmissionOutcome,
    plan_sha256: Sha256Hash,
    /// Always at least 1: `text_billable_units` never returns 0 and the plan
    /// rejects an empty item list (contracts.md 9.2).
    #[schemars(range(min = 1))]
    estimated_billable_units: u64,
    /// Always at least 1: `BulkSubmissionPlan::new` rejects an empty list.
    #[schemars(range(min = 1))]
    item_count: u64,
}

impl BulkDryRun {
    /// Builds the canonical dry-run record. `status` is always `queued` and
    /// `submission_outcome` is always `not_submitted`; construction is
    /// infallible because the validated plan owns every upstream invariant.
    #[must_use]
    pub fn new(
        id: BulkId,
        plan_sha256: Sha256Hash,
        estimated_billable_units: u64,
        item_count: u64,
    ) -> Self {
        Self {
            dry: BulkDryRunMarker {
                noop: true,
                observed: false,
            },
            id,
            status: AnalysisStatus::Queued,
            submission_outcome: SubmissionOutcome::NotSubmitted,
            plan_sha256,
            estimated_billable_units,
            item_count,
        }
    }

    #[must_use]
    pub const fn id(&self) -> BulkId {
        self.id
    }

    #[must_use]
    pub const fn plan_sha256(&self) -> Sha256Hash {
        self.plan_sha256
    }

    #[must_use]
    pub const fn estimated_billable_units(&self) -> u64 {
        self.estimated_billable_units
    }

    #[must_use]
    pub const fn item_count(&self) -> u64 {
        self.item_count
    }
}

/// The `bulk_submit` success data root (contracts.md 3.1, 9.2): a submitted
/// run projects its queued [`BulkCollection`]; a `--dry-run` projects the
/// canonical [`BulkDryRun`] reconciliation shape. The untagged union stays
/// closed through the generator's schema union.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum BulkSubmitOutput {
    Collection(Box<BulkCollection>),
    DryRun(BulkDryRun),
}

impl BulkSubmitOutput {
    pub fn collection(collection: BulkCollection) -> Self {
        Self::Collection(Box::new(collection))
    }

    #[must_use]
    pub fn dry_run(dry_run: BulkDryRun) -> Self {
        Self::DryRun(dry_run)
    }
}

impl<'de> Deserialize<'de> for BulkSubmitOutput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        // A dry run carries the `dry` marker; a real collection carries the
        // counter set. Discriminate structurally before falling back, so the
        // untagged union never silently coerces one shape into the other.
        if value.get("dry").is_some() {
            serde_json::from_value(value)
                .map(Self::DryRun)
                .map_err(D::Error::custom)
        } else {
            serde_json::from_value(value)
                .map(Box::new)
                .map(Self::Collection)
                .map_err(D::Error::custom)
        }
    }
}
