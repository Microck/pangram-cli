//! Process-facing CLI dispatch and rendering.
//!
//! The parent module owns the compiled Clap grammar. This module owns how one
//! parsed invocation selects an adapter, resolves its analyzer, and renders
//! its final process outcome.

use std::ffi::OsString;

use clap::ArgMatches;

use super::local_setup::PhaseOneOutcome;
use super::{FULL_GRAMMAR, bulk, history, local_setup, runtime_command};
use crate::analysis::AnalyzerSource;

/// Terminal stream sharing used to decide whether an interactive prompt is
/// allowed at all. A prompt is only permitted when stdin, stdout, and stderr
/// are all TTYs.
pub(crate) trait StreamTty {
    fn stdin(&self) -> bool;
    fn stdout(&self) -> bool;
    fn stderr(&self) -> bool;

    fn all_interactive(&self) -> bool {
        self.stdin() && self.stdout() && self.stderr()
    }
}

pub(crate) struct RealStreams;

impl StreamTty for RealStreams {
    fn stdin(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stdin())
    }

    fn stdout(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stdout())
    }

    fn stderr(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stderr())
    }
}

/// Whether an invocation may render process-owned output and enter the TUI.
#[derive(Clone, Copy, PartialEq, Eq)]
enum InvocationMode {
    Library,
    Process,
}

/// Dependencies and output policy shared throughout one dispatch.
struct InvocationContext<'a> {
    streams: &'a dyn StreamTty,
    mode: InvocationMode,
    analyzer_source: &'a AnalyzerSource,
}

impl<'a> InvocationContext<'a> {
    fn new(
        streams: &'a dyn StreamTty,
        mode: InvocationMode,
        analyzer_source: &'a AnalyzerSource,
    ) -> Self {
        Self {
            streams,
            mode,
            analyzer_source,
        }
    }

    fn is_process_facing(&self) -> bool {
        self.mode == InvocationMode::Process
    }
}

/// Parses a caller-supplied argv without exiting the process.
///
/// A bare invocation (or one carrying only global flags) displays the same
/// help as `--help`, keeping the successful exit status required by the
/// executable contract. Matched Phase 1 commands run through the shared
/// configuration and diagnostics modules and print one canonical envelope.
///
/// Phase 0 exposed this parsing hook; the runtime surface is process-owned,
/// so the hook now shares `run()`'s execution path directly.
pub fn try_run_from<I, T>(arguments: I) -> Result<(), clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let streams = RealStreams;
    let analyzer_source = AnalyzerSource::Production;
    let invocation = InvocationContext::new(&streams, InvocationMode::Library, &analyzer_source);
    match run_arguments(arguments, &invocation).clap_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Runs the process-facing CLI and returns a portable process status.
///
/// Clap owns whether help goes to stdout or usage errors go to stderr.
/// Canonical runtime failures print as one JSON envelope on stdout with
/// empty stderr. This function never exits, so guarded callers remain in
/// control of process lifetime.
pub fn run() -> u8 {
    let streams = RealStreams;
    let analyzer_source = AnalyzerSource::Production;
    let invocation = InvocationContext::new(&streams, InvocationMode::Process, &analyzer_source);
    run_arguments(std::env::args_os(), &invocation).exit_code
}

/// Runs the real process-facing adapter with a development-only injected
/// analyzer. The shipped entry point never calls this seam.
#[cfg(feature = "dev-tools")]
pub(crate) fn run_with_analyzer<I, T>(arguments: I, analyzer: crate::analysis::Analyzer) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let streams = RealStreams;
    let analyzer_source = AnalyzerSource::injected(analyzer);
    let invocation = InvocationContext::new(&streams, InvocationMode::Process, &analyzer_source);
    run_arguments(arguments, &invocation).exit_code
}

/// The resolved result of one invocation before the process renders it.
struct RunOutcome {
    exit_code: u8,
    /// A Clap parse error returned to [`try_run_from`] callers; `run()`
    /// already printed it through Clap.
    clap_error: Option<clap::Error>,
}

