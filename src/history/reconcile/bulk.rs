//! Atomic bulk-collection and child reconciliation.

use rusqlite::params;

use crate::domain::BulkId;

use super::super::analysis_writes::{
    ANALYSES_INSERT, insert_search_row, legacy_checks_for_reconcile, replace_check_rows,
    replace_search_row, set_check_count, upsert_observation_row, validate_check_rows,
    validate_observation_rows,
};
use super::super::collections::BULK_INSERT;
use super::super::records::{StoredAnalysis, StoredCheck, StoredUpstreamTask};
use super::super::store::HistoryStore;
use super::super::wire::{AnalysisRow, BulkRow, wire_status};
use super::super::{HistoryError, HistoryErrorCode};
use super::common::{
    IncomingBody, SearchColumns, certify_existing_analysis_tx, child_search_by_id_tx,
    incoming_body, stored_terminal_snapshot_tx,
};
use super::task::task_lookup_targets;
use super::{CompletePreparedChild, PreparedChild, ReconciledBulk};

impl HistoryStore {
    /// Atomically reconciles one remotely read or submitted bulk collection
    /// onto the one stored row for its upstream job identity
    /// (docs/history-contract.md uniqueness), inside one `IMMEDIATE`
    /// transaction: the prior-row lookup by `upstream_bulk_id`, the
    /// collection upsert, every child upsert keyed by its
    /// `(bulk_id, bulk_index)` membership, and every observation upsert all
    /// commit together, so two concurrent processes refreshing one job
    /// serialize on the write lock and the `upstream_bulk_id` unique
    /// constraint, and one stored collection (with its children exactly
    /// once) ever persists for the job.
    ///
    /// `collection` carries the fresh local `bulk_` identity the read
    /// minted; when a stored row for the upstream job already exists the
    /// reconcile rebinds onto the stored identity (the children's
    /// membership keys and their observation foreign keys follow), so the
    /// stored row's original `created_at` and each child's first-recorded
    /// authorship are preserved exactly, and only status, counters, and the
    /// refresh stamps move. Local authorship invariants (content, caller
    /// ID, FTS payload) hold through the child upsert path.
    pub fn reconcile_bulk_collection_atomic(
        &mut self,
        collection: &super::super::records::StoredBulkCollection,
        children: &[PreparedChild],
    ) -> Result<ReconciledBulk, HistoryError> {
        let complete = children
            .iter()
            .map(|(child, observations)| {
                Ok((
                    child.clone(),
                    legacy_checks_for_reconcile(child, observations)?,
                    observations.clone(),
                ))
            })
            .collect::<Result<Vec<_>, HistoryError>>()?;
        self.reconcile_bulk_collection_impl(collection, &complete, false)
    }

    /// Reconciles a bulk collection while replacing every child's complete
    /// authoritative ordered check payload in the same transaction.
    pub fn reconcile_bulk_collection_complete(
        &mut self,
        collection: &super::super::records::StoredBulkCollection,
        children: &[CompletePreparedChild],
    ) -> Result<ReconciledBulk, HistoryError> {
        self.reconcile_bulk_collection_impl(collection, children, true)
    }

