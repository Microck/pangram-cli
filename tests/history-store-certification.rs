//! Complete aggregate and foreign-key certification before destructive history mutation.
//!
//! Every case installs corruption through real SQLite, invokes one public
//! destructive surface, and proves the rejected transaction changed no logical
//! row. Foreign-key corruption is installed with enforcement disabled to
//! model an externally modified database.

#![forbid(unsafe_code)]

use std::str::FromStr;

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, BulkCounters, BulkId, CheckKind, CheckStatus, SaveState,
    Sha256Hash, SubmissionOutcome, UtcTimestamp,
};
use microck_pangram_cli::history::{
    HistoryError, HistoryErrorCode, HistoryExportError, HistoryExportFormat, HistoryStore,
    InputKind, ObservationSnapshot, StoredAnalysis, StoredBulkCollection, StoredCheck,
    StoredUpstreamTask, TerminalResult, export_history,
};
use microck_pangram_cli::output::{CanonicalError, ErrorCode};
use rusqlite::Connection;

const TASK_ID: &str = "anl_01983c20-0180-7a80-a001-00000000c001";
const BULK_ID: &str = "bulk_01983c20-0180-7a80-a001-00000000c002";
const MEMBER_ID: &str = "anl_01983c20-0180-7a80-a001-00000000c003";
const CARRIER_ID: &str = "anl_01983c20-0180-7a80-a001-00000000c004";
const FRESH_ID: &str = "anl_01983c20-0180-7a80-a001-00000000c005";
const FRESH_BULK_ID: &str = "bulk_01983c20-0180-7a80-a001-00000000c006";

#[derive(Clone, Copy, Debug)]
enum Mutation {
    LegacySave,
    AtomicSave,
    CompleteSave,
    RecordObservation,
    TerminalUpdate,
    CompleteTerminalUpdate,
    ObservationUpdate,
    TaskRefresh,
    TaskInsert,
    SaveBulkCollection,
    AtomicBulkUpsert,
    BulkRefresh,
    BulkInsert,
    BulkMemberInsert,
    BulkMemberCoalesce,
    Delete,
    Clear,
}

const MUTATIONS: [Mutation; 17] = [
    Mutation::LegacySave,
    Mutation::AtomicSave,
    Mutation::CompleteSave,
    Mutation::RecordObservation,
    Mutation::TerminalUpdate,
    Mutation::CompleteTerminalUpdate,
    Mutation::ObservationUpdate,
    Mutation::TaskRefresh,
    Mutation::TaskInsert,
    Mutation::SaveBulkCollection,
    Mutation::AtomicBulkUpsert,
    Mutation::BulkRefresh,
    Mutation::BulkInsert,
    Mutation::BulkMemberInsert,
    Mutation::BulkMemberCoalesce,
    Mutation::Delete,
    Mutation::Clear,
];

const DESTRUCTIVE_MUTATIONS: [Mutation; 2] = [Mutation::Delete, Mutation::Clear];

#[derive(Clone, Copy, Debug)]
enum AggregateCorruption {
    ResultMismatch,
    ResultMalformed,
    ErrorMismatch,
    SearchHeadline,
    SearchSourceUrls,
}

const AGGREGATE_CORRUPTIONS: [AggregateCorruption; 5] = [
    AggregateCorruption::ResultMismatch,
    AggregateCorruption::ResultMalformed,
    AggregateCorruption::ErrorMismatch,
    AggregateCorruption::SearchHeadline,
    AggregateCorruption::SearchSourceUrls,
];

#[derive(Clone, Copy, Debug)]
enum ForeignKeyCorruption {
    CheckOwner,
    TaskOwner,
    BulkMembership,
    RetryLineage,
    RerunLineage,
}

const FOREIGN_KEY_CORRUPTIONS: [ForeignKeyCorruption; 5] = [
    ForeignKeyCorruption::CheckOwner,
    ForeignKeyCorruption::TaskOwner,
    ForeignKeyCorruption::BulkMembership,
    ForeignKeyCorruption::RetryLineage,
    ForeignKeyCorruption::RerunLineage,
];

fn stamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("timestamp")
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

fn plagiarism_result() -> String {
    serde_json::json!({
        "plagiarism_detected": true,
        "total_sentences": 1,
        "plagiarized_sentence_count": 1,
        "percent_plagiarized": 100.0,
        "matches": [{
            "source_url": "https://canonical.example/source",
            "matched_text": "Synthetic match",
            "similarity_score": 1.0
        }]
    })
    .to_string()
}

