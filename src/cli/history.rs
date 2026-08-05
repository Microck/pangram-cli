//! Thin CLI adapter for local history reads and mutations.

use std::io::Write as _;

use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::domain::{
    AnalysisId, AnalysisInputKind, AnalysisStatus, AnalysisSummary, AnalysisSummaryPage, CheckKind,
    OrderedChecks,
};
use crate::history::{HistoryError, HistoryErrorCode, HistoryStore, InputKind, StoredSearchHit};
use crate::output::{
    CanonicalError, CommandData, CommandEnvelope, EnvelopeMeta, ErrorCode, MutationAcknowledgement,
    OutputFormat, ResolvedCommand,
};

use super::StreamTty;
use super::detect::{
    DetectOutcome, ErrorSurface, GlobalFlags, ResolvedOutput, color_policy, failure_outcome,
    usage_error,
};

mod rerun;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 1000;

pub(super) fn command() -> Command {
    let filters = |command: Command| {
        command
            .arg(
                Arg::new("status")
                    .long("status")
                    .value_name("STATUS")
                    .value_parser(["queued", "running", "succeeded", "failed", "partial"]),
            )
            .arg(
                Arg::new("check")
                    .long("check")
                    .value_name("CHECK")
                    .value_parser(["ai_detection", "plagiarism"]),
            )
            .arg(
                Arg::new("limit")
                    .long("limit")
                    .value_name("N")
                    .help("Maximum summaries in 1..=1000 (default 50)"),
            )
    };
    let list = filters(Command::new("list").about("List saved local analyses"));
    let search = filters(
        Command::new("search")
            .about("Search saved local analyses as literal text")
            .arg(Arg::new("QUERY").required(true)),
    );
    let show = Command::new("show")
        .about("Show one saved local analysis")
        .arg(Arg::new("ID").required(true))
        .arg(
            Arg::new("include-input")
                .long("include-input")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("format")
                .long("format")
                .value_name("FORMAT")
                .value_parser(["json", "jsonl", "toon", "markdown", "pretty"]),
        );
    let delete = Command::new("delete")
        .about("Delete one saved local analysis")
        .arg(Arg::new("ID").required(true))
        .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue));
    let clear = Command::new("clear")
        .about("Delete all saved local analyses")
        .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue));
    let export = Command::new("export")
        .about("Export complete saved analyses to stdout")
        .arg(
            Arg::new("format")
                .long("format")
                .value_name("FORMAT")
                .default_value("jsonl")
                .value_parser(["jsonl", "markdown"]),
        )
        .arg(
            Arg::new("redact-content")
                .long("redact-content")
                .action(ArgAction::SetTrue),
        );
    let rerun = Command::new("rerun")
        .about("Rerun a saved text AI-detection analysis")
        .arg(Arg::new("ID").required(true))
        .arg(
            Arg::new("format")
                .long("format")
                .value_name("FORMAT")
                .value_parser(["json", "jsonl", "toon", "markdown", "pretty"]),
        )
        .arg(
            Arg::new("progress")
                .long("progress")
                .value_name("MODE")
                .value_parser(["auto", "never", "jsonl"]),
        );
    Command::new("history")
        .about("Inspect and manage optional local analysis history")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(list)
        .subcommand(show)
        .subcommand(search)
        .subcommand(delete)
        .subcommand(clear)
        .subcommand(export)
        .subcommand(rerun)
}

