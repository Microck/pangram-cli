//! Packet D compiled history-command tests.
//!
//! Every seeded command reads a real SQLite database through the production
//! `HistoryStore`; no Pangram endpoint or credit is reachable.

#[path = "support/history_cli_env.rs"]
mod harness;

use std::process::Command;

use microck_pangram_cli::domain::Sha256Hash;
use microck_pangram_cli::history::HistoryStore;
use serde_json::Value;

use harness::{Env, json};

#[test]
fn missing_and_disabled_databases_are_read_without_creation() {
    let env = Env::new();
    let database = env.data_dir().join("history").join("pangram-history.db");

    for command in [
        &["history", "list"][..],
        &["history", "search", "anything"][..],
    ] {
        let output = env.run(command);
        assert!(output.status.success());
        assert_eq!(json(&output)["data"]["items"], serde_json::json!([]));
        assert!(!database.exists(), "read must not create history storage");
    }
    let clear = env.run(&["history", "clear", "--yes"]);
    assert!(clear.status.success());
    assert_eq!(json(&clear)["data"]["ok"], true);
    assert!(
        !database.exists(),
        "empty clear must not create history storage"
    );

    env.seed(
        "anl_01983c20-0180-7a80-a001-000000000001",
        "2026-08-05T10:00:00Z",
        "retained even while disabled",
        None,
    );
    let output = env.run(&["history", "list"]);
    assert!(output.status.success());
    assert_eq!(json(&output)["data"]["items"].as_array().unwrap().len(), 1);
}

