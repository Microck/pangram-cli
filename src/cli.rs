use std::ffi::OsString;

use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command};

pub(crate) mod detect;
pub mod grammar;
mod local_setup;

pub(crate) use crate::config::redact_io;

pub use grammar::{
    ArgumentGroupSpec, ArgumentKind, ArgumentSpec, Availability, CommandKind, CommandSpec,
    FULL_GRAMMAR, GrammarSpec, Phase, full_grammar_reference,
};

use local_setup::PhaseOneOutcome;

const CONFIG_HELP: &str = "Explicit configuration file path for this invocation";

const API_KEY_HELP: &str = "\
SECURITY WARNING: argv may be visible in process listings and shell history.
Prefer `pangram auth`, `pangram auth set --api-key-stdin`, or the
PANGRAM_API_KEY environment variable.";

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

/// Builds the current runtime surface. Grammar entries graduate from the
/// planned table only when their compiled behavior and contract tests land
/// together; this Clap tree is the compiled mirror of those entries.
///
/// The compiled tree, not a debug-print of it, must stay the help surface:
/// keep the root `about` on the package description so `--help`, `version`,
/// and the committed `generated/cli-help.txt` fixture stay byte-identical.
pub fn runtime_command() -> Command {
    let auth_set = Command::new("set")
        .about("Store a Pangram API key in the protected local credential file")
        .arg(
            Arg::new("api-key")
                .long("api-key")
                .value_name("VALUE")
                .num_args(1)
                .help(API_KEY_HELP),
        )
        .arg(
            Arg::new("api-key-stdin")
                .long("api-key-stdin")
                .action(ArgAction::SetTrue)
                .help("Read exactly one API-key line from stdin (preferred for agents)"),
        )
        // A required, non-multiple group already enforces exactly one source:
        // self-conflicts would incorrectly reject every invocation, so the
        // group relies on `multiple(false)` alone for the mutual exclusion.
        .group(
            ArgGroup::new("api_key_source")
                .args(["api-key", "api-key-stdin"])
                .required(true)
                .multiple(false),
        );

    let auth = Command::new("auth")
        .about("Manage the locally stored Pangram API key")
        .long_about(
            "Manage the locally stored Pangram API key.\n\n\
             Without a subcommand, an interactive terminal prompts for a masked \
             key; over pipes it prints the same typed status as `auth status`.",
        )
        .subcommand_required(false)
        .arg_required_else_help(false)
        .subcommand(auth_set)
        .subcommand(Command::new("status").about("Show the local credential source (non-billable)"))
        .subcommand(
            Command::new("logout")
                .about("Remove the stored Pangram API key from this machine")
                .arg(
                    Arg::new("yes")
                        .long("yes")
                        .action(ArgAction::SetTrue)
                        .help("Remove the stored key without an interactive confirmation"),
                ),
        );

    let config = Command::new("config")
        .about("Inspect and edit the local Pangram configuration")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("list").about("Show the effective configuration"))
        .subcommand(
            Command::new("get")
                .about("Show one configuration value")
                .arg(
                    Arg::new("KEY")
                        .required(true)
                        .help("Configuration key, for example tui.intro"),
                ),
        )
        .subcommand(
            Command::new("set")
                .about("Set one configuration value")
                .arg(
                    Arg::new("KEY")
                        .required(true)
                        .help("Configuration key; credential keys are rejected"),
                )
                .arg(
                    Arg::new("VALUE")
                        .required(true)
                        .help("New value; closed values are validated before writing"),
                ),
        )
        .subcommand(Command::new("path").about("Show the resolved configuration file path"));

    let doctor = Command::new("doctor")
        .about("Run local diagnostics without network access or credential validation")
        .arg(
            Arg::new("format")
                .long("format")
                .value_name("FORMAT")
                .value_parser(["json", "pretty"])
                .help("Render the canonical JSON envelope or a readable check list"),
        );

    let detect = Command::new("detect")
        .about("Detect AI-generated text through Pangram 4")
        .arg(
            Arg::new("TEXT")
                .value_name("TEXT")
                .num_args(1)
                .help("Literal text to analyze; the literal `-` reads stdin"),
        )
        .arg(
            Arg::new("file")
                .long("file")
                .value_name("PATH")
                .num_args(1)
                .action(ArgAction::Append)
                .help("Read a UTF-8 text file; may be repeated"),
        )
        .arg(
            Arg::new("detach")
                .long("detach")
                .action(ArgAction::SetTrue)
                .help("Report the accepted task without waiting for the result"),
        )
        .arg(
            Arg::new("format")
                .long("format")
                .value_name("FORMAT")
                .value_parser(["json", "jsonl", "toon", "markdown", "pretty"])
                .help("Render the canonical envelope in the selected projection"),
        )
        .arg(
            Arg::new("include-input")
                .long("include-input")
                .action(ArgAction::SetTrue)
                .help("Include the submitted text in the canonical input record"),
        )
        .arg(
            Arg::new("save")
                .long("save")
                .action(ArgAction::SetTrue)
                .help("Save to local history (unavailable; history arrives in a later phase)"),
        )
        .arg(
            Arg::new("public-link")
                .long("public-link")
                .action(ArgAction::SetTrue)
                .help("Ask Pangram to create a public dashboard link for this analysis"),
        )
        .arg(
            Arg::new("timeout")
                .long("timeout")
                .value_name("DURATION")
                .num_args(1)
                .help("Bound the wait (seconds, or a value with an s, ms, m, or h suffix)"),
        )
        .arg(
            Arg::new("progress")
                .long("progress")
                .value_name("MODE")
                .value_parser(["auto", "never", "jsonl"])
                .help("Progress reporting on stderr: auto, never, or canonical jsonl"),
        )
        .arg(
            Arg::new("max-billable-units")
                .long("max-billable-units")
                .value_name("N")
                .num_args(1)
                .help("Reject the request when the estimated cost exceeds this ceiling"),
        )
        .group(
            ArgGroup::new("source_category")
                .args(["TEXT", "file"])
                .multiple(false),
        );

    Command::new(FULL_GRAMMAR.name)
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .version(env!("CARGO_PKG_VERSION"))
        .arg(
            Arg::new("TEXT")
                .value_name("TEXT")
                .num_args(1)
                .help("Bare text analyzes it through AI detection; the literal `-` reads stdin"),
        )
        .arg(
            Arg::new("config")
                .long("config")
                .value_name("PATH")
                .num_args(1)
                .global(true)
                .help(CONFIG_HELP),
        )
        .arg(
            Arg::new("data-dir")
                .long("data-dir")
                .value_name("PATH")
                .num_args(1)
                .global(true)
                .help("Explicit history and state directory for this invocation"),
        )
        .arg(
            Arg::new("error-format")
                .long("error-format")
                .value_name("FORMAT")
                .num_args(1)
                .global(true)
                .value_parser(["json", "text"])
                .help("Surface failures as a JSON envelope or a text message"),
        )
        .arg(
            Arg::new("no-color")
                .long("no-color")
                .action(ArgAction::SetTrue)
                .global(true)
                .help("Disable terminal color in pretty output"),
        )
        .subcommand(auth)
        .subcommand(config)
        .subcommand(doctor)
        .subcommand(detect)
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
    let error = run_arguments(arguments, &RealStreams, false).clap_error;
    match error {
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
    run_arguments(std::env::args_os(), &RealStreams, true).exit_code
}

