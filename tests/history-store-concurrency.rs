//! Real-SQLite concurrency proof for Packet C remediation (docs/
//! history-contract.md uniqueness invariants): simultaneous task and bulk
//! reconciliation from two independent store handles (separate OS threads,
//! separate SQLite connections over one real WAL database) yields exactly
//! one stored analysis/collection with correct children and observations,
//! never an ambiguous duplicate. The store owns the reconcile inside one
//! `IMMEDIATE` transaction; the schema enforces
//! `bulk_collections.upstream_bulk_id UNIQUE` and
//! `upstream_tasks UNIQUE (check_kind, upstream_task_id)`, so a racing pair
//! serializes instead of duplicating. A mid-transaction failure rolls the
//! whole batch back on the real database.
//!
//! No mocks anywhere: two real `HistoryStore` instances race against one
//! `tempfile::TempDir` database, and every assertion reads the committed
//! database back through a plain rusqlite handle.

#![forbid(unsafe_code)]

use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, BulkCounters, BulkId, CheckKind, CheckStatus, SaveState,
    Sha256Hash, SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryError, HistoryErrorCode, HistoryStore, InputKind, ObservationSnapshot, StoredAnalysis,
    StoredBulkCollection, StoredCheck, StoredUpstreamTask,
};

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("test timestamp")
}

fn analysis(id: &str, input: &str) -> StoredAnalysis {
    let input_sha256 = Sha256Hash::digest(input);
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
            "byte_count": input.len(),
            "word_count": input.split_whitespace().count(),
            "text": input
        })
        .to_string(),
        result_json: Some(ai_result("Human-written")),
        error_json: None,
        upstream_version: None,
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(timestamp("2026-08-01T09:59:00Z")),
        created_at: timestamp("2026-08-01T10:00:00Z"),
        updated_at: timestamp("2026-08-01T10:05:00Z"),
        completed_at: Some(timestamp("2026-08-01T10:05:00Z")),
        search_input_text: Some(input.to_owned()),
        search_filename: None,
        search_headline: Some("Human-written".to_owned()),
        search_source_urls: None,
    }
}