    fn reconcile_bulk_collection_impl(
        &mut self,
        collection: &super::super::records::StoredBulkCollection,
        children: &[CompletePreparedChild],
        authoritative_checks: bool,
    ) -> Result<ReconciledBulk, HistoryError> {
        self.in_immediate_transaction(|transaction| {
            for (child, checks, observations) in children {
                validate_supplied_child_membership(collection, child)?;
                incoming_body(&child.result_json, &child.error_json)?;
                if authoritative_checks {
                    validate_check_rows(child.id, checks)?;
                }
                // Observation ownership is part of the incoming aggregate,
                // not a value that identity reconciliation may rewrite.
                // Validate it before task-key lookup can select a different
                // durable child and before this transaction mutates anything.
                validate_observation_rows(child.id, checks, observations)?;
            }
            let stored_id = resolve_bulk_identity_tx(transaction, collection, true)?;
            // The upstream identity is authoritative. If it resolves an
            // existing collection, certify that collection and all of its
            // members before the upsert can repair or overwrite anything.
            let resolved_exists = transaction
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM bulk_collections WHERE id = ?1)",
                    [stored_id.to_string()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "read bulk collection",
                    )
                })?;
            if resolved_exists {
                super::super::read_validation::certify_bulk_aggregate(transaction, &stored_id)?;
            }
            // `inserted` is true exactly when no stored row for the resolved
            // identity existed before this transaction wrote one.
            let pre_existing: bool = transaction
                .query_row(
                    "SELECT 1 FROM bulk_collections WHERE id = ?1",
                    [stored_id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .map(|_| true)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(false),
                    _ => Err(HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "read bulk collection",
                    )),
                })?;
            let inserted = !pre_existing;

            // Certify every existing analysis row this reconciliation may
            // read or refresh before the collection upsert performs the
            // transaction's first write. The exact canonical `history show`
            // reconstruction covers parent/check state, task evidence, FTS,
            // input/provenance, lineage, and bulk membership.
            for (child, _, observations) in children {
                let rebound = StoredAnalysis {
                    bulk: child.bulk.map(|(_, index)| (stored_id, index)),
                    ..child.clone()
                };
                validate_existing_child_state_tx(transaction, &rebound, observations)?;
            }

            // Upsert the collection row on the resolved stored identity.
            let row = super::super::records::StoredBulkCollection {
                id: stored_id,
                ..collection.clone()
            };
            upsert_bulk_row_tx(transaction, &row)?;

            for (child, checks, observations) in children {
                // Rebind the membership link onto the stored collection
                // identity, then upsert the child resolved by BOTH
                // identities at once (its `(bulk_id, bulk_index)`
                // membership and every attested
                // `(check_kind, upstream_task_id)` key), reusing the one
                // existing durable row when they agree and failing closed
                // when they conflict (docs/history-contract.md
                // task-first/bulk-second rule).
                let rebound = StoredAnalysis {
                    bulk: child.bulk.map(|(_, index)| (stored_id, index)),
                    ..child.clone()
                };
                let child_id = upsert_child_row_tx(transaction, &rebound, observations)?;
                let rebound_checks = checks
                    .iter()
                    .cloned()
                    .map(|check| StoredCheck {
                        analysis_id: child_id,
                        ..check
                    })
                    .collect::<Vec<_>>();
                let stored_check_rows = transaction
                    .query_row(
                        "SELECT COUNT(*) FROM analysis_checks WHERE analysis_id = ?1",
                        [child_id.to_string()],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|_| {
                        HistoryError::from_sqlite(
                            HistoryErrorCode::HistoryCorrupt,
                            "read check count",
                        )
                    })?;
                let body = incoming_body(&child.result_json, &child.error_json)?;
                let terminal_dominates = matches!(body, IncomingBody::Empty)
                    && stored_terminal_snapshot_tx(transaction, &child_id)?;
                let replace_checks = !terminal_dominates
                    && (stored_check_rows == 0
                        || authoritative_checks
                        || !matches!(body, IncomingBody::Empty));
                if replace_checks {
                    replace_check_rows(transaction, child_id, &rebound_checks)?;
                    set_check_count(transaction, child_id, rebound_checks.len())?;
                }
                for task in observations {
                    let rebound_observation = StoredUpstreamTask {
                        analysis_id: child_id,
                        ..task.clone()
                    };
                    upsert_observation_row(transaction, &rebound_observation)?;
                }
            }
            super::super::read_validation::certify_bulk_aggregate(transaction, &stored_id)?;
            Ok(ReconciledBulk {
                stored_id,
                inserted,
            })
        })
    }
}

/// Validates the membership a caller supplied for one child before an
/// owning bulk operation writes anything. Reconciliation may later rebind
/// this valid provisional membership onto the durable collection identity
/// selected by `upstream_bulk_id`, but it must never reinterpret a
/// contradictory foreign collection or out-of-range position as belonging
/// to the collection being refreshed.
pub(in crate::history) fn validate_supplied_child_membership(
    collection: &super::super::records::StoredBulkCollection,
    child: &StoredAnalysis,
) -> Result<(), HistoryError> {
    let Some((bulk_id, bulk_index)) = child.bulk else {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "a bulk child without its membership link cannot be reconciled",
        ));
    };
    if bulk_id != collection.id {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "a bulk child belongs to a different collection",
        ));
    }
    if bulk_index < 0 || (bulk_index as u64) >= collection.counters.total_items() {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "a bulk child index is outside its collection range",
        ));
    }
    Ok(())
}

