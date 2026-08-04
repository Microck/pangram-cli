//! The detection/bulk adapter's history save seam (contracts.md 14.2 note,
//! docs/history-contract.md): explicit `--save`, the disabled-by-default
//! automatic gate, the warning/error split the contract locks, and the
//! storage-lazy policy (no history directory is ever created unless a
//! completed analysis is actually persisted).
//!
//! Storage itself stays inside [`crate::history`]; this module owns only the
//! adapter decisions: when to open the store (once per invocation, lazily),
//! which save state each analysis reports, and how a failure surfaces (one
//! sanitized warning for the automatic path, a canonical local error for an
//! explicit failure). The adapter never duplicates SQL or endpoint logic.

use crate::domain::{Analysis, AnalysisStatus, SaveState};
use crate::history::{HistoryError, HistoryStore};
use crate::output::CanonicalError;

/// The history gate resolved at plan time. Storage is never opened here;
/// the gate only records which persistence paths the invocation armed. The
/// `history.enabled` configuration read happens later, at the one persist
/// seam (`policy_for`), so planning performs no configuration or storage
/// work and a disabled run resolves the gate without ever opening the
/// directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveStoreGate {
    /// Explicit `--save`: persists even with `history.enabled = false`, and
    /// its reported state is `saved_manual`.
    ManualOnly,
    /// Only the automatic path may apply: persists when the effective
    /// configuration resolves `history.enabled = true`; otherwise nothing is
    /// persisted and the store is never opened.
    Automatic,
}

/// Resolves the plan-time history gate from the explicit flag alone. An
/// explicit `--save` arms the manual path; anything else leaves the
/// automatic path governed entirely by the configuration read at persist
/// time.
pub(crate) fn resolve_gate(manual: bool) -> SaveStoreGate {
    if manual {
        SaveStoreGate::ManualOnly
    } else {
        SaveStoreGate::Automatic
    }
}

/// The one invocation-scoped automatic-history warning latch (contracts.md
/// 14.2 note). One bulk command (`bulk submit --wait`, `bulk status`,
/// `bulk wait`) shares it across its observed-children read phase and its
/// persistence phase, so a run in which both phases fail still emits
/// exactly one direct sanitized `warning:` line; a repeated-file detect
/// run shares one across every member. The latch is a plain flag the
/// adapter passes by mutable reference into the store seams; it never
/// touches storage itself.
pub(crate) type InvocationWarningLatch = bool;

/// The bulk command's latch owner. `bulk submit --wait`/`bulk status`/
/// `bulk wait` create one at invocation start, lend it to the observed-
/// children read failure branch, and then hand it to
/// [`persist_bulk_collection`], so the first failure emits and every later
/// failure in the same invocation is silent.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct BulkSaveWarning {
    warned: InvocationWarningLatch,
}

impl BulkSaveWarning {
    pub(crate) fn new() -> Self {
        Self { warned: false }
    }

    /// Lends the shared latch to one fallible phase.
    pub(crate) fn latch(&mut self) -> &mut InvocationWarningLatch {
        &mut self.warned
    }
}

/// One bulk child handed to the save seam: the canonical child analysis and
/// its caller ID, in strictly ascending plan/source order. The shared
/// analysis projection (`acceptance_children` / observed-children) emits
/// children in ascending `(bulk_index)` order by construction; this seam
/// enumerates that order into the `(bulk_id, bulk_index)` membership key,
/// so repeated reads can never duplicate or misorder a stored child.
pub(crate) type BulkChild = (Analysis<CanonicalError>, Option<String>);

/// The resolved history policy for one invocation.
#[derive(Debug, Clone, Copy)]
struct SavePolicy {
    manual: bool,
}

impl SavePolicy {
    fn save_state(self) -> SaveState {
        if self.manual {
            SaveState::SavedManual
        } else {
            SaveState::SavedHistory
        }
    }
}

/// Resolves the effective policy from the gate and the configuration
/// service. The automatic gate reads the effective configuration here, at
/// the one seam; a read failure defaults the gate off (privacy-safe), and a
/// disabled run never opens the directory.
fn policy_for(gate: SaveStoreGate, service: &crate::config::ConfigService) -> Option<SavePolicy> {
    if matches!(gate, SaveStoreGate::ManualOnly) {
        return Some(SavePolicy { manual: true });
    }
    if automatic_enabled(service) {
        Some(SavePolicy { manual: false })
    } else {
        None
    }
}

