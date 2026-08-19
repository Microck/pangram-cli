//! Projection of canonical analyses and bulk collections onto the history
//! schema v1 rows (docs/history-contract.md): the one place that turns typed
//! domain values into [`StoredAnalysis`], [`StoredBulkCollection`], and
//! [`StoredUpstreamTask`] inserts.
//!
//! JSON columns carry canonical schema-major-1 bodies. A locally authored
//! retained input includes its caller-owned plaintext even when the primary
//! output omitted `--include-input`; manual save or enabled automatic history
//! is the separate retention choice that authorizes it. Terminal results and
//! canonical check errors remain full typed values. The search payload follows
//! product-spec 10.4: retained submitted text, the input filename, the AI
//! detection headline, and plagiarism source URLs. Nothing else (no credentials, headers, raw
//! response bodies, segments, or matched text) ever lands in a row.

use crate::domain::{
    Analysis, AnalysisInput, BulkCollection, Check, CheckState, SaveState, TextOrigin,
};
use crate::output::CanonicalError;

use super::records::{
    InputKind, StoredAnalysis, StoredBulkCollection, StoredCheck, StoredUpstreamTask,
};
use super::{HistoryError, HistoryErrorCode};

pub(super) struct CanonicalCheckProjection {
    pub result_json: Option<String>,
    pub error_json: Option<String>,
    pub search_headline: Option<String>,
    pub search_source_urls: Option<String>,
}

/// Projects one canonical analysis onto its schema v1 row (typed columns,
/// opaque canonical JSON, and the FTS payload).
pub fn stored_analysis(
    analysis: &Analysis<CanonicalError>,
    save_state: SaveState,
) -> Result<StoredAnalysis, HistoryError> {
    stored_analysis_with_retained_text(analysis, save_state, None)
}

/// Projects a locally submitted analysis while retaining caller-owned
/// plaintext for truthful history show/rerun even when the primary output
/// omitted `--include-input`.
pub fn stored_analysis_with_retained_text(
    analysis: &Analysis<CanonicalError>,
    save_state: SaveState,
    retained_text: Option<&str>,
) -> Result<StoredAnalysis, HistoryError> {
    let retained = retained_text.map(|text| super::RetainedInput::Text(text.to_owned()));
    stored_analysis_with_retained_input(analysis, save_state, retained.as_ref())
}

pub fn stored_analysis_with_retained_input(
    analysis: &Analysis<CanonicalError>,
    save_state: SaveState,
    retained_input: Option<&super::RetainedInput>,
) -> Result<StoredAnalysis, HistoryError> {
    let mut input_value = serde_json::to_value(&analysis.input)
        .map_err(|_| serialization_failure("serialize analysis input"))?;
    if let (Some(retained), Some(input)) = (retained_input, input_value.as_object_mut()) {
        match retained {
            super::RetainedInput::Text(text)
                if input.get("type").and_then(serde_json::Value::as_str) == Some("text") =>
            {
                input.insert("text".to_owned(), serde_json::Value::String(text.clone()));
            }
            super::RetainedInput::File {
                path,
                extracted_text,
            } if input.get("type").and_then(serde_json::Value::as_str) == Some("file") => {
                input.insert("path".to_owned(), serde_json::Value::String(path.clone()));
                if let Some(extracted_text) = extracted_text {
                    input.insert(
                        "extracted_text".to_owned(),
                        serde_json::Value::String(extracted_text.clone()),
                    );
                }
            }
            _ => return Err(serialization_failure("retain analysis input")),
        }
    }
    let input_json = serde_json::to_string(&input_value)
        .map_err(|_| serialization_failure("serialize analysis input"))?;

    let check_projection = project_canonical_checks(analysis.checks())?;

    let (input_kind, input_sha256, display_name, mut search_input_text, search_filename) =
        project_input(analysis);
    if let Some(retained) = retained_input
        && analysis.input().is_some()
    {
        search_input_text = match retained {
            super::RetainedInput::Text(text) => Some(text.clone()),
            super::RetainedInput::File { extracted_text, .. } => extracted_text.clone(),
        };
    }

    Ok(StoredAnalysis {
        id: analysis.id,
        bulk: None,
        caller_id: None,
        status: analysis.status(),
        submission_outcome: analysis.submission_outcome(),
        save_state,
        input_kind,
        input_sha256,
        display_name,
        input_json,
        result_json: check_projection.result_json,
        error_json: check_projection.error_json,
        upstream_version: analysis.provenance().upstream_version.clone(),
        retry_of: analysis.retry_of(),
        rerun_of: analysis.rerun_of(),
        submitted_at: analysis.provenance().submitted_at,
        created_at: analysis.created_at,
        updated_at: analysis.updated_at,
        completed_at: analysis.completed_at,
        search_input_text,
        search_filename,
        search_headline: check_projection.search_headline,
        search_source_urls: check_projection.search_source_urls,
    })
}