fn canonical_error(message: &str) -> String {
    serde_json::to_string(
        &CanonicalError::new(ErrorCode::UpstreamAnalysisFailed, message).expect("canonical error"),
    )
    .expect("serialize error")
}

fn analysis(
    id: &str,
    bulk: Option<(&str, i64)>,
    failed: bool,
) -> (StoredAnalysis, Vec<StoredCheck>, Vec<StoredUpstreamTask>) {
    let id = AnalysisId::from_str(id).expect("analysis id");
    let text = format!("certification input {id}");
    let input_sha256 = Sha256Hash::digest(&text);
    let result = ai_result("Canonical headline");
    let plagiarism = plagiarism_result();
    let error = canonical_error("Canonical failure.");
    let status = if failed {
        AnalysisStatus::Failed
    } else {
        AnalysisStatus::Succeeded
    };
    let record = StoredAnalysis {
        id,
        bulk: bulk.map(|(bulk, index)| (BulkId::from_str(bulk).expect("bulk id"), index)),
        caller_id: bulk.map(|_| "row-000".to_owned()),
        status,
        submission_outcome: SubmissionOutcome::Terminal,
        save_state: SaveState::SavedHistory,
        input_kind: InputKind::Text,
        input_sha256,
        display_name: None,
        input_json: serde_json::json!({
            "type": "text",
            "origin": "literal",
            "sha256": input_sha256,
            "byte_count": text.len(),
            "word_count": text.split_whitespace().count(),
            "text": text
        })
        .to_string(),
        result_json: (!failed).then(|| result.clone()),
        error_json: failed.then(|| error.clone()),
        upstream_version: Some("4.0".to_owned()),
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(stamp("2026-08-01T09:59:00Z")),
        created_at: stamp("2026-08-01T10:00:00Z"),
        updated_at: stamp("2026-08-01T10:05:00Z"),
        completed_at: Some(stamp("2026-08-01T10:05:00Z")),
        search_input_text: Some(text),
        search_filename: None,
        search_headline: (!failed).then(|| "Canonical headline".to_owned()),
        search_source_urls: (!failed).then(|| "https://canonical.example/source".to_owned()),
    };
    let checks = if failed {
        vec![StoredCheck {
            analysis_id: id,
            check_index: 0,
            check_kind: CheckKind::AiDetection,
            status: CheckStatus::Failed,
            result_json: None,
            error_json: Some(error),
        }]
    } else {
        vec![
            StoredCheck {
                analysis_id: id,
                check_index: 0,
                check_kind: CheckKind::AiDetection,
                status: CheckStatus::Succeeded,
                result_json: Some(result),
                error_json: None,
            },
            StoredCheck {
                analysis_id: id,
                check_index: 1,
                check_kind: CheckKind::Plagiarism,
                status: CheckStatus::Succeeded,
                result_json: Some(plagiarism),
                error_json: None,
            },
        ]
    };
    let observations = vec![StoredUpstreamTask {
        analysis_id: id,
        check_kind: CheckKind::AiDetection,
        upstream_task_id: format!("task-{id}"),
        last_stage: Some(if failed { "FAILED" } else { "COMPLETE" }.to_owned()),
        observed_at: stamp("2026-08-01T10:05:00Z"),
    }];
    (record, checks, observations)
}

fn collection() -> StoredBulkCollection {
    StoredBulkCollection {
        id: BulkId::from_str(BULK_ID).expect("bulk id"),
        upstream_bulk_id: Some("certification-bulk".to_owned()),
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Accepted,
        counters: BulkCounters::new(1, 1, 1, 0).expect("counters"),
        estimated_billable_units: Some(1),
        created_at: stamp("2026-08-01T09:00:00Z"),
        updated_at: stamp("2026-08-01T10:05:00Z"),
        completed_at: Some(stamp("2026-08-01T10:05:00Z")),
    }
}

fn fresh_collection() -> StoredBulkCollection {
    StoredBulkCollection {
        id: BulkId::from_str(FRESH_BULK_ID).expect("fresh bulk id"),
        upstream_bulk_id: Some("fresh-certification-bulk".to_owned()),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        counters: BulkCounters::new(1, 0, 0, 0).expect("fresh counters"),
        estimated_billable_units: None,
        created_at: stamp("2026-08-02T09:00:00Z"),
        updated_at: stamp("2026-08-02T09:00:00Z"),
        completed_at: None,
    }
}

