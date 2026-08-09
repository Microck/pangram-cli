//! Serialized inputs, typed check states, results, and collection models.

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    AnalysisId, AnalysisStatus, BulkId, CheckKind, CheckStatus, DomainError, Fraction,
    NonEmptyString, OrderedCheck, OrderedChecks, Percentage, Sha256Hash, UpstreamBulkId,
    UpstreamTaskId, UtcTimestamp, derive_parent_status, deserialize_missing_only,
};

#[derive(Default)]
pub(super) enum FieldPresence<T> {
    #[default]
    Missing,
    Present(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for FieldPresence<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Self::Present)
    }
}

#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOrigin {
    Literal,
    Stdin,
    File,
    /// The input of a remotely authored operation the client observes by
    /// explicit upstream ID (contracts.md 4.6). Valid only on resumed
    /// reads; never produced by a locally submitted command.
    Unknown,
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Eq, Serialize)]
pub struct TextInput {
    origin: TextOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    pub sha256: Sha256Hash,
    pub byte_count: u64,
    pub word_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl TextInput {
    pub fn new(
        origin: TextOrigin,
        name: Option<String>,
        sha256: Sha256Hash,
        byte_count: u64,
        word_count: u64,
        text: Option<String>,
    ) -> Result<Self, DomainError> {
        if matches!(origin, TextOrigin::File) != name.is_some() {
            return Err(DomainError::OutOfRange("text input name"));
        }
        Ok(Self {
            origin,
            name,
            sha256,
            byte_count,
            word_count,
            text,
        })
    }

    #[must_use]
    pub const fn origin(&self) -> TextOrigin {
        self.origin
    }

    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

#[derive(Deserialize)]
struct TextInputWire {
    origin: TextOrigin,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    name: Option<String>,
    sha256: Sha256Hash,
    byte_count: u64,
    word_count: u64,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    text: Option<String>,
}

impl<'de> Deserialize<'de> for TextInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TextInputWire::deserialize(deserializer)?;
        Self::new(
            wire.origin,
            wire.name,
            wire.sha256,
            wire.byte_count,
            wire.word_count,
            wire.text,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInput {
    pub filename: NonEmptyString,
    pub media_type: NonEmptyString,
    pub sha256: Sha256Hash,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub extracted_text: Option<String>,
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnalysisInput {
    Text(TextInput),
    File(FileInput),
}

#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiClassification {
    Ai,
    Human,
    Mixed,
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub text: String,
    pub label: NonEmptyString,
    pub ai_assistance_score: Fraction,
    pub confidence: Confidence,
    pub start_index: u64,
    pub end_index: u64,
    pub word_count: u64,
    pub token_length: u64,
    /// Pangram's per-segment estimate that a humanizer modified the text.
    pub humanizer_score: Fraction,
    /// Pangram's thresholded humanizer decision, preserved without local derivation.
    pub is_humanized: bool,
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize, Deserialize)]
pub struct AiDetectionResult {
    pub classification: AiClassification,
    pub headline: String,
    pub prediction: String,
    pub fraction_ai: Fraction,
    pub fraction_ai_assisted: Fraction,
    pub fraction_human: Fraction,
    pub num_ai_segments: u64,
    pub num_ai_assisted_segments: u64,
    pub num_human_segments: u64,
    pub segments: Vec<Segment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    #[schemars(url)]
    pub dashboard_link: Option<String>,
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize, Deserialize)]
pub struct PlagiarismMatch {
    pub source_url: String,
    pub matched_text: String,
    pub similarity_score: Fraction,
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize, Deserialize)]
pub struct PlagiarismResult {
    pub plagiarism_detected: bool,
    pub total_sentences: u64,
    pub plagiarized_sentence_count: u64,
    pub percent_plagiarized: Percentage,
    pub matches: Vec<PlagiarismMatch>,
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "result", rename_all = "snake_case")]
pub enum CheckResult {
    AiDetection(AiDetectionResult),
    Plagiarism(PlagiarismResult),
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpstreamIdentity {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub task_id: Option<UpstreamTaskId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub last_stage: Option<NonEmptyString>,
}

/// State variants encode the result/error exclusivity contract directly.
#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CheckState<R, E> {
    Queued {
        #[serde(skip_serializing_if = "Option::is_none")]
        upstream: Option<UpstreamIdentity>,
    },
    Running {
        #[serde(skip_serializing_if = "Option::is_none")]
        upstream: Option<UpstreamIdentity>,
    },
    Succeeded {
        #[serde(skip_serializing_if = "Option::is_none")]
        upstream: Option<UpstreamIdentity>,
        result: R,
    },
    Failed {
        #[serde(skip_serializing_if = "Option::is_none")]
        upstream: Option<UpstreamIdentity>,
        error: E,
    },
}

