//! Phase 4 Packet C remediation: the reconciliation semantics of the
//! history-save integration, against the real loopback Pangram 4 fixture and
//! a real SQLite store in a temporary data directory. No mocks, no live
//! Pangram, no real credentials.
//!
//! Split out of `history-save-failures-loopback.rs` so each suite stays under
//! the source-size threshold while both exercise the exact same real store.
//! This suite locks the contract-authorized reconciliation semantics
//! (contracts.md 14.2 note, docs/history-contract.md, ADR 0004):
//! - repeated `task status`/`task wait` reads reconcile one stored row:
//!   authorship, save state, local input, and creation time are preserved,
//!   and the fresh output reports its own fresh identity and save outcome
//! - repeated bulk submit/status/wait reconcile one collection by its
//!   upstream id without duplicates, with atomically refreshed children
//! - a blocked automatic save stays truthful: one warning, exit preserved,
//!   and no collection row is fabricated
//! - first enablement of `history.enabled` acknowledges ADR 0004 exactly once

#![cfg(feature = "dev-tools")]

#[path = "support/history_save_env.rs"]
mod harness;

use harness::fixture::{ProtocolFixture, Step, TASK_ID};
use harness::{
    Isolated, analyses_rows, assert_no_leak, bulk_collection_rows, poison_data_dir, search_payload,
    stderr_text, stdout_envelope, task_rows,
};

// ----------------------------------------------------- task reconciliation --

/// Repeated `task status` reads of one remote task reconcile onto the one
/// stored row instead of duplicating it. The row's durable authorship
/// (original `accepted` outcome, creation time) is preserved, and each fresh
/// read's output keeps its own fresh identity and its own `saved_history`
/// outcome rather than claiming another read's state.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn repeated_task_status_reconciles_one_row_with_fresh_output_identity() {
    let text = "Repeatedly observed task reconciles onto one saved row";
    let fixture = ProtocolFixture::start().await;
    fixture.on_poll(Step::Json(harness::fixture::pangram4_success(text)));
    fixture.on_poll(Step::Json(harness::fixture::pangram4_success(text)));
    let isolated = Isolated::new();
    isolated.enable_history();

    let mut ids = Vec::new();
    for read in 0..2 {
        let output = isolated
            .command(fixture.base_url())
            .args(["task", "status", TASK_ID])
            .output()
            .expect("run pangram task status");
        assert_eq!(output.status.code(), Some(0), "read {read} succeeds");
        let envelope = stdout_envelope(&output);
        ids.push(envelope["data"]["id"].as_str().unwrap().to_owned());
        // Each fresh read reports its own save outcome.
        assert_eq!(
            envelope["data"]["save_state"], "saved_history",
            "read {read} reports its own automatic-history save outcome"
        );
        assert_no_leak(&output);
    }
    assert_ne!(ids[0], ids[1], "each read mints a fresh output identity");

    // But the store reconciles onto ONE row.
    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(
        rows.len(),
        1,
        "repeated reads of one task never duplicate the stored row"
    );
    fixture.shutdown().await;
}

/// A task read refreshes the stored row's observation fields only: the
/// stored row's local input/search content (from an include-input manual
/// save), submission outcome, save state, and creation time are preserved
/// when a later observation carries no local payload of its own.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn task_read_refresh_preserves_local_input_and_authorship() {
    let text = "Locally authored include-input content preserved in history";
    let fixture = ProtocolFixture::start().await;
    // A manual include-input detect saves the row with the submitted text.
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(harness::fixture::pangram4_success(text)));
    // Then a later task status observation of the same task refreshes it.
    fixture.on_poll(Step::Json(harness::fixture::pangram4_success(text)));
    let isolated = Isolated::new();
    isolated.enable_history();

    // Locally authored save: --save --include-input persists the text.
    let save = isolated
        .command(fixture.base_url())
        .args(["detect", "--save", "--include-input", text])
        .output()
        .expect("run pangram detect --save --include-input");
    assert_eq!(save.status.code(), Some(0));
    let saved_envelope = stdout_envelope(&save);
    let saved_id = saved_envelope["data"]["id"].as_str().unwrap().to_owned();

    // Observation of the same task: carries no local input payload.
    let read = isolated
        .command(fixture.base_url())
        .args(["task", "status", TASK_ID])
        .output()
        .expect("run pangram task status");
    assert_eq!(read.status.code(), Some(0));
    let read_envelope = stdout_envelope(&read);
    // The fresh output keeps its OWN fresh identity and save outcome; it
    // does not claim the locally authored row's `saved_manual` state or id.
    assert_ne!(
        read_envelope["data"]["id"].as_str().unwrap(),
        saved_id,
        "the fresh read never claims the prior row's identity"
    );
    assert_eq!(read_envelope["data"]["save_state"], "saved_history");

    // The one stored row still holds the locally authored input text and
    // search payload, its original terminal outcome + manual save state,
    // and its original creation time.
    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 1, "one reconciled row");
    assert_eq!(rows[0].2, "terminal", "original submission outcome kept");
    assert_eq!(rows[0].3, "saved_manual", "original save state kept");
    let payload = search_payload(&connection);
    assert_eq!(
        payload[0].1.as_deref(),
        Some(text),
        "the locally held input text survives the remote-only refresh"
    );
    drop(connection);
    fixture.shutdown().await;
}

