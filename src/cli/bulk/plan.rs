//! Bulk JSONL input reading and whole-file validation, plus the upstream-ID
//! argument parses. The adapter reads the whole source as UTF-8, validates it
//! into the shared domain contract, and prices it against the caller ceiling
//! before any credential or network work. Failures name the source shape and
//! line, never the item text.

use std::io::Read as _;

use clap::ArgMatches;

use crate::cli::detect::{self};
use crate::domain::{
    BulkJsonlError, BulkSubmissionPlan, UpstreamBulkId, UpstreamTaskId, parse_bulk_jsonl,
};
use crate::output::{CanonicalError, ErrorCode};

/// Reads the bulk JSONL source text: a UTF-8 file, stdin for the literal
/// `-` marker, or stdin when the positional path is omitted (the piped
/// default). Failures name the channel, never item text.
#[allow(clippy::result_large_err)]
pub(super) fn read_jsonl_source(sub: &ArgMatches) -> Result<String, CanonicalError> {
    match sub.get_one::<String>("JSONL_PATH").map(String::as_str) {
        Some("-") | None => {
            let mut text = String::new();
            let read = std::io::stdin().read_to_string(&mut text);
            match read {
                Ok(_) if !text.is_empty() => Ok(text),
                Ok(_) => Err(detect::usage_error(
                    ErrorCode::InputRequired,
                    "the bulk JSONL source on stdin was empty",
                )),
                Err(_) => Err(detect::usage_error(
                    ErrorCode::UnsupportedInput,
                    "the bulk JSONL source on stdin was not valid UTF-8",
                )),
            }
        }
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) if !text.is_empty() => Ok(text),
            Ok(_) => Err(detect::usage_error(
                ErrorCode::InputRequired,
                "the bulk JSONL file was empty",
            )),
            Err(_) => Err(detect::usage_error(
                ErrorCode::UnsupportedInput,
                "the bulk JSONL file could not be read as UTF-8 text",
            )),
        },
    }
}

/// The canonical word count shared with detection input summaries:
/// whitespace-split words.
fn word_count(text: &str) -> u64 {
    u64::try_from(text.split_whitespace().count()).unwrap_or(u64::MAX)
}

/// Whole-file JSONL validation into the shared domain contract. The error
/// carries the line number and structural reason only.
#[allow(clippy::result_large_err)]
pub(super) fn plan_from_jsonl(
    text: &str,
    max_billable_units: u64,
) -> Result<BulkSubmissionPlan, CanonicalError> {
    let items = parse_bulk_jsonl(text, word_count).map_err(|error| match error {
        BulkJsonlError::EmptyFile => detect::usage_error(
            ErrorCode::InputRequired,
            "the bulk JSONL source contained no items",
        ),
        error @ BulkJsonlError::InvalidLine { .. } => {
            let message = error.to_string();
            detect::usage_error(ErrorCode::UnsupportedInput, &message)
        }
    })?;
    BulkSubmissionPlan::new(items, max_billable_units).map_err(|error| match error {
        crate::domain::DomainError::BulkLimitExceeded => detect::usage_error(
            ErrorCode::BulkLimitExceeded,
            "the bulk submission exceeds the billable-unit ceiling",
        ),
        crate::domain::DomainError::DuplicateBulkCallerId => detect::usage_error(
            ErrorCode::UnsupportedInput,
            "the bulk JSONL contains a duplicate caller id",
        ),
        other => {
            let message = other.to_string();
            detect::usage_error(ErrorCode::UnsupportedInput, &message)
        }
    })
}

/// Parses a caller-supplied ID string into the validated upstream identity
/// type. The ID itself is trusted terminal input, so a parse failure names
/// no content beyond the empty-shape contract.
#[allow(clippy::result_large_err)]
pub(super) fn parse_upstream_bulk_id(raw: &str) -> Result<UpstreamBulkId, CanonicalError> {
    UpstreamBulkId::new(raw).map_err(|_| {
        detect::usage_error(ErrorCode::InputRequired, "a bulk job ID must not be empty")
    })
}

#[allow(clippy::result_large_err)]
pub(super) fn parse_upstream_task_id(raw: &str) -> Result<UpstreamTaskId, CanonicalError> {
    UpstreamTaskId::new(raw)
        .map_err(|_| detect::usage_error(ErrorCode::InputRequired, "a task ID must not be empty"))
}