impl<R, E> CheckState<R, E> {
    #[must_use]
    pub const fn status(&self) -> CheckStatus {
        match self {
            Self::Queued { .. } => CheckStatus::Queued,
            Self::Running { .. } => CheckStatus::Running,
            Self::Succeeded { .. } => CheckStatus::Succeeded,
            Self::Failed { .. } => CheckStatus::Failed,
        }
    }

    fn upstream(&self) -> Option<&UpstreamIdentity> {
        match self {
            Self::Queued { upstream }
            | Self::Running { upstream }
            | Self::Succeeded { upstream, .. }
            | Self::Failed { upstream, .. } => upstream.as_ref(),
        }
    }
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "R: Deserialize<'de>, E: Deserialize<'de>"))]
struct CheckStateWire<R, E> {
    status: CheckStatus,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    upstream: Option<UpstreamIdentity>,
    #[serde(default)]
    result: FieldPresence<R>,
    #[serde(default)]
    error: FieldPresence<E>,
}

impl<'de, R, E> Deserialize<'de> for CheckState<R, E>
where
    R: Deserialize<'de>,
    E: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CheckStateWire::deserialize(deserializer)?;
        match (wire.status, wire.result, wire.error) {
            (CheckStatus::Queued, FieldPresence::Missing, FieldPresence::Missing) => {
                Ok(Self::Queued {
                    upstream: wire.upstream,
                })
            }
            (CheckStatus::Running, FieldPresence::Missing, FieldPresence::Missing) => {
                Ok(Self::Running {
                    upstream: wire.upstream,
                })
            }
            (CheckStatus::Succeeded, FieldPresence::Present(result), FieldPresence::Missing) => {
                Ok(Self::Succeeded {
                    upstream: wire.upstream,
                    result,
                })
            }
            (CheckStatus::Failed, FieldPresence::Missing, FieldPresence::Present(error)) => {
                Ok(Self::Failed {
                    upstream: wire.upstream,
                    error,
                })
            }
            _ => Err(D::Error::custom(DomainError::InvalidState("check"))),
        }
    }
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Check<E> {
    AiDetection(CheckState<AiDetectionResult, E>),
    Plagiarism(CheckState<PlagiarismResult, E>),
}

impl<E> Check<E> {
    #[must_use]
    pub const fn status(&self) -> CheckStatus {
        match self {
            Self::AiDetection(state) => state.status(),
            Self::Plagiarism(state) => state.status(),
        }
    }

    fn has_upstream_task_id(&self) -> bool {
        match self {
            Self::AiDetection(state) => state.upstream(),
            Self::Plagiarism(state) => state.upstream(),
        }
        .and_then(|upstream| upstream.task_id.as_ref())
        .is_some()
    }
}

impl<E> OrderedCheck for Check<E> {
    fn check_kind(&self) -> CheckKind {
        match self {
            Self::AiDetection(_) => CheckKind::AiDetection,
            Self::Plagiarism(_) => CheckKind::Plagiarism,
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveState {
    Ephemeral,
    SavedManual,
    SavedHistory,
}

#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionOutcome {
    NotSubmitted,
    Accepted,
    Terminal,
    AcceptanceUnknown,
}

#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Pangram,
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Eq, Serialize)]
#[serde(transparent)]
#[schemars(with = "std::collections::BTreeSet<UpstreamTaskId>")]
pub struct UpstreamTaskIds(Vec<UpstreamTaskId>);

impl UpstreamTaskIds {
    pub fn new(ids: Vec<UpstreamTaskId>) -> Result<Self, DomainError> {
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        if sorted.len() != ids.len() {
            return Err(DomainError::DuplicateUpstreamTaskId);
        }
        Ok(Self(ids))
    }

    #[must_use]
    pub fn as_slice(&self) -> &[UpstreamTaskId] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for UpstreamTaskIds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub provider: Provider,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub upstream_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub upstream_task_ids: Option<UpstreamTaskIds>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub upstream_bulk_id: Option<UpstreamBulkId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub submitted_at: Option<UtcTimestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub completed_at: Option<UtcTimestamp>,
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalOperationId {
    AnalysisId(AnalysisId),
    BulkId(BulkId),
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Eq, Serialize)]
pub struct SubmissionOutcomeUnknownDetails {
    #[serde(flatten)]
    operation_id: LocalOperationId,
    pub request_sha256: Sha256Hash,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_task_id: Option<UpstreamTaskId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_bulk_id: Option<UpstreamBulkId>,
    pub last_status: NonEmptyString,
}

