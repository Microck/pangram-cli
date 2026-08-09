//! Bulk-collection persistence and reconciliation.
//!
//! One remote bulk job owns at most one stored row, reconciled by its
//! contracted `upstream_bulk_id`: repeated submissions and observations of
//! one job refresh the one collection row, its member analyses, and their
//! observation rows atomically and without duplicates. Local authorship
//! (the row identity, submission outcome, save state, caller ID, input
//! content, and the original creation time) is never rewritten by a later
//! observation; only the observed status, terminal bodies, and refresh
//! stamps move.

use rusqlite::params;

use crate::domain::BulkId;

use super::analysis_writes::{
    insert_check_rows, legacy_checks_for_reconcile, set_check_count, upsert_observation_row,
    validate_observation_rows,
};
use super::records::{StoredAnalysis, StoredBulkCollection, StoredUpstreamTask};
use super::store::HistoryStore;
use super::wire::{BulkRow, row_to_bulk};
use super::{HistoryError, HistoryErrorCode};

impl HistoryStore {
    /// Inserts one bulk collection row on its own (used by the schema and
    /// core-store suites). Whole-collection persistence with children goes
    /// through [`Self::upsert_bulk_collection_atomic`].
    pub fn save_bulk_collection(
        &mut self,
        record: &StoredBulkCollection,
    ) -> Result<(), HistoryError> {
        self.in_transaction(|transaction| {
            insert_bulk_row(transaction, record)?;
            super::read_validation::certify_bulk_aggregate(transaction, &record.id)
        })
    }

    /// Resolves the stored collection that reconciles one upstream bulk job,
    /// if any. SQL absence (`Ok(None)`) stays distinct from a storage failure
    /// (`Err`): only a real read failure is an error, so the caller treats a
    /// first observation of the job as an insert and never blind-inserts a
    /// duplicate row after a failed lookup.
    pub fn find_bulk_collection_by_upstream(
        &self,
        upstream_bulk_id: &str,
    ) -> Result<Option<StoredBulkCollection>, HistoryError> {
        self.with_read_snapshot(|connection| {
            let found: Option<String> = connection
                .query_row(
                    "SELECT id FROM bulk_collections WHERE upstream_bulk_id = ?1",
                    params![upstream_bulk_id],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    _ => Err(HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "find bulk collection",
                    )),
                })?;
            match found {
                None => Ok(None),
                Some(id) => {
                    let parsed = id.parse().map_err(|_| {
                        HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "read row")
                    })?;
                    get_bulk_collection_on(connection, &parsed).map(Some)
                }
            }
        })
    }

    /// Atomically refreshes one bulk collection with its children and their
    /// current observation rows, in one transaction: the collection row
    /// upserts on its identity, each child upserts on its
    /// `(bulk_id, bulk_index)` membership, and each observation upserts on
    /// its `(analysis_id, check_kind)` key. A commit persists one consistent
    /// whole-collection snapshot; a rollback undoes all of it, so a
    /// half-committed collection can never exist.
    ///
    /// Reconciliation preserves local authorship. The collection keeps its
    /// original `created_at` (a later `bulk status`/`bulk wait` never moves
    /// the job's first local stamp). Each child keeps its stored identity,
    /// `submission_outcome`, `save_state`, `caller_id`, `created_at`, and
    /// input payload (`input_json` and the search columns) exactly as first
    /// recorded; the refresh moves only status, terminal bodies, and the
    /// refresh stamps, so a remote-only observation never discards the input
    /// text or descriptor the stored row already holds. A refreshed child's
    /// observation rows are rebound onto the stored child identity so they
    /// refresh the one child's task rows rather than dangling on the fresh
    /// projection's identity (the child identity is stable across reads;
    /// only the fresh projections mint new `anl_` values).
    pub fn upsert_bulk_collection_atomic(
        &mut self,
        collection: &StoredBulkCollection,
        children: &[(StoredAnalysis, Vec<StoredUpstreamTask>)],
    ) -> Result<(), HistoryError> {
        self.in_immediate_transaction(|transaction| {
            // Reject foreign-owner or wrong-kind observation vectors before
            // task identities can resolve a child and before the collection
            // or any member is mutated. Rebinding is only valid after the
            // incoming child aggregate has proved its own ownership.
            for (child, observations) in children {
                super::reconcile::validate_supplied_child_membership(collection, child)?;
                let checks = legacy_checks_for_reconcile(child, observations)?;
                validate_observation_rows(child.id, &checks, observations)?;
            }
            let mut existing = std::collections::BTreeSet::new();
            if transaction
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM bulk_collections WHERE id = ?1)",
                    [collection.id.to_string()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "read bulk collection",
                    )
                })?
            {
                existing.insert(collection.id);
            }
            if let Some(upstream) = &collection.upstream_bulk_id {
                let by_upstream = transaction
                    .query_row(
                        "SELECT id FROM bulk_collections WHERE upstream_bulk_id = ?1",
                        [upstream],
                        |row| row.get::<_, String>(0),
                    )
                    .map(Some)
                    .or_else(|error| match error {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        _ => Err(HistoryError::from_sqlite(
                            HistoryErrorCode::HistoryUnavailable,
                            "read bulk collection",
                        )),
                    })?;
                if let Some(id) = by_upstream {
                    existing.insert(id.parse().map_err(|_| {
                        HistoryError::new(
                            HistoryErrorCode::HistoryCorrupt,
                            "the stored bulk collection identity is invalid",
                        )
                    })?);
                }
            }
            for id in existing {
                super::read_validation::certify_bulk_aggregate(transaction, &id)?;
            }
            // A child may resolve to a pre-existing standalone row by its
            // observation identity even when this collection is new. Prove
            // every such candidate before the collection upsert performs
            // the transaction's first write.
            for (child, observations) in children {
                super::reconcile::validate_existing_child_state_tx(
                    transaction,
                    child,
                    observations,
                )?;
            }
            super::reconcile::upsert_bulk_row_tx(transaction, collection)?;
            for (child, observations) in children {
                let stored_id =
                    super::reconcile::upsert_child_row_tx(transaction, child, observations)?;
                let stored_check_rows = transaction
                    .query_row(
                        "SELECT COUNT(*) FROM analysis_checks WHERE analysis_id = ?1",
                        [stored_id.to_string()],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|_| {
                        HistoryError::from_sqlite(
                            HistoryErrorCode::HistoryCorrupt,
                            "read check count",
                        )
                    })?;
                if stored_check_rows == 0 {
                    let rebound_checks = legacy_checks_for_reconcile(child, observations)?
                        .into_iter()
                        .map(|check| super::records::StoredCheck {
                            analysis_id: stored_id,
                            ..check
                        })
                        .collect::<Vec<_>>();
                    insert_check_rows(transaction, &rebound_checks)?;
                    set_check_count(transaction, stored_id, rebound_checks.len())?;
                }
                for task in observations {
                    // Rebind onto the child's stored identity: a refreshed
                    // child's row keeps its first-saved id, so its task rows
                    // belong to that id, never to the ephemeral projection
                    // id the read minted for its envelope.
                    let rebound = StoredUpstreamTask {
                        analysis_id: stored_id,
                        ..task.clone()
                    };
                    upsert_observation_row(transaction, &rebound)?;
                }
            }
            super::read_validation::certify_bulk_aggregate(transaction, &collection.id)
        })
    }

    /// Members of one bulk collection in index order.
    ///
    /// Same structural FTS rule as `get_analysis`: a member row missing its
    /// synchronized search entry is corruption, not an absent payload.
    pub fn list_bulk_analyses(&self, bulk: &BulkId) -> Result<Vec<StoredAnalysis>, HistoryError> {
        self.with_read_snapshot(|connection| {
            let bulk_exists = connection
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM bulk_collections WHERE id = ?1)",
                    [bulk.to_string()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "list bulk analyses",
                    )
                })?;
            if bulk_exists {
                super::read_validation::certify_bulk_analyses(connection, bulk)
            } else {
                Ok(Vec::new())
            }
        })
    }

    /// Fetches one full bulk collection row by its canonical identity.
    pub fn get_bulk_collection(&self, id: &BulkId) -> Result<StoredBulkCollection, HistoryError> {
        self.with_read_snapshot(|connection| get_bulk_collection_on(connection, id))
    }
}