pub(super) fn execute(
    matches: &ArgMatches,
    root: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
) -> DetectOutcome {
    let started = crate::domain::UtcTimestamp::now();
    let Some((name, leaf)) = matches.subcommand() else {
        unreachable!("history requires a leaf");
    };
    let command = resolved_command(name);
    let output = resolved_output(name, leaf, &global, streams);

    let parsed = match parse_request(name, leaf) {
        Ok(parsed) => parsed,
        Err(error) => return failure_outcome(command, output, started, *error),
    };
    if parsed.destructive && !parsed.yes {
        if std::env::var_os("CI").is_some() || !streams.all_interactive() {
            return failure_outcome(
                command,
                output,
                started,
                usage_error(
                    ErrorCode::UnsupportedCombination,
                    "history delete and clear require --yes outside an interactive terminal",
                ),
            );
        }
        if !confirm(name) {
            return DetectOutcome {
                exit_code: 130,
                envelopes: Vec::new(),
                rendered: true,
                primary_ok: true,
            };
        }
    }

    let service = match config_service(root) {
        Ok(service) => service,
        Err(error) => return failure_outcome(command, output, started, *error),
    };
    let store = match HistoryStore::open_existing(service.paths().data_dir()) {
        Ok(store) => store,
        Err(error) => {
            return failure_outcome(command, output, started, error.into_canonical());
        }
    };

    if parsed.command == ResolvedCommand::HistoryRerun {
        return rerun::execute(parsed, store, &service, output, started, streams);
    }

    if parsed.command == ResolvedCommand::HistoryExport {
        return match run_export(parsed, store) {
            Ok(()) => DetectOutcome {
                exit_code: 0,
                envelopes: Vec::new(),
                rendered: true,
                primary_ok: true,
            },
            Err(ExportError::History(error)) => {
                failure_outcome(command, output, started, error.into_canonical())
            }
            Err(ExportError::Output) => DetectOutcome {
                exit_code: 1,
                envelopes: Vec::new(),
                rendered: true,
                primary_ok: false,
            },
        };
    }

    match run(parsed, store) {
        Ok(data) => success(data, output, started),
        Err(error) => failure_outcome(command, output, started, error.into_canonical()),
    }
}

struct Request {
    command: ResolvedCommand,
    id: Option<AnalysisId>,
    query: Option<String>,
    status: Option<AnalysisStatus>,
    check: Option<CheckKind>,
    limit: u32,
    include_input: bool,
    destructive: bool,
    yes: bool,
    redact_content: bool,
    export_markdown: bool,
    progress: super::detect::ProgressMode,
}

fn parse_request(name: &str, matches: &ArgMatches) -> Result<Request, Box<CanonicalError>> {
    let id = matches
        .try_get_one::<String>("ID")
        .ok()
        .flatten()
        .map(|value| {
            value.parse().map_err(|_| {
                Box::new(usage_error(
                    ErrorCode::UnsupportedInput,
                    "history ID must be a canonical local anl_ UUIDv7 identifier",
                ))
            })
        })
        .transpose()?;
    let limit = matches
        .try_get_one::<String>("limit")
        .ok()
        .flatten()
        .map(|raw| parse_limit(raw))
        .transpose()?
        .unwrap_or(DEFAULT_LIMIT);
    let status = matches
        .try_get_one::<String>("status")
        .ok()
        .flatten()
        .map(|value| match value.as_str() {
            "queued" => AnalysisStatus::Queued,
            "running" => AnalysisStatus::Running,
            "succeeded" => AnalysisStatus::Succeeded,
            "failed" => AnalysisStatus::Failed,
            "partial" => AnalysisStatus::Partial,
            _ => unreachable!("clap validates history status"),
        });
    let check = matches
        .try_get_one::<String>("check")
        .ok()
        .flatten()
        .map(|value| match value.as_str() {
            "ai_detection" => CheckKind::AiDetection,
            "plagiarism" => CheckKind::Plagiarism,
            _ => unreachable!("clap validates history check"),
        });
    Ok(Request {
        command: resolved_command(name),
        id,
        query: matches
            .try_get_one::<String>("QUERY")
            .ok()
            .flatten()
            .cloned(),
        status,
        check,
        limit,
        include_input: matches
            .try_get_one::<bool>("include-input")
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false),
        destructive: matches!(name, "delete" | "clear"),
        yes: matches
            .try_get_one::<bool>("yes")
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false),
        redact_content: matches
            .try_get_one::<bool>("redact-content")
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false),
        export_markdown: matches
            .try_get_one::<String>("format")
            .ok()
            .flatten()
            .is_some_and(|format| format == "markdown"),
        progress: match matches
            .try_get_one::<String>("progress")
            .ok()
            .flatten()
            .map(String::as_str)
        {
            Some("jsonl") => super::detect::ProgressMode::Jsonl,
            Some("never") => super::detect::ProgressMode::Quiet,
            Some("auto") | None => super::detect::ProgressMode::Auto,
            Some(_) => unreachable!("clap validates progress"),
        },
    })
}

fn parse_limit(raw: &str) -> Result<u32, Box<CanonicalError>> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(limit_error());
    }
    let value = raw.parse::<u32>().map_err(|_| limit_error())?;
    if !(1..=MAX_LIMIT).contains(&value) {
        return Err(limit_error());
    }
    Ok(value)
}

fn limit_error() -> Box<CanonicalError> {
    Box::new(usage_error(
        ErrorCode::UnsupportedInput,
        "--limit must be a positive decimal integer in 1..=1000",
    ))
}

