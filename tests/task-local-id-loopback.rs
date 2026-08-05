//! Saved local task-ID resolution through the compiled CLI, real SQLite, and
//! the real loopback Pangram 4 fixture. No mocks or live credentials.

#![cfg(feature = "dev-tools")]

#[path = "support/history_save_env.rs"]
mod harness;

use std::str::FromStr as _;

use harness::fixture::{ProtocolFixture, Step, TASK_ID, pangram4_success};
use harness::{Isolated, stdout_envelope};
use microck_pangram_cli::domain::AnalysisId;
use microck_pangram_cli::history::HistoryStore;
use serde_json::json;

fn save_original(fixture: &ProtocolFixture, isolated: &Isolated, text: &str) -> String {
    let output = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", text])
        .output()
        .expect("save original");
    assert_eq!(output.status.code(), Some(0));
    stdout_envelope(&output)["data"]["id"]
        .as_str()
        .expect("local analysis ID")
        .to_owned()
}

fn assert_local_error(output: &std::process::Output, code: &str) {
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(stdout_envelope(output)["error"]["code"], code);
}

#[tokio::test(flavor = "multi_thread")]
async fn local_status_resolves_saved_task_and_reconciles_original_row() {
    let text = "retained authorship and search text";
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    let isolated = Isolated::new();
    isolated.enable_history();
    let original = save_original(&fixture, &isolated, text);

    let output = isolated
        .command(fixture.base_url())
        .args(["task", "status", &original])
        .output()
        .expect("status by local ID");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "task_status");
    assert_eq!(envelope["data"]["status"], "succeeded");
    assert_eq!(envelope["data"]["save_state"], "saved_history");
    assert_eq!(fixture.post_count(), 1, "the task read never submits");
    assert_eq!(fixture.get_count(), 2, "save poll plus one status snapshot");

    let connection = isolated.open_database();
    let row: (i64, String, String, String, String) = connection
        .query_row(
            "SELECT COUNT(*), id, save_state, input_json, s.input_text
             FROM analyses a JOIN analysis_search s ON s.analysis_id = a.id",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("one reconciled row");
    assert_eq!(row.0, 1);
    assert_eq!(row.1, original);
    assert_eq!(row.2, "saved_manual", "observation preserves authorship");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&row.3).unwrap()["text"],
        text
    );
    assert_eq!(row.4, text, "observation preserves FTS content");
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn local_status_reads_disabled_history_and_keeps_running_ephemeral() {
    let text = "disabled history remains readable";
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    fixture.on_poll(Step::Json(json!({"stage": "STAGE_INFERENCE"})));
    let isolated = Isolated::new();
    let original = save_original(&fixture, &isolated, text);

    let output = isolated
        .command(fixture.base_url())
        .args(["task", "status", &original])
        .output()
        .expect("running status by local ID");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["status"], "running");
    assert_eq!(envelope["data"]["save_state"], "ephemeral");
    assert_eq!(fixture.post_count(), 1);

    let connection = isolated.open_database();
    let stored: (String, String) = connection
        .query_row("SELECT id, status FROM analyses", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("saved original");
    assert_eq!(stored, (original, "succeeded".to_owned()));
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn local_wait_resolves_saved_task_through_nonterminal_to_terminal() {
    let text = "local wait reaches its terminal result";
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    fixture.on_poll(Step::Json(json!({"stage": "STAGE_INFERENCE"})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    let isolated = Isolated::new();
    let original = save_original(&fixture, &isolated, text);

    let output = isolated
        .command(fixture.base_url())
        .args(["task", "wait", &original, "--progress", "never"])
        .output()
        .expect("wait by local ID");
    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "task_wait");
    assert_eq!(envelope["data"]["status"], "succeeded");
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(fixture.get_count(), 3);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_missing_and_absent_local_ids_fail_before_auth_or_network() {
    let fixture = ProtocolFixture::start().await;

    let missing = Isolated::new();
    let output = missing
        .command(fixture.base_url())
        .env_remove("PANGRAM_API_KEY")
        .args(["task", "status", "anl_01983c20-0180-7a80-a001-000000000099"])
        .output()
        .expect("missing database");
    assert_local_error(&output, "local_task_unresolvable");
    assert!(!missing.history_directory().exists());

    let malformed = Isolated::new();
    let output = malformed
        .command(fixture.base_url())
        .env_remove("PANGRAM_API_KEY")
        .args(["task", "wait", "anl_not-a-canonical-id"])
        .output()
        .expect("malformed local ID");
    assert_local_error(&output, "local_task_unresolvable");

    let absent = Isolated::new();
    drop(HistoryStore::open(&absent.data).expect("create empty real history"));
    let output = absent
        .command(fixture.base_url())
        .env_remove("PANGRAM_API_KEY")
        .args(["task", "status", "anl_01983c20-0180-7a80-a001-000000000098"])
        .output()
        .expect("absent local row");
    assert_local_error(&output, "local_task_unresolvable");

    assert_eq!(fixture.post_count(), 0);
    assert_eq!(fixture.get_count(), 0);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn corrupt_evidence_fails_while_combined_evidence_resolves_the_ai_task() {
    for combined in [false, true] {
        let fixture = ProtocolFixture::start().await;
        fixture.on_submit(Step::Json(json!({"task_id": TASK_ID})));
        fixture.on_poll(Step::Json(pangram4_success("evidence source text")));
        let isolated = Isolated::new();
        let original = save_original(&fixture, &isolated, "evidence source text");
        let connection = isolated.open_database();
        if combined {
            connection
                .execute(
                    "UPDATE analyses SET check_count = 2 WHERE id = ?1",
                    [&original],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO analysis_checks
                     (analysis_id, check_index, check_kind, status, result_json)
                     VALUES (?1, 1, 'plagiarism', 'succeeded', ?2)",
                    rusqlite::params![
                        original,
                        json!({
                            "plagiarism_detected": false,
                            "total_sentences": 1,
                            "plagiarized_sentence_count": 0,
                            "percent_plagiarized": 0.0,
                            "matches": []
                        })
                        .to_string()
                    ],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO upstream_tasks
                     (analysis_id, check_kind, upstream_task_id, observed_at)
                     VALUES (?1, 'plagiarism', 'task-plagiarism', '2026-08-06T00:00:00Z')",
                    [&original],
                )
                .unwrap();
        } else {
            connection
                .execute(
                    "UPDATE upstream_tasks SET upstream_task_id = '' WHERE analysis_id = ?1",
                    [&original],
                )
                .unwrap();
        }
        drop(connection);

        let output = isolated
            .command(fixture.base_url())
            .env_remove("PANGRAM_API_KEY")
            .args(["task", "status", &original])
            .output()
            .expect("resolve invalid task evidence");
        if combined {
            assert_eq!(output.status.code(), Some(4));
            assert_eq!(stdout_envelope(&output)["error"]["code"], "missing_api_key");
        } else {
            assert_local_error(&output, "history_corrupt");
        }
        assert_eq!(fixture.post_count(), 1, "only the seed submission");
        assert_eq!(fixture.get_count(), 1, "only the seed observation");
        fixture.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn local_record_without_task_evidence_is_unresolvable_without_a_request() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success("task evidence removed")));
    let isolated = Isolated::new();
    let original = save_original(&fixture, &isolated, "task evidence removed");
    let connection = isolated.open_database();
    connection
        .execute(
            "DELETE FROM upstream_tasks WHERE analysis_id = ?1",
            [&original],
        )
        .unwrap();
    drop(connection);

    let output = isolated
        .command(fixture.base_url())
        .env_remove("PANGRAM_API_KEY")
        .args(["task", "wait", &original])
        .output()
        .expect("resolve missing task evidence");
    assert_local_error(&output, "local_task_unresolvable");
    assert_eq!(fixture.post_count(), 1);
    assert_eq!(fixture.get_count(), 1);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn opaque_upstream_task_id_still_bypasses_history_lookup() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Json(pangram4_success("upstream regression")));
    let isolated = Isolated::new();
    let output = isolated
        .command(fixture.base_url())
        .args(["task", "status", TASK_ID])
        .output()
        .expect("status by upstream ID");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(fixture.get_count(), 1);
    assert!(!isolated.history_directory().exists());
    fixture.shutdown().await;
}

#[test]
fn local_id_fixture_is_canonical_uuid_v7() {
    AnalysisId::from_str("anl_01983c20-0180-7a80-a001-000000000099").unwrap();
}