/// The resolved result of one invocation before the process renders it.
struct RunOutcome {
    exit_code: u8,
    /// A Clap parse error returned to [`try_run_from`] callers; `run()`
    /// already printed it through Clap.
    clap_error: Option<clap::Error>,
}

/// Executes one argv and returns its intended exit code. When `render_clap`
/// is false, Clap output is suppressed; the library parsing hook uses this so
/// the process-facing `run()` path stays the only renderer.
fn run_arguments<I, T>(arguments: I, streams: &dyn StreamTty, render_clap: bool) -> RunOutcome
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut arguments: Vec<OsString> = arguments.into_iter().map(Into::into).collect();
    if arguments.is_empty() {
        arguments.push(FULL_GRAMMAR.name.into());
    }
    // Only a literally bare invocation behaves like `--help`. Explicit help
    // anywhere else is Clap-owned, including after a subcommand.
    if arguments.len() == 1 {
        arguments.push("--help".into());
    }

    // Bare-input dispatch runs before Clap's errors surface only for the
    // source-category rules Clap cannot express: `pangram -` (stdin), and a
    // bare non-TTY launch whose piped stdin is the implicit input. Every other
    // path stays Clap-owned so usage text and help keep their exact form.
    if let Some(outcome) = bare_dispatch(&arguments, streams) {
        return outcome;
    }

    let matches = match runtime_command().try_get_matches_from(arguments) {
        Ok(matches) => matches,
        Err(error) => {
            let exit_code = u8::try_from(error.exit_code()).unwrap_or(1);
            // Clap owns help/version stdout and usage-error stderr. A failed
            // render cannot make the output wrong; it only downgrades the
            // exit code to 1.
            let exit_code = if render_clap && error.print().is_err() {
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
        Some(("detect", sub)) => execute_detect(sub, global, streams),
        // A bare literal-text reach (`pangram some text`) resolves to implicit
        // detection; the literal `-` reads stdin. A bare launch with no text
        // and no subcommand falls through to the successful help surface (the
        // pre-TUI fallback for the all-TTY case).
        None if matches.get_one::<String>("TEXT").is_some() => {
            let text = matches.get_one::<String>("TEXT").unwrap().clone();
            if text == "-" {
                execute_detect_bare_source(
                    crate::cli::detect::Source::Stdin,
                    &matches,
                    global,
                    streams,
                )
            } else {
                execute_detect_bare_source(
                    crate::cli::detect::Source::Literal(text),
                    &matches,
                    global,
                    streams,
                )
            }
        }
        // A no-subcommand, no-text reach (`pangram --config PATH`, or a
        // `pangram --help` that reached the match stage) displays the same
        // help as `--help` with a successful exit, not an internal failure.
        None => help_outcome(render_clap),
        _ => {
            let config_flag = matches.get_one::<String>("config").map(String::as_str);
            let data_dir_flag = matches.get_one::<String>("data-dir").map(String::as_str);
            let outcome = local_setup::dispatch(&matches, config_flag, data_dir_flag, streams);
            finish(outcome)
        }
    }
}

/// Intercepts the bare-source cases Clap cannot express. Returns `Some` when
/// it produced the final outcome; `None` falls through to the exact
/// Clap/help surface for everything else.
fn bare_dispatch(arguments: &[OsString], streams: &dyn StreamTty) -> Option<RunOutcome> {
    // `pangram -` exactly: the literal stdin marker. Parse the root grammar
    // (which accepts it as the `[TEXT]` value) and run detection from stdin.
    if arguments.len() == 2 && arguments[1] == "-" {
        let matches = runtime_command().try_get_matches_from(arguments).ok()?;
        let global = crate::cli::detect::GlobalFlags::from_matches(&matches);
        return Some(execute_detect_bare_source(
            crate::cli::detect::Source::Stdin,
            &matches,
            global,
            streams,
        ));
    }
    // Literally bare (`pangram`, no argv beyond the program name): a piped
    // non-TTY stdin is the detection source. A TTY stdin falls through to
    // the bare-help/pre-TUI path below; an empty pipe resolves to
    // input_required inside the detection flow.
    if arguments.len() == 1 && !streams.stdin() {
        let matches = runtime_command().try_get_matches_from(arguments).ok()?;
        let global = crate::cli::detect::GlobalFlags::from_matches(&matches);
        return Some(execute_detect_bare_source(
            crate::cli::detect::Source::Stdin,
            &matches,
            global,
            streams,
        ));
    }
    None
}

/// Builds configuration, credentials, and the analyzer for a detection
/// request, returning them or (on failure) an already-renderable outcome.
/// The triple is heavy, so a boxed tuple keeps the error type small.
#[allow(clippy::type_complexity)]
fn prepare_detection(
    root_matches: &ArgMatches,
    global: crate::cli::detect::GlobalFlags,
    streams: &dyn StreamTty,
) -> Result<crate::analysis::Analyzer, crate::cli::detect::DetectOutcome> {
    let started = crate::domain::UtcTimestamp::now();
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
        crate::cli::detect::early_failure(
            global,
            streams,
            started,
            crate::cli::detect::credential_error(error),
        )
    })?;
    let api_key = crate::cli::detect::resolve_api_key(&service)
        .map_err(|error| crate::cli::detect::early_failure(global, streams, started, error))?;
    crate::cli::detect::build_analyzer(&service, api_key)
        .map_err(|error| crate::cli::detect::early_failure(global, streams, started, error))
}