/// Validates every complete stored aggregate that one child reconciliation
/// can involve. This preflight is read-only and runs before the bulk
/// transaction's first write.
pub(in crate::history) fn validate_existing_child_state_tx(
    transaction: &rusqlite::Transaction<'_>,
    record: &StoredAnalysis,
    observations: &[StoredUpstreamTask],
) -> Result<(), HistoryError> {
    let Some((bulk_id, bulk_index)) = record.bulk else {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "a bulk child without its membership link cannot be reconciled",
        ));
    };
    let mut candidates = std::collections::BTreeSet::new();
    if super::super::reads::stored_analysis_opt_on(transaction, &record.id)?.is_some() {
        candidates.insert(record.id);
    }
    if let Some(membership) = child_prior_state_tx(transaction, bulk_id, bulk_index)? {
        candidates.insert(membership);
    }
    if let Some(task_target) = task_lookup_targets(transaction, observations)? {
        candidates.insert(task_target);
    }
    for candidate in candidates {
        certify_existing_analysis_tx(transaction, &candidate)?;
    }
    Ok(())
}

/// The bulk collection upsert by identity, extracted so the external
/// [`HistoryStore::upsert_bulk_collection_atomic`] and the reconcile path
/// share one statement and its authorship-preserving update rules.
pub(in crate::history) fn upsert_bulk_row_tx(
    transaction: &rusqlite::Transaction<'_>,
    record: &super::super::records::StoredBulkCollection,
) -> Result<(), HistoryError> {
    resolve_bulk_identity_tx(transaction, record, false)?;
    let row = BulkRow::of(record);
    transaction
        .execute(
            &format!(
                "{BULK_INSERT}
                 ON CONFLICT (id) DO UPDATE SET
                    upstream_bulk_id = COALESCE(excluded.upstream_bulk_id, bulk_collections.upstream_bulk_id),
                    status = CASE
                        WHEN bulk_collections.completed_at IS NOT NULL
                             AND excluded.completed_at IS NULL
                        THEN bulk_collections.status ELSE excluded.status
                    END,
                    submission_outcome = CASE
                        WHEN bulk_collections.completed_at IS NOT NULL
                             AND excluded.completed_at IS NULL
                        THEN bulk_collections.submission_outcome
                        ELSE excluded.submission_outcome
                    END,
                    total_items = CASE
                        WHEN bulk_collections.completed_at IS NOT NULL
                             AND excluded.completed_at IS NULL
                        THEN bulk_collections.total_items ELSE excluded.total_items
                    END,
                    accepted = CASE
                        WHEN bulk_collections.completed_at IS NOT NULL
                             AND excluded.completed_at IS NULL
                        THEN bulk_collections.accepted ELSE excluded.accepted
                    END,
                    succeeded = CASE
                        WHEN bulk_collections.completed_at IS NOT NULL
                             AND excluded.completed_at IS NULL
                        THEN bulk_collections.succeeded ELSE excluded.succeeded
                    END,
                    failed = CASE
                        WHEN bulk_collections.completed_at IS NOT NULL
                             AND excluded.completed_at IS NULL
                        THEN bulk_collections.failed ELSE excluded.failed
                    END,
                    estimated_billable_units = MAX(bulk_collections.estimated_billable_units, excluded.estimated_billable_units),
                    updated_at = excluded.updated_at,
                    completed_at = COALESCE(excluded.completed_at, bulk_collections.completed_at)"
            ),
            row.as_params(),
        )
        .map_err(|_| {
            HistoryError::from_sqlite(
                HistoryErrorCode::HistoryWriteFailed,
                "refresh bulk collection",
            )
        })?;
    Ok(())
}

