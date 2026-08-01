//! The `bulk results` paged read. An explicit `--offset`/`--limit` reads one
//! documented page; omitting `--limit` with a zero offset fetches every page
//! and reassembles one canonical ordered page through the domain owner. The
//! adapter owns only flag resolution and the projection handoff.

use clap::ArgMatches;

use crate::analysis::StopObserving;
use crate::cli::StreamTty;
use crate::cli::detect::{self, DetectOutcome, GlobalFlags};
use crate::domain::UtcTimestamp;
use crate::output::{CommandData, ErrorCode, ExitCode, ResolvedCommand};

use super::plan::parse_upstream_bulk_id;
use super::policy::{parse_u64_arg, resolve_policy};
use super::{new_runtime, prepare, succeed};

const BULK_RESULTS_DEFAULT_LIMIT: u64 = 100;
const BULK_RESULTS_MAX_LIMIT: u64 = 1000;
/// The read bound for one fetch-all walk: the maximum number of safe-GET
/// result pages the walker requests before giving up. Each walked page holds
/// up to `BULK_FETCH_ALL_PAGE_SIZE` (100) items, so 10 reads cover the
/// documented 1,000-item job cap exactly. This is a page-read budget, not an
/// item count.
const BULK_RESULTS_FETCH_ALL_MAX_READS: u64 = 10;

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
                .bulk_results_all(&running, BULK_RESULTS_FETCH_ALL_MAX_READS, &cancel, |_| {})
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
        Ok(page) => succeed_page(page, output, started),
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

/// Projects one bulk results page. Both the explicit single-page read and
/// the fetch-all composition return the canonical page the domain owner
/// assembled; the fetch-all aggregate already carries the synthetic window
/// metadata (`offset: 0`, `limit: max(1, total_items)` bounded by 1,000,
/// absent `next_offset`) documented in contracts.md 9.1/14.3, so the adapter
/// never reassembles or re-windows it here.
fn succeed_page(
    page_result: crate::analysis::BulkPageResult,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
) -> DetectOutcome {
    succeed(
        ResolvedCommand::BulkResults,
        CommandData::BulkResults(page_result.page),
        ExitCode::Success,
        output,
        started,
    )
}