/// Executes one argv and returns its intended exit code. Library invocations
/// suppress process-owned output and interactive dispatch, so `run()` stays
/// the only process renderer.
fn run_arguments<I, T>(arguments: I, invocation: &InvocationContext<'_>) -> RunOutcome
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut arguments: Vec<OsString> = arguments.into_iter().map(Into::into).collect();
    if arguments.is_empty() {
        arguments.push(FULL_GRAMMAR.name.into());
    }

    // Bare-input dispatch runs before Clap's errors surface (and before the
    // bare `--help` fallback below) for the source-category rules Clap cannot
    // express: `pangram -` (stdin), a bare non-TTY launch whose piped stdin is
    // the implicit input, and a bare mixed-stream launch that cannot safely
    // enter the TUI. Evaluating this first is what lets `printf text | pangram`
    // detect instead of printing help while empty or unsafe implicit input
    // returns input_required. Every other path stays Clap-owned so usage text
    // and help keep their exact form.
    if let Some(outcome) = bare_dispatch(&arguments, invocation) {
        return outcome;
    }

    // A literally bare process-facing launch owns the interactive adapter
    // only when every standard stream is a terminal. Library parsing keeps
    // its historical no-I/O behavior.
    if invocation.is_process_facing()
        && arguments.len() == 1
        && invocation.streams.all_interactive()
    {
        return RunOutcome {
            exit_code: crate::tui::run(invocation.analyzer_source.clone()),
            clap_error: None,
        };
    }

    // Only a literally bare non-rendering parse behaves like `--help` here.
    // The process-facing all-TTY path entered the TUI above; piped stdin was
    // already claimed by `bare_dispatch`. Explicit help remains Clap-owned.
    if arguments.len() == 1 {
        arguments.push("--help".into());
    }

    let matches = match runtime_command().try_get_matches_from(arguments) {
        Ok(matches) => matches,
        Err(error) => {
            let exit_code = u8::try_from(error.exit_code()).unwrap_or(1);
            // Clap owns help/version stdout and usage-error stderr. A failed
            // render cannot make the output wrong; it only downgrades the
            // exit code to 1.
            let exit_code = if invocation.is_process_facing() && error.print().is_err() {
                1
            } else {
                exit_code
            };
            let clap_error = if error.exit_code() == 0 {
                None
            } else {
                Some(error)
            };
            return RunOutcome {
                exit_code,
                clap_error,
            };
        }
    };

    let global = crate::cli::detect::GlobalFlags::from_matches(&matches);
    match matches.subcommand() {
        Some(("detect", sub)) => execute_detect(sub, global, invocation),
        Some(("bulk", sub)) => execute_bulk_leaf(sub, &matches, global, invocation),
        Some(("task", sub)) => execute_task_leaf(sub, &matches, global, invocation),
        Some(("history", sub)) => finish_detect(history::execute(
            sub,
            &matches,
            global,
            invocation.streams,
            invocation.analyzer_source,
        )),
        Some(("mcp", sub)) if sub.subcommand().is_none() => {
            execute_mcp_server(sub, &matches, invocation)
        }
        Some(("mcp", sub)) => {
            if invocation.is_process_facing() {
                RunOutcome {
                    exit_code: super::mcp::execute(sub, global),
                    clap_error: None,
                }
            } else {
                RunOutcome {
                    exit_code: 0,
                    clap_error: None,
                }
            }
        }
        Some(("agent", _)) => {
            finish_raw_bytes(crate::mcp::embedded::AGENT_REFERENCE.bytes, invocation)
        }
        Some(("skills", sub)) => {
            let bytes = match sub.subcommand() {
                Some(("get", arguments)) if arguments.get_flag("full") => {
                    crate::mcp::embedded::PANGRAM_SKILL.bytes
                }
                Some(("get", _)) => crate::mcp::embedded::AGENT_REFERENCE.bytes,
                Some(("list", _)) => crate::mcp::embedded::SKILL_LIST,
                Some(("path", arguments))
                    if arguments.value_source("SKILL")
                        == Some(clap::parser::ValueSource::DefaultValue) =>
                {
                    crate::mcp::embedded::SKILL_ROOT_PATH
                }
                Some(("path", _)) => crate::mcp::embedded::PANGRAM_SKILL_PATH,
                _ => return help_outcome(invocation),
            };
            finish_raw_bytes(bytes, invocation)
        }
        Some(("completions", sub)) => execute_completions(sub, invocation),
        // A bare literal-text reach (`pangram some text`) resolves to implicit
        // detection; the literal `-` reads stdin. A no-text reach can only
        // come from the non-rendering parsing hook because process-facing bare
        // launches were claimed by bare dispatch or the TUI above.
        None if matches.get_one::<String>("TEXT").is_some() => {
            let text = matches.get_one::<String>("TEXT").unwrap().clone();
            if text == "-" {
                execute_detect_bare_source(
                    crate::cli::detect::Source::Stdin,
                    &matches,
                    global,
                    invocation,
                )
            } else {
                execute_detect_bare_source(
                    crate::cli::detect::Source::Literal(text),
                    &matches,
                    global,
                    invocation,
                )
            }
        }
        // A no-subcommand, no-text reach (`pangram --config PATH`, or a
        // `pangram --help` that reached the match stage) displays the same
        // help as `--help` with a successful exit, not an internal failure.
        None => help_outcome(invocation),
        _ => {
            let config_flag = matches.get_one::<String>("config").map(String::as_str);
            let data_dir_flag = matches.get_one::<String>("data-dir").map(String::as_str);
            let outcome =
                local_setup::dispatch(&matches, config_flag, data_dir_flag, invocation.streams);
            finish(outcome)
        }
    }
}