/// Resolves and validates the collection's two durable identities before
/// mutation. A newly minted local id may rebind to an existing upstream row
/// only on the reconciliation surface. Once the local id itself is durable,
/// its non-null upstream identity cannot change, and the two keys must never
/// resolve different rows. A missing upstream identity may still be enriched.
fn resolve_bulk_identity_tx(
    transaction: &rusqlite::Transaction<'_>,
    record: &super::super::records::StoredBulkCollection,
    allow_rebind: bool,
) -> Result<BulkId, HistoryError> {
    let local_upstream = transaction
        .query_row(
            "SELECT upstream_bulk_id FROM bulk_collections WHERE id = ?1",
            [record.id.to_string()],
            |row| row.get::<_, Option<String>>(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            _ => Err(HistoryError::from_sqlite(
                HistoryErrorCode::HistoryUnavailable,
                "resolve bulk collection identity",
            )),
        })?;
    let upstream_owner = match &record.upstream_bulk_id {
        Some(upstream) => transaction
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
                    "resolve bulk collection identity",
                )),
            })?
            .map(|id| {
                id.parse().map_err(|_| {
                    HistoryError::new(
                        HistoryErrorCode::HistoryCorrupt,
                        "the stored bulk collection identity is invalid",
                    )
                })
            })
            .transpose()?,
        None => None,
    };

    if let Some(stored_upstream) = local_upstream {
        if matches!(
            (&stored_upstream, &record.upstream_bulk_id),
            (Some(stored), Some(incoming)) if stored != incoming
        ) || upstream_owner.is_some_and(|owner| owner != record.id)
        {
            return Err(HistoryError::new(
                HistoryErrorCode::HistoryWriteFailed,
                "the local and upstream bulk identities resolve different collections",
            ));
        }
        return Ok(record.id);
    }

    match upstream_owner {
        Some(owner) if allow_rebind => Ok(owner),
        Some(_) => Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "the local and upstream bulk identities resolve different collections",
        )),
        None => Ok(record.id),
    }
}

