//! Phase 4 Packet C remediation: order-independent task/bulk reconciliation
//! through the COMPILED binary against the real loopback Pangram 4 fixture
//! and a real SQLite store in a temporary data directory. No mocks, no live
//! Pangram, no real credentials.
//!
//! Split out of `history-save-reconciliation-loopback.rs` so both suites
//! stay under the source-size threshold. This suite locks the
//! docs/history-contract.md task-first/bulk-second rule end to end: a
//! standalone `task status` observation and a bulk read that attest the SAME
//! upstream task identity converge on exactly one durable row (the store
//! adopts in place), never duplicating an analysis/observation row, never
//! rolling an honest read back, and always preserving the first-recorded
//! authorship, save state, local input/FTS payload, and creation time.

#![cfg(feature = "dev-tools")]

#[path = "support/history_save_env.rs"]
mod harness;

use harness::fixture::{ProtocolFixture, Step, TASK_ID, pangram4_success};
use harness::{
    Isolated, analyses_rows, assert_no_leak, bulk_collection_rows, search_payload, stderr_text,
    stdout_envelope, task_rows,
};
use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, CheckKind, CheckStatus, SaveState, Sha256Hash, SubmissionOutcome,
    UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryStore, InputKind, StoredAnalysis, StoredCheck, StoredUpstreamTask,
};
use std::str::FromStr;

// ------------------------------------- task-first / bulk-second ordering --

/// Order independence through the compiled binary (docs/history-contract.md
/// task-first/bulk-second rule): a standalone `task status` observation of
/// `task-123` saves the one stored row first; a later `bulk status` of a
/// job whose results window attests THE SAME task identity for index 0 must
/// adopt that existing durable row instead of colliding with
/// `UNIQUE (check_kind, upstream_task_id)`. Exactly one analysis, one
/// observation row, and one FTS payload persist; the adoption adds the
/// membership link and moves only the observed status, and the locally held
/// standalone authorship (its `saved_history` state, local input text, and
/// creation time) is preserved exactly. The bulk command still exits 0 and
/// its output series is truthful.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn standalone_task_then_bulk_status_adopts_one_row_without_collision() {
    let text = "the standalone task observation reconciles first";
    let fixture = ProtocolFixture::start().await;
    // The standalone task read of `task-123`.
    fixture.on_poll(Step::Json(pangram4_success(text)));
    // The later bulk status read of `blk_cross` and its results window:
    // index 0 attests the very same upstream task identity.
    fixture.on_bulk_status(Step::Json(serde_json::json!({
        "bulk_id": "blk_cross",
        "status": "succeeded",
        "total_items": 1,
        "accepted": 1,
        "succeeded": 1,
        "failed": 0,
        "created_at": "1760000000.0",
        "completed_at": "1760000100.0"
    })));
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": "blk_cross",
        "offset": 0,
        "limit": 100,
        "total_items": 1,
        "items": [
            {"index": 0, "id": "row-000", "task_id": TASK_ID, "stage": "STAGE_SUCCESS",
             "error": null, "result": pangram4_success(text)}
        ],
        "failed_items": []
    })));
    let isolated = Isolated::new();
    isolated.enable_history();

    // 1. The standalone task status read saves the one row.
    let task_read = isolated
        .command(fixture.base_url())
        .args(["task", "status", TASK_ID])
        .output()
        .expect("run pangram task status");
    assert_eq!(task_read.status.code(), Some(0));
    assert_eq!(
        stdout_envelope(&task_read)["data"]["save_state"],
        "saved_history"
    );
    assert_no_leak(&task_read);

    // 2. The bulk status read attests the same task identity for its
    // child at index 0. Before this remediation the child insert collided
    // on UNIQUE (check_kind, upstream_task_id) and rolled the whole batch
    // back (an automatic-save warning on an honest remote read).
    let bulk_read = isolated
        .command(fixture.base_url())
        .args(["bulk", "status", "blk_cross"])
        .output()
        .expect("run pangram bulk status");
    assert_eq!(bulk_read.status.code(), Some(0));
    let stderr = stderr_text(&bulk_read);
    assert!(
        !stderr.contains("warning:"),
        "the adoption emits no automatic-save warning: {stderr}"
    );
    assert_no_leak(&bulk_read);

    // Exactly ONE analysis row, ONE observation row, ONE FTS payload: the
    // bulk read adopted the standalone row (no duplicate, no rollback).
    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 1, "the bulk read adopted the standalone row");
    // The adopted row's observed status moved to the attested success,
    // and its save state stayed the standalone read's `saved_history`.
    assert_eq!(rows[0].1, "succeeded");
    assert_eq!(rows[0].3, "saved_history");
    let tasks = task_rows(&connection);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].1, "ai_detection");
    assert_eq!(tasks[0].2, TASK_ID);
    let collections = bulk_collection_rows(&connection);
    assert_eq!(collections.len(), 1, "the collection row persisted");
    assert_eq!(collections[0].1.as_deref(), Some("blk_cross"));
    // The adopted row carries its membership link.
    let (bulk_id, bulk_index): (Option<String>, Option<i64>) = connection
        .query_row("SELECT bulk_id, bulk_index FROM analyses", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("the adopted row's membership");
    assert_eq!(bulk_id.as_deref(), Some(collections[0].0.as_str()));
    assert_eq!(bulk_index, Some(0));
    drop(connection);
    fixture.shutdown().await;
}