#[test]
fn list_is_typed_filtered_limited_and_deterministic() {
    let env = Env::new();
    for id in [
        "anl_01983c20-0180-7a80-a001-000000000002",
        "anl_01983c20-0180-7a80-a001-000000000001",
    ] {
        env.seed(
            id,
            "2026-08-05T10:00:00Z",
            "same timestamp",
            Some("sample.txt"),
        );
    }
    let output = env.run(&[
        "history",
        "list",
        "--status",
        "succeeded",
        "--check",
        "ai_detection",
        "--limit",
        "1",
    ]);
    assert!(output.status.success());
    let value = json(&output);
    let items = value["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], "anl_01983c20-0180-7a80-a001-000000000001");
    assert!(items[0].get("checks").is_some());
    assert!(items[0].get("input").is_none());
}

#[test]
fn limits_and_ids_are_strict_usage_errors_before_storage() {
    let env = Env::new();
    for limit in ["0", "+1", " 1", "1.0", "1001", "01x"] {
        let output = env.run(&["history", "list", "--limit", limit]);
        assert_eq!(output.status.code(), Some(2), "limit {limit:?}");
        assert!(!env.data_dir().join("history").exists());
    }
    let output = env.run(&["history", "show", "task-not-local"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!env.data_dir().join("history").exists());
}

#[test]
fn search_treats_fts_metacharacters_as_literal_text() {
    let env = Env::new();
    env.seed(
        "anl_01983c20-0180-7a80-a001-000000000001",
        "2026-08-05T10:00:00Z",
        "alpha beta special",
        None,
    );
    for query in [
        "alpha OR missing",
        "\"alpha\"",
        "alpha*",
        "(alpha)",
        "headline:alpha",
    ] {
        let output = env.run(&["history", "search", query]);
        assert!(output.status.success(), "query {query:?}");
    }
    let output = env.run(&["history", "search", "alpha OR missing"]);
    assert_eq!(json(&output)["data"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn show_redacts_plaintext_unless_explicitly_included() {
    let env = Env::new();
    let id = "anl_01983c20-0180-7a80-a001-000000000001";
    env.seed(
        id,
        "2026-08-05T10:00:00Z",
        "durable private plaintext",
        None,
    );

    let redacted = env.run(&["history", "show", id]);
    assert!(redacted.status.success());
    assert!(
        redacted
            .stdout
            .windows(b"durable private plaintext".len())
            .all(|window| window != b"durable private plaintext")
    );

    let included = env.run(&["history", "show", id, "--include-input"]);
    assert!(included.status.success());
    assert!(String::from_utf8_lossy(&included.stdout).contains("durable private plaintext"));
    let shown = json(&included);
    assert_eq!(
        shown["data"]["provenance"]["submitted_at"], "2026-08-05T09:59:00Z",
        "show uses the durable submission timestamp, not created_at"
    );
}

#[test]
fn resumed_show_and_export_preserve_upstream_provenance_without_authorship() {
    let env = Env::new();
    let id = "anl_01983c20-0180-7a80-a001-000000000009";
    let bulk_id = "bulk_01983c20-0180-7a80-a001-000000000009";
    env.seed(id, "2026-08-05T10:00:00Z", "placeholder", None);
    let store = HistoryStore::open(env.data_dir()).expect("open real history");
    store
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO bulk_collections (
                    id, upstream_bulk_id, status, submission_outcome,
                    total_items, accepted, succeeded, failed,
                    estimated_billable_units, created_at, updated_at, completed_at
                 ) VALUES (?1, 'upstream-bulk-resumed', 'succeeded', 'accepted',
                           1, 1, 1, 0, 1, ?2, ?2, ?2)",
                [bulk_id, "2026-08-05T10:00:00Z"],
            )?;
            connection.execute(
                "UPDATE analyses
                 SET bulk_id = ?1, bulk_index = 0, submission_outcome = 'accepted',
                     submitted_at = NULL,
                     input_json = json_set(json_remove(input_json, '$.text'), '$.origin', 'unknown')
                 WHERE id = ?2",
                [bulk_id, id],
            )?;
            connection.execute(
                "UPDATE analysis_search SET input_text = NULL WHERE analysis_id = ?1",
                [id],
            )?;
            connection.execute(
                "INSERT INTO upstream_tasks
                 (analysis_id, check_kind, upstream_task_id, last_stage, observed_at)
                 VALUES (?1, 'ai_detection', 'task-resumed', 'STAGE_SUCCESS', ?2)",
                [id, "2026-08-05T10:00:00Z"],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .expect("borrow connection")
        .expect("seed resumed provenance");
    drop(store);

    let shown = json(&env.run(&["history", "show", id]));
    assert_eq!(
        shown["data"]["provenance"]["upstream_bulk_id"],
        "upstream-bulk-resumed"
    );
    assert_eq!(
        shown["data"]["provenance"]["upstream_task_ids"],
        serde_json::json!(["task-resumed"])
    );
    assert!(
        shown["data"]["provenance"].get("submitted_at").is_none(),
        "a resumed read must not claim submission authorship"
    );

    let export = env.run(&["history", "export"]);
    assert!(export.status.success());
    let row: Value = serde_json::from_slice(&export.stdout).expect("one JSONL row");
    assert_eq!(
        row["provenance"]["upstream_bulk_id"],
        "upstream-bulk-resumed"
    );
    assert!(row["provenance"].get("submitted_at").is_none());
}

#[test]
fn queued_resumed_show_and_export_omit_absent_input() {
    let env = Env::new();
    let id = "anl_01983c20-0180-7a80-a001-00000000000a";
    env.seed(id, "2026-08-05T10:00:00Z", "placeholder", None);
    let store = HistoryStore::open(env.data_dir()).expect("open real history");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analyses
                 SET status = 'running', submission_outcome = 'accepted',
                     input_sha256 = ?1, input_json = 'null',
                     result_json = NULL, error_json = NULL,
                     submitted_at = NULL, completed_at = NULL
                 WHERE id = ?2",
                [Sha256Hash::from_bytes([0; 32]).to_string().as_str(), id],
            )?;
            connection.execute(
                "UPDATE analysis_checks
                 SET status = 'running', result_json = NULL, error_json = NULL
                 WHERE analysis_id = ?1",
                [id],
            )?;
            connection.execute(
                "UPDATE analysis_search
                 SET input_text = NULL, filename = NULL,
                     headline = NULL, source_urls = NULL
                 WHERE analysis_id = ?1",
                [id],
            )?;
            connection.execute(
                "INSERT INTO upstream_tasks
                    (analysis_id, check_kind, upstream_task_id, last_stage, observed_at)
                 VALUES (?1, 'ai_detection', 'task-input-absent', 'STAGE_RUNNING', ?2)",
                [id, "2026-08-05T10:00:00Z"],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .expect("borrow connection")
        .expect("seed resumed running row");
    drop(store);

    let shown = json(&env.run(&["history", "show", id]));
    assert_eq!(shown["data"]["status"], "running");
    assert!(shown["data"].get("input").is_none());
    let exported = env.run(&["history", "export"]);
    assert!(exported.status.success());
    let row: Value = serde_json::from_slice(&exported.stdout).unwrap();
    assert!(row.get("input").is_none());
}

#[test]
fn malformed_stored_canonical_json_fails_closed_without_leaking_content() {
    let env = Env::new();
    let id = "anl_01983c20-0180-7a80-a001-000000000001";
    env.seed(id, "2026-08-05T10:00:00Z", "private malformed row", None);
    let store = HistoryStore::open(env.data_dir()).expect("open real history");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analyses SET input_json = ?1 WHERE id = ?2",
                ["{private malformed row", id],
            )
        })
        .expect("borrow connection")
        .expect("corrupt test row");
    drop(store);

    let output = env.run(&["history", "show", id, "--include-input"]);
    assert_eq!(output.status.code(), Some(7));
    let body = json(&output);
    assert_eq!(body["error"]["code"], "history_corrupt");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private malformed row"));
}

#[test]
fn export_is_raw_newest_first_and_redacts_retained_content() {
    let env = Env::new();
    env.seed(
        "anl_01983c20-0180-7a80-a001-000000000001",
        "2026-08-05T10:00:00Z",
        "older private text",
        None,
    );
    env.seed(
        "anl_01983c20-0180-7a80-a001-000000000002",
        "2026-08-05T11:00:00Z",
        "newer private text",
        None,
    );

    let full = env.run(&["history", "export", "--format", "jsonl"]);
    assert!(full.status.success());
    let lines = String::from_utf8(full.stdout).unwrap();
    let rows = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], "anl_01983c20-0180-7a80-a001-000000000002");
    assert_eq!(rows[0]["input"]["text"], "newer private text");
    assert!(rows[0].get("command").is_none(), "export is raw");

    let redacted = env.run(&["history", "export", "--redact-content"]);
    assert!(redacted.status.success());
    let body = String::from_utf8(redacted.stdout).unwrap();
    assert!(!body.contains("private text"));
    for line in body.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        assert!(row["input"].get("text").is_none());
        assert!(
            row["checks"][0]["result"]["segments"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    let markdown = env.run(&["history", "export", "--format", "markdown"]);
    assert!(markdown.status.success());
    let markdown = String::from_utf8(markdown.stdout).unwrap();
    assert!(markdown.starts_with("# Pangram history export\n"));
    assert_eq!(
        markdown.matches("\n```").count(),
        4,
        "only the two structural JSON fences are emitted"
    );
}

#[test]
fn list_and_search_check_filters_use_authoritative_check_kinds() {
    let env = Env::new();
    let ai = "anl_01983c20-0180-7a80-a001-000000000011";
    let plagiarism = "anl_01983c20-0180-7a80-a001-000000000012";
    env.seed(ai, "2026-08-05T10:00:00Z", "shared filter term", None);
    env.seed(
        plagiarism,
        "2026-08-05T11:00:00Z",
        "shared filter term",
        None,
    );
    let store = HistoryStore::open(env.data_dir()).expect("open real history");
    store
        .with_connection(|connection| {
            let result_json = serde_json::json!({
                "plagiarism_detected": false,
                "total_sentences": 1,
                "plagiarized_sentence_count": 0,
                "percent_plagiarized": 0.0,
                "matches": []
            })
            .to_string();
            connection.execute(
                "UPDATE analysis_checks
                 SET check_kind = 'plagiarism',
                     result_json = ?1
                 WHERE analysis_id = ?2",
                rusqlite::params![result_json, plagiarism],
            )?;
            connection.execute(
                "UPDATE analyses
                 SET result_json = ?1
                 WHERE id = ?2",
                rusqlite::params![result_json, plagiarism],
            )?;
            connection.execute(
                "UPDATE analysis_search SET headline = NULL WHERE analysis_id = ?1",
                [plagiarism],
            )
        })
        .expect("borrow connection")
        .expect("convert authoritative check");
    drop(store);

    for command in ["list", "search"] {
        let arguments = if command == "list" {
            vec!["history", command, "--check", "ai_detection"]
        } else {
            vec![
                "history",
                command,
                "shared filter term",
                "--check",
                "ai_detection",
            ]
        };
        let value = json(&env.run(&arguments));
        let items = value["data"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], ai);

        let arguments = if command == "list" {
            vec!["history", command, "--check", "plagiarism"]
        } else {
            vec![
                "history",
                command,
                "shared filter term",
                "--check",
                "plagiarism",
            ]
        };
        let value = json(&env.run(&arguments));
        let items = value["data"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], plagiarism);
    }
}