/// The child upsert resolved by both of its attested identities at once
/// (docs/history-contract.md task-first/bulk-second rule): its
/// `(bulk_id, bulk_index)` membership and every attested
/// `(check_kind, upstream_task_id)` key in `observations`. When both
/// identities resolve the one same stored row (or the task keys resolve
/// one row and the membership is unoccupied), that existing durable row is
/// reused: a previously standalone row gains its membership link, and only
/// observation fields move, so the row's first-recorded identity,
/// authorship, `save_state`, local input and FTS payload, and creation
/// time are preserved exactly. When the candidates conflict (more than one
/// distinct row, a task-key row different from the membership row, or an
/// overlapping-but-different attested task set on the membership row),
/// the whole reconcile fails and its batch rolls back; the ambiguous
/// taskless-membership/distinct-task-row case is `history_corrupt`, while
/// ordinary conflicting candidate sets are `history_write_failed`. No
/// unrelated row is ever deleted, merged, or rekeyed to force a fit.
/// Returns the child's stored identity.
pub(in crate::history) fn upsert_child_row_tx(
    transaction: &rusqlite::Transaction<'_>,
    record: &StoredAnalysis,
    observations: &[StoredUpstreamTask],
) -> Result<crate::domain::AnalysisId, HistoryError> {
    let Some((bulk_id, bulk_index)) = record.bulk else {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "a bulk child without its membership link cannot be reconciled",
        ));
    };
    let total_items = transaction
        .query_row(
            "SELECT total_items FROM bulk_collections WHERE id = ?1",
            [bulk_id.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| {
            HistoryError::from_sqlite(
                HistoryErrorCode::HistoryWriteFailed,
                "validate bulk child membership",
            )
        })?;
    if bulk_index < 0 || bulk_index >= total_items {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "a bulk child index is outside its collection range",
        ));
    }

    let prior_id = child_prior_state_tx(transaction, bulk_id, bulk_index)?;
    let prior_search = prior_id
        .as_ref()
        .map(|id| child_search_by_id_tx(transaction, id))
        .transpose()?
        .unwrap_or((None, None, None, None));
    let targets = task_lookup_targets(transaction, observations)?;
    let membership_id = prior_id;
    // Occupied-membership conflict (docs/history-contract.md fail-closed
    // rule): the membership holder already attests an overlapping but
    // different set of the task keys this read carries; it cannot be the
    // same row as the task resolution, so the batch fails closed and
    // rolls back rather than annexing or rewriting an unrelated row.
    if let Some(holder) = &membership_id {
        let attested = child_attested_task_keys_tx(transaction, holder)?;
        let this_read: std::collections::BTreeSet<(String, String)> = observations
            .iter()
            .map(|task| {
                (
                    super::super::wire::wire_check_kind(task.check_kind).to_owned(),
                    task.upstream_task_id.clone(),
                )
            })
            .collect();
        let overlapping = attested.iter().any(|key| this_read.contains(key));
        // A different upstream id for an already-attested check kind is
        // also overlap: `upstream_tasks` stores one row per
        // `(analysis_id, check_kind)`, so accepting it would silently
        // replace the holder's identity even though the exact key strings
        // are disjoint.
        let same_kind_conflict = attested.iter().any(|(kind, task_id)| {
            this_read
                .iter()
                .any(|(read_kind, read_task_id)| read_kind == kind && read_task_id != task_id)
        });
        if same_kind_conflict || (overlapping && attested != this_read) {
            return Err(HistoryError::new(
                HistoryErrorCode::HistoryWriteFailed,
                "the membership row already attests a different set of the \
                 observed task identities",
            ));
        }
        // A task-less membership and a distinct standalone row that already
        // owns this task identity are ambiguous provenance. Neither row may
        // donate, delete, merge, or rekey evidence to force convergence.
        if attested.is_empty() && targets.as_ref().is_some_and(|target| target != holder) {
            return Err(HistoryError::new(
                HistoryErrorCode::HistoryCorrupt,
                "a taskless membership and a distinct task row claim the same observation",
            ));
        }
    }
    let target = resolve_child_target(membership_id.as_ref(), &targets)?;

    match target {
        ChildTarget::Adopt(stored_id) => {
            // The task keys resolved one existing durable row that does
            // not yet carry this membership: adopt it in place with a
            // refresh-only UPDATE, never an INSERT, so its identity,
            // authorship, save state, local input, and creation time stay
            // exactly as first recorded.
            let adopted_search = child_search_by_id_tx(transaction, &stored_id)?;
            let terminal_dominates = refresh_existing_child_tx(transaction, &stored_id, record)?;
            // The membership-holder search payload coalesces onto the
            // adopted row's own search payload (its locally held content
            // wins over a body-less refresh).
            let (input_text, filename, headline, source_urls) =
                coalesce_search_tx(record, adopted_search, terminal_dominates);
            replace_search_row(
                transaction,
                &stored_id.to_string(),
                &input_text,
                &filename,
                &headline,
                &source_urls,
            )?;
            Ok(stored_id)
        }
        ChildTarget::Membership => {
            // The membership path: the row at this membership is reused
            // (or a first save inserts), through the conflict-targeted
            // upsert with its authorship/coalescing rules.
            let row = AnalysisRow::of(record);
            let body = incoming_body(&record.result_json, &record.error_json)?;
            let (body_result_sql, body_error_sql) = match body {
                IncomingBody::Empty => (
                    "COALESCE(excluded.result_json, analyses.result_json)",
                    "COALESCE(excluded.error_json, analyses.error_json)",
                ),
                IncomingBody::Result => ("excluded.result_json", "NULL"),
                IncomingBody::Error => ("NULL", "excluded.error_json"),
            };
            let completed_sql = if record.completed_at.is_some() {
                "excluded.completed_at"
            } else {
                "COALESCE(excluded.completed_at, analyses.completed_at)"
            };
            transaction
                .execute(
                    &format!(
                        "{ANALYSES_INSERT}
                         ON CONFLICT (bulk_id, bulk_index) DO UPDATE SET
                            status = CASE
                                WHEN analyses.result_json IS NOT NULL
                                     OR analyses.error_json IS NOT NULL
                                THEN CASE
                                    WHEN excluded.result_json IS NULL
                                         AND excluded.error_json IS NULL
                                    THEN analyses.status
                                    ELSE excluded.status
                                END
                                ELSE excluded.status
                            END,
                            result_json = {body_result_sql},
                            error_json = {body_error_sql},
                            upstream_version = CASE
                                WHEN (analyses.result_json IS NOT NULL
                                      OR analyses.error_json IS NOT NULL)
                                     AND excluded.result_json IS NULL
                                     AND excluded.error_json IS NULL
                                THEN analyses.upstream_version
                                ELSE COALESCE(excluded.upstream_version, analyses.upstream_version)
                            END,
                            updated_at = excluded.updated_at,
                            completed_at = CASE
                                WHEN (analyses.result_json IS NOT NULL
                                      OR analyses.error_json IS NOT NULL)
                                     AND excluded.result_json IS NULL
                                     AND excluded.error_json IS NULL
                                THEN analyses.completed_at
                                ELSE {completed_sql}
                            END"
                    ),
                    row.as_params(),
                )
                .map_err(|_| {
                    HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryWriteFailed,
                        "refresh bulk child",
                    )
                })?;

            let Some(search_id) = prior_id else {
                // A fresh membership has no FTS row to replace. Avoid a
                // table scan through the contentless FTS table before every
                // insert, which would make large first-time bulks quadratic.
                insert_search_row(transaction, record)?;
                return Ok(record.id);
            };
            let terminal_dominates = matches!(body, IncomingBody::Empty)
                && stored_terminal_snapshot_tx(transaction, &search_id)?;
            let (input_text, filename, headline, source_urls) =
                coalesce_search_tx(record, prior_search, terminal_dominates);
            replace_search_row(
                transaction,
                &search_id.to_string(),
                &input_text,
                &filename,
                &headline,
                &source_urls,
            )?;
            Ok(search_id)
        }
    }
}