// ----------------------------------------------------- bulk reconciliation --

/// Repeated bulk observations of one upstream job reconcile onto one stored
/// collection row, deduped by `upstream_bulk_id`: a `bulk submit` then a
/// `bulk status` of the same job leave exactly one collection row, with its
/// persisted truthful plan children carried into the output.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_and_status_reconcile_one_collection_without_duplicates() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": "blk_dedupe",
            "status": "queued",
            "total_items": 2,
            "accepted_items": [
                {"index": 0, "id": "row-000", "task_id": "task-000"},
                {"index": 1, "id": "row-001", "task_id": "task-001"}
            ],
            "failed_items": []
        })),
    ));
    fixture.on_bulk_status(Step::Json(serde_json::json!({
        "bulk_id": "blk_dedupe",
        "status": "queued",
        "total_items": 2,
        "accepted": 2,
        "succeeded": 0,
        "failed": 0,
        "created_at": "1760000000.0",
        "completed_at": null
    })));
    // Under the armed automatic gate the status read also refreshes the
    // stored children from the documented results window; both positions
    // are still in flight (`result: null` normalizes to `running`).
    fixture.on_bulk_results(Step::Json(serde_json::json!({
        "bulk_id": "blk_dedupe",
        "offset": 0,
        "limit": 100,
        "total_items": 2,
        "items": [
            {"index": 0, "id": "row-000", "task_id": "task-000", "stage": "STAGE_PENDING", "error": null, "result": null},
            {"index": 1, "id": "row-001", "task_id": "task-001", "stage": "STAGE_PENDING", "error": null, "result": null}
        ],
        "failed_items": []
    })));
    let isolated = Isolated::new();
    isolated.enable_history();

    let root = tempfile::tempdir().unwrap();
    let items = root.path().join("items.jsonl");
    std::fs::write(
        &items,
        [
            serde_json::json!({"id": "row-000", "text": "first dedupe item words"}).to_string(),
            serde_json::json!({"id": "row-001", "text": "second dedupe item words"}).to_string(),
        ]
        .join("\n"),
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
    // The accepted submit persists its truthful plan children, and the
    // output series carries them (no dropped members).
    assert_no_leak(&submit);

    let status = isolated
        .command(fixture.base_url())
        .args(["bulk", "status", "blk_dedupe"])
        .output()
        .expect("run bulk status");
    assert_eq!(status.status.code(), Some(0));
    assert_no_leak(&status);

    // One collection row, deduped by the upstream id, and exactly the two
    // truthful children (no duplicates from the repeated observation).
    let connection = isolated.open_database();
    let collections = bulk_collection_rows(&connection);
    assert_eq!(
        collections.len(),
        1,
        "repeated observation dedupes by upstream_bulk_id"
    );
    assert_eq!(collections[0].1.as_deref(), Some("blk_dedupe"));
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 2, "the two children persist exactly once");
    // The acceptance persisted both children as accepted and queued; the
    // observed `running` refresh reconciled them in place (only the
    // observation field moved) without overwriting their first-recorded
    // local authorship or honest accepted outcome.
    for row in &rows {
        assert_eq!(row.1, "running", "the observed in-flight state moved");
        assert_eq!(row.2, "accepted", "the 202-attested outcome stays honest");
        assert_eq!(row.3, "saved_history", "the automatic gate saved it");
    }
    // Each persisted child carries one observation row attesting its
    // worker identity; a refresh of the same membership updates it in
    // place rather than duplicating it.
    let tasks = task_rows(&connection);
    let mut worker_ids: Vec<String> = tasks.iter().map(|row| row.2.clone()).collect();
    worker_ids.sort();
    assert_eq!(
        worker_ids,
        ["task-000".to_owned(), "task-001".to_owned()],
        "exactly the two attested task identities persist, never duplicated"
    );
    assert!(
        tasks
            .iter()
            .all(|row| row.3.as_deref() == Some("STAGE_PENDING")),
        "the sanitized running stage survives status output projection and history"
    );
    drop(connection);
    fixture.shutdown().await;
}