/// Runs an explicit `detect [TEXT|--file ...]` invocation. Argument and
/// source extraction never fail before this point: Clap already enforced the
/// closed flags and the exclusive source group.
fn execute_detect(
    matches: &ArgMatches,
    global: crate::cli::detect::GlobalFlags,
    streams: &dyn StreamTty,
) -> RunOutcome {
    let started = crate::domain::UtcTimestamp::now();
    let arguments = match crate::cli::detect::DetectArgs::from_matches(matches) {
        Ok(arguments) => arguments,
        Err(error) => {
            return finish_detect(crate::cli::detect::early_failure(
                global, streams, started, error,
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

    run_detection(source, arguments, matches, global, streams, None)
}

/// Runs a bare-source detection (literal text, `-`, or piped stdin) using
/// only defaults; no `detect` flags were supplied.
fn execute_detect_bare_source(
    source: crate::cli::detect::Source,
    root_matches: &ArgMatches,
    global: crate::cli::detect::GlobalFlags,
    streams: &dyn StreamTty,
) -> RunOutcome {
    let arguments = crate::cli::detect::DetectArgs::for_bare();
    run_detection(source, arguments, root_matches, global, streams, None)
}

/// Plans (validates and prices) the request before any credential work, then
/// resolves credentials and the analyzer only for a viable plan. Local input
/// errors therefore surface even when no key is configured.
fn run_detection(
    source: crate::cli::detect::Source,
    arguments: crate::cli::detect::DetectArgs,
    root_matches: &ArgMatches,
    global: crate::cli::detect::GlobalFlags,
    streams: &dyn StreamTty,
    stdin_text: Option<String>,
) -> RunOutcome {
    let plan = match crate::cli::detect::plan(source, arguments, &global, streams, stdin_text) {
        Ok(plan) => plan,
        Err(outcome) => return finish_detect(outcome),
    };
    let analyzer = match prepare_detection(root_matches, global, streams) {
        Ok(analyzer) => analyzer,
        Err(outcome) => return finish_detect(outcome),
    };
    finish_detect(crate::cli::detect::execute(&plan, analyzer, streams))
}

/// Renders one executed detect command. When the dispatch already streamed a
/// projection (or a text error), the process layer only reports the exit
/// code; otherwise it prints the canonical JSON envelope(s) to stdout.
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
fn help_outcome(render_clap: bool) -> RunOutcome {
    let mut command = runtime_command();
    let mut buffer = Vec::new();
    if command.write_long_help(&mut buffer).is_err() {
        return RunOutcome {
            exit_code: 1,
            clap_error: None,
        };
    }
    if render_clap {
        use std::io::Write as _;
        let mut stdout = std::io::stdout().lock();
        if stdout.write_all(&buffer).is_err() {
            return RunOutcome {
                exit_code: 1,
                clap_error: None,
            };
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ArgMatches;

    fn try_parse(arguments: &[&str]) -> Result<ArgMatches, clap::Error> {
        let mut argv: Vec<OsString> = vec![FULL_GRAMMAR.name.into()];
        argv.extend(arguments.iter().map(OsString::from));
        runtime_command().try_get_matches_from(argv)
    }

    #[test]
    fn auth_set_requires_exactly_one_key_source() {
        try_parse(&["auth", "set"]).unwrap_err();
        try_parse(&["auth", "set", "--api-key", "x", "--api-key-stdin"]).unwrap_err();
        try_parse(&["auth", "set", "--api-key", "x"]).unwrap();
        try_parse(&["auth", "set", "--api-key-stdin"]).unwrap();
    }

    #[test]
    fn api_key_help_warns_about_argv_exposure() {
        let mut command = runtime_command();
        let auth = command.find_subcommand_mut("auth").unwrap();
        let set = auth.find_subcommand_mut("set").unwrap();
        let rendered = set.render_help().to_string();
        assert!(
            rendered.contains("argv may be visible in process listings and shell history"),
            "help must warn about argv exposure:\n{rendered}"
        );
    }

    #[test]
    fn hyphen_leading_unknowns_are_rejected_before_runtime_work() {
        // Only a hyphen-leading unknown stays a Clap usage error. A bare
        // token that spells a planned command name is legitimate literal
        // text for detection, not a rejected subcommand.
        for unknown in ["--frobnicate", "-z", "--not-a-real-flag"] {
            try_parse(&[unknown]).unwrap_err();
        }
        for literal in ["history", "mcp", "update", "agent", "plagiarism"] {
            let parsed = try_parse(&[literal]).unwrap();
            assert_eq!(parsed.get_one::<String>("TEXT").unwrap(), literal);
            assert!(parsed.subcommand().is_none());
        }
    }

    #[test]
    fn global_flags_are_accepted_everywhere() {
        for arguments in [
            &["--config", "/tmp/p.toml", "auth", "status"][..],
            &["auth", "status", "--config", "/tmp/p.toml"][..],
            &["config", "list", "--data-dir", "/tmp/d"][..],
            &["--data-dir=/tmp/d", "doctor"][..],
        ] {
            try_parse(arguments).unwrap();
        }
        // A mangled spelling stays a Clap usage error, never a silent flag.
        try_parse(&["--confi", "/tmp/p.toml"]).unwrap_err();
        try_parse(&["--config"]).unwrap_err();
    }

    #[test]
    fn doctor_format_is_closed() {
        try_parse(&["doctor", "--format", "json"]).unwrap();
        try_parse(&["doctor", "--format", "pretty"]).unwrap();
        try_parse(&["doctor", "--format", "yaml"]).unwrap_err();
    }

    #[test]
    fn detect_source_category_is_exactly_one() {
        try_parse(&["detect"]).unwrap();
        try_parse(&["detect", "some text"]).unwrap();
        try_parse(&["detect", "-"]).unwrap();
        try_parse(&["detect", "--file", "a.txt", "--file", "b.txt"]).unwrap();
        try_parse(&["detect", "text", "--file", "a.txt"]).unwrap_err();
        try_parse(&["detect", "-", "--file", "a.txt"]).unwrap_err();
    }

    #[test]
    fn detect_analysis_flags_are_closed_and_validated() {
        try_parse(&["detect", "t", "--format", "xml"]).unwrap_err();
        try_parse(&["detect", "t", "--progress", "sometimes"]).unwrap_err();
        try_parse(&["detect", "t", "--detach", "extra"]).unwrap_err();
        try_parse(&["detect", "--file"]).unwrap_err();
        for format in ["json", "jsonl", "toon", "markdown", "pretty"] {
            try_parse(&["detect", "t", "--format", format]).unwrap();
        }
        for progress in ["auto", "never", "jsonl"] {
            try_parse(&["detect", "t", "--progress", progress]).unwrap();
        }
    }

    #[test]
    fn detect_help_lists_the_phase_two_surface() {
        let mut command = runtime_command();
        let detect = command.find_subcommand_mut("detect").unwrap();
        let rendered = detect.render_help().to_string();
        for fragment in [
            "[TEXT]",
            "--file",
            "--detach",
            "--format",
            "--include-input",
            "--public-link",
            "--timeout",
            "--progress",
            "--max-billable-units",
        ] {
            assert!(
                rendered.contains(fragment),
                "help missing {fragment}:\n{rendered}"
            );
        }
    }
}