/// Where one bulk child's write lands after both its identities were
/// resolved inside the transaction.
enum ChildTarget {
    /// Reuse the existing durable row this identity resolved to (a
    /// task-first adoption or a task-keyed lead): never INSERT.
    Adopt(crate::domain::AnalysisId),
    /// The `(bulk_id, bulk_index)` conflict-target upsert: the membership
    /// row is reused when present, else the fresh row inserts.
    Membership,
}

/// Resolves the one durable identity the attested task keys point at for
/// this child. `Ok(None)` means no observation attested a resolvable
/// durable row (either no task keys at all, or none of them is recorded
/// yet). `Err(history_write_failed)` means the keys resolve two different
/// stored rows: the candidates disagree and the batch rolls back.
fn resolve_child_target(
    membership: Option<&crate::domain::AnalysisId>,
    task_target: &Option<crate::domain::AnalysisId>,
) -> Result<ChildTarget, HistoryError> {
    match (membership, task_target) {
        // The membership row exists and already attests an overlapping but
        // different task-key set: an occupied-membership conflict. The
        // task keys resolve a DIFFERENT row while the membership row is
        // occupied: the candidates disagree. Both fail closed.
        (Some(m), Some(t)) if m != t => Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "the membership row and the observed task identity resolve \
             different stored analyses",
        )),
        // Same row by both identities: reuse through the membership path
        // (the conflict-target update preserves its authorship).
        (Some(_), Some(_)) => Ok(ChildTarget::Membership),
        // Only the task keys resolve: adopt that one row in place.
        (None, Some(target)) => Ok(ChildTarget::Adopt(*target)),
        // No task resolution (or none attested): pure membership path.
        (_, None) => Ok(ChildTarget::Membership),
    }
}