fn project_canonical_checks(
    checks: &[Check<CanonicalError>],
) -> Result<CanonicalCheckProjection, HistoryError> {
    // Terminal payloads: the first succeeded check stores its result; only
    // when no check succeeded does the first failed check store its complete
    // canonical error object. This precedence is independent of check order
    // and keeps the parent row's result/error body exactly-one invariant.
    let mut projection = CanonicalCheckProjection {
        result_json: None,
        error_json: None,
        search_headline: None,
        search_source_urls: None,
    };
    let mut source_urls = Vec::new();
    for check in checks {
        match check {
            Check::AiDetection(CheckState::Succeeded { result, .. }) => {
                if projection.result_json.is_none() {
                    projection.result_json = Some(
                        serde_json::to_string(result)
                            .map_err(|_| serialization_failure("serialize terminal result"))?,
                    );
                }
                projection.error_json = None;
                if projection.search_headline.is_none() {
                    projection.search_headline = Some(result.headline.clone());
                }
            }
            Check::Plagiarism(CheckState::Succeeded { result, .. }) => {
                if projection.result_json.is_none() {
                    projection.result_json = Some(
                        serde_json::to_string(result)
                            .map_err(|_| serialization_failure("serialize terminal result"))?,
                    );
                }
                projection.error_json = None;
                source_urls.extend(result.matches.iter().map(|found| found.source_url.clone()));
            }
            Check::AiDetection(CheckState::Failed { error, .. })
            | Check::Plagiarism(CheckState::Failed { error, .. }) => {
                if projection.result_json.is_none() && projection.error_json.is_none() {
                    projection.error_json =
                        Some(serde_json::to_string(error).map_err(|_| {
                            serialization_failure("serialize canonical check error")
                        })?);
                }
            }
            Check::AiDetection(CheckState::Queued { .. } | CheckState::Running { .. })
            | Check::Plagiarism(CheckState::Queued { .. } | CheckState::Running { .. }) => {}
        }
    }
    projection.search_source_urls = (!source_urls.is_empty()).then(|| source_urls.join("\n"));
    Ok(projection)
}

pub(super) fn project_stored_checks(
    checks: &[StoredCheck],
) -> Result<CanonicalCheckProjection, HistoryError> {
    let checks = checks
        .iter()
        .map(|check| {
            let mut value = serde_json::json!({
                "kind": super::wire::wire_check_kind(check.check_kind),
                "status": super::wire::wire_check_status(check.status),
            });
            if let Some(result) = &check.result_json {
                value["result"] = serde_json::from_str(result)
                    .map_err(|_| serialization_failure("decode stored check result"))?;
            }
            if let Some(error) = &check.error_json {
                value["error"] = serde_json::from_str(error)
                    .map_err(|_| serialization_failure("decode stored check error"))?;
            }
            serde_json::from_value(value)
                .map_err(|_| serialization_failure("decode stored canonical check"))
        })
        .collect::<Result<Vec<Check<CanonicalError>>, _>>()?;
    project_canonical_checks(&checks)
}