/// Whether the effective configuration resolves `history.enabled = true`.
/// A configuration read failure defaults the gate off: never opening
/// history silently is the privacy-safe default.
fn automatic_enabled(service: &crate::config::ConfigService) -> bool {
    service
        .effective()
        .ok()
        .and_then(|config| config.history)
        .and_then(|history| history.enabled)
        .unwrap_or(false)
}

/// Whether the automatic history gate is armed for this invocation. The
/// adapter reads this only to decide whether to fetch observed children for
/// the save seam (a cheap configuration read that never opens storage); the
/// persistence decision itself still funnels through the one save seam.
pub(crate) fn automatic_history_armed(service: &crate::config::ConfigService) -> bool {
    automatic_enabled(service)
}

/// Persists completed detect analyses per the resolved gate, then hands
/// back the analyses with their honest save state for rendering. Shared by
/// the single-analysis, repeated-file, and terminal-failure
/// `--include-input` flows because every one of them funnels its completed
/// members through this one seam before projection.
///
/// - Manual path: one atomic save per completed member, every member
///   attempted in order even after one fails. A member whose save failed
///   renders with `ephemeral`; a member that saved renders its committed
///   state. The first store failure surfaces as the canonical local error
///   for an exit-7 finish after the full series.
/// - Automatic path: best effort. The whole invocation produces at most one
///   sanitized warning (shared `warned`), every member keeps flowing, and
///   the path never changes an analysis's upstream-derived outcome.
///
/// When neither gate is armed this function is a pass-through: storage is
/// never opened, so a disabled run creates no directory or database.
pub(crate) fn persist_analyses(
    analyses: Vec<Analysis<CanonicalError>>,
    gate: SaveStoreGate,
    service: &crate::config::ConfigService,
) -> (Vec<Analysis<CanonicalError>>, Option<CanonicalError>) {
    let Some(policy) = policy_for(gate, service) else {
        return (analyses, None);
    };
    persist_series(analyses, policy, service.paths().data_dir())
}

/// Persists one remotely observed analysis (a `task status`/`task wait`
/// read) under the automatic gate. A repeated read of the same remote task
/// refreshes that one stored row in place (status and any terminal snapshot
/// only, plus the new observation row) rather than inserting a duplicate
/// (contracts.md 14.2 note).
///
/// Reconciliation never overwrites durable authorship: the stored row keeps
/// its original `submission_outcome` and `save_state`, and an observation
/// never discards the input/filename/search content the stored row already
/// holds. Output keeps the section 4.6 fresh-read identity and reports this
/// read's own save outcome (`saved_history` when this observation persisted,
/// `ephemeral` otherwise) rather than claiming the prior row's save state.
pub(crate) fn persist_observed_analysis(
    analysis: Analysis<CanonicalError>,
    service: &crate::config::ConfigService,
) -> Analysis<CanonicalError> {
    let mut bulk_warning = BulkSaveWarning::new();
    persist_observed(analysis, service, bulk_warning.latch()).0
}