fn fresh_member_collection() -> StoredBulkCollection {
    StoredBulkCollection {
        counters: BulkCounters::new(1, 1, 0, 0).expect("fresh member counters"),
        ..fresh_collection()
    }
}

fn task_analysis() -> (StoredAnalysis, Vec<StoredCheck>, Vec<StoredUpstreamTask>) {
    let mut task = analysis(TASK_ID, None, false);
    task.1.truncate(1);
    task.0.search_source_urls = None;
    task
}

fn fresh_analysis() -> (StoredAnalysis, Vec<StoredCheck>, Vec<StoredUpstreamTask>) {
    let mut fresh = analysis(FRESH_ID, None, false);
    fresh.1.truncate(1);
    fresh.0.search_source_urls = None;
    fresh
}

fn seed_store(root: &tempfile::TempDir, failed_carrier: bool) -> HistoryStore {
    let mut store = HistoryStore::open(root.path()).expect("store");
    let task = task_analysis();
    store
        .save_analysis_complete(&task.0, &task.1, &task.2)
        .expect("task aggregate");
    let member = analysis(MEMBER_ID, Some((BULK_ID, 0)), false);
    store
        .reconcile_bulk_collection_complete(&collection(), &[member])
        .expect("bulk aggregate");
    let carrier = analysis(CARRIER_ID, None, failed_carrier);
    store
        .save_analysis_complete(&carrier.0, &carrier.1, &carrier.2)
        .expect("carrier aggregate");
    store
}

fn logical_state(store: &HistoryStore) -> Vec<String> {
    store
        .with_connection(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT 'b|' || quote(id) || '|' || quote(upstream_bulk_id) || '|' ||
                            quote(status) || '|' || quote(submission_outcome) || '|' ||
                            quote(total_items) || '|' || quote(accepted) || '|' ||
                            quote(succeeded) || '|' || quote(failed) || '|' ||
                            quote(estimated_billable_units) || '|' || quote(created_at) || '|' ||
                            quote(updated_at) || '|' || quote(completed_at)
                     FROM bulk_collections
                     UNION ALL
                     SELECT 'a|' || quote(id) || '|' || quote(bulk_id) || '|' ||
                            quote(bulk_index) || '|' || quote(caller_id) || '|' ||
                            quote(status) || '|' || quote(submission_outcome) || '|' ||
                            quote(save_state) || '|' || quote(input_type) || '|' ||
                            quote(input_sha256) || '|' || quote(display_name) || '|' ||
                            quote(input_json) || '|' || quote(check_count) || '|' ||
                            quote(result_json) || '|' || quote(error_json) || '|' ||
                            quote(upstream_version) || '|' || quote(retry_of) || '|' ||
                            quote(rerun_of) || '|' || quote(submitted_at) || '|' ||
                            quote(created_at) || '|' || quote(updated_at) || '|' ||
                            quote(completed_at)
                     FROM analyses
                     UNION ALL
                     SELECT 'c|' || quote(analysis_id) || '|' || quote(check_index) || '|' ||
                            quote(check_kind) || '|' || quote(status) || '|' ||
                            quote(result_json) || '|' || quote(error_json)
                     FROM analysis_checks
                     UNION ALL
                     SELECT 't|' || quote(analysis_id) || '|' || quote(check_kind) || '|' ||
                            quote(upstream_task_id) || '|' || quote(last_stage) || '|' ||
                            quote(observed_at)
                     FROM upstream_tasks
                     UNION ALL
                     SELECT 's|' || quote(analysis_id) || '|' || quote(input_text) || '|' ||
                            quote(filename) || '|' || quote(headline) || '|' ||
                            quote(source_urls)
                     FROM analysis_search
                     ORDER BY 1",
                )
                .expect("prepare state");
            statement
                .query_map([], |row| row.get(0))
                .expect("query state")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect state")
        })
        .expect("read state")
}