impl SubmissionOutcomeUnknownDetails {
    #[must_use]
    pub fn new(
        operation_id: LocalOperationId,
        request_sha256: Sha256Hash,
        upstream_task_id: Option<UpstreamTaskId>,
        upstream_bulk_id: Option<UpstreamBulkId>,
        last_status: NonEmptyString,
    ) -> Self {
        Self {
            operation_id,
            request_sha256,
            upstream_task_id,
            upstream_bulk_id,
            last_status,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &LocalOperationId {
        &self.operation_id
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmissionOutcomeUnknownDetailsWire {
    #[serde(default)]
    analysis_id: FieldPresence<AnalysisId>,
    #[serde(default)]
    bulk_id: FieldPresence<BulkId>,
    request_sha256: Sha256Hash,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    upstream_task_id: Option<UpstreamTaskId>,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    upstream_bulk_id: Option<UpstreamBulkId>,
    last_status: NonEmptyString,
}

impl<'de> Deserialize<'de> for SubmissionOutcomeUnknownDetails {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SubmissionOutcomeUnknownDetailsWire::deserialize(deserializer)?;
        let operation_id = match (wire.analysis_id, wire.bulk_id) {
            (FieldPresence::Present(id), FieldPresence::Missing) => {
                LocalOperationId::AnalysisId(id)
            }
            (FieldPresence::Missing, FieldPresence::Present(id)) => LocalOperationId::BulkId(id),
            _ => {
                return Err(D::Error::custom(DomainError::InvalidSubmissionIdentity));
            }
        };
        Ok(Self::new(
            operation_id,
            wire.request_sha256,
            wire.upstream_task_id,
            wire.upstream_bulk_id,
            wire.last_status,
        ))
    }
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize)]
pub struct Analysis<E> {
    pub id: AnalysisId,
    status: AnalysisStatus,
    submission_outcome: SubmissionOutcome,
    /// The canonical input descriptor. Omitted only on a resumed-observation
    /// read whose remote operation has not yet reached a terminal document
    /// (contracts.md 4.6); locally submitted commands always carry one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<AnalysisInput>,
    checks: OrderedChecks<Check<E>>,
    pub save_state: SaveState,
    provenance: Provenance,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_of: Option<AnalysisId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rerun_of: Option<AnalysisId>,
    pub created_at: UtcTimestamp,
    pub updated_at: UtcTimestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<UtcTimestamp>,
}

impl<E> Analysis<E> {
    /// Builds a locally submitted analysis: the input descriptor always
    /// exists because the caller supplied the content.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: AnalysisId,
        submission_outcome: SubmissionOutcome,
        input: AnalysisInput,
        checks: OrderedChecks<Check<E>>,
        save_state: SaveState,
        provenance: Provenance,
        retry_of: Option<AnalysisId>,
        rerun_of: Option<AnalysisId>,
        created_at: UtcTimestamp,
        updated_at: UtcTimestamp,
        completed_at: Option<UtcTimestamp>,
    ) -> Result<Self, DomainError> {
        Self::with_optional_input(
            id,
            submission_outcome,
            Some(input),
            checks,
            save_state,
            provenance,
            retry_of,
            rerun_of,
            created_at,
            updated_at,
            completed_at,
        )
    }

    /// Builds an analysis whose input descriptor may be absent: the sole
    /// `None` case is a resumed-observation read without an attained
    /// terminal document (contracts.md 4.6). Every locally submitted command
    /// uses [`Self::new`].
    #[allow(clippy::too_many_arguments)]
    pub fn with_optional_input(
        id: AnalysisId,
        submission_outcome: SubmissionOutcome,
        input: Option<AnalysisInput>,
        checks: OrderedChecks<Check<E>>,
        save_state: SaveState,
        provenance: Provenance,
        retry_of: Option<AnalysisId>,
        rerun_of: Option<AnalysisId>,
        created_at: UtcTimestamp,
        updated_at: UtcTimestamp,
        completed_at: Option<UtcTimestamp>,
    ) -> Result<Self, DomainError> {
        if retry_of.is_some() && rerun_of.is_some() {
            return Err(DomainError::ConflictingLineage);
        }
        let statuses: Vec<_> = checks.iter().map(Check::status).collect();
        let status = derive_parent_status(&statuses)?;
        let has_upstream_id = checks.iter().any(Check::has_upstream_task_id)
            || provenance
                .upstream_task_ids
                .as_ref()
                .is_some_and(|ids| !ids.as_slice().is_empty())
            || provenance.upstream_bulk_id.is_some();
        // Accepted requires a concrete provider ID. NotSubmitted is stricter:
        // even an empty upstream-only field would falsely imply submission.
        let has_upstream_evidence = checks.iter().any(Check::has_upstream_task_id)
            || provenance.upstream_task_ids.is_some()
            || provenance.upstream_bulk_id.is_some()
            || provenance.submitted_at.is_some()
            || provenance.completed_at.is_some();
        let outcome_is_valid = match submission_outcome {
            SubmissionOutcome::Accepted => has_upstream_id,
            SubmissionOutcome::Terminal => {
                !matches!(status, AnalysisStatus::Queued | AnalysisStatus::Running)
            }
            SubmissionOutcome::NotSubmitted => !has_upstream_evidence,
            SubmissionOutcome::AcceptanceUnknown => true,
        };
        if !outcome_is_valid {
            return Err(DomainError::InvalidSubmissionOutcome);
        }
        // Input absence is bounded to the resumed-observation path: only an
        // accepted (remotely authored) read may lack a local input
        // descriptor. Every locally submitted or unaccepted analysis carries
        // one (contracts.md 4.6).
        if input.is_none() && submission_outcome != SubmissionOutcome::Accepted {
            return Err(DomainError::InvalidState("analysis input"));
        }
        Ok(Self {
            id,
            status,
            submission_outcome,
            input,
            checks,
            save_state,
            provenance,
            retry_of,
            rerun_of,
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
    pub const fn submission_outcome(&self) -> SubmissionOutcome {
        self.submission_outcome
    }

    #[must_use]
    pub fn checks(&self) -> &[Check<E>] {
        &self.checks
    }

    #[must_use]
    pub const fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    /// The canonical input descriptor, or `None` on a resumed observation
    /// that has not yet reached a terminal document (contracts.md 4.6).
    #[must_use]
    pub const fn input(&self) -> Option<&AnalysisInput> {
        self.input.as_ref()
    }

    #[must_use]
    pub const fn retry_of(&self) -> Option<AnalysisId> {
        self.retry_of
    }

    #[must_use]
    pub const fn rerun_of(&self) -> Option<AnalysisId> {
        self.rerun_of
    }

    /// Records the save state the history store committed for this analysis
    /// (contracts.md 4.2). The field never affects the status derivation, so
    /// the adapter flips it right before projecting, after persistence.
    #[must_use]
    pub const fn with_save_state(mut self, save_state: SaveState) -> Self {
        self.save_state = save_state;
        self
    }
}

#[derive(Deserialize)]
struct AnalysisWire<E> {
    id: AnalysisId,
    status: AnalysisStatus,
    submission_outcome: SubmissionOutcome,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    input: Option<AnalysisInput>,
    checks: OrderedChecks<Check<E>>,
    save_state: SaveState,
    provenance: Provenance,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    retry_of: Option<AnalysisId>,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    rerun_of: Option<AnalysisId>,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    completed_at: Option<UtcTimestamp>,
}

impl<'de, E> Deserialize<'de> for Analysis<E>
where
    E: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AnalysisWire::deserialize(deserializer)?;
        let expected_status = wire.status;
        let analysis = Self::with_optional_input(
            wire.id,
            wire.submission_outcome,
            wire.input,
            wire.checks,
            wire.save_state,
            wire.provenance,
            wire.retry_of,
            wire.rerun_of,
            wire.created_at,
            wire.updated_at,
            wire.completed_at,
        )
        .map_err(D::Error::custom)?;
        if analysis.status != expected_status {
            return Err(D::Error::custom(DomainError::AnalysisStatusMismatch));
        }
        Ok(analysis)
    }
}

#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize, Deserialize)]
pub struct AnalysisPage<E> {
    pub items: Vec<Analysis<E>>,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde_json::json;

    use super::*;

    fn queued_analysis() -> Analysis<String> {
        let input = TextInput::new(
            TextOrigin::Literal,
            None,
            Sha256Hash::from_str(&"0".repeat(64)).unwrap(),
            4,
            1,
            None,
        )
        .unwrap();
        let checks =
            OrderedChecks::new([Check::AiDetection(CheckState::Queued { upstream: None })])
                .unwrap();
        let timestamp = UtcTimestamp::from_str("2026-07-23T12:00:00Z").unwrap();

        Analysis::new(
            AnalysisId::new(),
            SubmissionOutcome::NotSubmitted,
            AnalysisInput::Text(input),
            checks,
            SaveState::Ephemeral,
            Provenance {
                provider: Provider::Pangram,
                upstream_version: None,
                upstream_task_ids: None,
                upstream_bulk_id: None,
                submitted_at: None,
                completed_at: None,
            },
            None,
            None,
            timestamp,
            timestamp,
            None,
        )
        .unwrap()
    }

    #[test]
    fn check_tags_and_state_tags_share_one_canonical_object() {
        let value = serde_json::to_value(queued_analysis()).unwrap();

        assert_eq!(
            value["checks"][0],
            json!({"kind": "ai_detection", "status": "queued"})
        );
    }

    #[test]
    fn analysis_deserialization_rejects_a_stale_derived_status() {
        let mut value = serde_json::to_value(queued_analysis()).unwrap();
        value["status"] = json!("succeeded");

        assert!(serde_json::from_value::<Analysis<String>>(value).is_err());
    }
}
