//! HistoryStore-owned atomic reconciliation of remotely observed reads
//! (contracts.md 14.2 note, docs/history-contract.md uniqueness invariants).
//!
//! The adapter hands the store content-pure projections and the store runs
//! lookup, evidence validation, merge, and insert-or-refresh inside one
//! `IMMEDIATE` transaction. Task, bulk, and common write mechanics are split
//! into cohesive submodules while [`super::store::HistoryStore`] remains the
//! one concrete persistence owner.

mod bulk;
mod common;
mod task;

use crate::domain::{AnalysisId, BulkId, SaveState};

use super::records::{StoredAnalysis, StoredCheck, StoredUpstreamTask};
pub(super) use bulk::{
    upsert_bulk_row_tx, upsert_child_row_tx, validate_existing_child_state_tx,
    validate_supplied_child_membership,
};
pub(super) use common::update_observation_snapshot_tx;
pub(super) use task::{task_lookup_targets, validate_owned_task_evidence};

/// The outcome of one atomic observed-analysis reconciliation: the stored
/// row's identity, its untouched `save_state`, and whether this reconcile
/// inserted the fresh row or refreshed a stored one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconciledAnalysis {
    pub stored_id: AnalysisId,
    pub save_state: SaveState,
    pub inserted: bool,
}

/// The outcome of one atomic bulk reconciliation: the stored collection
/// identity the children and observations were rebound onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconciledBulk {
    pub stored_id: BulkId,
    /// `true` when this reconcile inserted the collection row for the first
    /// time; `false` when it refreshed the one stored row for the upstream
    /// job.
    pub inserted: bool,
}

/// One already-prepared child of a bulk reconcile, handed to the store with
/// its membership link and observation rows.
pub(crate) type PreparedChild = (StoredAnalysis, Vec<StoredUpstreamTask>);

/// One bulk child with its authoritative ordered checks and task evidence.
pub(crate) type CompletePreparedChild = (StoredAnalysis, Vec<StoredCheck>, Vec<StoredUpstreamTask>);