fn invoke_mutation(store: &mut HistoryStore, mutation: Mutation) -> Result<(), HistoryError> {
    match mutation {
        Mutation::LegacySave => {
            let fresh = fresh_analysis();
            store.save_analysis(&fresh.0)
        }
        Mutation::AtomicSave => {
            let fresh = fresh_analysis();
            store.save_analysis_atomic(&fresh.0, &fresh.2)
        }
        Mutation::CompleteSave => {
            let fresh = fresh_analysis();
            store.save_analysis_complete(&fresh.0, &fresh.1, &fresh.2)
        }
        Mutation::RecordObservation => {
            let task = task_analysis();
            store.record_observation(&task.2[0])
        }
        Mutation::TerminalUpdate => {
            let task = task_analysis();
            store.update_terminal_result(&task.0.id, &terminal_result(&task.0))
        }
        Mutation::CompleteTerminalUpdate => {
            let task = task_analysis();
            store.update_terminal_result_complete(&task.0.id, &terminal_result(&task.0), &task.1)
        }
        Mutation::ObservationUpdate => {
            let task = task_analysis();
            store.update_observation_snapshot(
                &task.0.id,
                stamp("2026-08-02T10:00:00Z"),
                &observation_snapshot(&task.0),
            )
        }
        Mutation::TaskRefresh => {
            let incoming = task_analysis();
            store
                .reconcile_observed_analysis_complete(
                    &incoming.0,
                    &incoming.1,
                    &incoming.2,
                    stamp("2026-08-02T10:00:00Z"),
                    |prior| {
                        Ok(ObservationSnapshot {
                            status: prior.status,
                            submission_outcome: prior.submission_outcome,
                            result_json: prior.result_json.clone(),
                            error_json: prior.error_json.clone(),
                            upstream_version: prior.upstream_version.clone(),
                            completed_at: prior.completed_at,
                            search_input_text: prior.search_input_text.clone(),
                            search_filename: prior.search_filename.clone(),
                            search_headline: prior.search_headline.clone(),
                            search_source_urls: prior.search_source_urls.clone(),
                        })
                    },
                )
                .map(|_| ())
        }
        Mutation::TaskInsert => {
            let incoming = fresh_analysis();
            store
                .reconcile_observed_analysis_complete(
                    &incoming.0,
                    &incoming.1,
                    &incoming.2,
                    stamp("2026-08-02T10:00:00Z"),
                    |prior| Ok(observation_snapshot(prior)),
                )
                .map(|_| ())
        }
        Mutation::SaveBulkCollection => store.save_bulk_collection(&fresh_collection()),
        Mutation::AtomicBulkUpsert => store.upsert_bulk_collection_atomic(&fresh_collection(), &[]),
        Mutation::BulkRefresh => {
            let member = analysis(MEMBER_ID, Some((BULK_ID, 0)), false);
            store
                .reconcile_bulk_collection_complete(&collection(), &[member])
                .map(|_| ())
        }
        Mutation::BulkInsert => store
            .reconcile_bulk_collection_complete(&fresh_collection(), &[])
            .map(|_| ()),
        Mutation::BulkMemberInsert => {
            let member = analysis(FRESH_ID, Some((FRESH_BULK_ID, 0)), false);
            store
                .reconcile_bulk_collection_complete(&fresh_member_collection(), &[member])
                .map(|_| ())
        }
        Mutation::BulkMemberCoalesce => {
            let mut incoming = analysis(FRESH_ID, Some((FRESH_BULK_ID, 0)), false);
            incoming.2[0].upstream_task_id = format!(
                "task-{}",
                AnalysisId::from_str(TASK_ID).expect("task analysis id")
            );
            store
                .reconcile_bulk_collection_complete(&fresh_member_collection(), &[incoming])
                .map(|_| ())
        }
        Mutation::Delete => {
            store.delete_analysis(&AnalysisId::from_str(MEMBER_ID).expect("delete target"))
        }
        Mutation::Clear => store.clear(),
    }
}

fn terminal_result(record: &StoredAnalysis) -> TerminalResult {
    TerminalResult {
        status: record.status,
        submission_outcome: record.submission_outcome,
        result_json: record.result_json.clone(),
        error_json: record.error_json.clone(),
        upstream_version: record.upstream_version.clone(),
        completed_at: stamp("2026-08-02T10:00:00Z"),
        search_input_text: record.search_input_text.clone(),
        search_filename: record.search_filename.clone(),
        search_headline: record.search_headline.clone(),
        search_source_urls: record.search_source_urls.clone(),
    }
}

