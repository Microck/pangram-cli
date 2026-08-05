//! Standalone task reconciliation preserves every task identity already
//! owned by the selected durable row.
//!
//! No mocks: each case uses the real SQLite HistoryStore, and concurrency
//! uses two independent connections over one WAL database.

#![forbid(unsafe_code)]

use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, CheckKind, CheckStatus, SaveState, Sha256Hash, SubmissionOutcome,
    UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryError, HistoryErrorCode, HistoryStore, InputKind, ObservationSnapshot, StoredAnalysis,
    StoredCheck, StoredUpstreamTask,
};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("test timestamp")
}

fn analysis(id: &str) -> StoredAnalysis {
    let input_sha256 = Sha256Hash::digest("evidence");
    StoredAnalysis {
        id: AnalysisId::from_str(id).expect("analysis id"),
        bulk: None,
        caller_id: None,
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        save_state: SaveState::SavedHistory,
        input_kind: InputKind::Text,
        input_sha256,
        display_name: None,
        input_json: serde_json::json!({
            "type": "text",
            "origin": "literal",
            "sha256": input_sha256,
            "byte_count": 8,
            "word_count": 1,
            "text": "evidence"
        })
        .to_string(),
        result_json: Some(ai_result()),
        error_json: None,
        upstream_version: None,
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(timestamp("2026-08-02T09:59:00Z")),
        created_at: timestamp("2026-08-02T10:00:00Z"),
        updated_at: timestamp("2026-08-02T10:01:00Z"),
        completed_at: Some(timestamp("2026-08-02T10:01:00Z")),
        search_input_text: Some("evidence".to_owned()),
        search_filename: None,
        search_headline: Some("Human-written".to_owned()),
        search_source_urls: None,
    }
}

fn ai_result() -> String {
    serde_json::json!({
        "classification": "human",
        "headline": "Human-written",
        "prediction": "Human",
        "fraction_ai": 0.0,
        "fraction_ai_assisted": 0.0,
        "fraction_human": 1.0,
        "num_ai_segments": 0,
        "num_ai_assisted_segments": 0,
        "num_human_segments": 1,
        "segments": []
    })
    .to_string()
}

fn checks(id: &str) -> Vec<StoredCheck> {
    let analysis_id = AnalysisId::from_str(id).expect("analysis id");
    vec![
        StoredCheck {
            analysis_id,
            check_index: 0,
            check_kind: CheckKind::AiDetection,
            status: CheckStatus::Succeeded,
            result_json: Some(ai_result()),
            error_json: None,
        },
        StoredCheck {
            analysis_id,
            check_index: 1,
            check_kind: CheckKind::Plagiarism,
            status: CheckStatus::Succeeded,
            result_json: Some(
                serde_json::json!({
                    "plagiarism_detected": false,
                    "total_sentences": 1,
                    "plagiarized_sentence_count": 0,
                    "percent_plagiarized": 0.0,
                    "matches": []
                })
                .to_string(),
            ),
            error_json: None,
        },
    ]
}

fn task(
    analysis_id: &str,
    kind: CheckKind,
    upstream_task_id: &str,
    stage: &str,
) -> StoredUpstreamTask {
    StoredUpstreamTask {
        analysis_id: AnalysisId::from_str(analysis_id).expect("analysis id"),
        check_kind: kind,
        upstream_task_id: upstream_task_id.to_owned(),
        last_stage: Some(stage.to_owned()),
        observed_at: timestamp("2026-08-02T10:01:00Z"),
    }
}

fn running_merge(_: &StoredAnalysis) -> Result<ObservationSnapshot, HistoryError> {
    Ok(ObservationSnapshot {
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        result_json: None,
        error_json: None,
        upstream_version: None,
        completed_at: None,
        search_input_text: None,
        search_filename: None,
        search_headline: None,
        search_source_urls: None,
    })
}

fn database(root: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(root.path().join("history").join("pangram-history.db"))
        .expect("open database")
}

fn stored_tasks(connection: &rusqlite::Connection) -> Vec<(String, String, Option<String>)> {
    connection
        .prepare(
            "SELECT check_kind, upstream_task_id, last_stage
             FROM upstream_tasks ORDER BY check_kind",
        )
        .expect("prepare task read")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query task read")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect task read")
}

