//! Authoritative multi-check persistence and corruption handling on real SQLite.

use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisInput, AnalysisStatus, CheckKind, CheckStatus, OrderedCheck, SaveState,
    Sha256Hash, SubmissionOutcome, TextInput, TextOrigin, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryErrorCode, HistoryStore, InputKind, ObservationSnapshot, StoredAnalysis, StoredCheck,
    StoredUpstreamTask, TerminalResult,
};
use microck_pangram_cli::output::{CanonicalError, ErrorCode};

#[test]
fn complete_rows_reconstruct_filter_export_rollback_and_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let id = AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a10").expect("analysis id");
    let created_at = UtcTimestamp::from_str("2026-08-01T10:00:00Z").expect("timestamp");
    let updated_at = UtcTimestamp::from_str("2026-08-01T10:05:00Z").expect("timestamp");
    let text = "multi check retained text";
    let input_sha256 = Sha256Hash::digest(text);
    let input = AnalysisInput::Text(
        TextInput::new(
            TextOrigin::Literal,
            None,
            input_sha256,
            u64::try_from(text.len()).unwrap(),
            4,
            Some(text.to_owned()),
        )
        .expect("text input"),
    );
    let error = serde_json::to_string(
        &CanonicalError::new(
            ErrorCode::UpstreamAnalysisFailed,
            "The synthetic analysis failed.",
        )
        .expect("canonical error"),
    )
    .expect("serialize canonical error");
    let record = StoredAnalysis {
        id,
        bulk: None,
        caller_id: None,
        status: AnalysisStatus::Failed,
        submission_outcome: SubmissionOutcome::Terminal,
        save_state: SaveState::SavedManual,
        input_kind: InputKind::Text,
        input_sha256,
        display_name: None,
        input_json: serde_json::to_string(&input).expect("serialize input"),
        result_json: None,
        error_json: Some(error.clone()),
        upstream_version: Some("4.0".to_owned()),
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(UtcTimestamp::from_str("2026-08-01T09:59:00Z").unwrap()),
        created_at,
        updated_at,
        completed_at: Some(updated_at),
        search_input_text: Some(text.to_owned()),
        search_filename: None,
        search_headline: None,
        search_source_urls: None,
    };
    let checks = [CheckKind::AiDetection, CheckKind::Plagiarism]
        .into_iter()
        .enumerate()
        .map(|(index, kind)| StoredCheck {
            analysis_id: id,
            check_index: u8::try_from(index).unwrap(),
            check_kind: kind,
            status: CheckStatus::Failed,
            result_json: None,
            error_json: Some(error.clone()),
        })
        .collect::<Vec<_>>();
    let observations = [
        (CheckKind::AiDetection, "task-multi-ai"),
        (CheckKind::Plagiarism, "task-multi-plagiarism"),
    ]
    .into_iter()
    .map(|(kind, task_id)| StoredUpstreamTask {
        analysis_id: id,
        check_kind: kind,
        upstream_task_id: task_id.to_owned(),
        last_stage: Some("STAGE_FAILED".to_owned()),
        observed_at: updated_at,
    })
    .collect::<Vec<_>>();

    let mut malformed = checks.clone();
    malformed[1].check_index = 0;
    assert_eq!(
        store
            .save_analysis_complete(&record, &malformed, &observations)
            .expect_err("invalid check order must roll back")
            .code(),
        HistoryErrorCode::HistoryWriteFailed
    );
    store
        .save_analysis_complete(&record, &checks, &observations)
        .expect("save after rollback");

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
    let succeeded_checks = vec![
        StoredCheck {
            analysis_id: id,
            check_index: 0,
            check_kind: CheckKind::AiDetection,
            status: CheckStatus::Succeeded,
            result_json: Some(ai_result.clone()),
            error_json: None,
        },
        StoredCheck {
            analysis_id: id,
            check_index: 1,
            check_kind: CheckKind::Plagiarism,
            status: CheckStatus::Succeeded,
            result_json: Some(plagiarism_result),
            error_json: None,
        },
    ];
    store
        .update_terminal_result_complete(
            &id,
            &TerminalResult {
                status: AnalysisStatus::Succeeded,
                submission_outcome: SubmissionOutcome::Terminal,
                result_json: Some(ai_result),
                error_json: None,
                upstream_version: Some("4.1".to_owned()),
                completed_at: updated_at,
                search_input_text: Some(text.to_owned()),
                search_filename: None,
                search_headline: Some("Human".to_owned()),
                search_source_urls: None,
            },
            &succeeded_checks,
        )
        .expect("terminal parent/check/FTS update");

    let reconstructed = store
        .canonical_analysis(&id, true)
        .expect("reconstruct multi-check analysis");
    assert_eq!(
        reconstructed
            .checks()
            .iter()
            .map(OrderedCheck::check_kind)
            .collect::<Vec<_>>(),
        vec![CheckKind::AiDetection, CheckKind::Plagiarism]
    );
    assert_eq!(reconstructed.status(), AnalysisStatus::Succeeded);
    assert_eq!(
        store
            .list_filtered(None, Some(CheckKind::Plagiarism), 10, 0)
            .expect("filter plagiarism")
            .len(),
        1
    );
    let exported = store.export_analyses(false).expect("export multi-check");
    assert_eq!(exported[0]["checks"].as_array().unwrap().len(), 2);

    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analyses SET status = 'failed' WHERE id = ?1",
                [id.to_string()],
            )
        })
        .expect("open database")
        .expect("install parent/check status disagreement");
    assert_eq!(
        store
            .list(10, 0)
            .expect_err("list must validate the authoritative check rows")
            .code(),
        HistoryErrorCode::HistoryCorrupt
    );
    assert_eq!(
        store
            .search("multi", 10)
            .expect_err("search must validate the authoritative check rows")
            .code(),
        HistoryErrorCode::HistoryCorrupt
    );
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analyses SET status = 'succeeded' WHERE id = ?1",
                [id.to_string()],
            )
        })
        .expect("open database")
        .expect("restore parent status");

    store
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO upstream_tasks
                    (analysis_id, check_kind, upstream_task_id, last_stage, observed_at)
                 VALUES (?1, 'unknown_check', 'task-unmatched', NULL, ?2)",
                [id.to_string(), updated_at.to_string()],
            )
        })
        .expect("open database")
        .expect("insert unmatched task evidence");
    assert_eq!(
        store
            .canonical_analysis(&id, true)
            .expect_err("unmatched task evidence must fail closed")
            .code(),
        HistoryErrorCode::HistoryCorrupt
    );
    assert_eq!(
        store
            .export_analyses(false)
            .expect_err("export must not silently omit unmatched evidence")
            .code(),
        HistoryErrorCode::HistoryCorrupt
    );
    store
        .with_connection(|connection| {
            connection.execute(
                "DELETE FROM upstream_tasks
                 WHERE analysis_id = ?1 AND check_kind = 'unknown_check'",
                [id.to_string()],
            )
        })
        .expect("open database")
        .expect("restore valid evidence");

    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analysis_checks SET result_json = '{'
                 WHERE analysis_id = ?1 AND check_index = 1",
                [id.to_string()],
            )
        })
        .expect("open database")
        .expect("corrupt one authoritative check");
    assert_eq!(
        store
            .list(10, 0)
            .expect_err("list must reject a malformed check result body")
            .code(),
        HistoryErrorCode::HistoryCorrupt
    );
    assert_eq!(
        store
            .search("multi", 10)
            .expect_err("search must reject a malformed check result body")
            .code(),
        HistoryErrorCode::HistoryCorrupt
    );
    let before_status = store.get_analysis(&id).unwrap().status;
    let error = store
        .update_terminal_result_complete(
            &id,
            &TerminalResult {
                status: AnalysisStatus::Failed,
                submission_outcome: SubmissionOutcome::Terminal,
                result_json: None,
                error_json: Some(error),
                upstream_version: Some("4.2".to_owned()),
                completed_at: updated_at,
                search_input_text: Some("must not replace".to_owned()),
                search_filename: None,
                search_headline: None,
                search_source_urls: None,
            },
            &checks,
        )
        .expect_err("malformed stored checks must roll back");
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    assert_eq!(store.get_analysis(&id).unwrap().status, before_status);
    assert_eq!(
        store
            .canonical_analysis(&id, true)
            .expect_err("missing check must fail closed")
            .code(),
        HistoryErrorCode::HistoryCorrupt
    );
    assert_eq!(
        store
            .export_analyses(false)
            .expect_err("corrupt export must fail closed")
            .code(),
        HistoryErrorCode::HistoryCorrupt
    );
}