/// A one-check `task status` followed by `task wait` refreshes only the AI
/// slot of an already-saved combined analysis. The omitted plagiarism check,
/// its result body and task evidence, ordered check set, local input/FTS
/// payload, save authorship, and submitted provenance all remain durable.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn task_status_then_wait_merge_one_check_into_a_saved_combined_analysis() {
    let isolated = Isolated::new();
    isolated.enable_history();
    let id = AnalysisId::from_str("anl_01983c20-0180-7a80-a001-00000000e001").unwrap();
    let created_at = UtcTimestamp::from_str("2026-08-05T10:00:00Z").unwrap();
    let text = "compiled combined history input";
    let input_sha256 = Sha256Hash::digest(text);
    let plagiarism_result = serde_json::json!({
        "plagiarism_detected": false,
        "total_sentences": 1,
        "plagiarized_sentence_count": 1,
        "percent_plagiarized": 100.0,
        "matches": [{
            "source_url": "https://example.invalid/retained",
            "matched_text": "synthetic retained match",
            "similarity_score": 1.0
        }]
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
        input_sha256,
        display_name: Some("combined.txt".to_owned()),
        input_json: serde_json::json!({
            "type": "text",
            "origin": "file",
            "name": "combined.txt",
            "sha256": input_sha256,
            "byte_count": text.len(),
            "word_count": 4,
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
        search_filename: Some("combined.txt".to_owned()),
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
            upstream_task_id: TASK_ID.to_owned(),
            last_stage: Some("STAGE_INFERENCE".to_owned()),
            observed_at: created_at,
        },
        StoredUpstreamTask {
            analysis_id: id,
            check_kind: CheckKind::Plagiarism,
            upstream_task_id: "task-combined-plagiarism".to_owned(),
            last_stage: Some("STAGE_SUCCESS".to_owned()),
            observed_at: created_at,
        },
    ];
    HistoryStore::open(&isolated.data)
        .expect("open real history")
        .save_analysis_complete(&record, &checks, &observations)
        .expect("seed combined analysis");

    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Json(serde_json::json!({"stage": "STAGE_INFERENCE"})));
    fixture.on_poll(Step::Json(pangram4_success(
        "compiled combined terminal input",
    )));

    let status = isolated
        .command(fixture.base_url())
        .args(["task", "status", TASK_ID])
        .output()
        .expect("run task status");
    assert_eq!(status.status.code(), Some(0));
    assert_eq!(
        stdout_envelope(&status)["data"]["save_state"],
        "saved_history"
    );
    assert!(!stderr_text(&status).contains("warning:"));
    assert_no_leak(&status);
    let connection = isolated.open_database();
    assert_eq!(analyses_rows(&connection).len(), 1);
    assert_eq!(task_rows(&connection).len(), 2);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM analysis_checks WHERE analysis_id = ?1",
                [id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
    );
    drop(connection);

    let wait = isolated
        .command(fixture.base_url())
        .args(["task", "wait", TASK_ID])
        .output()
        .expect("run task wait");
    assert_eq!(wait.status.code(), Some(0));
    assert_eq!(
        stdout_envelope(&wait)["data"]["save_state"],
        "saved_history"
    );
    assert!(!stderr_text(&wait).contains("warning:"));
    assert_no_leak(&wait);

    let store = HistoryStore::open(&isolated.data).expect("reopen history");
    let canonical = store
        .canonical_analysis(&id, true)
        .expect("reconstruct combined analysis");
    assert_eq!(canonical.status(), AnalysisStatus::Succeeded);
    let value = serde_json::to_value(canonical).unwrap();
    assert_eq!(value["checks"][0]["kind"], "ai_detection");
    assert_eq!(value["checks"][1]["kind"], "plagiarism");
    assert_eq!(
        value["checks"][1]["result"],
        serde_json::from_str::<serde_json::Value>(&plagiarism_result).unwrap()
    );
    assert_eq!(value["input"]["text"], text);
    let after = store.get_analysis(&id).expect("stored parent");
    assert_eq!(after.save_state, SaveState::SavedManual);
    assert_eq!(after.caller_id.as_deref(), Some("retained-caller"));
    assert_eq!(after.submitted_at, Some(created_at));
    drop(store);
    let connection = isolated.open_database();
    let tasks = task_rows(&connection);
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().any(|row| row.2 == "task-combined-plagiarism"));
    assert_eq!(
        search_payload(&connection),
        vec![(
            id.to_string(),
            Some(text.to_owned()),
            Some("Human-written".to_owned())
        )]
    );
    let source_urls: Option<String> = connection
        .query_row(
            "SELECT source_urls FROM analysis_search WHERE analysis_id = ?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        source_urls.as_deref(),
        Some("https://example.invalid/retained")
    );
    fixture.shutdown().await;
}