#[test]
fn selection_by_one_key_cannot_replace_another_owned_kind() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open store");
    let stored_id = "anl_01983c20-0180-7a80-a001-00000000f101";
    store
        .save_analysis_complete(
            &analysis(stored_id),
            &checks(stored_id),
            &[
                task(stored_id, CheckKind::AiDetection, "ai-original", "AI_DONE"),
                task(
                    stored_id,
                    CheckKind::Plagiarism,
                    "plag-selector",
                    "PLAG_DONE",
                ),
            ],
        )
        .expect("seed evidence");

    let fresh_id = "anl_01983c20-0180-7a80-a001-00000000f102";
    let error = store
        .reconcile_observed_analysis_atomic(
            &analysis(fresh_id),
            &[
                task(
                    fresh_id,
                    CheckKind::Plagiarism,
                    "plag-selector",
                    "PLAG_REFRESH",
                ),
                task(
                    fresh_id,
                    CheckKind::AiDetection,
                    "ai-replacement",
                    "AI_REFRESH",
                ),
            ],
            timestamp("2026-08-02T10:02:00Z"),
            running_merge,
        )
        .expect_err("replacement evidence must fail closed");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    let connection = database(&root);
    assert_eq!(
        stored_tasks(&connection),
        vec![
            (
                "ai_detection".to_owned(),
                "ai-original".to_owned(),
                Some("AI_DONE".to_owned()),
            ),
            (
                "plagiarism".to_owned(),
                "plag-selector".to_owned(),
                Some("PLAG_DONE".to_owned()),
            ),
        ],
        "the whole refresh rolled back and original evidence survived"
    );
    let status: String = connection
        .query_row("SELECT status FROM analyses", [], |row| row.get(0))
        .expect("stored status");
    assert_eq!(status, "succeeded", "snapshot mutation also rolled back");
}

#[test]
fn selected_row_allows_add_same_and_omitted_kinds() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open store");
    let stored_id = "anl_01983c20-0180-7a80-a001-00000000f111";
    store
        .save_analysis_complete(
            &analysis(stored_id),
            &checks(stored_id),
            &[task(
                stored_id,
                CheckKind::AiDetection,
                "ai-same",
                "AI_SEED",
            )],
        )
        .expect("seed evidence");

    let add_id = "anl_01983c20-0180-7a80-a001-00000000f112";
    store
        .reconcile_observed_analysis_atomic(
            &analysis(add_id),
            &[
                task(add_id, CheckKind::AiDetection, "ai-same", "AI_REFRESH"),
                task(add_id, CheckKind::Plagiarism, "plag-added", "PLAG_ADDED"),
            ],
            timestamp("2026-08-02T10:02:00Z"),
            running_merge,
        )
        .expect("same key refresh and missing kind add are allowed");

    let same_only_id = "anl_01983c20-0180-7a80-a001-00000000f113";
    store
        .reconcile_observed_analysis_atomic(
            &analysis(same_only_id),
            &[task(
                same_only_id,
                CheckKind::Plagiarism,
                "plag-added",
                "PLAG_REFRESH",
            )],
            timestamp("2026-08-02T10:03:00Z"),
            running_merge,
        )
        .expect("same key refresh is allowed");

    let connection = database(&root);
    assert_eq!(
        stored_tasks(&connection),
        vec![
            (
                "ai_detection".to_owned(),
                "ai-same".to_owned(),
                Some("AI_REFRESH".to_owned()),
            ),
            (
                "plagiarism".to_owned(),
                "plag-added".to_owned(),
                Some("PLAG_REFRESH".to_owned()),
            ),
        ],
        "the omitted AI kind remains while the supplied same key refreshes"
    );
}

#[test]
fn concurrent_same_and_replacement_refreshes_preserve_owned_evidence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().to_path_buf();
    let stored_id = "anl_01983c20-0180-7a80-a001-00000000f121";
    let mut seed = HistoryStore::open(&path).expect("open seed store");
    seed.save_analysis_complete(
        &analysis(stored_id),
        &checks(stored_id),
        &[
            task(stored_id, CheckKind::AiDetection, "ai-owned", "AI_SEED"),
            task(stored_id, CheckKind::Plagiarism, "plag-common", "PLAG_SEED"),
        ],
    )
    .expect("seed evidence");
    drop(seed);

    let store_same = HistoryStore::open(&path).expect("open same store");
    let store_replace = HistoryStore::open(&path).expect("open replacement store");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

    let same_barrier = barrier.clone();
    let same = std::thread::spawn(move || {
        let mut store = store_same;
        let id = "anl_01983c20-0180-7a80-a001-00000000f122";
        same_barrier.wait();
        store.reconcile_observed_analysis_atomic(
            &analysis(id),
            &[task(id, CheckKind::Plagiarism, "plag-common", "PLAG_SAME")],
            timestamp("2026-08-02T10:02:00Z"),
            running_merge,
        )
    });
    let replacement = std::thread::spawn(move || {
        let mut store = store_replace;
        let id = "anl_01983c20-0180-7a80-a001-00000000f123";
        barrier.wait();
        store.reconcile_observed_analysis_atomic(
            &analysis(id),
            &[
                task(id, CheckKind::Plagiarism, "plag-common", "PLAG_REPLACE"),
                task(id, CheckKind::AiDetection, "ai-forbidden", "AI_REPLACE"),
            ],
            timestamp("2026-08-02T10:02:00Z"),
            running_merge,
        )
    });

    same.join()
        .expect("same thread joins")
        .expect("same-key refresh commits");
    let error = replacement
        .join()
        .expect("replacement thread joins")
        .expect_err("replacement fails closed after serialized comparison");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    let connection = database(&root);
    assert_eq!(
        stored_tasks(&connection),
        vec![
            (
                "ai_detection".to_owned(),
                "ai-owned".to_owned(),
                Some("AI_SEED".to_owned()),
            ),
            (
                "plagiarism".to_owned(),
                "plag-common".to_owned(),
                Some("PLAG_SAME".to_owned()),
            ),
        ]
    );
}