fn observation_snapshot(record: &StoredAnalysis) -> ObservationSnapshot {
    ObservationSnapshot {
        status: record.status,
        submission_outcome: record.submission_outcome,
        result_json: record.result_json.clone(),
        error_json: record.error_json.clone(),
        upstream_version: record.upstream_version.clone(),
        completed_at: Some(stamp("2026-08-02T10:00:00Z")),
        search_input_text: record.search_input_text.clone(),
        search_filename: record.search_filename.clone(),
        search_headline: record.search_headline.clone(),
        search_source_urls: record.search_source_urls.clone(),
    }
}

fn install_aggregate_corruption(store: &HistoryStore, corruption: AggregateCorruption) {
    store
        .with_connection(|connection| match corruption {
            AggregateCorruption::ResultMismatch => connection.execute(
                "UPDATE analyses SET result_json = ?2 WHERE id = ?1",
                rusqlite::params![CARRIER_ID, ai_result("Wrong parent result")],
            ),
            AggregateCorruption::ResultMalformed => connection.execute(
                "UPDATE analyses SET result_json = '{' WHERE id = ?1",
                [CARRIER_ID],
            ),
            AggregateCorruption::ErrorMismatch => connection.execute(
                "UPDATE analyses SET error_json = ?2 WHERE id = ?1",
                rusqlite::params![CARRIER_ID, canonical_error("Wrong parent error.")],
            ),
            AggregateCorruption::SearchHeadline => connection.execute(
                "UPDATE analysis_search SET headline = 'Wrong headline'
                 WHERE analysis_id = ?1",
                [CARRIER_ID],
            ),
            AggregateCorruption::SearchSourceUrls => connection.execute(
                "UPDATE analysis_search SET source_urls = 'https://wrong.example/source'
                 WHERE analysis_id = ?1",
                [CARRIER_ID],
            ),
        })
        .expect("raw connection")
        .expect("install aggregate corruption");
}

fn install_foreign_key_corruption(store: &HistoryStore, corruption: ForeignKeyCorruption) {
    store
        .with_connection(|connection| {
            connection.execute_batch("PRAGMA foreign_keys = OFF;")?;
            let changed = match corruption {
                ForeignKeyCorruption::CheckOwner => connection.execute(
                    "INSERT INTO analysis_checks
                       (analysis_id, check_index, check_kind, status)
                     VALUES
                       ('anl_01983c20-0180-7a80-a001-00000000c099',
                        0, 'ai_detection', 'running')",
                    [],
                ),
                ForeignKeyCorruption::TaskOwner => connection.execute(
                    "INSERT INTO upstream_tasks
                       (analysis_id, check_kind, upstream_task_id, observed_at)
                     VALUES
                       ('anl_01983c20-0180-7a80-a001-00000000c098',
                        'ai_detection', 'task-orphan-owner', '2026-08-01T10:05:00Z')",
                    [],
                ),
                ForeignKeyCorruption::BulkMembership => connection.execute(
                    "UPDATE analyses
                     SET bulk_id = 'bulk_01983c20-0180-7a80-a001-00000000c097',
                         bulk_index = 0
                     WHERE id = ?1",
                    [CARRIER_ID],
                ),
                ForeignKeyCorruption::RetryLineage => connection.execute(
                    "UPDATE analyses
                     SET retry_of = 'anl_01983c20-0180-7a80-a001-00000000c096'
                     WHERE id = ?1",
                    [CARRIER_ID],
                ),
                ForeignKeyCorruption::RerunLineage => connection.execute(
                    "UPDATE analyses
                     SET rerun_of = 'anl_01983c20-0180-7a80-a001-00000000c095'
                     WHERE id = ?1",
                    [CARRIER_ID],
                ),
            }?;
            connection.execute_batch("PRAGMA foreign_keys = ON;")?;
            Ok::<_, rusqlite::Error>(changed)
        })
        .expect("raw connection")
        .expect("install foreign-key corruption");
}

fn search_segment_blocks(store: &HistoryStore) -> i64 {
    store
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT COUNT(*) FROM analysis_search_data WHERE id > 10",
                    [],
                    |row| row.get(0),
                )
                .expect("count FTS segment blocks")
        })
        .expect("raw connection")
}