/// The bulk/task half of the seam. A bulk collection owns no `save_state`,
/// so persistence is gate-only (contracts.md 14.2 note: bulk carries no
/// `--save`, so only the contracted `history.enabled = true` automatic path
/// ever applies).
///
/// One remote bulk job owns at most one stored row, reconciled by the
/// contracted `upstream_bulk_id` identity: a `bulk submit`, `bulk submit
/// --wait`, `bulk status`, or `bulk wait` of the same job refreshes the one
/// stored collection together with its children (keyed by their
/// `(bulk_id, bulk_index)` membership) and observation rows, atomically and
/// without duplicates. The returned children are the truthful input series:
/// each child's canonical state is remote-derived and honestly `ephemeral`
/// (a bulk child has no per-member manual save state to claim). Every
/// failure follows the automatic rule (one sanitized warning per
/// invocation, the remote outcome never degrades).
pub(crate) fn persist_bulk_collection(
    collection: &crate::domain::BulkCollection,
    children: Vec<BulkChild>,
    service: &crate::config::ConfigService,
    warning: &mut BulkSaveWarning,
) -> (crate::domain::BulkCollection, Vec<Analysis<CanonicalError>>) {
    let ephemeral = |children: Vec<BulkChild>| {
        children
            .into_iter()
            .map(|(child, _)| child.with_save_state(SaveState::Ephemeral))
            .collect()
    };
    if !automatic_enabled(service) {
        return (collection.clone(), ephemeral(children));
    }
    let mut store = match HistoryStore::open(service.paths().data_dir()) {
        Ok(store) => store,
        Err(error) => {
            automatic_warning_once(&error, warning.latch());
            return (collection.clone(), ephemeral(children));
        }
    };

    // Build every child's stored row with its provisional membership link,
    // then reconcile the collection, the children, and all observation rows
    // in ONE atomic store-owned transaction. The store resolves the real
    // stored collection identity by `upstream_bulk_id` inside the
    // transaction and rebinds every membership/observation key onto it, so
    // a concurrent second process refreshing the same job serializes on the
    // write lock and the `upstream_bulk_id` unique constraint and can never
    // duplicate the stored row. A child whose own projection fails is
    // skipped from the batch (its row simply is not persisted this read);
    // its truthful ephemeral rendering below is unaffected.
    let provisional_id = collection.id();
    let mut prepared: Vec<(
        crate::history::StoredAnalysis,
        Vec<crate::history::StoredUpstreamTask>,
    )> = Vec::with_capacity(children.len());
    for (index, (child, caller_id)) in children.iter().enumerate() {
        let bulk_index = i64::try_from(index).unwrap_or(i64::MAX);
        let mut record = match crate::history::save::stored_analysis(child, SaveState::SavedHistory)
        {
            Ok(record) => record,
            Err(error) => {
                automatic_warning_once(&error, warning.latch());
                continue;
            }
        };
        record.bulk = Some((provisional_id, bulk_index));
        record.caller_id = caller_id.clone();
        let observations = crate::history::save::stored_observations(child)
            .into_iter()
            .map(|task| crate::history::StoredUpstreamTask {
                analysis_id: record.id,
                ..task
            })
            .collect();
        prepared.push((record, observations));
    }
    let row = crate::history::save::stored_bulk_collection(collection);
    if let Err(error) = store.reconcile_bulk_collection_atomic(&row, &prepared) {
        automatic_warning_once(&error, warning.latch());
    }

    // A bulk collection owns no save_state and its children's canonical
    // state is remote-derived only; the truthful ephemeral rendering is
    // returned regardless of the upsert outcome (no fabrication,
    // contracts.md 4.5). The full input series is preserved, never dropped.
    (collection.clone(), ephemeral(children))
}

/// The shared observation-persist core. Returns the analysis with its
/// honest save state plus the shared one-warning latch so a caller reading
/// several observations in one command still warns exactly once.
fn persist_observed(
    analysis: Analysis<CanonicalError>,
    service: &crate::config::ConfigService,
    warned: &mut InvocationWarningLatch,
) -> (Analysis<CanonicalError>, ()) {
    if !automatic_enabled(service) {
        return (analysis, ());
    }
    let mut store = match HistoryStore::open(service.paths().data_dir()) {
        Ok(store) => store,
        Err(error) => {
            automatic_warning_once(&error, warned);
            return (analysis, ());
        }
    };
    let observations = crate::history::save::stored_observations(&analysis);
    // The stored-row projection of this read (`save_state` is the read's
    // own automatic state; the store keeps the stored row's authorship).
    let record = match crate::history::save::stored_analysis(&analysis, SaveState::SavedHistory) {
        Ok(record) => record,
        Err(error) => {
            automatic_warning_once(&error, warned);
            return (analysis, ());
        }
    };
    // One atomic store-owned reconcile: the prior-row lookup, the merge over
    // the real stored row, and the insert-or-refresh commit inside one
    // IMMEDIATE transaction, serialized by SQLite's write lock and the
    // `(check_kind, upstream_task_id)` unique constraint. The adapter never
    // performs its own lookup, so a concurrent second process can never
    // race a duplicate durable row in.
    let observed_at = analysis.updated_at;
    let outcome =
        store.reconcile_observed_analysis_atomic(&record, &observations, observed_at, |prior| {
            crate::history::save::observation_merge(&analysis, prior)
        });
    match outcome {
        Ok(_) => (analysis.with_save_state(SaveState::SavedHistory), ()),
        Err(error) => {
            automatic_warning_once(&error, warned);
            (analysis, ())
        }
    }
}