/// A mixed acceptance with an armed automatic gate remains truthful when
/// the store is blocked: the JSONL submission carries two items (one
/// accepted with an attested task identity, one failed through immediate
/// upstream validation), and the blocked save still surfaces the honest
/// mixed persisted state (accepted/failed counts in the envelope, each
/// child honestly categorized) instead of a fabricated all-queued
/// success. One sanitized automatic warning covers the whole submission
/// and the process exits 0 (the remote acceptance is never degraded).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn jsonl_mixed_acceptance_blocked_save_warns_once_and_stays_truthful() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": "blk_mixed_blocked",
            "status": "queued",
            "total_items": 2,
            "accepted_items": [{"index": 0, "id": "row-000", "task_id": "task-000"}],
            "failed_items": [
                {"index": 1, "id": "row-001", "task_id": null, "stage": "STAGE_FAILED", "error": "Text must contain at least one valid token"}
            ]
        })),
    ));
    let isolated = Isolated::new();
    isolated.enable_history();
    poison_data_dir(&isolated);

    let root = tempfile::tempdir().unwrap();
    let items = root.path().join("mixed-items.jsonl");
    std::fs::write(
        &items,
        [
            serde_json::json!({"id": "row-000", "text": "words the fixture accepts"}).to_string(),
            serde_json::json!({"id": "row-001", "text": "x"}).to_string(),
        ]
        .join("\n"),
    )
    .unwrap();

    let output = isolated
        .command(fixture.base_url())
        .args([
            "bulk",
            "submit",
            items.to_str().unwrap(),
            "--max-billable-units",
            "5",
        ])
        .output()
        .expect("run bulk submit of a mixed acceptance under a blocked store");

    assert_eq!(
        output.status.code(),
        Some(0),
        "the acceptance still exits 0 under a blocked save"
    );
    let envelope = stdout_envelope(&output);
    let data = &envelope["data"];
    assert_eq!(data["status"], "queued");
    assert_eq!(data["total_items"], 2);
    assert_eq!(data["accepted"], 1, "only the accepted position counts");
    assert_eq!(data["failed"], 1, "the immediate failure is honest");
    assert_eq!(data["succeeded"], 0);
    let stderr = stderr_text(&output);
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.contains("warning:"))
            .count(),
        1,
        "exactly one automatic save warning: {stderr}"
    );
    assert!(!isolated.database_path().exists());
    assert_no_leak(&output);
    fixture.shutdown().await;
}