/// Generates one raw shell script from the same Clap tree used to parse the
/// process. Library callers only validate argv and never write process-owned
/// output; the compiled binary owns the script bytes on stdout.
fn execute_completions(arguments: &ArgMatches, invocation: &InvocationContext<'_>) -> RunOutcome {
    if !invocation.is_process_facing() {
        return RunOutcome {
            exit_code: 0,
            clap_error: None,
        };
    }

    let generator = arguments
        .get_one::<clap_complete::aot::Shell>("SHELL")
        .copied()
        .expect("Clap requires and validates the completion shell");
    let mut command = runtime_command();
    let mut script = Vec::new();
    clap_complete::aot::generate(generator, &mut command, FULL_GRAMMAR.name, &mut script);
    finish_raw_bytes(&script, invocation)
}

/// Runs the blocking stdio server only for the real process entrypoint.
/// Library callers use `try_run_from` as a no-I/O parser and therefore must
/// never take ownership of stdin or emit protocol bytes.
fn execute_mcp_server(
    arguments: &ArgMatches,
    root_matches: &ArgMatches,
    invocation: &InvocationContext<'_>,
) -> RunOutcome {
    if !invocation.is_process_facing() {
        return RunOutcome {
            exit_code: 0,
            clap_error: None,
        };
    }

    let options = super::mcp::serve_options(arguments);
    let mut flags = crate::config::ConfigOverrides::default();
    if let Some(config) = root_matches.get_one::<String>("config") {
        flags = flags.with_config_file(config.clone());
    }
    if let Some(data_dir) = root_matches.get_one::<String>("data-dir") {
        flags = flags.with_data_dir(data_dir.clone());
    }
    let overrides = crate::config::ConfigOverrides::merge(
        flags,
        crate::config::ConfigOverrides::from_environment(),
    );
    let service = match crate::config::ConfigService::new(&overrides) {
        Ok(service) => service,
        Err(error) => {
            let error = crate::analysis::config_error(error);
            return finish_mcp_startup_error(error.message());
        }
    };
    match crate::mcp::serve_stdio(options, invocation.analyzer_source.clone(), service) {
        Ok(()) => RunOutcome {
            exit_code: 0,
            clap_error: None,
        },
        Err(error) => finish_mcp_startup_error(&error.to_string()),
    }
}