/// The current observation rows for one analysis: one row per check that
/// holds an upstream task identity at check level. When no check does (an
/// accepted-but-never-polled accepted snapshot reports its real task
/// identity in provenance only, with an honestly stage-less queued check),
/// the provenance identity becomes the `ai_detection` starting observation.
/// This applies to persistence surfaces that contractually retain accepted
/// observations (for example bulk/task reconciliation); detached `detect`
/// snapshots are filtered before reaching the store. The identity is real
/// remote evidence and never fabricates a stage.
pub fn stored_observations(analysis: &Analysis<CanonicalError>) -> Vec<StoredUpstreamTask> {
    let observed_at = analysis.updated_at;
    let mut tasks = Vec::new();
    for check in analysis.checks() {
        let (kind, upstream) = match check {
            Check::AiDetection(state) => (
                crate::domain::CheckKind::AiDetection,
                check_state_upstream(state),
            ),
            Check::Plagiarism(state) => (
                crate::domain::CheckKind::Plagiarism,
                check_state_upstream(state),
            ),
        };
        if let Some(identity) = upstream
            && let Some(task_id) = &identity.task_id
        {
            tasks.push(StoredUpstreamTask {
                analysis_id: analysis.id,
                check_kind: kind,
                upstream_task_id: task_id.to_string(),
                last_stage: identity.last_stage.as_ref().map(|stage| stage.to_string()),
                observed_at,
            });
        }
    }
    if tasks.is_empty()
        && let Some(ids) = &analysis.provenance().upstream_task_ids
    {
        for task_id in ids.as_slice() {
            tasks.push(StoredUpstreamTask {
                analysis_id: analysis.id,
                check_kind: crate::domain::CheckKind::AiDetection,
                upstream_task_id: task_id.to_string(),
                last_stage: None,
                observed_at,
            });
        }
    }
    tasks
}

/// Projects every canonical check into its authoritative ordered payload
/// row. The check table, rather than the parent row's legacy aggregate body
/// columns, is the reconstruction source of truth.
pub fn stored_checks(
    analysis: &Analysis<CanonicalError>,
) -> Result<Vec<StoredCheck>, HistoryError> {
    analysis
        .checks()
        .iter()
        .enumerate()
        .map(|(index, check)| {
            let (check_kind, status, result_json, error_json) =
                match check {
                    Check::AiDetection(state) => match state {
                        CheckState::Queued { .. } => (
                            crate::domain::CheckKind::AiDetection,
                            crate::domain::CheckStatus::Queued,
                            None,
                            None,
                        ),
                        CheckState::Running { .. } => (
                            crate::domain::CheckKind::AiDetection,
                            crate::domain::CheckStatus::Running,
                            None,
                            None,
                        ),
                        CheckState::Succeeded { result, .. } => (
                            crate::domain::CheckKind::AiDetection,
                            crate::domain::CheckStatus::Succeeded,
                            Some(serde_json::to_string(result).map_err(|_| {
                                serialization_failure("serialize AI detection result")
                            })?),
                            None,
                        ),
                        CheckState::Failed { error, .. } => (
                            crate::domain::CheckKind::AiDetection,
                            crate::domain::CheckStatus::Failed,
                            None,
                            Some(serde_json::to_string(error).map_err(|_| {
                                serialization_failure("serialize AI detection error")
                            })?),
                        ),
                    },
                    Check::Plagiarism(state) => match state {
                        CheckState::Queued { .. } => (
                            crate::domain::CheckKind::Plagiarism,
                            crate::domain::CheckStatus::Queued,
                            None,
                            None,
                        ),
                        CheckState::Running { .. } => (
                            crate::domain::CheckKind::Plagiarism,
                            crate::domain::CheckStatus::Running,
                            None,
                            None,
                        ),
                        CheckState::Succeeded { result, .. } => (
                            crate::domain::CheckKind::Plagiarism,
                            crate::domain::CheckStatus::Succeeded,
                            Some(serde_json::to_string(result).map_err(|_| {
                                serialization_failure("serialize plagiarism result")
                            })?),
                            None,
                        ),
                        CheckState::Failed { error, .. } => (
                            crate::domain::CheckKind::Plagiarism,
                            crate::domain::CheckStatus::Failed,
                            None,
                            Some(serde_json::to_string(error).map_err(|_| {
                                serialization_failure("serialize plagiarism error")
                            })?),
                        ),
                    },
                };
            Ok(StoredCheck {
                analysis_id: analysis.id,
                check_index: u8::try_from(index).expect("at most two checks"),
                check_kind,
                status,
                result_json,
                error_json,
            })
        })
        .collect()
}

fn check_state_upstream<R, E>(
    state: &CheckState<R, E>,
) -> Option<&crate::domain::UpstreamIdentity> {
    match state {
        CheckState::Queued { upstream }
        | CheckState::Running { upstream }
        | CheckState::Succeeded { upstream, .. }
        | CheckState::Failed { upstream, .. } => upstream.as_ref(),
    }
}

