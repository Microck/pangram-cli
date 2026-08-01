//! The rendering policy, timeout, and numeric-flag resolution shared by the
//! bulk/task flows. Every decision funnels through the detection-owner
//! helpers so format, color, progress, and duration grammar stay identical
//! across adapters. No protocol or JSON parsing happens here.

use clap::ArgMatches;

use crate::cli::StreamTty;
use crate::cli::detect::{self, DetectOutcome, ErrorSurface, GlobalFlags, ProgressMode};
use crate::domain::UtcTimestamp;
use crate::output::{ErrorCode, OutputFormat, ResolvedCommand};

/// The rendering policy for one bulk/task invocation: `--format` where the
/// grammar permits it, the JSON default elsewhere; the shared color policy;
/// and the shared global error surface.
pub(super) fn resolve_policy(
    resolved: ResolvedCommand,
    sub: &ArgMatches,
    global: &GlobalFlags,
    streams: &dyn StreamTty,
) -> detect::ResolvedOutput {
    // `--format` exists only where the grammar carries it (bulk submit and
    // bulk results); reading an undefined Clap argument id panics, so every
    // other command resolves the JSON default without touching the match.
    let format = if matches!(
        resolved,
        ResolvedCommand::BulkSubmit | ResolvedCommand::BulkResults
    ) {
        match sub.get_one::<String>("format").map(String::as_str) {
            Some("jsonl") => OutputFormat::Jsonl,
            Some("toon") => OutputFormat::Toon,
            Some("markdown") => OutputFormat::Markdown,
            Some("pretty") => OutputFormat::Pretty,
            _ => OutputFormat::Json,
        }
    } else {
        OutputFormat::Json
    };
    detect::ResolvedOutput {
        format,
        color: detect::color_policy(global, format, streams),
        error: if global.error_format_text == Some(true) {
            ErrorSurface::Text
        } else {
            ErrorSurface::Json
        },
    }
}

/// The progress mode for `bulk wait` and `task wait` (the only bulk/task
/// commands whose grammar carries `--progress`); resolved against the
/// terminal and selected format exactly like detection.
pub(super) fn resolve_wait_progress(
    sub: &ArgMatches,
    output: detect::ResolvedOutput,
    streams: &dyn StreamTty,
) -> ProgressMode {
    let selected = match sub.get_one::<String>("progress").map(String::as_str) {
        Some("never") => ProgressMode::Quiet,
        Some("jsonl") => ProgressMode::Jsonl,
        _ => ProgressMode::Auto,
    };
    detect::resolve_progress(selected, output, streams)
}

/// Parses the shared `--timeout` duration; the locked grammar lives inside
/// `detect::parse_duration`, reused here one-to-one. An invalid token is a
/// usage error on this command.
pub(super) fn resolve_timeout(
    resolved: ResolvedCommand,
    sub: &ArgMatches,
    started: UtcTimestamp,
    output: detect::ResolvedOutput,
) -> Result<Option<std::time::Duration>, DetectOutcome> {
    match sub.get_one::<String>("timeout") {
        Some(raw) => match detect::parse_duration(raw) {
            Some(duration) => Ok(Some(duration)),
            None => Err(detect::failure_outcome(
                resolved,
                output,
                started,
                detect::usage_error(
                    ErrorCode::UnsupportedInput,
                    "--timeout must be an ASCII decimal count with an optional s, ms, m, or h suffix",
                ),
            )),
        },
        None => Ok(None),
    }
}

/// A numeric flag with the shared usage-error surface; `default` applies
/// when the flag is absent.
pub(super) fn parse_u64_arg(
    sub: &ArgMatches,
    name: &str,
    default: u64,
    resolved: ResolvedCommand,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
) -> Result<u64, DetectOutcome> {
    match sub.get_one::<String>(name) {
        Some(raw) => raw.parse::<u64>().map_err(|_| {
            detect::failure_outcome(
                resolved,
                output,
                started,
                detect::usage_error(
                    ErrorCode::UnsupportedInput,
                    &format!("--{name} must be a decimal integer"),
                ),
            )
        }),
        None => Ok(default),
    }
}