fn finish_mcp_startup_error(message: &str) -> RunOutcome {
    RunOutcome {
        exit_code: super::mcp::render_stderr_line(message, 1),
        clap_error: None,
    }
}

/// Writes one immutable embedded document without decoding or normalizing it.
fn finish_raw_bytes(bytes: &[u8], invocation: &InvocationContext<'_>) -> RunOutcome {
    if !invocation.is_process_facing() {
        return RunOutcome {
            exit_code: 0,
            clap_error: None,
        };
    }

    use std::io::Write as _;
    let mut stdout = std::io::stdout().lock();
    let exit_code = if stdout.write_all(bytes).and_then(|_| stdout.flush()).is_ok() {
        0
    } else {
        1
    };
    RunOutcome {
        exit_code,
        clap_error: None,
    }
}

/// Intercepts the bare-source cases Clap cannot express. Returns `Some` when
/// it produced the final outcome; `None` falls through to the exact
/// Clap/help surface for everything else.
fn bare_dispatch(arguments: &[OsString], invocation: &InvocationContext<'_>) -> Option<RunOutcome> {
    // `pangram -` exactly: the literal stdin marker. Parse the root grammar
    // (which accepts it as the `[TEXT]` value) and run detection from stdin.
    if arguments.len() == 2 && arguments[1] == "-" {
        let matches = runtime_command().try_get_matches_from(arguments).ok()?;
        let global = crate::cli::detect::GlobalFlags::from_matches(&matches);
        return Some(execute_detect_bare_source(
            crate::cli::detect::Source::Stdin,
            &matches,
            global,
            invocation,
        ));
    }
    // Literally bare (`pangram`, no argv beyond the program name): a piped
    // non-TTY stdin is the detection source. For a process-facing invocation,
    // a TTY stdin with either output stream redirected is unsafe for the TUI
    // and resolves to input_required inside the same detection flow. The
    // non-rendering parsing hook retains its successful-help behavior for a
    // bare invocation whose stdin is a TTY.
    if arguments.len() == 1
        && (!invocation.streams.stdin()
            || (invocation.is_process_facing() && !invocation.streams.all_interactive()))
    {
        let matches = runtime_command().try_get_matches_from(arguments).ok()?;
        let global = crate::cli::detect::GlobalFlags::from_matches(&matches);
        return Some(execute_detect_bare_source(
            crate::cli::detect::Source::Stdin,
            &matches,
            global,
            invocation,
        ));
    }
    None
}

/// The configuration service and analyzer one analysis-family invocation
/// resolved. The service is shared with the history save seam so
/// `--data-dir`/`PANGRAM_DATA_DIR` precedence applies identically to storage.
pub(crate) struct PreparedAnalysis {
    pub(crate) analyzer: crate::analysis::Analyzer,
    pub(crate) service: crate::config::ConfigService,
}

/// Builds configuration, credentials, and the analyzer for a detection
/// request, returning them or (on failure) an already-renderable outcome.
///
/// `output` is the fully resolved rendering policy from the already-planned
/// request: prepare/credential/client failures surface through that exact
/// format and error surface (F6), so explicit `--format pretty` emits a
/// sanitized text error on stderr with empty stdout and its category exit
/// unless `--error-format json` overrides it.
pub(crate) fn prepare_detection(
    command: crate::output::ResolvedCommand,
    root_matches: &ArgMatches,
    output: crate::cli::detect::ResolvedOutput,
    started: crate::domain::UtcTimestamp,
    analyzer_source: &AnalyzerSource,
) -> Result<PreparedAnalysis, crate::cli::detect::DetectOutcome> {
    let mut flags = crate::config::ConfigOverrides::default();
    if let Some(config) = root_matches.get_one::<String>("config") {
        flags = flags.with_config_file(config.clone());
    }
    if let Some(data_dir) = root_matches.get_one::<String>("data-dir") {
        flags = flags.with_data_dir(data_dir.clone());
    }
    let overrides = crate::config::ConfigOverrides::merge(
        flags,
        crate::config::ConfigOverrides::from_environment(),
    );
    let service = crate::config::ConfigService::new(&overrides).map_err(|error| {
        crate::cli::detect::failure_outcome(
            command,
            output,
            started,
            crate::cli::detect::config_error(error),
        )
    })?;
    let analyzer = analyzer_source
        .resolve(&service)
        .map_err(|error| crate::cli::detect::failure_outcome(command, output, started, error))?;
    Ok(PreparedAnalysis { analyzer, service })
}