#[test]
fn standalone_observations_merge_only_the_matching_check() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let id = AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a11").unwrap();
    let created_at = UtcTimestamp::from_str("2026-08-01T10:00:00Z").unwrap();
    let observed_at = UtcTimestamp::from_str("2026-08-01T10:05:00Z").unwrap();
    let plagiarism_result = serde_json::json!({
        "plagiarism_detected": false,
        "total_sentences": 1,
        "plagiarized_sentence_count": 0,
        "percent_plagiarized": 0.0,
        "matches": []
    })
    .to_string();
    let record = StoredAnalysis {
        id,
        bulk: None,
        caller_id: Some("retained-caller".to_owned()),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        save_state: SaveState::SavedManual,
        input_kind: InputKind::Text,
        input_sha256: Sha256Hash::digest("retained combined input"),
        display_name: None,
        input_json: serde_json::json!({
            "type": "text",
            "origin": "literal",
            "sha256": Sha256Hash::digest("retained combined input"),
            "byte_count": 23,
            "word_count": 3,
            "text": "retained combined input"
        })
        .to_string(),
        result_json: Some(plagiarism_result.clone()),
        error_json: None,
        upstream_version: Some("4.0".to_owned()),
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(created_at),
        created_at,
        updated_at: created_at,
        completed_at: None,
        search_input_text: Some("retained combined input".to_owned()),
        search_filename: None,
        search_headline: None,
        search_source_urls: Some("https://example.invalid/retained".to_owned()),
    };
    let checks = vec![
        StoredCheck {
            analysis_id: id,
            check_index: 0,
            check_kind: CheckKind::AiDetection,
            status: CheckStatus::Running,
            result_json: None,
            error_json: None,
        },
        StoredCheck {
            analysis_id: id,
            check_index: 1,
            check_kind: CheckKind::Plagiarism,
            status: CheckStatus::Succeeded,
            result_json: Some(plagiarism_result.clone()),
            error_json: None,
        },
    ];
    let observations = vec![
        StoredUpstreamTask {
            analysis_id: id,
            check_kind: CheckKind::AiDetection,
            upstream_task_id: "task-combined-ai".to_owned(),
            last_stage: Some("STAGE_INFERENCE".to_owned()),
            observed_at: created_at,
        },
        StoredUpstreamTask {
            analysis_id: id,
            check_kind: CheckKind::Plagiarism,
            upstream_task_id: "task-combined-plagiarism".to_owned(),
            last_stage: Some("STAGE_COMPLETE".to_owned()),
            observed_at: created_at,
        },
    ];
    store
        .save_analysis_complete(&record, &checks, &observations)
        .expect("seed combined row");

    let ai_result = serde_json::json!({
        "classification": "human",
        "headline": "Retained headline",
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
    let incoming = StoredAnalysis {
        id: AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8aff").unwrap(),
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Accepted,
        save_state: SaveState::SavedHistory,
        input_sha256: Sha256Hash::from_bytes([0; 32]),
        input_json: "null".to_owned(),
        result_json: Some(ai_result.clone()),
        upstream_version: Some("4.1".to_owned()),
        created_at: observed_at,
        updated_at: observed_at,
        completed_at: Some(observed_at),
        search_headline: Some("Retained headline".to_owned()),
        ..record.clone()
    };
    let incoming_check = StoredCheck {
        analysis_id: incoming.id,
        check_index: 0,
        check_kind: CheckKind::AiDetection,
        status: CheckStatus::Succeeded,
        result_json: Some(ai_result),
        error_json: None,
    };
    let incoming_task = StoredUpstreamTask {
        analysis_id: incoming.id,
        check_kind: CheckKind::AiDetection,
        upstream_task_id: "task-combined-ai".to_owned(),
        last_stage: Some("STAGE_COMPLETE".to_owned()),
        observed_at,
    };
    store
        .reconcile_observed_analysis_complete(
            &incoming,
            &[incoming_check],
            &[incoming_task],
            observed_at,
            |prior| {
                Ok(ObservationSnapshot {
                    status: incoming.status,
                    submission_outcome: prior.submission_outcome,
                    result_json: incoming.result_json.clone(),
                    error_json: None,
                    upstream_version: incoming.upstream_version.clone(),
                    completed_at: incoming.completed_at,
                    search_input_text: prior.search_input_text.clone(),
                    search_filename: prior.search_filename.clone(),
                    search_headline: incoming.search_headline.clone(),
                    search_source_urls: prior.search_source_urls.clone(),
                })
            },
        )
        .expect("merge one authoritative observation");

    let canonical = store
        .canonical_analysis(&id, true)
        .expect("combined row remains readable");
    assert_eq!(canonical.status(), AnalysisStatus::Succeeded);
    assert_eq!(canonical.checks().len(), 2);
    assert_eq!(
        canonical
            .checks()
            .iter()
            .map(OrderedCheck::check_kind)
            .collect::<Vec<_>>(),
        vec![CheckKind::AiDetection, CheckKind::Plagiarism]
    );
    let canonical_value = serde_json::to_value(&canonical).expect("canonical JSON");
    assert_eq!(
        canonical_value["checks"][1]["result"],
        serde_json::from_str::<serde_json::Value>(&plagiarism_result).unwrap(),
        "the omitted plagiarism body survives byte-for-byte reconstruction"
    );
    assert_eq!(
        store
            .list_filtered(None, Some(CheckKind::Plagiarism), 10, 0)
            .expect("list remains readable")
            .len(),
        1
    );
    assert_eq!(
        store
            .search("retained", 10)
            .expect("FTS remains readable")
            .len(),
        1
    );
    let exported = store
        .export_analyses(false)
        .expect("export remains readable");
    assert_eq!(exported[0]["checks"].as_array().unwrap().len(), 2);
    assert_eq!(
        exported[0]["input"]["text"], "retained combined input",
        "input survives one-check reconciliation"
    );
    let after = store.get_analysis(&id).expect("stored parent");
    assert_eq!(after.save_state, SaveState::SavedManual);
    assert_eq!(after.caller_id.as_deref(), Some("retained-caller"));
    assert_eq!(after.retry_of, record.retry_of);
    assert_eq!(after.submitted_at, record.submitted_at);
    assert_eq!(after.upstream_version.as_deref(), Some("4.1"));
    let tasks: Vec<(String, String, Option<String>)> = store
        .with_connection(|connection| {
            connection
                .prepare(
                    "SELECT check_kind, upstream_task_id, last_stage
                     FROM upstream_tasks WHERE analysis_id = ?1 ORDER BY check_kind",
                )?
                .query_map([id.to_string()], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect::<Result<Vec<_>, _>>()
        })
        .expect("borrow database")
        .expect("read retained task evidence");
    let search: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = store
        .with_connection(|connection| {
            connection.query_row(
                "SELECT input_text, filename, headline, source_urls
                 FROM analysis_search WHERE analysis_id = ?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
        })
        .expect("borrow database")
        .expect("read retained search evidence");
    assert_eq!(
        tasks,
        vec![
            (
                "ai_detection".to_owned(),
                "task-combined-ai".to_owned(),
                Some("STAGE_COMPLETE".to_owned())
            ),
            (
                "plagiarism".to_owned(),
                "task-combined-plagiarism".to_owned(),
                Some("STAGE_COMPLETE".to_owned())
            ),
        ],
        "the refreshed task and omitted task evidence both survive"
    );
    assert_eq!(
        search,
        (
            Some("retained combined input".to_owned()),
            None,
            Some("Retained headline".to_owned()),
            Some("https://example.invalid/retained".to_owned()),
        ),
        "FTS input metadata is retained while result-derived headline advances"
    );
}

#[test]
fn corrupt_combined_rows_roll_back_a_standalone_refresh_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open history store");
    let id = AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a12").unwrap();
    let created_at = UtcTimestamp::from_str("2026-08-01T10:00:00Z").unwrap();
    let observed_at = UtcTimestamp::from_str("2026-08-01T10:05:00Z").unwrap();
    let text = "rollback combined input";
    let plagiarism_result = serde_json::json!({
        "plagiarism_detected": false,
        "total_sentences": 1,
        "plagiarized_sentence_count": 0,
        "percent_plagiarized": 0.0,
        "matches": []
    })
    .to_string();
    let record = StoredAnalysis {
        id,
        bulk: None,
        caller_id: None,
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        save_state: SaveState::SavedHistory,
        input_kind: InputKind::Text,
        input_sha256: Sha256Hash::digest(text),
        display_name: None,
        input_json: serde_json::json!({
            "type": "text",
            "origin": "literal",
            "sha256": Sha256Hash::digest(text),
            "byte_count": text.len(),
            "word_count": 3,
            "text": text
        })
        .to_string(),
        result_json: Some(plagiarism_result.clone()),
        error_json: None,
        upstream_version: Some("4.0".to_owned()),
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(created_at),
        created_at,
        updated_at: created_at,
        completed_at: None,
        search_input_text: Some(text.to_owned()),
        search_filename: None,
        search_headline: None,
        search_source_urls: Some("https://example.invalid/original".to_owned()),
    };
    let checks = vec![
        StoredCheck {
            analysis_id: id,
            check_index: 0,
            check_kind: CheckKind::AiDetection,
            status: CheckStatus::Running,
            result_json: None,
            error_json: None,
        },
        StoredCheck {
            analysis_id: id,
            check_index: 1,
            check_kind: CheckKind::Plagiarism,
            status: CheckStatus::Succeeded,
            result_json: Some(plagiarism_result),
            error_json: None,
        },
    ];
    let observations = vec![
        StoredUpstreamTask {
            analysis_id: id,
            check_kind: CheckKind::AiDetection,
            upstream_task_id: "task-rollback-ai".to_owned(),
            last_stage: Some("STAGE_INFERENCE".to_owned()),
            observed_at: created_at,
        },
        StoredUpstreamTask {
            analysis_id: id,
            check_kind: CheckKind::Plagiarism,
            upstream_task_id: "task-rollback-plagiarism".to_owned(),
            last_stage: Some("STAGE_COMPLETE".to_owned()),
            observed_at: created_at,
        },
    ];
    store
        .save_analysis_complete(&record, &checks, &observations)
        .expect("seed combined row");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analysis_checks SET result_json = '{'
                 WHERE analysis_id = ?1 AND check_index = 1",
                [id.to_string()],
            )
        })
        .expect("borrow database")
        .expect("install malformed omitted check");

    let before = raw_reconcile_state(&store, id);
    let incoming_id = AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8aff").unwrap();
    let incoming_check = StoredCheck {
        analysis_id: incoming_id,
        check_index: 0,
        check_kind: CheckKind::AiDetection,
        status: CheckStatus::Succeeded,
        result_json: Some("{\"headline\":\"must roll back\"}".to_owned()),
        error_json: None,
    };
    let incoming_task = StoredUpstreamTask {
        analysis_id: incoming_id,
        check_kind: CheckKind::AiDetection,
        upstream_task_id: "task-rollback-ai".to_owned(),
        last_stage: Some("STAGE_COMPLETE".to_owned()),
        observed_at,
    };
    let error = store
        .reconcile_observed_analysis_complete(
            &StoredAnalysis {
                id: incoming_id,
                updated_at: observed_at,
                ..record
            },
            &[incoming_check],
            &[incoming_task],
            observed_at,
            |_| panic!("merge closure must not run over corrupt stored checks"),
        )
        .expect_err("corruption fails before any refresh");
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    assert_eq!(
        raw_reconcile_state(&store, id),
        before,
        "parent, checks, tasks, and FTS all roll back unchanged"
    );
}