fn run(request: Request, store: Option<HistoryStore>) -> Result<CommandData, HistoryError> {
    match request.command {
        ResolvedCommand::HistoryList => {
            let hits = match store {
                Some(store) => {
                    store.list_filtered(request.status, request.check, request.limit, 0)?
                }
                None => Vec::new(),
            };
            Ok(CommandData::HistoryList(summary_page(hits)?))
        }
        ResolvedCommand::HistorySearch => {
            let hits = match store {
                Some(store) => store.search_filtered(
                    request.query.as_deref().unwrap_or_default(),
                    request.status,
                    request.check,
                    request.limit,
                )?,
                None => Vec::new(),
            };
            Ok(CommandData::HistorySearch(summary_page(hits)?))
        }
        ResolvedCommand::HistoryShow => {
            let store = store.ok_or_else(missing_record)?;
            let analysis = store.canonical_analysis(
                request.id.as_ref().expect("show requires ID"),
                request.include_input,
            )?;
            Ok(CommandData::HistoryShow(analysis))
        }
        ResolvedCommand::HistoryDelete => {
            let mut store = store.ok_or_else(missing_record)?;
            store.delete_analysis(request.id.as_ref().expect("delete requires ID"))?;
            Ok(CommandData::HistoryDelete(MutationAcknowledgement::new()))
        }
        ResolvedCommand::HistoryClear => {
            if let Some(mut store) = store {
                store.clear()?;
            }
            Ok(CommandData::HistoryClear(MutationAcknowledgement::new()))
        }
        _ => unreachable!("only Packet D stage-1 history commands dispatch here"),
    }
}

enum ExportError {
    History(HistoryError),
    Output,
}

impl From<HistoryError> for ExportError {
    fn from(error: HistoryError) -> Self {
        Self::History(error)
    }
}

fn run_export(request: Request, store: Option<HistoryStore>) -> Result<(), ExportError> {
    let values = match store {
        Some(store) => store.export_analyses(request.redact_content)?,
        None => Vec::new(),
    };
    let mut bytes = Vec::new();
    if request.export_markdown {
        bytes.extend_from_slice(b"# Pangram history export\n");
        for value in values {
            let id = value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("invalid");
            bytes.extend_from_slice(format!("\n## `{id}`\n\n```json\n").as_bytes());
            let rendered = serde_json::to_string_pretty(&value)
                .map_err(|_| {
                    HistoryError::new(
                        HistoryErrorCode::HistoryCorrupt,
                        "export history: a canonical analysis could not be encoded",
                    )
                })?
                .replace('`', "\\u0060");
            bytes.extend_from_slice(rendered.as_bytes());
            bytes.extend_from_slice(b"\n```\n");
        }
    } else {
        for value in values {
            let rendered = serde_json::to_vec(&value).map_err(|_| {
                HistoryError::new(
                    HistoryErrorCode::HistoryCorrupt,
                    "export history: a canonical analysis could not be encoded",
                )
            })?;
            bytes.extend_from_slice(&rendered);
            bytes.push(b'\n');
        }
    }
    let mut stdout = std::io::stdout().lock();
    write_export(&mut stdout, &bytes).map_err(|_| ExportError::Output)
}

fn write_export(writer: &mut impl std::io::Write, bytes: &[u8]) -> std::io::Result<()> {
    writer.write_all(bytes)?;
    writer.flush()
}

fn unresolvable() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::LocalTaskUnresolvable,
        "The saved analysis does not retain exact text that can be rerun.",
    )
    .expect("static error")
}

fn summary_page(hits: Vec<StoredSearchHit>) -> Result<AnalysisSummaryPage, HistoryError> {
    let items = hits
        .into_iter()
        .map(|hit| {
            let checks = OrderedChecks::new(hit.checks).map_err(|_| {
                HistoryError::new(
                    HistoryErrorCode::HistoryCorrupt,
                    "a stored history summary has invalid check ordering",
                )
            })?;
            Ok(AnalysisSummary {
                id: hit.analysis_id,
                status: hit.status,
                checks,
                save_state: hit.save_state,
                input_kind: match hit.input_kind {
                    InputKind::Text => AnalysisInputKind::Text,
                    InputKind::File => AnalysisInputKind::File,
                },
                display_name: hit.display_name,
                created_at: hit.created_at,
            })
        })
        .collect::<Result<Vec<_>, HistoryError>>()?;
    Ok(AnalysisSummaryPage { items })
}