fn search_docsize_rows(store: &HistoryStore) -> i64 {
    store
        .with_connection(|connection| {
            connection
                .query_row("SELECT COUNT(*) FROM analysis_search_docsize", [], |row| {
                    row.get(0)
                })
                .expect("count FTS docsize rows")
        })
        .expect("raw connection")
}

fn search_integrity_results(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("PRAGMA main.integrity_check('analysis_search')")
        .expect("prepare targeted FTS integrity pragma");
    statement
        .query_map([], |row| row.get(0))
        .expect("query targeted FTS integrity pragma")
        .collect::<Result<Vec<_>, _>>()
        .expect("consume targeted FTS integrity results")
}

fn install_search_shadow_corruption(store: &HistoryStore) {
    use rusqlite::config::DbConfig;

    store
        .with_connection(|connection| {
            let defensive = connection.db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE)?;
            let trusted_schema = connection.db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA)?;

            // SQLite deliberately protects FTS shadow tables from ordinary
            // SQL. Disable only the connection-local defensive flag needed to
            // model external corruption, and keep the schema untrusted while
            // the shadow index is damaged.
            connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
            connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, false)?;
            let changed = connection.execute("DELETE FROM analysis_search_data WHERE id > 10", []);
            connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, defensive)?;
            connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, trusted_schema)?;
            changed
        })
        .expect("raw connection")
        .expect("install FTS shadow corruption");
}

fn install_search_docsize_corruption(store: &HistoryStore) {
    use rusqlite::config::DbConfig;

    let changed = store
        .with_connection(|connection| {
            let defensive = connection.db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE)?;
            let trusted_schema = connection.db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA)?;

            connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
            connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, false)?;
            let changed = connection.execute(
                "DELETE FROM analysis_search_docsize
                 WHERE id = (SELECT rowid FROM analysis_search ORDER BY rowid LIMIT 1)",
                [],
            );
            connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, defensive)?;
            connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, trusted_schema)?;
            changed
        })
        .expect("raw connection")
        .expect("install FTS docsize corruption");
    assert_eq!(changed, 1, "one FTS docsize row must be removed");
}

fn install_forged_search_projection(store: &HistoryStore) {
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE analysis_search SET input_text = 'forged input sentinel'
                 WHERE analysis_id = ?1",
                [CARRIER_ID],
            )
        })
        .expect("raw connection")
        .expect("install forged search projection");
}

fn assert_canonical_search_corruption(error: &HistoryError) {
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    assert!(
        error.message().contains("invalid canonical value"),
        "unexpected canonical corruption error: {error:?}"
    );
}

fn assert_sanitized_search_corruption(error: &HistoryError) {
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    assert_eq!(
        error.message(),
        "validate search index: the database reported an error"
    );
}

#[derive(Clone, Copy, Debug)]
enum TargetedRead {
    Stored,
    Show,
    RerunInput,
}

#[derive(Clone, Copy, Debug)]
enum TargetedCorruption {
    SearchProjection,
    ParentProjection,
    InputProjection,
    Lifecycle,
}

fn install_targeted_corruption(store: &HistoryStore, corruption: TargetedCorruption) {
    store
        .with_connection(|connection| match corruption {
            TargetedCorruption::SearchProjection => connection.execute(
                "UPDATE analysis_search SET headline = 'Forged targeted headline'
                 WHERE analysis_id = ?1",
                [CARRIER_ID],
            ),
            TargetedCorruption::ParentProjection => connection.execute(
                "UPDATE analyses SET result_json = ?2 WHERE id = ?1",
                rusqlite::params![CARRIER_ID, ai_result("Forged targeted parent")],
            ),
            TargetedCorruption::InputProjection => connection.execute(
                "UPDATE analyses SET input_json = '{}' WHERE id = ?1",
                [CARRIER_ID],
            ),
            TargetedCorruption::Lifecycle => connection.execute(
                "UPDATE analyses SET completed_at = NULL WHERE id = ?1",
                [CARRIER_ID],
            ),
        })
        .expect("raw connection")
        .expect("install targeted read corruption");
}

fn invoke_targeted_read(store: &HistoryStore, read: TargetedRead) -> Result<(), HistoryError> {
    let id = AnalysisId::from_str(CARRIER_ID).expect("targeted analysis id");
    match read {
        TargetedRead::Stored => store.get_analysis(&id).map(drop),
        TargetedRead::Show => store.canonical_analysis(&id, false).map(drop),
        TargetedRead::RerunInput => store.canonical_analysis(&id, true).map(drop),
    }
}