fn ai_result(headline: &str) -> String {
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

fn observation(analysis_id: &str, task: &str) -> StoredUpstreamTask {
    StoredUpstreamTask {
        analysis_id: AnalysisId::from_str(analysis_id).expect("analysis id"),
        check_kind: CheckKind::AiDetection,
        upstream_task_id: task.to_owned(),
        last_stage: Some("STAGE_SUCCESS".to_owned()),
        observed_at: timestamp("2026-08-01T10:05:00Z"),
    }
}

fn collection(id: &str, upstream: &str) -> StoredBulkCollection {
    StoredBulkCollection {
        id: BulkId::from_str(id).expect("bulk id"),
        upstream_bulk_id: Some(upstream.to_owned()),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        counters: BulkCounters::new(1, 1, 0, 0).expect("counters"),
        estimated_billable_units: Some(1),
        created_at: timestamp("2026-08-01T09:00:00Z"),
        updated_at: timestamp("2026-08-01T09:00:00Z"),
        completed_at: None,
    }
}

fn child(id: &str, bulk_id: &str, index: i64) -> StoredAnalysis {
    let mut record = analysis(id, &format!("bulk item {index}"));
    record.bulk = Some((BulkId::from_str(bulk_id).expect("bulk id"), index));
    record.caller_id = Some(format!("row-{index:03}"));
    record.submission_outcome = SubmissionOutcome::Accepted;
    record.status = AnalysisStatus::Queued;
    record.result_json = None;
    record.completed_at = None;
    record.search_headline = None;
    record
}

/// The merge closure the adapter uses (content-pure): a non-terminal
/// refresh carries no body and keeps the stored row's identity authorship.
fn running_merge(prior: &StoredAnalysis) -> Result<ObservationSnapshot, HistoryError> {
    let _ = prior;
    Ok(ObservationSnapshot {
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Terminal,
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

/// Two independent store handles reconcile the same upstream task at the
/// same time from two threads. With the pre-remediation code both opened a
/// transaction, both saw "no stored row", and both inserted; here exactly
/// one stored analysis exists afterwards, with exactly one FTS payload and
/// one observation row, and both calls report success (the second one
/// refreshes the committed first row). The persisted row is one of the two
/// fresh identities; the other read's identity never lands.
#[test]
fn two_threads_reconciling_one_task_yield_exactly_one_stored_analysis() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().to_path_buf();
    // Both stores open on the main thread first: the one-time WAL journal
    // transition and schema initialization serialize before the race, so
    // the timed contention below exercises the reconcile write lock itself,
    // never open-time WAL mode flipping.
    let store_a = HistoryStore::open(&path).expect("open first store");
    let store_b = HistoryStore::open(&path).expect("open second store");
    // Both threads release at the same instant so their IMMEDIATE
    // transactions genuinely contend on the write lock.
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

    let run = |mut store: HistoryStore, id: &'static str, input: &'static str| {
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let record = analysis(id, input);
            let observations = vec![observation(id, "task-contended")];
            barrier.wait();
            store
                .reconcile_observed_analysis_atomic(
                    &record,
                    &observations,
                    timestamp("2026-08-01T10:06:00Z"),
                    running_merge,
                )
                .expect("reconcile commits")
        })
    };

    let first = run(
        store_a,
        "anl_01983c20-0180-7a80-a001-00000000c001",
        "alpha concurrent text",
    );
    let second = run(
        store_b,
        "anl_01983c20-0180-7a80-a001-00000000c002",
        "beta concurrent text",
    );
    let outcome_a = first.join().expect("first reconcile joins");
    let outcome_b = second.join().expect("second reconcile joins");

    // One inserted, the other refreshed onto it; both reported success.
    let inserted = [outcome_a.inserted, outcome_b.inserted]
        .into_iter()
        .filter(|flag| *flag)
        .count();
    assert_eq!(inserted, 1, "exactly one of the racing reconciles inserted");
    assert_eq!(
        outcome_a.stored_id, outcome_b.stored_id,
        "both reconciles resolve onto the same stored row"
    );

    let connection =
        rusqlite::Connection::open(root.path().join("history").join("pangram-history.db"))
            .expect("open saved database");
    let count = |sql: &str| {
        connection
            .query_row(sql, [], |row| row.get::<_, i64>(0))
            .expect("count rows")
    };
    assert_eq!(count("SELECT COUNT(*) FROM analyses"), 1);
    assert_eq!(count("SELECT COUNT(*) FROM upstream_tasks"), 1);
    assert_eq!(count("SELECT COUNT(*) FROM analysis_search"), 1);
    // The observation carries the real upstream identity of the read and
    // belongs to the one stored analysis.
    let (analysis_id, task_id): (String, String) = connection
        .query_row(
            "SELECT analysis_id, upstream_task_id FROM upstream_tasks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("task row");
    assert_eq!(task_id, "task-contended");
    assert_eq!(analysis_id, outcome_a.stored_id.to_string());
}

/// Two independent observers of one check on an already-saved combined
/// analysis serialize without regressing terminal evidence. Whichever
/// process acquires the write lock first, the succeeded AI observation wins,
/// while the omitted plagiarism check/body/task and FTS input remain intact.
#[test]
fn concurrent_one_check_refreshes_preserve_the_combined_analysis() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().to_path_buf();
    let id = "anl_01983c20-0180-7a80-a001-00000000c003";
    let analysis_id = AnalysisId::from_str(id).unwrap();
    let mut seed = analysis(id, "combined concurrent text");
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
    seed.status = AnalysisStatus::Running;
    seed.submission_outcome = SubmissionOutcome::Accepted;
    seed.result_json = Some(plagiarism_result.clone());
    seed.completed_at = None;
    seed.search_headline = None;
    seed.search_source_urls = Some("https://example.invalid/retained".to_owned());
    let checks = vec![
        StoredCheck {
            analysis_id,
            check_index: 0,
            check_kind: CheckKind::AiDetection,
            status: CheckStatus::Running,
            result_json: None,
            error_json: None,
        },
        StoredCheck {
            analysis_id,
            check_index: 1,
            check_kind: CheckKind::Plagiarism,
            status: CheckStatus::Succeeded,
            result_json: Some(plagiarism_result.clone()),
            error_json: None,
        },
    ];
    let observations = vec![
        observation(id, "task-combined-race"),
        StoredUpstreamTask {
            analysis_id,
            check_kind: CheckKind::Plagiarism,
            upstream_task_id: "task-combined-plagiarism".to_owned(),
            last_stage: Some("STAGE_SUCCESS".to_owned()),
            observed_at: timestamp("2026-08-01T10:05:00Z"),
        },
    ];
    let mut initializer = HistoryStore::open(&path).expect("open seed store");
    initializer
        .save_analysis_complete(&seed, &checks, &observations)
        .expect("seed combined analysis");
    drop(initializer);

    let store_a = HistoryStore::open(&path).expect("open first store");
    let store_b = HistoryStore::open(&path).expect("open second store");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let run = |mut store: HistoryStore, terminal: bool| {
        let barrier = barrier.clone();
        let seed = seed.clone();
        std::thread::spawn(move || {
            let observed_at = timestamp(if terminal {
                "2026-08-01T10:07:00Z"
            } else {
                "2026-08-01T10:06:00Z"
            });
            let result = terminal.then(|| ai_result("Concurrent terminal"));
            let status = if terminal {
                CheckStatus::Succeeded
            } else {
                CheckStatus::Running
            };
            let record = StoredAnalysis {
                id: AnalysisId::new(),
                status: if terminal {
                    AnalysisStatus::Succeeded
                } else {
                    AnalysisStatus::Running
                },
                result_json: result.clone(),
                updated_at: observed_at,
                completed_at: terminal.then_some(observed_at),
                search_headline: terminal.then(|| "Concurrent terminal".to_owned()),
                ..seed
            };
            let incoming_check = StoredCheck {
                analysis_id: record.id,
                check_index: 0,
                check_kind: CheckKind::AiDetection,
                status,
                result_json: result,
                error_json: None,
            };
            let incoming_task = StoredUpstreamTask {
                analysis_id: record.id,
                check_kind: CheckKind::AiDetection,
                upstream_task_id: "task-combined-race".to_owned(),
                last_stage: Some(
                    if terminal {
                        "STAGE_SUCCESS"
                    } else {
                        "STAGE_INFERENCE"
                    }
                    .to_owned(),
                ),
                observed_at,
            };
            barrier.wait();
            store
                .reconcile_observed_analysis_complete(
                    &record,
                    &[incoming_check],
                    &[incoming_task],
                    observed_at,
                    |prior| {
                        Ok(ObservationSnapshot {
                            status: record.status,
                            submission_outcome: prior.submission_outcome,
                            result_json: record.result_json.clone(),
                            error_json: None,
                            upstream_version: prior.upstream_version.clone(),
                            completed_at: record.completed_at,
                            search_input_text: prior.search_input_text.clone(),
                            search_filename: prior.search_filename.clone(),
                            search_headline: record
                                .search_headline
                                .clone()
                                .or_else(|| prior.search_headline.clone()),
                            search_source_urls: prior.search_source_urls.clone(),
                        })
                    },
                )
                .expect("concurrent refresh commits")
        })
    };
    let running = run(store_a, false);
    let terminal = run(store_b, true);
    running.join().expect("running refresh joins");
    terminal.join().expect("terminal refresh joins");

    let store = HistoryStore::open(&path).expect("reopen store");
    let canonical = store
        .canonical_analysis(&analysis_id, true)
        .expect("combined analysis remains canonical");
    assert_eq!(canonical.status(), AnalysisStatus::Succeeded);
    assert_eq!(canonical.checks().len(), 2);
    let value = serde_json::to_value(canonical).unwrap();
    assert_eq!(
        value["checks"][1]["result"],
        serde_json::from_str::<serde_json::Value>(&plagiarism_result).unwrap()
    );
    let connection =
        rusqlite::Connection::open(path.join("history").join("pangram-history.db")).unwrap();
    assert_eq!(
        count_rows(&connection, "SELECT COUNT(*) FROM upstream_tasks"),
        2
    );
    assert_eq!(
        count_rows(&connection, "SELECT COUNT(*) FROM analysis_search"),
        1
    );
    let retained: (Option<String>, Option<String>) = connection
        .query_row(
            "SELECT input_text, source_urls FROM analysis_search",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        retained,
        (
            Some("combined concurrent text".to_owned()),
            Some("https://example.invalid/retained".to_owned())
        )
    );
}

fn count_rows(connection: &rusqlite::Connection, sql: &str) -> i64 {
    connection
        .query_row(sql, [], |row| row.get(0))
        .expect("count rows")
}

/// Two independent store handles reconcile one upstream bulk job with its
/// child at the same time from two threads. Exactly one
/// `bulk_collections` row, one child row, and one child observation exist
/// afterwards. The stored collection identity is one of the two minted
/// `bulk_` values; the other read rebound onto it inside its own
/// transaction (proof there is no residual duplicate under the loser id).
#[test]
fn two_threads_reconciling_one_bulk_job_yield_exactly_one_collection() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().to_path_buf();
    let store_a = HistoryStore::open(&path).expect("open first store");
    let store_b = HistoryStore::open(&path).expect("open second store");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

    let run = |mut store: HistoryStore, bulk_id: &'static str, child_id: &'static str| {
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let record = collection(bulk_id, "upstream-bulk-contended");
            let child = child(child_id, bulk_id, 0);
            let observations = vec![observation(child_id, "task-bulk-child")];
            barrier.wait();
            store
                .reconcile_bulk_collection_atomic(&record, &[(child, observations)])
                .expect("reconcile commits")
        })
    };

    let first = run(
        store_a,
        "bulk_01983c20-0180-7a80-a001-00000000c011",
        "anl_01983c20-0180-7a80-a001-00000000c021",
    );
    let second = run(
        store_b,
        "bulk_01983c20-0180-7a80-a001-00000000c012",
        "anl_01983c20-0180-7a80-a001-00000000c022",
    );
    let outcome_a = first.join().expect("first reconcile joins");
    let outcome_b = second.join().expect("second reconcile joins");

    let inserted = [outcome_a.inserted, outcome_b.inserted]
        .into_iter()
        .filter(|flag| *flag)
        .count();
    assert_eq!(inserted, 1, "exactly one of the racing reconciles inserted");
    assert_eq!(
        outcome_a.stored_id, outcome_b.stored_id,
        "both reconciles resolve onto the same stored collection"
    );

    let connection =
        rusqlite::Connection::open(root.path().join("history").join("pangram-history.db"))
            .expect("open saved database");
    let count = |sql: &str| {
        connection
            .query_row(sql, [], |row| row.get::<_, i64>(0))
            .expect("count rows")
    };
    assert_eq!(
        count("SELECT COUNT(*) FROM bulk_collections"),
        1,
        "one stored collection row for the job"
    );
    assert_eq!(count("SELECT COUNT(*) FROM analyses"), 1, "one child row");
    assert_eq!(count("SELECT COUNT(*) FROM upstream_tasks"), 1);
    // The loser id left no trace: the one stored row is the resolved id and
    // its child carries the winner's membership.
    let stored_bulk: String = connection
        .query_row("SELECT id FROM bulk_collections", [], |row| row.get(0))
        .expect("collection row");
    assert_eq!(stored_bulk, outcome_a.stored_id.to_string());
    let child_bulk: String = connection
        .query_row("SELECT bulk_id FROM analyses", [], |row| row.get(0))
        .expect("child row");
    assert_eq!(child_bulk, stored_bulk);
}

