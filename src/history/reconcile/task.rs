//! Atomic reconciliation for standalone task observations.

use std::collections::BTreeMap;

use rusqlite::params;

use super::super::analysis_writes::{
    insert_analysis_row, insert_check_rows, insert_search_row, load_stored_check_rows,
    set_check_count, upsert_observation_row,
};
use super::super::records::{StoredAnalysis, StoredCheck, StoredUpstreamTask};
use super::super::store::HistoryStore;
use super::super::{HistoryError, HistoryErrorCode, ObservationSnapshot};
use super::ReconciledAnalysis;
use super::common::{incoming_body, update_observation_snapshot_tx};

#[derive(Clone, Copy)]
enum ReconcileMode {
    Legacy,
    Authoritative { insert_if_missing: bool },
}

impl HistoryStore {
    /// Atomically reconciles one remotely observed analysis onto the one
    /// stored row for its upstream task identity.
    pub fn reconcile_observed_analysis_atomic(
        &mut self,
        record: &StoredAnalysis,
        observations: &[StoredUpstreamTask],
        observed_at: crate::domain::UtcTimestamp,
        merge: impl Fn(&StoredAnalysis) -> Result<ObservationSnapshot, HistoryError>,
    ) -> Result<ReconciledAnalysis, HistoryError> {
        let checks =
            super::super::analysis_writes::legacy_checks_for_reconcile(record, observations)?;
        self.reconcile_observed_analysis_impl(
            record,
            &checks,
            observations,
            observed_at,
            ReconcileMode::Legacy,
            merge,
        )
    }

    /// Reconciles one observation while replacing the complete authoritative
    /// ordered check payload in the same transaction.
    pub fn reconcile_observed_analysis_complete(
        &mut self,
        record: &StoredAnalysis,
        checks: &[StoredCheck],
        observations: &[StoredUpstreamTask],
        observed_at: crate::domain::UtcTimestamp,
        merge: impl Fn(&StoredAnalysis) -> Result<ObservationSnapshot, HistoryError>,
    ) -> Result<ReconciledAnalysis, HistoryError> {
        self.reconcile_observed_analysis_impl(
            record,
            checks,
            observations,
            observed_at,
            ReconcileMode::Authoritative {
                insert_if_missing: true,
            },
            merge,
        )
    }

    /// Reconciles one authoritative standalone check only when its task
    /// identity already belongs to a saved analysis. Non-terminal task reads
    /// use this path so they can refresh an existing combined analysis
    /// without creating a new durable row for an otherwise ephemeral poll.
    pub fn reconcile_existing_observed_analysis_complete(
        &mut self,
        record: &StoredAnalysis,
        checks: &[StoredCheck],
        observations: &[StoredUpstreamTask],
        observed_at: crate::domain::UtcTimestamp,
        merge: impl Fn(&StoredAnalysis) -> Result<ObservationSnapshot, HistoryError>,
    ) -> Result<ReconciledAnalysis, HistoryError> {
        self.reconcile_observed_analysis_impl(
            record,
            checks,
            observations,
            observed_at,
            ReconcileMode::Authoritative {
                insert_if_missing: false,
            },
            merge,
        )
    }

