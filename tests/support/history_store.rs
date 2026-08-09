#![allow(dead_code)]

use microck_pangram_cli::domain::{AnalysisStatus, CheckKind, CheckStatus};
use microck_pangram_cli::history::{HistoryStore, StoredAnalysis, StoredCheck, StoredUpstreamTask};
use microck_pangram_cli::output::{CanonicalError, ErrorCode};

pub(crate) fn save_complete(store: &mut HistoryStore, record: &StoredAnalysis) {
    let check = complete_check(record);
    store
        .save_analysis_complete(record, &[check], &[])
        .expect("save complete analysis fixture");
}

pub(crate) fn prepared_child(
    record: StoredAnalysis,
) -> (StoredAnalysis, Vec<StoredCheck>, Vec<StoredUpstreamTask>) {
    let check = complete_check(&record);
    (record, vec![check], Vec::new())
}

fn complete_check(record: &StoredAnalysis) -> StoredCheck {
    let (status, result_json, error_json) = match record.status {
        AnalysisStatus::Queued => (CheckStatus::Queued, None, None),
        AnalysisStatus::Running => (CheckStatus::Running, None, None),
        AnalysisStatus::Succeeded => (CheckStatus::Succeeded, record.result_json.clone(), None),
        AnalysisStatus::Failed => (CheckStatus::Failed, None, record.error_json.clone()),
        _ => panic!("fixture helper supports queued, running, succeeded, or failed rows"),
    };
    StoredCheck {
        analysis_id: record.id,
        check_index: 0,
        check_kind: CheckKind::AiDetection,
        status,
        result_json,
        error_json,
    }
}

pub(crate) fn ai_result(headline: &str) -> String {
    serde_json::json!({
        "classification": "human",
        "headline": headline,
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

pub(crate) fn canonical_error(code: ErrorCode, message: &str) -> String {
    serde_json::to_string(&CanonicalError::new(code, message).expect("canonical fixture error"))
        .expect("serialize canonical fixture error")
}