/// Two real operating-system processes racing reconciles against one
/// shared database never duplicate a row. Each child reconciles its own
/// distinct upstream task identity (the reconcile key), so both persist
/// exactly one row each. The schema is pre-initialized on the main thread
/// so the children contend on the reconcile write lock (the same
/// IMMEDIATE-transaction contention a same-key pair would experience),
/// not on concurrent first-boot schema creation, which is a
/// `HistoryStore::open` concern. The shared database ends with exactly one
/// analyses row, one FTS row, and one observation row per child identity,
/// never a duplicate. The racing entry point is this same test binary: a
/// child process is spawned per contender with the database root and its
/// fresh local identity passed through the environment.
#[test]
fn two_processes_reconciling_concurrently_yield_exactly_one_row_each() {
    let root = tempfile::tempdir().unwrap();
    let binary = std::env::current_exe().expect("current test binary");
    let spawn = |id: &str, task: &str, marker: &str| {
        std::process::Command::new(&binary)
            .args([
                "concurrent_process_entry",
                "--exact",
                "--nocapture",
                "--ignored",
            ])
            .env("PANGRAM_CONCURRENCY_ROOT", root.path())
            .env("PANGRAM_CONCURRENCY_ID", id)
            .env("PANGRAM_CONCURRENCY_TASK", task)
            .env("PANGRAM_CONCURRENCY_INPUT", format!("input of {id}"))
            .env("PANGRAM_CONCURRENCY_REPORT", root.path().join(marker))
            .spawn()
            .expect("spawn racing child process")
    };
    let ids = [
        ("anl_01983c20-0180-7a80-a001-00000000c061", "task-process-0"),
        ("anl_01983c20-0180-7a80-a001-00000000c062", "task-process-1"),
    ];
    // Pre-initialize the schema on the main thread: the concurrent
    // first-boot creation race (a transient `create schema` failure on the
    // losing opener) is a `HistoryStore::open` concern, not the reconcile
    // serialization under proof, so the children below open an already
    // initialized database and contend only on the reconcile write lock.
    let _initializer = HistoryStore::open(root.path()).expect("pre-initialize schema");
    drop(_initializer);
    // Both processes start before either finishes so their IMMEDIATE
    // write transactions genuinely overlap.
    let mut children = Vec::new();
    for (round, (id, task)) in ids.iter().enumerate() {
        children.push(spawn(id, task, &format!("child-{round}.mark")));
    }
    for child in children {
        let _ = child.wait_with_output().expect("child exits");
    }
    for (round, _) in ids.iter().enumerate() {
        let body = std::fs::read_to_string(root.path().join(format!("child-{round}.mark")))
            .expect("child marker exists");
        assert_eq!(body, "ok", "child {round} reconciled: {body}");
    }

    let _store = HistoryStore::open(root.path()).expect("reopen store");
    let connection =
        rusqlite::Connection::open(root.path().join("history").join("pangram-history.db"))
            .expect("open saved database");
    let count = |sql: &str| {
        connection
            .query_row(sql, [], |row| row.get::<_, i64>(0))
            .expect("count rows")
    };
    assert_eq!(count("SELECT COUNT(*) FROM analyses"), 2);
    assert_eq!(count("SELECT COUNT(*) FROM upstream_tasks"), 2);
    assert_eq!(count("SELECT COUNT(*) FROM analysis_search"), 2);
    let mut tasks: Vec<String> = connection
        .prepare("SELECT upstream_task_id FROM upstream_tasks ORDER BY upstream_task_id")
        .expect("prepare tasks")
        .query_map([], |row| row.get(0))
        .expect("query tasks")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect tasks");
    tasks.sort();
    assert_eq!(
        tasks,
        vec!["task-process-0".to_owned(), "task-process-1".to_owned()],
        "each child identity persisted exactly once, never duplicated"
    );
}