/// The input-column projection. The ordinary projection sees text only when
/// the analysis carries it; the retained-text entry point above supplies the
/// separately authorized plaintext for a local save. Remotely authored reads
/// never fabricate submitted text (contracts.md 4.6).
#[allow(clippy::type_complexity)]
fn project_input(
    analysis: &Analysis<CanonicalError>,
) -> (
    InputKind,
    crate::domain::Sha256Hash,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    match analysis.input() {
        Some(AnalysisInput::Text(input)) => {
            let is_resumed_unknown = matches!(input.origin(), TextOrigin::Unknown);
            let search_text = if is_resumed_unknown {
                None
            } else {
                input.text.clone()
            };
            (
                InputKind::Text,
                input.sha256,
                input.name().map(str::to_owned),
                search_text,
                input.name().map(str::to_owned),
            )
        }
        Some(AnalysisInput::File(input)) => (
            InputKind::File,
            input.sha256,
            Some(input.filename.to_string()),
            input.extracted_text.clone(),
            Some(input.filename.to_string()),
        ),
        // A resumed read without a terminal document carries no input
        // descriptor (contracts.md 4.6); the row records a zeroed hash and
        // no searchable text rather than inventing one.
        None => (
            InputKind::Text,
            crate::domain::Sha256Hash::from_bytes([0u8; 32]),
            None,
            None,
            None,
        ),
    }
}

/// Projects one canonical bulk collection onto its schema v1 row.
pub fn stored_bulk_collection(collection: &BulkCollection) -> StoredBulkCollection {
    StoredBulkCollection {
        id: collection.id(),
        upstream_bulk_id: collection.upstream_bulk_id().map(|id| id.to_string()),
        status: collection.status(),
        submission_outcome: collection.submission_outcome(),
        counters: *collection.counters(),
        estimated_billable_units: collection.estimated_billable_units(),
        created_at: collection.created_at(),
        updated_at: collection.updated_at(),
        completed_at: collection.completed_at(),
    }
}

/// Merges one remotely observed read over the stored row it reconciles onto
/// (contracts.md 14.2 note: durable authorship invariance). The refresh moves
/// only observation fields: status, terminal JSON bodies, and a
/// terminal-observed `completed_at`. It never overwrites the stored row's
/// original `submission_outcome` (a locally authored `terminal` row stays
/// `terminal`; it is never rewritten to the observation's `accepted`), and it
/// never discards locally held content: when the observation carries no
/// search payload of its own (a read with no local input), the stored row's
/// input text, filename, headline, and source URLs are kept exactly.
///
/// `completed_at` stays optional on the returned snapshot: a non-terminal
/// observation moves nothing terminal, and the store's `COALESCE` write keeps
/// any previously recorded terminal stamp.
pub fn observation_merge(
    observation: &Analysis<CanonicalError>,
    prior: &StoredAnalysis,
) -> Result<super::ObservationSnapshot, HistoryError> {
    let observed = stored_analysis(observation, prior.save_state)?;
    let (search_headline, search_source_urls) = if observed.is_terminal_record() {
        (
            observed.search_headline.clone(),
            observed.search_source_urls.clone(),
        )
    } else {
        (
            prior.search_headline.clone(),
            prior.search_source_urls.clone(),
        )
    };
    Ok(super::ObservationSnapshot {
        status: observation.status(),
        submission_outcome: prior.submission_outcome,
        result_json: observed.result_json,
        error_json: observed.error_json,
        upstream_version: observed
            .upstream_version
            .or_else(|| prior.upstream_version.clone()),
        completed_at: observation.completed_at,
        search_input_text: observed
            .search_input_text
            .or_else(|| prior.search_input_text.clone()),
        search_filename: observed
            .search_filename
            .or_else(|| prior.search_filename.clone()),
        search_headline,
        search_source_urls,
    })
}