/// A refresh-only UPDATE of an existing durable child row, used when a
/// bulk read adopts a previously standalone observation (or leads a
/// no-task-identity membership row): the identity, authorship columns,
/// save state, local input, and creation time never move. `bulk_id` and
/// `bulk_index` are written only when the stored pair is both NULL or
/// already exactly equal to the incoming pair; a cross-position,
/// cross-collection, or partial membership makes the guarded update affect
/// no row and fails the transaction closed. A record carrying no membership
/// link leaves the stored pair untouched.
fn refresh_existing_child_tx(
    transaction: &rusqlite::Transaction<'_>,
    stored_id: &crate::domain::AnalysisId,
    record: &StoredAnalysis,
) -> Result<bool, HistoryError> {
    let body = incoming_body(&record.result_json, &record.error_json)?;
    let terminal_dominates =
        matches!(body, IncomingBody::Empty) && stored_terminal_snapshot_tx(transaction, stored_id)?;
    let (body_result_sql, body_error_sql) = match body {
        IncomingBody::Empty => ("COALESCE(?3, result_json)", "COALESCE(?4, error_json)"),
        IncomingBody::Result => ("?3", "NULL"),
        IncomingBody::Error => ("NULL", "?4"),
    };
    let completed_sql = if record.completed_at.is_some() {
        "?7"
    } else {
        "COALESCE(?7, completed_at)"
    };
    let statement = if record.bulk.is_some() {
        format!(
            "UPDATE analyses SET
                status = CASE WHEN ?10 THEN status ELSE ?2 END,
                result_json = {body_result_sql},
                error_json = {body_error_sql},
                bulk_id = ?5,
                bulk_index = ?6,
                upstream_version = CASE
                    WHEN ?10 THEN upstream_version
                    ELSE COALESCE(?8, upstream_version)
                END,
                updated_at = ?9,
                completed_at = CASE WHEN ?10 THEN completed_at ELSE {completed_sql} END
             WHERE id = ?1
               AND ((bulk_id IS NULL AND bulk_index IS NULL)
                    OR (bulk_id = ?5 AND bulk_index = ?6))"
        )
    } else {
        format!(
            "UPDATE analyses SET
                status = CASE WHEN ?10 THEN status ELSE ?2 END,
                result_json = {body_result_sql},
                error_json = {body_error_sql},
                upstream_version = CASE
                    WHEN ?10 THEN upstream_version
                    ELSE COALESCE(?8, upstream_version)
                END,
                updated_at = ?9,
                completed_at = CASE WHEN ?10 THEN completed_at ELSE {completed_sql} END
             WHERE id = ?1"
        )
    };
    let (bulk_id, bulk_index) = match record.bulk {
        Some((bulk_id, bulk_index)) => (Some(bulk_id.to_string()), Some(bulk_index)),
        None => (None, None),
    };
    let written = transaction
        .execute(
            &statement,
            params![
                stored_id.to_string(),
                wire_status(record.status),
                record.result_json,
                record.error_json,
                bulk_id,
                bulk_index,
                record.completed_at.map(|instant| instant.to_string()),
                record.upstream_version,
                record.updated_at.to_string(),
                terminal_dominates,
            ],
        )
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "adopt bulk child")
        })?;
    if written == 0 {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "adopt bulk child",
        ));
    }
    Ok(terminal_dominates)
}

/// The search payload of the adopted child row, so the refresh coalesces
/// onto ITS locally held content (never discarding an authored payload).
fn child_prior_state_tx(
    transaction: &rusqlite::Transaction<'_>,
    bulk_id: BulkId,
    bulk_index: i64,
) -> Result<Option<crate::domain::AnalysisId>, HistoryError> {
    transaction
        .query_row(
            "SELECT id FROM analyses WHERE bulk_id = ?1 AND bulk_index = ?2",
            params![bulk_id.to_string(), bulk_index],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                HistoryError::from_sqlite(HistoryErrorCode::NotFound, "resolve bulk child")
            }
            _ => HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "resolve bulk child"),
        })
        .and_then(|id| {
            id.parse().map(Some).map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "resolve bulk child")
            })
        })
        .or_else(|error| match error {
            error if error.code() == HistoryErrorCode::NotFound => Ok(None),
            error => Err(error),
        })
}

/// Every `(check_kind, upstream_task_id)` key the row `id` currently
/// attests, for the occupied-membership conflict rule.
fn child_attested_task_keys_tx(
    transaction: &rusqlite::Transaction<'_>,
    id: &crate::domain::AnalysisId,
) -> Result<std::collections::BTreeSet<(String, String)>, HistoryError> {
    let mut statement = transaction
        .prepare(
            "SELECT check_kind, upstream_task_id FROM upstream_tasks \
             WHERE analysis_id = ?1 ORDER BY check_kind, upstream_task_id",
        )
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "read task keys")
        })?;
    let keys = statement
        .query_map([id.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "read task keys")
        })?
        .collect::<Result<std::collections::BTreeSet<_>, _>>()
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "read task keys")
        })?;
    Ok(keys)
}

/// The search payload of a refreshed child: stored input authorship wins,
/// while result-owned metadata exactly follows the incoming authoritative
/// terminal projection. A body-less refresh of terminal evidence keeps the
/// complete prior projection.
fn coalesce_search_tx(
    record: &StoredAnalysis,
    prior: SearchColumns,
    terminal_dominates: bool,
) -> SearchColumns {
    if terminal_dominates {
        return prior;
    }
    (
        prior.0.or_else(|| record.search_input_text.clone()),
        prior.1.or_else(|| record.search_filename.clone()),
        record.search_headline.clone(),
        record.search_source_urls.clone(),
    )
}