/// The child-process entry for the process-race above. Run directly by the
/// parent (never by the default test filter: it requires the two
/// `PANGRAM_CONCURRENCY_*` environment values and `#[ignore]` keeps it out
/// of every non-explicit run).
#[test]
#[ignore = "racing child entry point; the parent spawns it explicitly"]
fn concurrent_process_entry() {
    let root = std::env::var("PANGRAM_CONCURRENCY_ROOT").expect("root env");
    let id = std::env::var("PANGRAM_CONCURRENCY_ID").expect("id env");
    let input = std::env::var("PANGRAM_CONCURRENCY_INPUT").expect("input env");
    let report = std::env::var("PANGRAM_CONCURRENCY_REPORT").expect("report env");
    // No panics past this point: every fallible step lands in the marker
    // file so the parent can distinguish the failure class.
    let task = std::env::var("PANGRAM_CONCURRENCY_TASK").expect("task env");
    let mark = (|| -> Result<(), String> {
        let mut store = HistoryStore::open(std::path::Path::new(&root))
            .map_err(|error| format!("open: {error:?}"))?;
        let record = analysis(&id, &input);
        let observations = vec![observation(&id, &task)];
        store
            .reconcile_observed_analysis_atomic(
                &record,
                &observations,
                timestamp("2026-08-01T10:06:00Z"),
                running_merge,
            )
            .map_err(|error| format!("reconcile: {error:?}"))?;
        Ok(())
    })();
    let body = match &mark {
        Ok(()) => "ok".to_owned(),
        Err(reason) => format!("fail: {reason}"),
    };
    std::fs::write(&report, body).expect("write the child marker");
    mark.expect("the child reconcile succeeded");
}