fn serialization_failure(operation: &'static str) -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::HistoryWriteFailed,
        format!("{operation}: the canonical value could not be encoded"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AnalysisStatus, CheckKind, CheckStatus, SubmissionOutcome, UtcTimestamp};
    use crate::output::ErrorCode;

    // The row projection is covered end-to-end by the compiled-binary
    // loopback suite; these unit checks lock the pure projection edges the
    // subprocess boundary cannot reach directly.

    use std::str::FromStr;

    fn instant(value: &str) -> UtcTimestamp {
        UtcTimestamp::from_str(value).expect("test timestamp")
    }

    fn prior_row() -> StoredAnalysis {
        StoredAnalysis {
            id: analysis_id(),
            bulk: None,
            caller_id: None,
            status: AnalysisStatus::Queued,
            submission_outcome: SubmissionOutcome::Terminal,
            save_state: SaveState::SavedManual,
            input_kind: InputKind::Text,
            input_sha256: crate::domain::Sha256Hash::from_bytes([3; 32]),
            display_name: Some("original.txt".to_owned()),
            input_json: "{}".to_owned(),
            result_json: None,
            error_json: None,
            upstream_version: Some("4.0".to_owned()),
            retry_of: None,
            rerun_of: None,
            submitted_at: Some(instant("2026-08-01T09:59:00Z")),
            created_at: instant("2026-08-01T10:00:00Z"),
            updated_at: instant("2026-08-01T10:00:00Z"),
            completed_at: None,
            search_input_text: Some("the locally submitted text".to_owned()),
            search_filename: Some("original.txt".to_owned()),
            search_headline: None,
            search_source_urls: None,
        }
    }

    fn analysis_id() -> crate::domain::AnalysisId {
        crate::domain::AnalysisId::from_str("anl_01983c20-0180-7a80-a001-0000000000aa")
            .expect("analysis id")
    }

    fn observation_json(status: &str, completed: Option<&str>) -> serde_json::Value {
        // A terminal `succeeded` observation carries its full canonical
        // result payload; a `running` one carries none.
        let check = if status == "succeeded" {
            serde_json::json!({
                "kind": "ai_detection",
                "status": "succeeded",
                "upstream": {"task_id": "task-123"},
                "result": {
                    "classification": "human",
                    "headline": "Human-written",
                    "prediction": "The document appears to be human-written.",
                    "fraction_ai": 0.0,
                    "fraction_ai_assisted": 0.0,
                    "fraction_human": 1.0,
                    "num_ai_segments": 0,
                    "num_ai_assisted_segments": 0,
                    "num_human_segments": 1,
                    "segments": []
                }
            })
        } else {
            serde_json::json!({
                "kind": "ai_detection",
                "status": status,
                "upstream": {"task_id": "task-123"}
            })
        };
        let mut value = serde_json::json!({
            "id": "anl_01983c20-0180-7a80-a001-0000000000ff",
            "status": status,
            "submission_outcome": "accepted",
            "checks": [check],
            "save_state": "ephemeral",
            "provenance": {
                "provider": "pangram",
                "upstream_task_ids": ["task-123"]
            },
            "created_at": "2026-08-02T00:00:00Z",
            "updated_at": "2026-08-02T12:00:00Z"
        });
        if let Some(stamp) = completed {
            value["completed_at"] = serde_json::Value::String(stamp.to_owned());
        }
        value
    }

    #[test]
    fn observation_merge_preserves_authorship_and_discards_no_local_content() {
        // A running read of a locally authored (terminal-outcome, manual-save)
        // row refreshes only observation fields: the row's original
        // `submission_outcome` stays, and the read carrying no input/search
        // payload never wipes the stored local content. No terminal stamp is
        // fabricated by a running observation.
        let prior = prior_row();
        let observation: Analysis<CanonicalError> =
            serde_json::from_value(observation_json("running", None))
                .expect("a valid running observation");
        let snapshot = observation_merge(&observation, &prior).expect("merge works");
        assert_eq!(snapshot.status, AnalysisStatus::Running);
        assert_eq!(
            snapshot.submission_outcome,
            SubmissionOutcome::Terminal,
            "the stored row's original outcome is never rewritten"
        );
        assert_eq!(snapshot.completed_at, None);
        assert_eq!(
            snapshot.search_input_text.as_deref(),
            Some("the locally submitted text"),
            "the observation's absent input never wipes the stored text"
        );
        assert_eq!(
            snapshot.search_filename.as_deref(),
            Some("original.txt"),
            "the stored filename is kept"
        );
    }

    #[test]
    fn observation_merge_terminal_observation_moves_terminal_fields_only() {
        // A terminal read moves status and completed_at; authorship and
        // save-state-bearing columns are untouched (save_state is not part
        // of the snapshot at all, so the stored row's `saved_manual` stands).
        let prior = prior_row();
        let observation: Analysis<CanonicalError> =
            serde_json::from_value(observation_json("succeeded", Some("2026-08-02T12:00:00Z")))
                .expect("a valid terminal observation");
        let snapshot = observation_merge(&observation, &prior).expect("merge works");
        assert_eq!(snapshot.status, AnalysisStatus::Succeeded);
        assert_eq!(
            snapshot
                .completed_at
                .map(|stamp| stamp.to_string())
                .as_deref(),
            Some("2026-08-02T12:00:00Z")
        );
        assert_eq!(
            snapshot.submission_outcome,
            SubmissionOutcome::Terminal,
            "the observation's `accepted` never rewrites the stored `terminal`"
        );
    }

    #[test]
    fn resumed_read_without_a_descriptor_projects_a_zeroed_text_row() {
        // A `task status` read before any terminal document carries no
        // input (contracts.md 4.6); the persisted row must not fabricate
        // one. Projection itself never fails.
        let analysis_json = serde_json::json!({
            "id": "anl_01983c20-0180-7a80-a001-000000000001",
            "status": "running",
            "submission_outcome": "accepted",
            "checks": [
                {
                    "kind": "ai_detection",
                    "status": "running",
                    "upstream": {"task_id": "task-123"}
                }
            ],
            "save_state": "ephemeral",
            "provenance": {
                "provider": "pangram",
                "upstream_task_ids": ["task-123"]
            },
            "created_at": "2026-08-02T00:00:00Z",
            "updated_at": "2026-08-02T00:00:00Z"
        });
        let analysis: Analysis<CanonicalError> =
            serde_json::from_value(analysis_json).expect("a valid resumed-observation analysis");
        let row = stored_analysis(&analysis, SaveState::SavedHistory).expect("projection works");
        assert_eq!(row.input_kind, InputKind::Text);
        assert_eq!(
            row.input_sha256,
            crate::domain::Sha256Hash::from_bytes([0u8; 32])
        );
        assert!(row.search_input_text.is_none());
        let tasks = stored_observations(&analysis);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].upstream_task_id, "task-123");
    }

    #[test]
    fn mixed_terminal_checks_project_only_the_succeeded_parent_body_in_either_order() {
        let error = serde_json::to_string(
            &CanonicalError::new(
                ErrorCode::UpstreamAnalysisFailed,
                "The synthetic check failed.",
            )
            .expect("canonical error"),
        )
        .expect("serialize error");
        let ai_result = serde_json::json!({
            "classification": "human",
            "headline": "Human",
            "prediction": "Human",
            "fraction_ai": 0.0,
            "fraction_ai_assisted": 0.0,
            "fraction_human": 1.0,
            "num_ai_segments": 0,
            "num_ai_assisted_segments": 0,
            "num_human_segments": 1,
            "segments": []
        })
        .to_string();
        let plagiarism_result = serde_json::json!({
            "plagiarism_detected": false,
            "total_sentences": 1,
            "plagiarized_sentence_count": 0,
            "percent_plagiarized": 0.0,
            "matches": []
        })
        .to_string();

        let failed_then_succeeded = [
            StoredCheck {
                analysis_id: analysis_id(),
                check_index: 0,
                check_kind: CheckKind::AiDetection,
                status: CheckStatus::Failed,
                result_json: None,
                error_json: Some(error.clone()),
            },
            StoredCheck {
                analysis_id: analysis_id(),
                check_index: 1,
                check_kind: CheckKind::Plagiarism,
                status: CheckStatus::Succeeded,
                result_json: Some(plagiarism_result),
                error_json: None,
            },
        ];
        let projection =
            project_stored_checks(&failed_then_succeeded).expect("project mixed checks");
        assert!(projection.result_json.is_some());
        assert!(
            projection.error_json.is_none(),
            "a later success must clear the earlier failed-check body"
        );

        let succeeded_then_failed = [
            StoredCheck {
                analysis_id: analysis_id(),
                check_index: 0,
                check_kind: CheckKind::AiDetection,
                status: CheckStatus::Succeeded,
                result_json: Some(ai_result),
                error_json: None,
            },
            StoredCheck {
                analysis_id: analysis_id(),
                check_index: 1,
                check_kind: CheckKind::Plagiarism,
                status: CheckStatus::Failed,
                result_json: None,
                error_json: Some(error),
            },
        ];
        let projection =
            project_stored_checks(&succeeded_then_failed).expect("project mixed checks");
        assert!(projection.result_json.is_some());
        assert!(
            projection.error_json.is_none(),
            "a later failed check must not add a second parent body"
        );
    }
}