/// Routes one `pangram bulk <verb>` invocation to the shared bulk/task
/// adapter. Clap already enforced the leaf name and the closed flags.
fn execute_bulk_leaf(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: crate::cli::detect::GlobalFlags,
    invocation: &InvocationContext<'_>,
) -> RunOutcome {
    let Some((name, leaf)) = sub.subcommand() else {
        unreachable!("bulk requires a leaf subcommand");
    };
    let resolved = match name {
        "submit" => crate::output::ResolvedCommand::BulkSubmit,
        "status" => crate::output::ResolvedCommand::BulkStatus,
        "wait" => crate::output::ResolvedCommand::BulkWait,
        "results" => crate::output::ResolvedCommand::BulkResults,
        // arg_required_else_help makes a non-leaf reach impossible.
        _ => unreachable!("bulk requires a leaf subcommand"),
    };
    finish_detect(bulk::execute(
        resolved,
        leaf,
        root_matches,
        global,
        invocation.streams,
        invocation.analyzer_source,
    ))
}

/// Routes one `pangram task <verb>` invocation to the same adapter.
fn execute_task_leaf(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: crate::cli::detect::GlobalFlags,
    invocation: &InvocationContext<'_>,
) -> RunOutcome {
    let Some((name, leaf)) = sub.subcommand() else {
        unreachable!("task requires a leaf subcommand");
    };
    let resolved = match name {
        "status" => crate::output::ResolvedCommand::TaskStatus,
        "wait" => crate::output::ResolvedCommand::TaskWait,
        _ => unreachable!("task requires a leaf subcommand"),
    };
    finish_detect(bulk::execute(
        resolved,
        leaf,
        root_matches,
        global,
        invocation.streams,
        invocation.analyzer_source,
    ))
}

/// Runs an explicit `detect [TEXT|--file ...]` invocation. Argument and
/// source extraction never fail before this point: Clap already enforced the
/// closed flags and the exclusive source group.
fn execute_detect(
    matches: &ArgMatches,
    global: crate::cli::detect::GlobalFlags,
    invocation: &InvocationContext<'_>,
) -> RunOutcome {
    let started = crate::domain::UtcTimestamp::now();
    let arguments = match crate::cli::detect::DetectArgs::from_matches(matches) {
        Ok(arguments) => arguments,
        Err(error) => {
            return finish_detect(crate::cli::detect::early_failure(
                crate::output::ResolvedCommand::Detect,
                global,
                invocation.streams,
                started,
                error,
            ));
        }
    };
    let source = if let Some(files) = matches.get_many::<String>("file") {
        crate::cli::detect::Source::Files(files.cloned().collect())
    } else if let Some(text) = matches.get_one::<String>("TEXT") {
        if text == "-" {
            crate::cli::detect::Source::Stdin
        } else {
            crate::cli::detect::Source::Literal(text.clone())
        }
    } else {
        crate::cli::detect::Source::Stdin
    };

    run_detection(source, arguments, matches, global, None, invocation)
}

/// Runs a bare-source detection (literal text, `-`, or piped stdin) using
/// only defaults; no `detect` flags were supplied.
fn execute_detect_bare_source(
    source: crate::cli::detect::Source,
    root_matches: &ArgMatches,
    global: crate::cli::detect::GlobalFlags,
    invocation: &InvocationContext<'_>,
) -> RunOutcome {
    let arguments = crate::cli::detect::DetectArgs::for_bare();
    run_detection(source, arguments, root_matches, global, None, invocation)
}