#[test]
fn list_and_search_fail_closed_on_check_cardinality_or_kind_corruption() {
    for corruption in [
        "UPDATE analyses SET check_count = 2",
        "UPDATE analysis_checks SET check_kind = 'unknown_check'",
    ] {
        let env = Env::new();
        env.seed(
            "anl_01983c20-0180-7a80-a001-000000000021",
            "2026-08-05T10:00:00Z",
            "summary corruption term",
            None,
        );
        let store = HistoryStore::open(env.data_dir()).expect("open real history");
        store
            .with_connection(|connection| connection.execute(corruption, []))
            .expect("borrow connection")
            .expect("corrupt summary rows");
        drop(store);
        for arguments in [
            &["history", "list"][..],
            &["history", "search", "summary corruption term"][..],
        ] {
            let output = env.run(arguments);
            assert_eq!(output.status.code(), Some(7));
            assert_eq!(json(&output)["error"]["code"], "history_corrupt");
        }
    }
}

#[test]
fn deleting_an_original_preserves_dependents_and_clears_lineage() {
    let env = Env::new();
    let original = "anl_01983c20-0180-7a80-a001-000000000031";
    let retry = "anl_01983c20-0180-7a80-a001-000000000032";
    let rerun = "anl_01983c20-0180-7a80-a001-000000000033";
    env.seed(
        original,
        "2026-08-05T10:00:00Z",
        "original private text",
        None,
    );
    env.seed(retry, "2026-08-05T11:00:00Z", "retry private text", None);
    env.seed(rerun, "2026-08-05T12:00:00Z", "rerun private text", None);
    let store = HistoryStore::open(env.data_dir()).expect("open real history");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analyses SET retry_of = ?1 WHERE id = ?2",
                [original, retry],
            )?;
            connection.execute(
                "UPDATE analyses SET rerun_of = ?1 WHERE id = ?2",
                [original, rerun],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .expect("borrow connection")
        .expect("set lineage");
    drop(store);

    let deleted = env.run(&["history", "delete", original, "--yes"]);
    assert!(deleted.status.success());
    let store = HistoryStore::open(env.data_dir()).expect("reopen real history");
    let (analyses, checks, search, retry_of, rerun_of) = store
        .with_connection(|connection| {
            let count = |table: &str| {
                connection
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap()
            };
            let retry_of = connection
                .query_row(
                    "SELECT retry_of FROM analyses WHERE id = ?1",
                    [retry],
                    |row| row.get::<_, Option<String>>(0),
                )
                .unwrap();
            let rerun_of = connection
                .query_row(
                    "SELECT rerun_of FROM analyses WHERE id = ?1",
                    [rerun],
                    |row| row.get::<_, Option<String>>(0),
                )
                .unwrap();
            (
                count("analyses"),
                count("analysis_checks"),
                count("analysis_search"),
                retry_of,
                rerun_of,
            )
        })
        .expect("read retained dependents");
    assert_eq!((analyses, checks, search), (2, 2, 2));
    assert_eq!(retry_of, None);
    assert_eq!(rerun_of, None);
}