/// The bulk-first direction stays contract-true through the compiled
/// binary across BOTH attestation paths: a `bulk submit` acceptance
/// attesting the task identity up front persists the child with its
/// observation row, and the later standalone `task status` read of that
/// identity refreshes the same child instead of inserting a duplicate.
/// Exactly one analysis, one observation row, and one FTS payload persist;
/// the led child keeps its JSONL caller ID, membership, and locally held
/// plaintext. (The no-acceptance-task-id flow is contract-identical: the
/// membership child simply gains its observation on the first read that
/// attests one, proven at the store level in
/// `history-store-reconcile-order.rs`.)
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_then_task_status_leads_the_one_membership_child() {
    let text = "the bulk acceptance child leads the task read";
    let fixture = ProtocolFixture::start().await;
    // Acceptance at the submit already attests the task identity; the
    // later standalone task read of the SAME identity leads that child.
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": "blk_lead_loop",
            "status": "queued",
            "total_items": 1,
            "accepted_items": [{"index": 0, "id": "row-000", "task_id": TASK_ID}],
            "failed_items": []
        })),
    ));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    let isolated = Isolated::new();
    isolated.enable_history();

    let root = tempfile::tempdir().unwrap();
    let items = root.path().join("items.jsonl");
    std::fs::write(
        &items,
        serde_json::json!({"id": "row-000", "text": text}).to_string(),
    )
    .unwrap();
    let submit = isolated
        .command(fixture.base_url())
        .args([
            "bulk",
            "submit",
            items.to_str().unwrap(),
            "--max-billable-units",
            "5",
        ])
        .output()
        .expect("run bulk submit");
    assert_eq!(submit.status.code(), Some(0));
    assert_no_leak(&submit);

    let task_read = isolated
        .command(fixture.base_url())
        .args(["task", "status", TASK_ID])
        .output()
        .expect("run pangram task status");
    assert_eq!(task_read.status.code(), Some(0));
    assert_eq!(
        stdout_envelope(&task_read)["data"]["save_state"],
        "saved_history"
    );
    assert_no_leak(&task_read);

    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(
        rows.len(),
        1,
        "the task read led the membership child (no duplicate)"
    );
    let tasks = task_rows(&connection);
    assert_eq!(tasks.len(), 1, "exactly one observation row");
    assert_eq!(tasks[0].2, TASK_ID);
    // The led child kept its JSONL plaintext and its membership.
    let payload = search_payload(&connection);
    assert_eq!(payload.len(), 1);
    assert_eq!(payload[0].1.as_deref(), Some(text));
    let bulk_index: Option<i64> = connection
        .query_row("SELECT bulk_index FROM analyses", [], |row| row.get(0))
        .expect("membership");
    assert_eq!(bulk_index, Some(0));
    drop(connection);
    fixture.shutdown().await;
}