/// The ordered-series persist core (detect). Every completed member is
/// preserved and rendered exactly once, in invocation order, whatever the
/// save outcome (contracts.md 14.2 note). The tail is never dropped.
fn persist_series(
    analyses: Vec<Analysis<CanonicalError>>,
    policy: SavePolicy,
    data_dir: &std::path::Path,
) -> (Vec<Analysis<CanonicalError>>, Option<CanonicalError>) {
    let is_terminal = |analysis: &Analysis<CanonicalError>| {
        !matches!(
            analysis.status(),
            AnalysisStatus::Queued | AnalysisStatus::Running
        )
    };
    if !analyses.iter().any(is_terminal) {
        // A detached detect has only an accepted queued/running snapshot.
        // It is not the completed envelope this seam persists, so automatic
        // history leaves it ephemeral without opening SQLite or warning.
        return (analyses, None);
    }

    let save_state = policy.save_state();
    let mut store = match HistoryStore::open(data_dir) {
        Ok(store) => store,
        Err(error) => return persist_open_failure(analyses, policy, error),
    };

    let mut warned = false;
    let mut first_error: Option<HistoryError> = None;
    let mut saved = Vec::with_capacity(analyses.len());
    for analysis in analyses {
        if !is_terminal(&analysis) {
            saved.push(analysis);
            continue;
        }
        match save_one(&mut store, &analysis, save_state) {
            Ok(()) => saved.push(analysis.with_save_state(save_state)),
            Err(error) => {
                if policy.manual {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                } else {
                    automatic_warning_once(&error, &mut warned);
                }
                // A member whose save failed stays in the series with the
                // honest `ephemeral` state; only that member's save state
                // changes. Later members still persist.
                saved.push(analysis);
            }
        }
    }
    let save_failure = if policy.manual {
        first_error.map(|error| error.into_canonical())
    } else {
        None
    };
    (saved, save_failure)
}

/// Persists one analysis atomically: the row insert plus one observation
/// row per check with an upstream task identity (the current remote
/// observation), committed in one transaction so a mid-write failure can
/// never leave a half-committed analysis. A bulk child additionally carries
/// its `(bulk_id, bulk_index)` membership link and caller ID into the same
/// row.
pub(crate) fn save_one_with_membership(
    store: &mut HistoryStore,
    analysis: &Analysis<CanonicalError>,
    save_state: SaveState,
    bulk: Option<(crate::domain::BulkId, i64)>,
    caller_id: Option<String>,
) -> Result<(), HistoryError> {
    let mut record = crate::history::save::stored_analysis(analysis, save_state)?;
    record.bulk = bulk;
    record.caller_id = caller_id;
    store.save_analysis_atomic(
        &record,
        &crate::history::save::stored_observations(analysis),
    )
}

/// Persists one local top analysis (no bulk membership link).
fn save_one(
    store: &mut HistoryStore,
    analysis: &Analysis<CanonicalError>,
    save_state: SaveState,
) -> Result<(), HistoryError> {
    save_one_with_membership(store, analysis, save_state, None, None)
}

/// Store-open failure: the manual path surfaces the canonical error; the
/// automatic path warns once and keeps the analyses ephemeral.
fn persist_open_failure(
    analyses: Vec<Analysis<CanonicalError>>,
    policy: SavePolicy,
    error: HistoryError,
) -> (Vec<Analysis<CanonicalError>>, Option<CanonicalError>) {
    if policy.manual {
        // The explicit request could not be honored (for example insecure
        // history permissions). Nothing was saved; every analysis reports
        // its honest ephemeral state, and the caller surfaces the error.
        (analyses, Some(error.into_canonical()))
    } else {
        let mut warned = false;
        automatic_warning_once(&error, &mut warned);
        (analyses, None)
    }
}