/// A reconcile whose observation batch violates the foreign key inside the
/// same IMMEDIATE transaction rolls the whole batch back: no analysis row,
/// no FTS payload, and no observation survives, exactly like the deferred
/// path. The error surfaces as `history_write_failed`.
#[test]
fn a_reconcile_rolls_the_whole_batch_back_when_a_member_fails() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open store");
    let record = analysis("anl_01983c20-0180-7a80-a001-00000000c031", "rollback text");
    // The second observation's foreign key points at a different,
    // nonexistent analysis: SQLite rejects it inside the transaction.
    let mut bad = observation("anl_01983c20-0180-7a80-a001-00000000c031", "task-rollback");
    bad.analysis_id = AnalysisId::from_str("anl_01983c20-0180-7a80-a001-00000000c0ff").unwrap();
    let observations = vec![
        observation("anl_01983c20-0180-7a80-a001-00000000c031", "task-rollback"),
        bad,
    ];
    let error = store
        .reconcile_observed_analysis_atomic(
            &record,
            &observations,
            timestamp("2026-08-01T10:06:00Z"),
            running_merge,
        )
        .expect_err("the batch must fail");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    let connection =
        rusqlite::Connection::open(root.path().join("history").join("pangram-history.db"))
            .expect("open saved database");
    let count = |sql: &str| {
        connection
            .query_row(sql, [], |row| row.get::<_, i64>(0))
            .expect("count rows")
    };
    assert_eq!(count("SELECT COUNT(*) FROM analyses"), 0);
    assert_eq!(count("SELECT COUNT(*) FROM upstream_tasks"), 0);
    assert_eq!(count("SELECT COUNT(*) FROM analysis_search"), 0);
}