/// A remote-only bulk status has no local plan to lend the child metadata.
/// The validated terminal results document is therefore the sole source of
/// its input descriptor and upstream evidence. The save projector must carry
/// that evidence into the real SQLite row instead of rebuilding a blank
/// child: input hash/counts, result headline, task ID, and last stage all
/// survive the compiled status flow.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn remote_bulk_status_persists_attested_child_metadata() {
    let text = "remote status preserves attested child metadata";
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_status(Step::Json(serde_json::json!({
        "bulk_id": "blk_remote_metadata",
        "status": "succeeded",
        "total_items": 1,
        "accepted": 1,
        "succeeded": 1,
        "failed": 0,
        "created_at": "1760000000.0",
        "completed_at": "1760000100.0"
    })));
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": "blk_remote_metadata",
        "offset": 0,
        "limit": 100,
        "total_items": 1,
        "items": [
            {"index": 0, "id": "row-000", "task_id": TASK_ID, "stage": "STAGE_SUCCESS",
             "error": null, "result": pangram4_success(text)}
        ],
        "failed_items": []
    })));
    let isolated = Isolated::new();
    isolated.enable_history();

    let output = isolated
        .command(fixture.base_url())
        .args(["bulk", "status", "blk_remote_metadata"])
        .output()
        .expect("run remote bulk status");
    assert_eq!(output.status.code(), Some(0));
    assert_no_leak(&output);

    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 1);
    let input: serde_json::Value = serde_json::from_str(&rows[0].5).unwrap();
    assert_eq!(input["origin"], "unknown");
    assert_eq!(input["byte_count"], text.len());
    assert_eq!(
        input["word_count"],
        text.split_whitespace().count(),
        "the upstream-attested descriptor counts survive"
    );
    assert!(
        input.get("text").is_none(),
        "remote plaintext is never persisted"
    );
    let result: serde_json::Value =
        serde_json::from_str(rows[0].6.as_ref().expect("terminal result")).unwrap();
    assert_eq!(result["headline"], "Human-written");
    let evidence: (String, Option<String>) = connection
        .query_row(
            "SELECT upstream_task_id, last_stage FROM upstream_tasks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("task evidence");
    assert_eq!(evidence.0, TASK_ID);
    assert_eq!(evidence.1.as_deref(), Some("STAGE_SUCCESS"));
    drop(connection);
    fixture.shutdown().await;
}

/// The opposite ordering starts from an accepted JSONL child with locally
/// held filename/plaintext, then refreshes it through `bulk wait`. The
/// terminal result contributes current task/stage/result metadata while the
/// durable local input and FTS payload remain authoritative.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn accepted_child_then_bulk_wait_merges_remote_evidence_without_losing_local_input() {
    let text = "local acceptance metadata remains searchable after wait";
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": "blk_accept_then_wait",
            "status": "queued",
            "total_items": 1,
            "accepted_items": [{"index": 0, "id": "row-000", "task_id": TASK_ID}],
            "failed_items": []
        })),
    ));
    fixture.on_bulk_status(Step::Json(serde_json::json!({
        "bulk_id": "blk_accept_then_wait",
        "status": "succeeded",
        "total_items": 1,
        "accepted": 1,
        "succeeded": 1,
        "failed": 0,
        "created_at": "1760000000.0",
        "completed_at": "1760000100.0"
    })));
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": "blk_accept_then_wait",
        "offset": 0,
        "limit": 100,
        "total_items": 1,
        "items": [
            {"index": 0, "id": "row-000", "task_id": TASK_ID, "stage": "STAGE_SUCCESS",
             "error": null, "result": pangram4_success(text)}
        ],
        "failed_items": []
    })));
    let isolated = Isolated::new();
    isolated.enable_history();
    let root = tempfile::tempdir().unwrap();
    let input_path = root.path().join("attested-source.jsonl");
    std::fs::write(
        &input_path,
        serde_json::json!({"id": "row-000", "text": text}).to_string(),
    )
    .unwrap();

    let submit = isolated
        .command(fixture.base_url())
        .args([
            "bulk",
            "submit",
            input_path.to_str().unwrap(),
            "--max-billable-units",
            "5",
        ])
        .output()
        .expect("submit accepted child");
    assert_eq!(submit.status.code(), Some(0));
    let wait = isolated
        .command(fixture.base_url())
        .args(["bulk", "wait", "blk_accept_then_wait"])
        .output()
        .expect("wait accepted child");
    assert_eq!(wait.status.code(), Some(0));
    assert!(
        !stderr_text(&wait).contains("warning:"),
        "{}",
        stderr_text(&wait)
    );
    assert_no_leak(&wait);

    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 1);
    let input: serde_json::Value = serde_json::from_str(&rows[0].5).unwrap();
    assert_eq!(input["origin"], "file");
    assert_eq!(input["name"], "attested-source.jsonl");
    assert_eq!(input["text"], text);
    let payload = search_payload(&connection);
    assert_eq!(payload[0].1.as_deref(), Some(text));
    let evidence: (String, Option<String>) = connection
        .query_row(
            "SELECT upstream_task_id, last_stage FROM upstream_tasks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("task evidence");
    assert_eq!(evidence.0, TASK_ID);
    assert_eq!(evidence.1.as_deref(), Some("STAGE_SUCCESS"));
    drop(connection);
    fixture.shutdown().await;
}