    fn reconcile_observed_analysis_impl(
        &mut self,
        record: &StoredAnalysis,
        checks: &[StoredCheck],
        observations: &[StoredUpstreamTask],
        observed_at: crate::domain::UtcTimestamp,
        mode: ReconcileMode,
        merge: impl Fn(&StoredAnalysis) -> Result<ObservationSnapshot, HistoryError>,
    ) -> Result<ReconciledAnalysis, HistoryError> {
        self.in_immediate_transaction(|transaction| {
            incoming_body(&record.result_json, &record.error_json)?;
            if matches!(mode, ReconcileMode::Authoritative { .. }) {
                super::super::analysis_writes::validate_check_rows(record.id, checks)?;
            }
            let existing = task_lookup_targets(transaction, observations)?
                .map(|id| super::super::reads::stored_analysis_opt_on(transaction, &id))
                .transpose()?
                .flatten();
            match existing {
                Some(prior) => {
                    let id = prior.id;
                    // Validate the complete stored aggregate, including
                    // parent/check agreement and unmatched task evidence,
                    // before deriving or writing any merged state.
                    super::super::read_validation::certify_analysis_aggregate(transaction, &id)?;
                    // Selection by one exact task key does not authorize a
                    // later incoming key to replace other durable evidence
                    // owned by this row. Compare before snapshot mutation so
                    // any conflict rolls the entire immediate transaction
                    // back unchanged.
                    validate_owned_task_evidence(transaction, &id, observations)?;
                    let mut snapshot = merge(&prior)?;
                    if matches!(mode, ReconcileMode::Authoritative { .. }) {
                        let stored_checks = load_stored_check_rows(transaction, &id)?;
                        let merged_checks =
                            merge_matching_checks(&stored_checks, checks, observations, id)?;
                        apply_merged_parent(&prior, &merged_checks, &mut snapshot)?;
                        incoming_body(&snapshot.result_json, &snapshot.error_json)?;
                        update_observation_snapshot_tx(
                            transaction,
                            &id,
                            observed_at,
                            &snapshot,
                            &merged_checks,
                            true,
                        )?;
                    } else {
                        incoming_body(&snapshot.result_json, &snapshot.error_json)?;
                        update_observation_snapshot_tx(
                            transaction,
                            &id,
                            observed_at,
                            &snapshot,
                            checks,
                            false,
                        )?;
                    }
                    for task in observations {
                        let rebound = StoredUpstreamTask {
                            analysis_id: id,
                            ..task.clone()
                        };
                        upsert_observation_row(transaction, &rebound)?;
                    }
                    super::super::read_validation::certify_analysis_aggregate(transaction, &id)?;
                    Ok(ReconciledAnalysis {
                        stored_id: id,
                        save_state: prior.save_state,
                        inserted: false,
                    })
                }
                None => {
                    if matches!(
                        mode,
                        ReconcileMode::Authoritative {
                            insert_if_missing: false
                        }
                    ) {
                        return Err(HistoryError::new(
                            HistoryErrorCode::NotFound,
                            "no saved analysis owns the observed task identity",
                        ));
                    }
                    if matches!(mode, ReconcileMode::Legacy) {
                        super::super::analysis_writes::validate_legacy_save(record, checks)?;
                    }
                    insert_analysis_row(transaction, record)?;
                    insert_search_row(transaction, record)?;
                    insert_check_rows(transaction, checks)?;
                    set_check_count(transaction, record.id, checks.len())?;
                    for task in observations {
                        upsert_observation_row(transaction, task)?;
                    }
                    super::super::analysis_writes::certify_new_analysis_tx(transaction, record)?;
                    Ok(ReconciledAnalysis {
                        stored_id: record.id,
                        save_state: record.save_state,
                        inserted: true,
                    })
                }
            }
        })
    }
}

/// Replaces only the authoritative slot attested by this standalone task
/// observation. Existing terminal evidence dominates a stale queued/running
/// snapshot, and every omitted kind remains byte-for-byte in place.
fn merge_matching_checks(
    stored: &[StoredCheck],
    incoming: &[StoredCheck],
    observations: &[StoredUpstreamTask],
    analysis_id: crate::domain::AnalysisId,
) -> Result<Vec<StoredCheck>, HistoryError> {
    if incoming.len() != 1
        || observations.len() != 1
        || incoming[0].check_kind != observations[0].check_kind
    {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "one standalone task observation must attest exactly one matching check kind",
        ));
    }
    super::super::analysis_writes::validate_check_rows(incoming[0].analysis_id, incoming)?;
    let incoming = &incoming[0];
    let mut merged = stored.to_vec();
    let slot = merged
        .iter_mut()
        .find(|check| check.check_kind == incoming.check_kind)
        .ok_or_else(|| {
            HistoryError::new(
                HistoryErrorCode::HistoryWriteFailed,
                "the observed check kind is not owned by the selected analysis",
            )
        })?;
    let stored_terminal = matches!(
        slot.status,
        crate::domain::CheckStatus::Succeeded | crate::domain::CheckStatus::Failed
    );
    let incoming_terminal = matches!(
        incoming.status,
        crate::domain::CheckStatus::Succeeded | crate::domain::CheckStatus::Failed
    );
    if !stored_terminal || incoming_terminal && slot.status == incoming.status {
        let index = slot.check_index;
        *slot = StoredCheck {
            analysis_id,
            check_index: index,
            check_kind: incoming.check_kind,
            status: incoming.status,
            result_json: incoming.result_json.clone(),
            error_json: incoming.error_json.clone(),
        };
    } else if incoming_terminal {
        return Err(HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "a terminal task observation conflicts with stored terminal evidence",
        ));
    }
    Ok(merged)
}