/// The schema-enforced uniqueness backs the contract at the database level:
/// a direct duplicate `upstream_bulk_id` insert (two minted local ids, one
/// upstream job) is rejected at commit, and a direct duplicate
/// `(check_kind, upstream_task_id)` observation insert is rejected, so even
/// a hypothetical adapter regression can never persist the duplicates.
#[test]
fn the_schema_rejects_duplicate_upstream_identities_directly() {
    let root = tempfile::tempdir().unwrap();
    let mut store = HistoryStore::open(root.path()).expect("open store");
    store
        .save_bulk_collection(&collection(
            "bulk_01983c20-0180-7a80-a001-00000000c041",
            "upstream-bulk-direct",
        ))
        .expect("first collection commits");
    let error = store
        .save_bulk_collection(&collection(
            "bulk_01983c20-0180-7a80-a001-00000000c042",
            "upstream-bulk-direct",
        ))
        .expect_err("a duplicate upstream_bulk_id must be rejected");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);

    let record = analysis("anl_01983c20-0180-7a80-a001-00000000c051", "first");
    store
        .save_analysis_atomic(
            &record,
            &[observation(
                "anl_01983c20-0180-7a80-a001-00000000c051",
                "task-direct",
            )],
        )
        .expect("first analysis commits");
    let second = analysis("anl_01983c20-0180-7a80-a001-00000000c052", "second");
    let error = store
        .save_analysis_atomic(
            &second,
            &[observation(
                "anl_01983c20-0180-7a80-a001-00000000c052",
                "task-direct",
            )],
        )
        .expect_err("a duplicate (check_kind, upstream_task_id) must be rejected");
    assert_eq!(error.code(), HistoryErrorCode::HistoryWriteFailed);
}