#[test]
fn targeted_single_analysis_reads_certify_search_parent_input_and_lifecycle() {
    for corruption in [
        TargetedCorruption::SearchProjection,
        TargetedCorruption::ParentProjection,
        TargetedCorruption::InputProjection,
        TargetedCorruption::Lifecycle,
    ] {
        for read in [
            TargetedRead::Stored,
            TargetedRead::Show,
            TargetedRead::RerunInput,
        ] {
            let root = tempfile::tempdir().expect("temp root");
            let store = seed_store(&root, false);
            install_targeted_corruption(&store, corruption);

            let error = invoke_targeted_read(&store, read)
                .expect_err("targeted read must reject forged aggregate");
            assert_eq!(
                error.code(),
                HistoryErrorCode::HistoryCorrupt,
                "corruption={corruption:?} read={read:?}: {error:?}"
            );
        }
    }
}

#[test]
fn unrelated_aggregate_corruption_does_not_block_targeted_existing_id_reads() {
    for read in [
        TargetedRead::Stored,
        TargetedRead::Show,
        TargetedRead::RerunInput,
    ] {
        let root = tempfile::tempdir().expect("temp root");
        let store = seed_store(&root, false);
        store
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE analyses SET result_json = ?2 WHERE id = ?1",
                    rusqlite::params![TASK_ID, ai_result("Forged unrelated parent")],
                )
            })
            .expect("raw connection")
            .expect("install unrelated aggregate corruption");

        invoke_targeted_read(&store, read)
            .unwrap_or_else(|error| panic!("read={read:?} must stay targeted: {error:?}"));
    }
}

#[test]
fn global_foreign_key_corruption_still_blocks_targeted_existing_id_reads() {
    for corruption in FOREIGN_KEY_CORRUPTIONS {
        for read in [
            TargetedRead::Stored,
            TargetedRead::Show,
            TargetedRead::RerunInput,
        ] {
            let root = tempfile::tempdir().expect("temp root");
            let store = seed_store(&root, false);
            install_foreign_key_corruption(&store, corruption);

            let error = invoke_targeted_read(&store, read)
                .expect_err("targeted reads must retain the global FK prerequisite");
            assert_eq!(
                error.code(),
                HistoryErrorCode::HistoryCorrupt,
                "corruption={corruption:?} read={read:?}: {error:?}"
            );
        }
    }
}

#[test]
fn targeted_fts_integrity_pragma_returns_one_ok_row_for_valid_store() {
    let root = tempfile::tempdir().expect("temp root");
    let store = seed_store(&root, false);

    let results = store
        .with_connection(search_integrity_results)
        .expect("raw connection");

    assert_eq!(results, ["ok"]);
}

#[test]
fn export_rejects_forged_fts_and_parent_projection_with_or_without_redaction() {
    for redact_content in [false, true] {
        let root = tempfile::tempdir().expect("temp root");
        let store = seed_store(&root, false);
        install_forged_search_projection(&store);
        assert_eq!(
            store
                .with_connection(search_integrity_results)
                .expect("raw connection"),
            ["ok"],
            "the forged FTS index remains internally valid"
        );
        let mut output = Vec::new();
        let error = export_history(
            Some(&store),
            &mut output,
            HistoryExportFormat::Jsonl,
            redact_content,
        )
        .expect_err("export must certify every canonical FTS projection");
        let HistoryExportError::History(error) = error else {
            panic!("canonical certification failure must be a history error")
        };
        assert!(
            output.is_empty(),
            "a failed certified export must not write partial output"
        );
        assert_canonical_search_corruption(&error);

        let root = tempfile::tempdir().expect("temp root");
        let store = seed_store(&root, false);
        install_aggregate_corruption(&store, AggregateCorruption::ResultMismatch);
        let mut output = Vec::new();
        let error = export_history(
            Some(&store),
            &mut output,
            HistoryExportFormat::Jsonl,
            redact_content,
        )
        .expect_err("export must certify every canonical parent projection");
        let HistoryExportError::History(error) = error else {
            panic!("canonical certification failure must be a history error")
        };
        assert!(
            output.is_empty(),
            "a failed certified export must not write partial output"
        );
        assert_canonical_search_corruption(&error);
    }
}