fn raw_reconcile_state(store: &HistoryStore, id: AnalysisId) -> Vec<String> {
    store
        .with_connection(|connection| {
            let mut rows = Vec::new();
            for sql in [
                "SELECT status || '|' || updated_at || '|' ||
                        COALESCE(result_json, '') || '|' || COALESCE(error_json, '')
                 FROM analyses WHERE id = ?1",
                "SELECT check_index || '|' || check_kind || '|' || status || '|' ||
                        COALESCE(result_json, '') || '|' || COALESCE(error_json, '')
                 FROM analysis_checks WHERE analysis_id = ?1 ORDER BY check_index",
                "SELECT check_kind || '|' || upstream_task_id || '|' ||
                        COALESCE(last_stage, '') || '|' || observed_at
                 FROM upstream_tasks WHERE analysis_id = ?1 ORDER BY check_kind",
                "SELECT COALESCE(input_text, '') || '|' || COALESCE(filename, '') || '|' ||
                        COALESCE(headline, '') || '|' || COALESCE(source_urls, '')
                 FROM analysis_search WHERE analysis_id = ?1",
            ] {
                let mut statement = connection.prepare(sql)?;
                let values = statement
                    .query_map([id.to_string()], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                rows.extend(values);
            }
            Ok::<_, rusqlite::Error>(rows)
        })
        .expect("borrow database")
        .expect("read raw reconcile state")
}
