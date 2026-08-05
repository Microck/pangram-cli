use std::process::{Command, Output};
use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, SaveState, Sha256Hash, SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{HistoryStore, InputKind, StoredAnalysis};
use serde_json::Value;

pub(crate) struct Env {
    root: tempfile::TempDir,
}

impl Env {
    pub(crate) fn new() -> Self {
        Self {
            root: tempfile::tempdir().expect("temporary root"),
        }
    }

    pub(crate) fn root_path(&self) -> &std::path::Path {
        self.root.path()
    }

    pub(crate) fn data_dir(&self) -> &std::path::Path {
        self.root.path()
    }

    pub(crate) fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_pangram"));
        command
            .env_clear()
            .env("HOME", self.root.path())
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .env("PANGRAM_DATA_DIR", self.data_dir())
            .env("CI", "true")
            .env("TERM", "dumb");
        command
    }

    pub(crate) fn run(&self, arguments: &[&str]) -> Output {
        self.command()
            .args(arguments)
            .output()
            .expect("run pangram")
    }

    pub(crate) fn seed(&self, id: &str, created_at: &str, text: &str, display_name: Option<&str>) {
        let mut store = HistoryStore::open(self.data_dir()).expect("open real history");
        let mut input = serde_json::json!({
            "type": "text",
            "origin": "literal",
            "sha256": Sha256Hash::digest(text).to_string(),
            "byte_count": text.len(),
            "word_count": text.split_whitespace().count(),
            "text": text
        });
        if let Some(name) = display_name {
            input["origin"] = serde_json::Value::String("file".to_owned());
            input["name"] = serde_json::Value::String(name.to_owned());
        }
        let record = StoredAnalysis {
            id: AnalysisId::from_str(id).expect("analysis id"),
            bulk: None,
            caller_id: None,
            status: AnalysisStatus::Succeeded,
            submission_outcome: SubmissionOutcome::Terminal,
            save_state: SaveState::SavedManual,
            input_kind: InputKind::Text,
            input_sha256: Sha256Hash::digest(text),
            display_name: display_name.map(str::to_owned),
            input_json: input.to_string(),
            result_json: Some(
                serde_json::json!({
                    "classification": "human",
                    "headline": "Literal [headline]",
                    "prediction": "Human",
                    "fraction_ai": 0.0,
                    "fraction_ai_assisted": 0.0,
                    "fraction_human": 1.0,
                    "num_ai_segments": 0,
                    "num_ai_assisted_segments": 0,
                    "num_human_segments": 1,
                    "segments": []
                })
                .to_string(),
            ),
            error_json: None,
            upstream_version: Some("4.0".to_owned()),
            retry_of: None,
            rerun_of: None,
            submitted_at: Some(
                UtcTimestamp::from_str("2026-08-05T09:59:00Z").expect("submitted timestamp"),
            ),
            created_at: UtcTimestamp::from_str(created_at).expect("created timestamp"),
            updated_at: UtcTimestamp::from_str(created_at).expect("updated timestamp"),
            completed_at: Some(UtcTimestamp::from_str(created_at).expect("completed timestamp")),
            search_input_text: Some(text.to_owned()),
            search_filename: display_name.map(str::to_owned),
            search_headline: Some("Literal [headline]".to_owned()),
            search_source_urls: None,
        };
        store.save_analysis(&record).expect("seed analysis");
    }
}

pub(crate) fn json(output: &Output) -> Value {
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("canonical JSON")
}