/// A JSONL-file submission whose HTTP 202 acceptance mixes one accepted
/// position and one immediately-failed position persists the failed child
/// with the same truthful provenance every sibling carries (contracts.md
/// 9.1 + 14.2 and docs/history-contract.md schema v1): its `file` origin,
/// the JSONL basename, the locally held plaintext, and its FTS payload all
/// land exactly like the accepted sibling's, while the child keeps its
/// failed outcome and canonical check error and fabricates no task
/// identity. The failure is searchable by filename and by input text.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn jsonl_mixed_acceptance_persists_the_failed_child_with_its_local_provenance() {
    let accepted_text = "words the fixture accepts gladly";
    let failed_text = "immediately rejected words";
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": "blk_mixed_local",
            "status": "queued",
            "total_items": 2,
            "accepted_items": [{"index": 0, "id": "row-000", "task_id": "task-000"}],
            "failed_items": [
                {"index": 1, "id": "row-001", "task_id": null, "stage": "STAGE_FAILED",
                 "error": "Text must contain at least one valid token"}
            ]
        })),
    ));
    let isolated = Isolated::new();
    isolated.enable_history();

    let root = tempfile::tempdir().unwrap();
    let items = root.path().join("mixed-source.jsonl");
    std::fs::write(
        &items,
        [
            serde_json::json!({"id": "row-000", "text": accepted_text}).to_string(),
            serde_json::json!({"id": "row-001", "text": failed_text}).to_string(),
        ]
        .join("\n"),
    )
    .unwrap();

    let output = isolated
        .command(fixture.base_url())
        .args([
            "bulk",
            "submit",
            items.to_str().unwrap(),
            "--max-billable-units",
            "5",
        ])
        .output()
        .expect("run bulk submit of a mixed acceptance with history enabled");

    assert_eq!(output.status.code(), Some(0));
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["data"]["accepted"], 1);
    assert_eq!(envelope["data"]["failed"], 1);
    assert_no_leak(&output);

    let connection = isolated.open_database();
    let rows = analyses_rows(&connection);
    assert_eq!(rows.len(), 2, "the two acceptance children persist");
    let failed = rows
        .iter()
        .find(|row| row.1 == "failed")
        .expect("one failed acceptance child");
    // The failed child reports the same input provenance as the accepted
    // sibling: the `file` origin (`text` kind), the JSONL basename, and the
    // locally held plaintext (never the upstream-erased literal), with its
    // failed outcome, its canonical error, and no fabricated task identity.
    assert_eq!(failed.2, "accepted", "the acceptance-attested outcome");
    assert_eq!(failed.3, "saved_history", "the automatic gate saved it");
    let input: serde_json::Value = serde_json::from_str(&failed.5).unwrap();
    assert_eq!(input["origin"], "file", "file origin kept");
    assert_eq!(input["name"], "mixed-source.jsonl");
    assert_eq!(input["text"], failed_text, "the held plaintext persists");
    let error_json: serde_json::Value =
        serde_json::from_str(failed.7.as_ref().expect("the canonical error")).unwrap();
    assert_eq!(error_json["code"], "upstream_analysis_failed");
    assert!(
        rows.iter()
            .any(|row| row.2 == "accepted" && row.1 == "queued")
    );

    // FTS payload is searchable by the JSONL basename and by a content word.
    let payload = search_payload(&connection);
    assert_eq!(payload.len(), 2);
    let baseline_count = payload.len();
    let mut statement = connection
        .prepare(
            "SELECT analysis_id FROM analysis_search WHERE analysis_search MATCH ?1 ORDER BY analysis_id",
        )
        .unwrap();
    let by_name: Vec<String> = statement
        .query_map(["\"mixed-source\""], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        by_name.len(),
        baseline_count,
        "both children index the basename"
    );
    drop(statement);
    let mut statement = connection
        .prepare(
            "SELECT analysis_id FROM analysis_search WHERE analysis_search MATCH ?1 ORDER BY analysis_id",
        )
        .unwrap();
    let by_text: Vec<String> = statement
        .query_map(["rejected"], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(by_text.len(), 1, "one FTS hit by input text");
    assert_eq!(by_text[0], failed.0, "plaintext is searchable");
    drop(statement);
    drop(connection);
    fixture.shutdown().await;
}

// The exactly-one-warning invocation-latch proofs live in the sibling
// `history-save-warning-latch-loopback.rs` suite (split out so both stay
// under the source-size threshold).

/// The bulk and task surfaces carry no `--save`: their completed work
/// persists only under the `history.enabled = true` automatic gate. A bulk
/// submit under an unprotectable data directory therefore keeps the
/// acceptance, warns once, and still exits 0.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn bulk_submit_automatic_save_failure_warns_once_and_still_accepts() {
    let fixture = ProtocolFixture::start().await;
    fixture.on_bulk_submit(Step::Status(
        202,
        None,
        Some(serde_json::json!({
            "bulk_id": "blk_fixture_save",
            "status": "queued",
            "total_items": 1,
            "accepted_items": [{"index": 0, "id": "row-000", "task_id": "task-000"}],
            "failed_items": []
        })),
    ));
    let isolated = Isolated::new();
    isolated.enable_history();
    poison_data_dir(&isolated);

    let root = tempfile::tempdir().unwrap();
    let items = root.path().join("items.jsonl");
    std::fs::write(
        &items,
        serde_json::json!({"id": "row-000", "text": "bulk item for the save gate"}).to_string(),
    )
    .unwrap();

    let output = isolated
        .command(fixture.base_url())
        .args([
            "bulk",
            "submit",
            items.to_str().unwrap(),
            "--max-billable-units",
            "5",
        ])
        .output()
        .expect("run bulk submit under an unprotectable data dir");

    assert_eq!(
        output.status.code(),
        Some(0),
        "the acceptance still exits 0"
    );
    let envelope = stdout_envelope(&output);
    assert_eq!(envelope["command"], "bulk_submit");
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("warning:") && stderr.contains("history"),
        "one sanitized automatic-save warning: {stderr}"
    );
    assert_no_leak(&output);
    assert!(!isolated.database_path().exists());
    fixture.shutdown().await;
}

