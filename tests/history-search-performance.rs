//! Executable 10,000-analysis history-search fixture.
//!
//! Every CI profile builds the real SQLite fixture through the production
//! bulk reconciliation path and executes the representative indexed query.
//! The wall-clock acceptance assertion is release-only because
//! `docs/testing-release-plan.md` defines the budget for release builds on
//! supported desktop hardware. It measures 30 warm queries after five
//! explicit warm-ups and requires the observed p95 to remain below 100 ms;
//! setup and fixture insertion are outside the timed region.

#![forbid(unsafe_code)]

use std::str::FromStr;
use std::time::{Duration, Instant};

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisStatus, BulkCounters, BulkId, SaveState, Sha256Hash, SubmissionOutcome,
    UtcTimestamp,
};
use microck_pangram_cli::history::{HistoryStore, InputKind, StoredAnalysis, StoredBulkCollection};

const ANALYSIS_COUNT: usize = 10_000;
const WARMUP_RUNS: usize = 5;
const MEASURED_RUNS: usize = 30;
const SEARCH_BUDGET: Duration = Duration::from_millis(100);

fn stamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::from_str(value).expect("timestamp")
}

fn fixture_child(index: usize, bulk_id: BulkId) -> StoredAnalysis {
    let id = format!("anl_01983c20-0180-7a80-a001-{index:012x}");
    let input = format!("representative indexed history document number {index}");
    let input_sha256 = Sha256Hash::digest(&input);
    StoredAnalysis {
        id: AnalysisId::from_str(&id).expect("analysis id"),
        bulk: Some((bulk_id, i64::try_from(index).expect("bulk index"))),
        caller_id: Some(format!("row-{index:05}")),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        save_state: SaveState::SavedHistory,
        input_kind: InputKind::Text,
        input_sha256,
        display_name: None,
        input_json: serde_json::json!({
            "type": "text",
            "origin": "literal",
            "sha256": input_sha256,
            "byte_count": input.len(),
            "word_count": 6,
            "text": input
        })
        .to_string(),
        result_json: None,
        error_json: None,
        upstream_version: Some("synthetic-performance-fixture".to_owned()),
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(stamp("2026-08-01T09:59:00Z")),
        created_at: stamp("2026-08-01T10:00:00Z"),
        updated_at: stamp("2026-08-01T10:00:00Z"),
        completed_at: None,
        search_input_text: Some(input),
        search_filename: None,
        search_headline: None,
        search_source_urls: None,
    }
}

#[test]
fn representative_indexed_search_at_ten_thousand_analyses_stays_under_budget() {
    let root = tempfile::tempdir().expect("temporary history root");
    let mut store = HistoryStore::open(root.path()).expect("history store");
    let bulk_id = BulkId::from_str("bulk_01983c20-0180-7a80-a001-00000000f100").expect("bulk id");
    let collection = StoredBulkCollection {
        id: bulk_id,
        upstream_bulk_id: Some("synthetic-performance-bulk".to_owned()),
        status: AnalysisStatus::Running,
        submission_outcome: SubmissionOutcome::Accepted,
        counters: BulkCounters::new(ANALYSIS_COUNT as u64, ANALYSIS_COUNT as u64, 0, 0)
            .expect("bulk counters"),
        estimated_billable_units: Some(ANALYSIS_COUNT as u64),
        created_at: stamp("2026-08-01T09:00:00Z"),
        updated_at: stamp("2026-08-01T10:00:00Z"),
        completed_at: None,
    };
    let children = (0..ANALYSIS_COUNT)
        .map(|index| (fixture_child(index, bulk_id), Vec::new()))
        .collect::<Vec<_>>();
    let setup_started = Instant::now();
    store
        .reconcile_bulk_collection_atomic(&collection, &children)
        .expect("insert valid 10,000-analysis fixture");
    eprintln!(
        "history-search-10k setup_ms={}",
        setup_started.elapsed().as_millis()
    );

    let warmup_started = Instant::now();
    for _ in 0..WARMUP_RUNS {
        let hits = store
            .search("representative indexed", 25)
            .expect("warm indexed search");
        assert_eq!(hits.len(), 25);
    }
    eprintln!(
        "history-search-10k warmup_ms={}",
        warmup_started.elapsed().as_millis()
    );

    let mut samples = Vec::with_capacity(MEASURED_RUNS);
    for _ in 0..MEASURED_RUNS {
        let started = Instant::now();
        let hits = store
            .search("representative indexed", 25)
            .expect("measured indexed search");
        samples.push(started.elapsed());
        assert_eq!(hits.len(), 25);
    }
    samples.sort_unstable();
    let median = samples[MEASURED_RUNS / 2];
    let p95 = samples[(MEASURED_RUNS * 95).div_ceil(100) - 1];
    eprintln!(
        "history-search-10k profile={} platform={}/{} median_us={} p95_us={}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        std::env::consts::OS,
        std::env::consts::ARCH,
        median.as_micros(),
        p95.as_micros()
    );
    if !cfg!(debug_assertions) {
        assert!(
            p95 < SEARCH_BUDGET,
            "10,000-analysis indexed search p95 {p95:?} exceeded {SEARCH_BUDGET:?}"
        );
    }
}