fn missing_record() -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::NotFound,
        "no analysis with that identity is recorded",
    )
}

fn config_service(root: &ArgMatches) -> Result<crate::config::ConfigService, Box<CanonicalError>> {
    let mut flags = crate::config::ConfigOverrides::default();
    if let Some(config) = root.get_one::<String>("config") {
        flags = flags.with_config_file(config.clone());
    }
    if let Some(data_dir) = root.get_one::<String>("data-dir") {
        flags = flags.with_data_dir(data_dir.clone());
    }
    let overrides = crate::config::ConfigOverrides::merge(
        flags,
        crate::config::ConfigOverrides::from_environment(),
    );
    crate::config::ConfigService::new(&overrides)
        .map_err(super::detect::credential_error)
        .map_err(Box::new)
}

fn resolved_command(name: &str) -> ResolvedCommand {
    match name {
        "list" => ResolvedCommand::HistoryList,
        "show" => ResolvedCommand::HistoryShow,
        "search" => ResolvedCommand::HistorySearch,
        "delete" => ResolvedCommand::HistoryDelete,
        "clear" => ResolvedCommand::HistoryClear,
        "export" => ResolvedCommand::HistoryExport,
        "rerun" => ResolvedCommand::HistoryRerun,
        _ => unreachable!("closed history leaf"),
    }
}

fn resolved_output(
    name: &str,
    matches: &ArgMatches,
    global: &GlobalFlags,
    streams: &dyn StreamTty,
) -> ResolvedOutput {
    let format = if matches!(name, "show" | "export" | "rerun") {
        match matches.get_one::<String>("format").map(String::as_str) {
            Some("jsonl") => OutputFormat::Jsonl,
            Some("toon") => OutputFormat::Toon,
            Some("markdown") => OutputFormat::Markdown,
            Some("pretty") => OutputFormat::Pretty,
            Some("json") | None => OutputFormat::Json,
            Some(_) => unreachable!("clap validates history format"),
        }
    } else {
        OutputFormat::Json
    };
    let error = match global.error_format_text {
        Some(true) => ErrorSurface::Text,
        Some(false) => ErrorSurface::Json,
        None if format == OutputFormat::Pretty => ErrorSurface::Text,
        None => ErrorSurface::Json,
    };
    ResolvedOutput {
        format,
        color: color_policy(global, format, streams),
        error,
    }
}

fn success(
    data: CommandData,
    output: ResolvedOutput,
    started: crate::domain::UtcTimestamp,
) -> DetectOutcome {
    let envelope = CommandEnvelope::success(
        data,
        EnvelopeMeta::default()
            .with_started_at(started)
            .with_completed_at(crate::domain::UtcTimestamp::now()),
    );
    if output.format == OutputFormat::Json {
        return DetectOutcome {
            exit_code: 0,
            envelopes: vec![envelope],
            rendered: false,
            primary_ok: true,
        };
    }
    let rendered = crate::output::render(
        output.format,
        output.color,
        std::slice::from_ref(&envelope),
        &mut std::io::stdout().lock(),
    )
    .is_ok();
    DetectOutcome {
        exit_code: if rendered { 0 } else { 1 },
        envelopes: Vec::new(),
        rendered: true,
        primary_ok: rendered,
    }
}

fn confirm(operation: &str) -> bool {
    let prompt = format!("Confirm history {operation}? [y/N] ");
    let mut stderr = std::io::stderr().lock();
    if stderr
        .write_all(prompt.as_bytes())
        .and_then(|_| stderr.flush())
        .is_err()
    {
        return false;
    }
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .is_ok_and(|_| matches!(answer.trim(), "y" | "Y" | "yes" | "YES"))
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use super::write_export;

    struct ShortWriter {
        remaining: usize,
        flush_fails: bool,
    }

    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
            }
            let written = self.remaining.min(bytes.len());
            self.remaining -= written;
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.flush_fails {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn export_short_write_and_broken_pipe_are_output_errors() {
        let error = write_export(
            &mut ShortWriter {
                remaining: 3,
                flush_fails: false,
            },
            b"long export",
        )
        .expect_err("short writer eventually closes");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn export_flush_failure_is_an_output_error() {
        let error = write_export(
            &mut ShortWriter {
                remaining: usize::MAX,
                flush_fails: true,
            },
            b"complete export",
        )
        .expect_err("flush fails");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
