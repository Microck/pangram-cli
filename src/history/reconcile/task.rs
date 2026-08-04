//! Atomic reconciliation for standalone task observations.

use std::collections::BTreeMap;

use rusqlite::params;

use super::super::analysis_writes::{
    insert_analysis_row, insert_search_row, upsert_observation_row,
};
use super::super::records::{StoredAnalysis, StoredUpstreamTask};
use super::super::store::HistoryStore;
use super::super::{HistoryError, HistoryErrorCode, ObservationSnapshot};
use super::ReconciledAnalysis;
use super::common::{incoming_body, update_observation_snapshot_tx};

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
        self.in_immediate_transaction(|transaction| {
            incoming_body(&record.result_json, &record.error_json)?;
            let existing = task_lookup_targets(transaction, observations)?
                .map(|id| super::super::reads::stored_analysis_opt_on(transaction, &id))
                .transpose()?
                .flatten();
            match existing {
                Some(prior) => {
                    let id = prior.id;
                    // Selection by one exact task key does not authorize a
                    // later incoming key to replace other durable evidence
                    // owned by this row. Compare before snapshot mutation so
                    // any conflict rolls the entire immediate transaction
                    // back unchanged.
                    validate_owned_task_evidence(transaction, &id, observations)?;
                    let snapshot = merge(&prior)?;
                    incoming_body(&snapshot.result_json, &snapshot.error_json)?;
                    update_observation_snapshot_tx(transaction, &id, observed_at, &snapshot)?;
                    for task in observations {
                        let rebound = StoredUpstreamTask {
                            analysis_id: id,
                            ..task.clone()
                        };
                        upsert_observation_row(transaction, &rebound)?;
                    }
                    Ok(ReconciledAnalysis {
                        stored_id: id,
                        save_state: prior.save_state,
                        inserted: false,
                    })
                }
                None => {
                    insert_analysis_row(transaction, record)?;
                    insert_search_row(transaction, record)?;
                    for task in observations {
                        upsert_observation_row(transaction, task)?;
                    }
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
