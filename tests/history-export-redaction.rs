//! Redacted history export privacy and schema-validity regressions.

use std::process::{Command, Output};

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, SaveState, Sha256Hash, SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{HistoryStore, InputKind, StoredAnalysis};
use serde_json::Value;

struct Env(tempfile::TempDir);

impl Env {
    fn new() -> Self {
        Self(tempfile::tempdir().expect("temporary root"))
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_pangram"));
        command
            .env_clear()
            .env("HOME", self.0.path())
            .env("XDG_CONFIG_HOME", self.0.path().join("config"))
            .env("XDG_DATA_HOME", self.0.path().join("data"))
            .env("PANGRAM_DATA_DIR", self.0.path())
            .env("CI", "true")
            .env("TERM", "dumb");
        command
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.command()
            .args(arguments)
            .output()
            .expect("run pangram")
    }

    fn seed(&self, id: &str, created_at: &str, text: &str) {
        let timestamp = created_at.parse::<UtcTimestamp>().expect("timestamp");
        let mut store = HistoryStore::open(self.0.path()).expect("open history");
        store
            .save_analysis(&StoredAnalysis {
                id: id.parse::<AnalysisId>().expect("analysis id"),
                bulk: None,
                caller_id: None,
                status: AnalysisStatus::Succeeded,
                submission_outcome: SubmissionOutcome::Terminal,
                save_state: SaveState::SavedManual,
                input_kind: InputKind::Text,
                input_sha256: Sha256Hash::digest(text),
                display_name: None,
                input_json: serde_json::json!({
                    "type": "text",
                    "origin": "literal",
                    "sha256": Sha256Hash::digest(text).to_string(),
                    "byte_count": text.len(),
                    "word_count": text.split_whitespace().count(),
                    "text": text
                })
                .to_string(),
                result_json: Some(valid_ai_result()),
                error_json: None,
                upstream_version: Some("4.0".to_owned()),
                retry_of: None,
                rerun_of: None,
                submitted_at: Some(timestamp),
                created_at: timestamp,
                updated_at: timestamp,
                completed_at: Some(timestamp),
                search_input_text: Some(text.to_owned()),
                search_filename: None,
                search_headline: Some("Human".to_owned()),
                search_source_urls: None,
            })
            .expect("seed analysis");
    }
}

fn valid_ai_result() -> String {
    serde_json::json!({
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
    .to_string()
}

#[test]
fn redacted_export_keeps_segment_evidence_and_only_safe_source_hostnames() {
    let env = Env::new();
    let ai = "anl_01983c20-0180-7a80-a001-000000000041";
    let plagiarism = "anl_01983c20-0180-7a80-a001-000000000042";
    env.seed(ai, "2026-08-05T10:00:00Z", "private segment source");
    env.seed(
        plagiarism,
        "2026-08-05T11:00:00Z",
        "private plagiarism source",
    );
    let store = HistoryStore::open(env.0.path()).expect("open real history");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analysis_checks SET result_json = ?1 WHERE analysis_id = ?2",
                rusqlite::params![
                    serde_json::json!({
                        "classification": "mixed",
                        "headline": "Mixed",
                        "prediction": "Mixed",
                        "fraction_ai": 0.5,
                        "fraction_ai_assisted": 0.25,
                        "fraction_human": 0.25,
                        "num_ai_segments": 1,
                        "num_ai_assisted_segments": 0,
                        "num_human_segments": 0,
                        "segments": [{
                            "text": "secret segment",
                            "label": "AI",
                            "ai_assistance_score": 0.8,
                            "confidence": "high",
                            "start_index": 4,
                            "end_index": 18,
                            "word_count": 2,
                            "token_length": 3,
                            "humanizer_score": 0.2,
                            "is_humanized": false
                        }],
                        "dashboard_link": "https://dashboard.example/private"
                    })
                    .to_string(),
                    ai
                ],
            )?;
            connection.execute(
                "UPDATE analysis_checks
                 SET check_kind = 'plagiarism', result_json = ?1
                 WHERE analysis_id = ?2",
                rusqlite::params![
                    serde_json::json!({
                        "plagiarism_detected": true,
                        "total_sentences": 6,
                        "plagiarized_sentence_count": 5,
                        "percent_plagiarized": 83.3,
                        "matches": [
                            {"source_url": "https://user:pass@Example.COM:8443/private?q=secret#fragment", "matched_text": "secret match", "similarity_score": 0.9},
                            {"source_url": "https://[2001:db8::1]:9443/path", "matched_text": "ipv6 match", "similarity_score": 0.8},
                            {"source_url": "https://bücher.example/private", "matched_text": "idn match", "similarity_score": 0.7},
                            {"source_url": "javascript:alert(secret)", "matched_text": "bad scheme", "similarity_score": 0.6},
                            {"source_url": "https://exa\u{0000}mple.com/private", "matched_text": "control input", "similarity_score": 0.5}
                        ]
                    })
                    .to_string(),
                    plagiarism
                ],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .expect("borrow connection")
        .expect("write redaction fixtures");
    drop(store);

    let output = env.run(&["history", "export", "--redact-content"]);
    assert!(output.status.success());
    let body = String::from_utf8(output.stdout).unwrap();
    for forbidden in [
        "secret segment",
        "secret match",
        "user:pass",
        "8443",
        "/private",
        "?q=secret",
        "#fragment",
        "javascript",
        "alert(secret)",
        "dashboard.example",
        "exa\\u0000mple",
    ] {
        assert!(!body.contains(forbidden), "redaction leaked {forbidden:?}");
    }
    let rows = body
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    for row in &rows {
        serde_json::from_value::<
            microck_pangram_cli::domain::Analysis<microck_pangram_cli::output::CanonicalError>,
        >(row.clone())
        .expect("redacted export remains canonical-schema valid");
    }
    let ai_row = rows.iter().find(|row| row["id"] == ai).unwrap();
    let segment = &ai_row["checks"][0]["result"]["segments"][0];
    assert_eq!(segment["text"], "");
    assert_eq!(segment["label"], "AI");
    assert_eq!(segment["start_index"], 4);
    assert_eq!(segment["humanizer_score"], 0.2);
    assert_eq!(segment["is_humanized"], false);
    assert!(
        ai_row["checks"][0]["result"]
            .get("dashboard_link")
            .is_none()
    );
    let plagiarism_row = rows.iter().find(|row| row["id"] == plagiarism).unwrap();
    let matches = plagiarism_row["checks"][0]["result"]["matches"]
        .as_array()
        .unwrap();
    assert_eq!(matches.len(), 3, "invalid and non-HTTP URLs are omitted");
    assert_eq!(matches[0]["source_url"], "example.com");
    assert_eq!(matches[1]["source_url"], "[2001:db8::1]");
    assert_eq!(matches[2]["source_url"], "xn--bcher-kva.example");
    assert!(matches.iter().all(|matched| matched["matched_text"] == ""));
}
