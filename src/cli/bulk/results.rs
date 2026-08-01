//! The `bulk results` paged read. An explicit `--offset`/`--limit` reads one
//! documented page; omitting `--limit` with a zero offset fetches every page
//! and reassembles one canonical ordered page through the domain owner. The
//! adapter owns only flag resolution and the projection handoff.

use clap::ArgMatches;

use crate::analysis::StopObserving;
use crate::cli::StreamTty;
use crate::cli::detect::{self, DetectOutcome, GlobalFlags};
use crate::domain::{BulkPage, UtcTimestamp};
use crate::output::{CommandData, ErrorCode, ExitCode, ResolvedCommand};

use super::plan::parse_upstream_bulk_id;
use super::policy::{parse_u64_arg, resolve_policy};
use super::{new_runtime, prepare, succeed};

const BULK_RESULTS_DEFAULT_LIMIT: u64 = 100;
const BULK_RESULTS_MAX_LIMIT: u64 = 1000;
const BULK_RESULTS_FETCH_ALL_MIN_ITEMS: u64 = 10;

pub(super) fn bulk_results(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
) -> DetectOutcome {
    let resolved = ResolvedCommand::BulkResults;
    let output = resolve_policy(resolved, sub, &global, streams);
    let raw = sub
        .get_one::<String>("ID")
        .map(String::as_str)
        .unwrap_or_default();
    let upstream_id = match parse_upstream_bulk_id(raw) {
        Ok(id) => id,
        Err(error) => return detect::failure_outcome(resolved, output, started, error),
    };
    let offset = match parse_u64_arg(sub, "offset", 0, resolved, output, started) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    let explicit_limit = sub.get_one::<String>("limit").is_some();
    let limit = match parse_u64_arg(
        sub,
        "limit",
        BULK_RESULTS_DEFAULT_LIMIT,
        resolved,
        output,
        started,
    ) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    if !(1..=BULK_RESULTS_MAX_LIMIT).contains(&limit) {
        return detect::failure_outcome(
            resolved,
            output,
            started,
            detect::usage_error(
                ErrorCode::UnsupportedInput,
                "--limit must be within 1..=1000",
            ),
        );
    }

    let analyzer = match prepare(resolved, root_matches, output, started) {
        Ok(analyzer) => analyzer,
        Err(outcome) => return outcome,
    };
    let runtime = match new_runtime(resolved, output, started) {
        Ok(runtime) => runtime,
        Err(outcome) => return outcome,
    };
    let stop = StopObserving::new();
    detect::install_sigint_driver();
    let fetch_all = !explicit_limit && offset == 0;
    let result = runtime.block_on(async {
        let bridge = tokio::spawn(detect::bridge_sigint(stop.token().clone()));
        let cancel = stop.token().child_token();
        let running = analyzer.observe_bulk(upstream_id);
        let outcome = if fetch_all {
            analyzer
                .bulk_results_all(&running, BULK_RESULTS_FETCH_ALL_MIN_ITEMS, &cancel, |_| {})
                .await
        } else {
            analyzer
                .bulk_results_page(&running, offset, limit, &cancel)
                .await
        };
        bridge.abort();
        outcome
    });
    detect::reset_sigint_flag();
    match result {
        Ok(page) => succeed_page(page, fetch_all, output, started, resolved),
        Err(failure) => {
            let error = failure.into_error();
            if matches!(error.code(), ErrorCode::UpstreamNotFound) {
                detect::note_stderr(
                    streams,
                    "Pangram does not recognize the bulk job; check the ID",
                );
            }
            detect::failure_outcome(resolved, output, started, error)
        }
    }
}

/// Projects one bulk results page: a terminal read returns the canonical
/// page (exit 0 for a single page; the fetch-all composition reassembles one
/// canonical page shape through the domain owner).
fn succeed_page(
    page_result: crate::analysis::BulkPageResult,
    fetch_all: bool,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
    resolved: ResolvedCommand,
) -> DetectOutcome {
    let page = if fetch_all {
        let count = page_result.page.items().len();
        match BulkPage::new(
            page_result.page.items().to_vec(),
            0,
            u64::try_from(count.max(1)).unwrap_or(1),
            None,
        ) {
            Ok(page) => page,
            Err(_) => {
                return detect::failure_outcome(
                    resolved,
                    output,
                    started,
                    detect::internal_error("the fetched bulk results could not be reassembled"),
                );
            }
        }
    } else {
        page_result.page
    };
    succeed(
        CommandData::BulkResults(page),
        ExitCode::Success,
        output,
        started,
    )
}
