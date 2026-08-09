//! Compiled CLI coverage for fail-closed bulk-membership reconstruction.

use std::process::{Command, Output};
use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, SaveState, Sha256Hash, SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{HistoryStore, InputKind, StoredAnalysis};

fn seed(root: &std::path::Path, id: &str) {
    let text = "membership source";
    let at = UtcTimestamp::from_str("2026-08-05T10:00:00Z").unwrap();
    let mut store = HistoryStore::open(root).expect("open real history");
    store
        .save_analysis(&StoredAnalysis {
            id: AnalysisId::from_str(id).unwrap(),
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
                "word_count": 2,
                "text": text
            })
            .to_string(),
            result_json: Some(
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
                .to_string(),
            ),
            error_json: None,
            upstream_version: Some("4.0".to_owned()),
            retry_of: None,
            rerun_of: None,
            submitted_at: Some(at),
            created_at: at,
            updated_at: at,
            completed_at: Some(at),
            search_input_text: Some(text.to_owned()),
            search_filename: None,
            search_headline: Some("Human".to_owned()),
            search_source_urls: None,
        })
        .expect("seed analysis");
}

fn run(root: &std::path::Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pangram"))
        .env_clear()
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("PANGRAM_DATA_DIR", root)
        .env("CI", "true")
        .env("TERM", "dumb")
        .args(arguments)
        .output()
        .expect("run pangram")
}

#[test]
fn partial_or_out_of_range_bulk_membership_fails_all_read_surfaces() {
    for (name, bulk_id, bulk_index) in [
        ("bulk_without_index", Some("bulk_valid"), None),
        ("index_without_bulk", None, Some(0_i64)),
        ("negative_index", Some("bulk_valid"), Some(-1)),
        ("past_end_index", Some("bulk_valid"), Some(1)),
        ("invalid_bulk_id", Some("not-a-bulk-id"), Some(0)),
    ] {
        let root = tempfile::tempdir().unwrap();
        let id = "anl_01983c20-0180-7a80-a001-000000000051";
        let valid_bulk_id = "bulk_01983c20-0180-7a80-a001-000000000051";
        seed(root.path(), id);
        let store = HistoryStore::open(root.path()).expect("open real history");
        store
            .with_connection(|connection| {
                let stored_bulk_id = if bulk_id == Some("not-a-bulk-id") {
                    "not-a-bulk-id"
                } else {
                    valid_bulk_id
                };
                connection.execute(
                    "INSERT INTO bulk_collections (
                        id, upstream_bulk_id, status, submission_outcome,
                        total_items, accepted, succeeded, failed,
                        estimated_billable_units, created_at, updated_at, completed_at
                     ) VALUES (?1, NULL, 'succeeded', 'accepted',
                               1, 1, 1, 0, 1, ?2, ?2, ?2)",
                    [stored_bulk_id, "2026-08-05T10:00:00Z"],
                )?;
                connection.execute(
                    "UPDATE analyses SET bulk_id = ?1, bulk_index = ?2 WHERE id = ?3",
                    rusqlite::params![
                        bulk_id.map(|value| {
                            if value == "bulk_valid" {
                                valid_bulk_id
                            } else {
                                value
                            }
                        }),
                        bulk_index,
                        id
                    ],
                )?;
                Ok::<_, rusqlite::Error>(())
            })
            .expect("borrow connection")
            .expect("corrupt membership");
        drop(store);

        for arguments in [
            &["history", "show", id][..],
            &["history", "export"][..],
            &["history", "list"][..],
        ] {
            let output = run(root.path(), arguments);
            assert_eq!(output.status.code(), Some(7), "{name}: {arguments:?}");
            let value: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("canonical error");
            assert_eq!(
                value["error"]["code"], "history_corrupt",
                "{name}: {arguments:?}"
            );
        }
    }
}