/// Recomputes the denormalized parent projection from the merged complete
/// check set. Check rows remain the reconstruction source of truth; these
/// columns are maintained only so list/filter and legacy readers observe the
/// same canonical state.
fn apply_merged_parent(
    prior: &StoredAnalysis,
    checks: &[StoredCheck],
    snapshot: &mut ObservationSnapshot,
) -> Result<(), HistoryError> {
    snapshot.status = crate::domain::derive_parent_status(
        &checks.iter().map(|check| check.status).collect::<Vec<_>>(),
    )
    .map_err(|_| {
        HistoryError::new(
            HistoryErrorCode::HistoryWriteFailed,
            "the merged analysis checks cannot derive a parent status",
        )
    })?;
    snapshot.submission_outcome = prior.submission_outcome;
    snapshot.completed_at = if matches!(
        snapshot.status,
        crate::domain::AnalysisStatus::Succeeded
            | crate::domain::AnalysisStatus::Failed
            | crate::domain::AnalysisStatus::Partial
    ) {
        snapshot.completed_at.or(prior.completed_at)
    } else {
        None
    };
    snapshot.result_json = checks.iter().find_map(|check| check.result_json.clone());
    snapshot.error_json = if snapshot.result_json.is_none() {
        checks.iter().find_map(|check| check.error_json.clone())
    } else {
        None
    };
    Ok(())
}

/// Resolves the one durable identity the attested task keys point at. A
/// conflicting projection or keys resolving distinct rows fails closed.
pub(super) fn task_lookup_targets(
    transaction: &rusqlite::Transaction<'_>,
    observations: &[StoredUpstreamTask],
) -> Result<Option<crate::domain::AnalysisId>, HistoryError> {
    let mut ids_by_kind = BTreeMap::<String, &str>::new();
    for task in observations {
        let kind = super::super::wire::wire_check_kind(task.check_kind).to_owned();
        match ids_by_kind.insert(kind, task.upstream_task_id.as_str()) {
            Some(existing) if existing != task.upstream_task_id.as_str() => {
                return Err(HistoryError::new(
                    HistoryErrorCode::HistoryWriteFailed,
                    "one observed check kind attests conflicting upstream task identities",
                ));
            }
            _ => {}
        }
    }

    let mut resolved: Option<crate::domain::AnalysisId> = None;
    for task in observations {
        let found: Option<String> = transaction
            .query_row(
                "SELECT analysis_id FROM upstream_tasks
                 WHERE check_kind = ?1 AND upstream_task_id = ?2",
                params![
                    super::super::wire::wire_check_kind(task.check_kind),
                    task.upstream_task_id
                ],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(HistoryError::from_sqlite(
                    HistoryErrorCode::HistoryUnavailable,
                    "find analysis",
                )),
            })?;
        if let Some(found) = found {
            let target = found.parse().map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryCorrupt, "find analysis")
            })?;
            match &resolved {
                None => resolved = Some(target),
                Some(existing) if *existing == target => {}
                Some(_) => {
                    return Err(HistoryError::new(
                        HistoryErrorCode::HistoryWriteFailed,
                        "the observed task identities resolve different stored analyses",
                    ));
                }
            }
        }
    }
    Ok(resolved)
}

/// Validates every incoming key against every task key already owned by the
/// selected row. Missing kinds may be added, exact keys may refresh, omitted
/// kinds remain untouched, and a different ID for an owned kind is a
/// fail-closed evidence conflict.
fn validate_owned_task_evidence(
    transaction: &rusqlite::Transaction<'_>,
    id: &crate::domain::AnalysisId,
    observations: &[StoredUpstreamTask],
) -> Result<(), HistoryError> {
    let mut statement = transaction
        .prepare(
            "SELECT check_kind, upstream_task_id
             FROM upstream_tasks
             WHERE analysis_id = ?1",
        )
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "read task evidence")
        })?;
    let owned = statement
        .query_map([id.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "read task evidence")
        })?
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "read task evidence")
        })?;

    for task in observations {
        let kind = super::super::wire::wire_check_kind(task.check_kind);
        if owned
            .get(kind)
            .is_some_and(|stored_id| stored_id != &task.upstream_task_id)
        {
            return Err(HistoryError::new(
                HistoryErrorCode::HistoryWriteFailed,
                "the selected analysis already attests a different task identity for this check kind",
            ));
        }
    }
    Ok(())
}
