use std::ffi::OsString;

use clap::{Arg, ArgAction, ArgGroup, Command};

pub mod grammar;
mod local_setup;

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

    Command::new(FULL_GRAMMAR.name)
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .version(env!("CARGO_PKG_VERSION"))
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
        .subcommand(auth)
        .subcommand(config)
        .subcommand(doctor)
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

    // A no-subcommand reach (bare, or only global flags such as
    // `pangram --config PATH`) must display the same help as `--help` with a
    // successful exit, not fall through to an internal failure. The bare
    // `--help` injection above already covers `arguments.len() == 1`; this
    // covers the global-flag-only spelling without changing a bare parse.
    if matches.subcommand().is_none() {
        let mut command = runtime_command();
        let mut buffer = Vec::new();
        // Help is fixed, renderable text; failure to render cannot honestly be
        // a 0 and degrades to the general failure exit 1.
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
        return RunOutcome {
            exit_code: 0,
            clap_error: None,
        };
    }

    // Matched global values override their environment counterparts; Clap
    // owns the spelling and value validation of every occurrence.
    let config_flag = matches.get_one::<String>("config").map(String::as_str);
    let data_dir_flag = matches.get_one::<String>("data-dir").map(String::as_str);
    let outcome = local_setup::dispatch(&matches, config_flag, data_dir_flag, streams);
    finish(outcome)
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
    fn planned_commands_are_rejected_before_runtime_work() {
        for planned in ["detect", "history", "mcp", "update", "agent"] {
            try_parse(&[planned]).unwrap_err();
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
}