#[test]
fn empty_export_succeeds_without_creating_history() {
    let env = Env::new();
    let output = env.run(&["history", "export"]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(!env.data_dir().join("history").exists());
}

#[test]
fn rerun_rejects_missing_retained_text_before_credentials() {
    let env = Env::new();
    let id = "anl_01983c20-0180-7a80-a001-000000000001";
    env.seed(id, "2026-08-05T10:00:00Z", "private rerun text", None);
    let store = HistoryStore::open(env.data_dir()).expect("open real history");
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analyses
                 SET input_json = json_remove(input_json, '$.text')
                 WHERE id = ?1",
                [id],
            )?;
            connection.execute(
                "UPDATE analysis_search SET input_text = NULL WHERE analysis_id = ?1",
                [id],
            )
        })
        .expect("borrow connection")
        .expect("redact retained text");
    drop(store);

    let output = env.run(&["history", "rerun", id]);
    assert_eq!(output.status.code(), Some(7));
    let value = json(&output);
    assert_eq!(value["command"], "history_rerun");
    assert_eq!(value["error"]["code"], "local_task_unresolvable");
}

#[test]
fn rerun_validates_retained_text_integrity_before_credentials() {
    for mutation in [
        "UPDATE analyses SET input_type = 'file' WHERE id = ?1",
        "UPDATE analyses SET input_sha256 = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE id = ?1",
        "UPDATE analyses SET input_json = json_set(input_json, '$.sha256', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa') WHERE id = ?1",
        "UPDATE analyses SET input_json = json_set(input_json, '$.byte_count', 1) WHERE id = ?1",
        "UPDATE analyses SET input_json = json_set(input_json, '$.word_count', 99) WHERE id = ?1",
        "UPDATE analyses SET input_json = json_set(input_json, '$.text', 'changed plaintext') WHERE id = ?1",
    ] {
        let env = Env::new();
        let id = "anl_01983c20-0180-7a80-a001-000000000041";
        env.seed(id, "2026-08-05T10:00:00Z", "integrity source", None);
        let store = HistoryStore::open(env.data_dir()).expect("open real history");
        store
            .with_connection(|connection| connection.execute(mutation, [id]))
            .expect("borrow connection")
            .expect("corrupt retained input");
        drop(store);

        let output = env.run(&["history", "rerun", id]);
        assert_eq!(output.status.code(), Some(7), "{mutation}");
        assert_eq!(json(&output)["error"]["code"], "history_corrupt");
    }

    let env = Env::new();
    let id = "anl_01983c20-0180-7a80-a001-000000000042";
    env.seed(
        id,
        "2026-08-05T10:00:00Z",
        "naive\u{301} café 世界\tsecond",
        None,
    );
    let output = env.run(&["history", "rerun", id]);
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(json(&output)["error"]["code"], "missing_api_key");
}