fn get_bulk_collection_on(
    connection: &rusqlite::Connection,
    id: &BulkId,
) -> Result<StoredBulkCollection, HistoryError> {
    super::read_validation::certify_bulk_aggregate(connection, id)?;
    connection
        .query_row(
            "SELECT id, upstream_bulk_id, status, submission_outcome, total_items,
                    accepted, succeeded, failed, estimated_billable_units, created_at,
                    updated_at, completed_at
             FROM bulk_collections WHERE id = ?1",
            [id.to_string()],
            row_to_bulk,
        )
        .map_err(|error| match error {
            rusqlite::Error::InvalidQuery => HistoryError::new(
                HistoryErrorCode::HistoryCorrupt,
                "the stored bulk collection is invalid",
            ),
            _ => HistoryError::from_sqlite(
                HistoryErrorCode::HistoryUnavailable,
                "read bulk collection",
            ),
        })
}

/// Inserts a new bulk collection row.
fn insert_bulk_row(
    transaction: &rusqlite::Transaction<'_>,
    record: &StoredBulkCollection,
) -> Result<(), HistoryError> {
    let row = BulkRow::of(record);
    transaction
        .execute(BULK_INSERT, row.as_params())
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "save bulk collection")
        })?;
    Ok(())
}

/// The column list shared by the `bulk_collections` insert and upsert.
pub(super) const BULK_INSERT: &str = "INSERT INTO bulk_collections (
    id, upstream_bulk_id, status, submission_outcome, total_items,
    accepted, succeeded, failed, estimated_billable_units, created_at,
    updated_at, completed_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)";