#[test]
fn aggregate_body_and_search_corruption_blocks_destructive_mutations_without_changes() {
    for corruption in AGGREGATE_CORRUPTIONS {
        for mutation in DESTRUCTIVE_MUTATIONS {
            let root = tempfile::tempdir().expect("temp root");
            let mut store = seed_store(
                &root,
                matches!(corruption, AggregateCorruption::ErrorMismatch),
            );
            install_aggregate_corruption(&store, corruption);
            let before = logical_state(&store);
            let error = match invoke_mutation(&mut store, mutation) {
                Err(error) => error,
                Ok(()) => {
                    panic!("corruption={corruption:?} mutation={mutation:?} unexpectedly succeeded")
                }
            };
            assert_eq!(
                error.code(),
                HistoryErrorCode::HistoryCorrupt,
                "corruption={corruption:?} mutation={mutation:?}: {error:?}"
            );
            assert_eq!(
                logical_state(&store),
                before,
                "corruption={corruption:?} mutation={mutation:?}"
            );
        }
    }
}

#[test]
fn fts_shadow_corruption_blocks_destructive_mutations_without_repair() {
    for mutation in DESTRUCTIVE_MUTATIONS {
        let root = tempfile::tempdir().expect("temp root");
        let mut store = seed_store(&root, false);
        install_search_shadow_corruption(&store);
        let before = logical_state(&store);
        let error = invoke_mutation(&mut store, mutation)
            .expect_err("FTS shadow corruption must fail before mutation");
        assert_sanitized_search_corruption(&error);
        assert_eq!(logical_state(&store), before, "mutation={mutation:?}");
        assert_eq!(
            search_segment_blocks(&store),
            0,
            "mutation={mutation:?} must not repair FTS"
        );
    }
}

#[test]
fn fts_docsize_corruption_blocks_destructive_mutations_without_repair() {
    for mutation in DESTRUCTIVE_MUTATIONS {
        let root = tempfile::tempdir().expect("temp root");
        let mut store = seed_store(&root, false);
        install_search_docsize_corruption(&store);
        let before = logical_state(&store);
        let error = invoke_mutation(&mut store, mutation)
            .expect_err("FTS docsize corruption must fail before mutation");
        assert_sanitized_search_corruption(&error);
        assert_eq!(logical_state(&store), before, "mutation={mutation:?}");
        assert_eq!(
            search_docsize_rows(&store),
            2,
            "mutation={mutation:?} must not repair FTS docsize"
        );
    }
}

#[test]
fn every_foreign_key_family_blocks_destructive_mutations_without_changes() {
    for corruption in FOREIGN_KEY_CORRUPTIONS {
        for mutation in DESTRUCTIVE_MUTATIONS {
            let root = tempfile::tempdir().expect("temp root");
            let mut store = seed_store(&root, false);
            install_foreign_key_corruption(&store, corruption);
            let before = logical_state(&store);
            let error = invoke_mutation(&mut store, mutation)
                .expect_err("foreign-key corruption must fail before mutation");
            assert_eq!(
                error.code(),
                HistoryErrorCode::HistoryCorrupt,
                "corruption={corruption:?} mutation={mutation:?}: {error:?}"
            );
            assert_eq!(
                logical_state(&store),
                before,
                "corruption={corruption:?} mutation={mutation:?}"
            );
        }
    }
}

#[test]
fn valid_store_passes_all_certified_mutation_surfaces() {
    for mutation in MUTATIONS {
        let root = tempfile::tempdir().expect("temp root");
        let mut store = seed_store(&root, false);
        invoke_mutation(&mut store, mutation)
            .unwrap_or_else(|error| panic!("mutation={mutation:?}: {error:?}"));
    }
}

#[test]
fn reopen_and_read_fail_closed_on_externally_inserted_orphans() {
    for corruption in FOREIGN_KEY_CORRUPTIONS {
        let root = tempfile::tempdir().expect("temp root");
        let store = seed_store(&root, false);
        install_foreign_key_corruption(&store, corruption);
        let path = root.path().to_owned();
        drop(store);

        let error =
            HistoryStore::open(&path).expect_err("open must reject a foreign-key-corrupt store");
        assert_eq!(
            error.code(),
            HistoryErrorCode::HistoryCorrupt,
            "corruption={corruption:?}: {error:?}"
        );
    }
}