#[test]
fn rerun_rejects_unicode_whitespace_and_accepts_one_word_before_credentials() {
    let whitespace = Env::new();
    let whitespace_id = "anl_01983c20-0180-7a80-a001-000000000043";
    whitespace.seed(
        whitespace_id,
        "2026-08-05T10:00:00Z",
        "\u{00a0}\u{2003}\u{2028}\u{3000}",
        None,
    );
    let output = whitespace.run(&["history", "rerun", whitespace_id]);
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(json(&output)["error"]["code"], "local_task_unresolvable");

    let one_word = Env::new();
    let one_word_id = "anl_01983c20-0180-7a80-a001-000000000044";
    one_word.seed(
        one_word_id,
        "2026-08-05T10:00:00Z",
        "\u{2003}boundary\u{00a0}",
        None,
    );
    let output = one_word.run(&["history", "rerun", one_word_id]);
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(json(&output)["error"]["code"], "missing_api_key");
}

#[cfg(unix)]
#[test]
fn history_output_failure_exits_general_failure() {
    use std::process::Stdio;

    let env = Env::new();
    env.seed(
        "anl_01983c20-0180-7a80-a001-000000000001",
        "2026-08-05T10:00:00Z",
        "output failure text",
        None,
    );
    for arguments in [
        &["history", "list"][..],
        &["history", "export"][..],
        &["history", "export", "--format", "markdown"][..],
    ] {
        let output = env
            .command()
            .args(arguments)
            .stdout(Stdio::from(
                std::fs::OpenOptions::new()
                    .write(true)
                    .open("/dev/full")
                    .expect("open /dev/full"),
            ))
            .output()
            .expect("run history output command");
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(arguments.get(1) != Some(&"export") || output.stderr.is_empty());
    }
}

#[test]
fn confirmed_delete_and_clear_mutate_real_sqlite() {
    let env = Env::new();
    let first = "anl_01983c20-0180-7a80-a001-000000000001";
    let second = "anl_01983c20-0180-7a80-a001-000000000002";
    env.seed(first, "2026-08-05T10:00:00Z", "first", None);
    env.seed(second, "2026-08-05T11:00:00Z", "second", None);

    let unconfirmed = env.run(&["history", "delete", first]);
    assert_eq!(unconfirmed.status.code(), Some(2));
    assert_eq!(
        json(&env.run(&["history", "list"]))["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    assert!(
        env.run(&["history", "delete", first, "--yes"])
            .status
            .success()
    );
    assert_eq!(
        json(&env.run(&["history", "list"]))["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    assert!(env.run(&["history", "clear", "--yes"]).status.success());
    assert_eq!(
        json(&env.run(&["history", "list"]))["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[cfg(unix)]
#[test]
fn interactive_decline_uses_a_real_pty_and_leaves_history_unchanged() {
    use std::io::Write as _;
    use std::process::Stdio;

    let env = Env::new();
    let id = "anl_01983c20-0180-7a80-a001-000000000001";
    env.seed(id, "2026-08-05T10:00:00Z", "keep this row", None);
    let command = format!(
        "env -u CI PANGRAM_DATA_DIR={} {} history delete {}",
        env.data_dir().display(),
        env!("CARGO_BIN_EXE_pangram"),
        id
    );
    let mut child = Command::new("/usr/bin/script")
        .args(["-qfec", &command, "/dev/null"])
        .env_clear()
        .env("HOME", env.root_path())
        .env("TERM", "dumb")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn real PTY through script");
    child
        .stdin
        .take()
        .expect("PTY stdin")
        .write_all(b"n\n")
        .expect("decline prompt");
    let output = child.wait_with_output().expect("wait for PTY");
    assert_eq!(output.status.code(), Some(130));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Confirm history delete?"),
        "interactive prompt is visible"
    );
    assert_eq!(
        json(&env.run(&["history", "list"]))["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