/// The one sanitized automatic-history warning for the whole invocation
/// (contracts.md 14.2 note, product-spec 10.5). The message names the
/// operation and platform state only; it never carries submitted content,
/// upstream text, or paths. `warned` is the invocation-scoped latch shared
/// across every analysis, bulk child, and bulk phase in one command, so a
/// multi-member or multi-phase automatic failure emits exactly one direct
/// `warning:` line, and it carries one `warning:` prefix with no doubling.
fn automatic_warning_once(error: &HistoryError, warned: &mut InvocationWarningLatch) {
    if *warned {
        return;
    }
    *warned = true;
    super::render::warning_stderr_raw(&format!(
        "automatic history save failed ({})",
        error.message()
    ));
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::config::{ConfigOverrides, ConfigService, Paths};
    use crate::domain::{
        AnalysisId, AnalysisStatus, BulkCollection, BulkCounters, BulkId, Check, CheckState,
        OrderedChecks, Provenance, Provider, SaveState, SubmissionOutcome, UpstreamBulkId,
        UpstreamIdentity, UpstreamTaskId, UpstreamTaskIds, UtcTimestamp,
    };
    use crate::history::HistoryStore;

    use super::{BulkChild, BulkSaveWarning, persist_bulk_collection};

    fn service(root: &tempfile::TempDir, enabled: bool) -> ConfigService {
        let config = root.path().join("config");
        let data = root.path().join("data");
        std::fs::create_dir_all(&config).expect("config directory");
        std::fs::create_dir_all(&data).expect("data directory");
        let service =
            ConfigService::for_test(Paths::for_test(config, data), ConfigOverrides::default());
        if enabled {
            service
                .set("history.enabled", "true")
                .expect("enable history");
        }
        service
    }

    fn bulk_fixture() -> (BulkCollection, Vec<BulkChild>) {
        let observed_at = UtcTimestamp::from_str("2026-08-04T12:00:00Z").unwrap();
        let upstream_bulk_id = UpstreamBulkId::from_str("bulk-return-state").unwrap();
        let task_id = UpstreamTaskId::from_str("task-return-state").unwrap();
        let checks = OrderedChecks::new([Check::AiDetection(CheckState::Queued {
            upstream: Some(UpstreamIdentity {
                task_id: Some(task_id.clone()),
                last_stage: None,
            }),
        })])
        .expect("one queued check");
        let child = crate::domain::Analysis::with_optional_input(
            AnalysisId::new(),
            SubmissionOutcome::Accepted,
            None,
            checks,
            SaveState::SavedHistory,
            Provenance {
                provider: Provider::Pangram,
                upstream_version: None,
                upstream_task_ids: Some(UpstreamTaskIds::new(vec![task_id]).unwrap()),
                upstream_bulk_id: Some(upstream_bulk_id.clone()),
                submitted_at: None,
                completed_at: None,
            },
            None,
            None,
            observed_at,
            observed_at,
            None,
        )
        .expect("accepted bulk child");
        let collection = BulkCollection::new(
            BulkId::new(),
            Some(upstream_bulk_id),
            AnalysisStatus::Queued,
            SubmissionOutcome::Accepted,
            BulkCounters::new(1, 1, 0, 0).unwrap(),
            Some(1),
            observed_at,
            observed_at,
            None,
        )
        .expect("queued bulk collection");
        (collection, vec![(child, Some("caller-0".to_owned()))])
    }

    #[test]
    fn disabled_bulk_persistence_returns_ephemeral_children_without_opening_sqlite() {
        let root = tempfile::tempdir().unwrap();
        let service = service(&root, false);
        let (collection, children) = bulk_fixture();

        let (_, returned) =
            persist_bulk_collection(&collection, children, &service, &mut BulkSaveWarning::new());

        assert_eq!(returned.len(), 1);
        assert_eq!(returned[0].save_state, SaveState::Ephemeral);
        assert!(!service.paths().data_dir().join("history").exists());
    }

    #[cfg(unix)]
    #[test]
    fn failed_bulk_persistence_returns_ephemeral_children_without_a_database() {
        let root = tempfile::tempdir().unwrap();
        let service = service(&root, true);
        std::fs::write(service.paths().data_dir().join("history"), b"hostile path")
            .expect("poison history path");
        let (collection, children) = bulk_fixture();

        let (_, returned) =
            persist_bulk_collection(&collection, children, &service, &mut BulkSaveWarning::new());

        assert_eq!(returned.len(), 1);
        assert_eq!(returned[0].save_state, SaveState::Ephemeral);
        assert!(
            !service
                .paths()
                .data_dir()
                .join("history/pangram-history.db")
                .exists()
        );
    }

    #[test]
    fn successful_bulk_persistence_returns_ephemeral_children_and_saves_durable_state() {
        let root = tempfile::tempdir().unwrap();
        let service = service(&root, true);
        let (collection, children) = bulk_fixture();

        let (_, returned) =
            persist_bulk_collection(&collection, children, &service, &mut BulkSaveWarning::new());

        assert_eq!(returned.len(), 1);
        assert_eq!(returned[0].save_state, SaveState::Ephemeral);
        let store = HistoryStore::open(service.paths().data_dir()).expect("reopen real SQLite");
        let durable_save_state: String = store
            .with_connection(|connection| {
                connection
                    .query_row("SELECT save_state FROM analyses", [], |row| row.get(0))
                    .expect("durable child")
            })
            .expect("query durable child");
        assert_eq!(durable_save_state, "saved_history");
    }
}
