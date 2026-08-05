//! Canonical privacy-bounded history summary values.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    AnalysisId, AnalysisStatus, CheckKind, OrderedChecks, SaveState, UtcTimestamp,
    deserialize_missing_only,
};

/// Privacy-bounded history list/search item.
///
/// This is intentionally not an `Analysis`: summary reads never fabricate
/// input descriptors, results, errors, or provenance that were not selected
/// from the durable record.
#[derive(Clone, Debug, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisSummary {
    pub id: AnalysisId,
    pub status: AnalysisStatus,
    pub checks: OrderedChecks<CheckKind>,
    pub save_state: SaveState,
    pub input_kind: AnalysisInputKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_missing_only")]
    pub display_name: Option<String>,
    pub created_at: UtcTimestamp,
}

/// Closed input discriminator used by history summaries.
#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisInputKind {
    Text,
    File,
}

/// Ordered page returned by `history list` and `history search`.
#[derive(Clone, Debug, Default, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisSummaryPage {
    pub items: Vec<AnalysisSummary>,
}