/// Plans (validates and prices) the request before any credential work, then
/// resolves credentials and the analyzer only for a viable plan. Local input
/// errors therefore surface even when no key is configured.
fn run_detection(
    source: crate::cli::detect::Source,
    arguments: crate::cli::detect::DetectArgs,
    root_matches: &ArgMatches,
    global: crate::cli::detect::GlobalFlags,
    stdin_text: Option<String>,
    invocation: &InvocationContext<'_>,
) -> RunOutcome {
    let plan = match crate::cli::detect::plan(
        source,
        arguments,
        &global,
        invocation.streams,
        stdin_text,
    ) {
        Ok(plan) => plan,
        Err(outcome) => return finish_detect(outcome),
    };
    let output = plan.resolved_output();
    let prepared = match prepare_detection(
        crate::output::ResolvedCommand::Detect,
        root_matches,
        output,
        crate::domain::UtcTimestamp::now(),
        invocation.analyzer_source,
    ) {
        Ok(prepared) => prepared,
        Err(outcome) => return finish_detect(outcome),
    };
    finish_detect(crate::cli::detect::execute(
        &plan,
        prepared.analyzer,
        &prepared.service,
        invocation.streams,
    ))
}

/// Renders one executed detect command. When the dispatch already streamed a
/// projection (or a text error), the process layer only reports the exit
/// code; otherwise it prints the canonical JSON envelope(s) to stdout. A
/// write failure clears `primary_ok`, so a post-primary attachment can
/// never overwrite the honest render exit (contracts.md 14.2 note).
fn finish_detect(outcome: crate::cli::detect::DetectOutcome) -> RunOutcome {
    if outcome.rendered {
        return RunOutcome {
            exit_code: outcome.exit_code,
            clap_error: None,
        };
    }
    let mut exit_code = outcome.exit_code;
    {
        use std::io::Write as _;
        let mut stdout = std::io::stdout().lock();
        for envelope in &outcome.envelopes {
            match serde_json::to_string(envelope) {
                Ok(line) => {
                    if writeln!(stdout, "{line}")
                        .and_then(|_| stdout.flush())
                        .is_err()
                    {
                        exit_code = 1;
                    }
                }
                Err(_) => {
                    exit_code = 1;
                }
            }
        }
    }
    RunOutcome {
        exit_code,
        clap_error: None,
    }
}

/// Renders the fixed long-help text with a successful exit, or exit 1 when
/// the render or write fails.
fn help_outcome(invocation: &InvocationContext<'_>) -> RunOutcome {
    if !invocation.is_process_facing() {
        return RunOutcome {
            exit_code: 0,
            clap_error: None,
        };
    }
    let mut command = runtime_command();
    let mut buffer = Vec::new();
    if command.write_long_help(&mut buffer).is_err() {
        return RunOutcome {
            exit_code: 1,
            clap_error: None,
        };
    }
    use std::io::Write as _;
    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(&buffer).is_err() {
        return RunOutcome {
            exit_code: 1,
            clap_error: None,
        };
    }
    RunOutcome {
        exit_code: 0,
        clap_error: None,
    }
}

/// Renders one executed Phase 1 command: exactly one trailing newline, empty
/// stderr, and the resolved exit code. A command whose projection replaced
/// the envelope (`doctor --format pretty`) already printed its own output.
/// Construction failures stay internal and exit 1 without a rendered
/// envelope rather than fabricating a payload.
fn finish(outcome: PhaseOneOutcome) -> RunOutcome {
    let exit_code = match &outcome.envelope {
        Some(envelope) => match serde_json::to_string(envelope) {
            Ok(line) => {
                use std::io::Write as _;
                let mut stdout = std::io::stdout().lock();
                match writeln!(stdout, "{line}").and_then(|_| stdout.flush()) {
                    Ok(()) => outcome.exit_code,
                    Err(_) => 1,
                }
            }
            Err(_) => 1,
        },
        None => outcome.exit_code,
    };
    RunOutcome {
        exit_code,
        clap_error: None,
    }
}
