//! Stored row shapes for the history tables.
//!
//! These mirror the docs/history-contract.md columns one-to-one. JSON columns
//! hold canonical schema-major-1 bodies produced by the output projection or
//! adapters; the store treats them as opaque strings and never re-serializes.

use crate::domain::{
    AnalysisId, AnalysisStatus, BulkCounters, BulkId, CheckKind, SaveState, Sha256Hash,
    SubmissionOutcome, UtcTimestamp,
};

/// The canonical input discriminator of an `analyses` row.
///
/// This is the closed persistence spelling locked by docs/history-contract.md
/// `input_type TEXT NOT NULL`: only `text` and `file` are valid. An unknown
/// persisted value is a contract violation and surfaces as `history_corrupt`,
/// never an unknown-string passthrough or a leak-allocated `&'static str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Text,
    File,
}

impl InputKind {
    /// The persistence spelling recorded in `analyses.input_type`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::File => "file",
        }
    }

    /// Parses a persisted `input_type`. Unknown values fail as
    /// `history_corrupt` because the schema locks this column to the closed
    /// `text`/`file` set.
    pub(crate) fn parse(value: &str) -> Result<Self, super::HistoryError> {
        match value {
            "text" => Ok(Self::Text),
            "file" => Ok(Self::File),
            _ => Err(super::HistoryError::from_sqlite(
                super::HistoryErrorCode::HistoryCorrupt,
                "read analysis input kind",
            )),
        }
    }
}

/// One `analyses` row, including the scoped FTS payload the row writes into
/// `analysis_search` in the same transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAnalysis {
    pub id: AnalysisId,
    /// `(bulk_id, bulk_index)` when this analysis is a bulk collection
    /// member; local tops carry `None`.
    pub bulk: Option<(BulkId, i64)>,
    pub caller_id: Option<String>,
    pub status: AnalysisStatus,
    pub submission_outcome: SubmissionOutcome,
    pub save_state: SaveState,
    /// The canonical input discriminator.
    pub input_kind: InputKind,
    pub input_sha256: Sha256Hash,
    pub display_name: Option<String>,
    pub input_json: String,
    pub result_json: Option<String>,
    pub error_json: Option<String>,
    /// Validated Pangram protocol version most recently observed.
    pub upstream_version: Option<String>,
    pub retry_of: Option<AnalysisId>,
    pub rerun_of: Option<AnalysisId>,
    pub created_at: UtcTimestamp,
    pub updated_at: UtcTimestamp,
    pub completed_at: Option<UtcTimestamp>,
    // FTS payload (product-spec 10.4). Kept as plain strings so the store
    // never interprets upstream or submitted content.
    pub search_input_text: Option<String>,
    pub search_filename: Option<String>,
    pub search_headline: Option<String>,
    pub search_source_urls: Option<String>,
}

impl StoredAnalysis {
    /// Whether any terminal result or error body is present.
    #[must_use]
    pub fn is_terminal_record(&self) -> bool {
        self.result_json.is_some() || self.error_json.is_some()
    }
}

/// One `bulk_collections` row. `estimated_billable_units` is the locally
/// validated plan ceiling and is always present on rows this store writes
/// (the column is `NOT NULL`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBulkCollection {
    pub id: BulkId,
    pub upstream_bulk_id: Option<String>,
    pub status: AnalysisStatus,
    pub submission_outcome: SubmissionOutcome,
    pub counters: BulkCounters,
    pub estimated_billable_units: Option<u64>,
    pub created_at: UtcTimestamp,
    pub updated_at: UtcTimestamp,
    pub completed_at: Option<UtcTimestamp>,
}

impl StoredBulkCollection {
    #[must_use]
    pub const fn id(&self) -> BulkId {
        self.id
    }
}

/// One `upstream_tasks` observation row: current remote observation state
/// for one check of one analysis. Terminal snapshots live on `analyses`;
/// this table is mutable observation scratch only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredUpstreamTask {
    pub analysis_id: AnalysisId,
    pub check_kind: CheckKind,
    pub upstream_task_id: String,
    pub last_stage: Option<String>,
    pub observed_at: UtcTimestamp,
}

/// The projection of an `analyses` search result: identity plus the display
/// columns a summary needs, without the heavy JSON bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSearchHit {
    pub analysis_id: AnalysisId,
    pub status: AnalysisStatus,
    pub save_state: SaveState,
    pub input_kind: InputKind,
    pub display_name: Option<String>,
    pub created_at: UtcTimestamp,
}