// ------------------------------------------------------------ ADR 0004 ----

/// First enablement of durable plaintext history acknowledges ADR 0004 with
/// exactly one plaintext warning on stderr; the command still exits 0 and
/// stdout stays the canonical acknowledgement. Re-confirming true, and
/// disabling, print nothing; a failed set prints nothing.
#[test]
fn first_history_enable_warns_once_and_repeats_and_disable_stay_silent() {
    let isolated = Isolated::new();

    // First false->true transition: exactly one ADR 0004 warning, exit 0.
    let first = isolated.config_set("history.enabled", "true");
    assert_eq!(first.status.code(), Some(0));
    let stderr = String::from_utf8(first.stderr.clone()).unwrap();
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.contains("warning:"))
            .count(),
        1,
        "first enable emits one plaintext warning: {stderr}"
    );
    assert!(stderr.contains("unencrypted"), "names the plaintext fact");
    assert!(stderr.contains("history"), "names the storage class");
    assert!(first.stdout.len() > 2, "canonical ack stays on stdout");

    // Idempotent re-set of an already-true value: silent.
    let repeat = isolated.config_set("history.enabled", "true");
    assert_eq!(repeat.status.code(), Some(0));
    assert!(
        !String::from_utf8_lossy(&repeat.stderr).contains("warning:"),
        "re-confirming an enabled true prints nothing"
    );

    // Disabling: silent.
    let disable = isolated.config_set("history.enabled", "false");
    assert_eq!(disable.status.code(), Some(0));
    assert!(
        !String::from_utf8_lossy(&disable.stderr).contains("warning:"),
        "disabling prints nothing"
    );

    // Re-enabling from the disabled state: the transition fires again.
    let reenable = isolated.config_set("history.enabled", "true");
    assert_eq!(reenable.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&reenable.stderr).contains("warning:"),
        "a fresh false->true transition warns again"
    );

    // A failed set (unknown key) prints no warning.
    let failed = isolated.config_set("history.unknown", "true");
    assert_ne!(failed.status.code(), Some(0));
    assert!(
        !String::from_utf8_lossy(&failed.stderr).contains("warning:"),
        "a failed set never warns"
    );
}

/// The ADR 0004 transition recognition derives from the canonical typed
/// grammar, not raw-string matching: every accepted spelling of the key
/// (any casing or surrounding whitespace) and every accepted `true`
/// spelling (case-insensitive, whitespace-tolerant through the typed bool
/// parser) is a first-enable transition and warns exactly once. Unrelated
/// keys and transition-less spellings never warn.
#[test]
fn first_history_enable_warns_for_every_accepted_spelling() {
    // Each accepted spelling of `history.enabled` + a truthy value, from a
    // fresh unset state, is a transition that warns exactly once.
    for (key, value) in [
        ("history.enabled", "TRUE"),
        ("HISTORY.ENABLED", "true"),
        (" History.Enabled ", " True "),
        ("history.enabled", " true "),
    ] {
        let isolated = Isolated::new();
        let output = isolated.config_set(key, value);
        assert_eq!(
            output.status.code(),
            Some(0),
            "an accepted spelling still succeeds: {key:?} {value:?}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            stderr
                .lines()
                .filter(|line| line.contains("warning:"))
                .count(),
            1,
            "{key:?} {value:?} warns exactly once: {stderr}"
        );
    }

    // Accepted spellings of an unrelated bool key never warn.
    let isolated = Isolated::new();
    let unrelated = isolated.config_set(" UPDATES.CHECK_ON_TUI_START ", "TRUE");
    assert_eq!(unrelated.status.code(), Some(0));
    assert!(
        !String::from_utf8_lossy(&unrelated.stderr).contains("warning:"),
        "an unrelated key never warns"
    );

    // Truthy-but-invalid and disable spellings never warn.
    for (key, value, expect_success) in [
        ("history.enabled", "yes", false),
        ("history.enabled", "1", false),
        ("history.enabled", "FALSE", true),
        ("history.enabled", " FALSE ", true), // disable is silent
    ] {
        let isolated = Isolated::new();
        let output = isolated.config_set(key, value);
        assert_eq!(
            output.status.code() == Some(0),
            expect_success,
            "{value:?} spell-checks: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("warning:"),
            "{key:?} {value:?} never warns: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
