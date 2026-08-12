use clap::{Arg, ArgAction, ArgGroup, Command};

pub(crate) mod bulk;
pub(crate) mod detect;
pub mod grammar;
mod history;
mod local_setup;
mod runtime;

pub(crate) use crate::config::redact_io;
#[cfg(test)]
pub(crate) use runtime::RealStreams;
pub(crate) use runtime::{StreamTty, prepare_detection};

#[cfg(feature = "dev-tools")]
pub(crate) use runtime::run_with_analyzer;
pub use runtime::{run, try_run_from};

pub use grammar::{
    ArgumentGroupSpec, ArgumentKind, ArgumentSpec, Availability, CommandKind, CommandSpec,
    FULL_GRAMMAR, GrammarSpec, Phase, full_grammar_reference,
};

const CONFIG_HELP: &str = "Explicit configuration file path for this invocation";

const API_KEY_HELP: &str = "\
SECURITY WARNING: argv may be visible in process listings and shell history.
Prefer `pangram auth`, `pangram auth set --api-key-stdin`, or the
PANGRAM_API_KEY environment variable.";

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
                .conflicts_with("save")
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
                .help("Persist this analysis in local history, even while automatic history is disabled"),
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

    let bulk_submit = Command::new("submit")
        .about("Submit an asynchronous Pangram 4 bulk AI-detection job")
        .arg(
            Arg::new("JSONL_PATH")
                .value_name("JSONL_PATH")
                .num_args(1)
                .help("Bulk JSONL file; the literal `-` reads stdin"),
        )
        .arg(
            Arg::new("max-billable-units")
                .long("max-billable-units")
                .value_name("N")
                .num_args(1)
                .required(true)
                .help("Reject the request when the estimated cost exceeds this ceiling"),
        )
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .action(ArgAction::SetTrue)
                .help("Report the canonical plan without credentials or network work"),
        )
        .arg(
            Arg::new("wait")
                .long("wait")
                .action(ArgAction::SetTrue)
                .help("Wait for the job to reach a terminal state before reporting"),
        )
        .arg(
            Arg::new("format")
                .long("format")
                .value_name("FORMAT")
                .value_parser(["json", "jsonl", "toon", "markdown", "pretty"])
                .help("Render the canonical envelope in the selected projection"),
        )
        .arg(
            Arg::new("progress")
                .long("progress")
                .value_name("MODE")
                .value_parser(["auto", "never", "jsonl"])
                .help("Progress reporting on stderr: auto, never, or canonical jsonl"),
        );

    let bulk_status = Command::new("status")
        .about("Read one Pangram 4 bulk job's canonical collection state")
        .arg(
            Arg::new("ID")
                .value_name("ID")
                .num_args(1)
                .required(true)
                .help("The upstream bulk job identity"),
        );

    let bulk_wait = Command::new("wait")
        .about("Wait for one Pangram 4 bulk job to reach a terminal state")
        .arg(
            Arg::new("ID")
                .value_name("ID")
                .num_args(1)
                .required(true)
                .help("The upstream bulk job identity"),
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
        );

    let bulk_results = Command::new("results")
        .about("Read one page of Pangram 4 bulk job results")
        .arg(
            Arg::new("ID")
                .value_name("ID")
                .num_args(1)
                .required(true)
                .help("The upstream bulk job identity"),
        )
        .arg(
            Arg::new("offset")
                .long("offset")
                .value_name("N")
                .num_args(1)
                .help("Zero-based page offset (default 0)"),
        )
        .arg(
            Arg::new("limit")
                .long("limit")
                .value_name("N")
                .num_args(1)
                .help("Page size within 1..=1000 (default 100; fetch-all only when no --limit is given and --offset stays 0)"),
        )
        .arg(
            Arg::new("format")
                .long("format")
                .value_name("FORMAT")
                .value_parser(["json", "jsonl", "toon", "markdown", "pretty"])
                .help("Render the canonical envelope in the selected projection"),
        );

    let bulk = Command::new("bulk")
        .about("Submit and inspect asynchronous Pangram 4 bulk AI-detection jobs")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(bulk_submit)
        .subcommand(bulk_status)
        .subcommand(bulk_wait)
        .subcommand(bulk_results);

    let task_status = Command::new("status")
        .about("Read one Pangram 4 task's canonical analysis state")
        .arg(
            Arg::new("ID")
                .value_name("ID")
                .num_args(1)
                .required(true)
                .help("An upstream Pangram task identity or saved local anl_ identity"),
        );

    let task_wait = Command::new("wait")
        .about("Wait for one Pangram 4 task to reach a terminal state")
        .arg(
            Arg::new("ID")
                .value_name("ID")
                .num_args(1)
                .required(true)
                .help("An upstream Pangram task identity or saved local anl_ identity"),
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
        );

    let task = Command::new("task")
        .about("Inspect or wait for a Pangram 4 text task")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(task_status)
        .subcommand(task_wait);

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
        .subcommand(bulk)
        .subcommand(task)
        .subcommand(history::command())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ArgMatches;
    use std::ffi::OsString;

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
        for literal in ["mcp", "update", "agent", "plagiarism"] {
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
        try_parse(&["detect", "t", "--detach", "--save"]).unwrap_err();
        try_parse(&["detect", "--file"]).unwrap_err();
        for format in ["json", "jsonl", "toon", "markdown", "pretty"] {
            try_parse(&["detect", "t", "--format", format]).unwrap();
        }
        for progress in ["auto", "never", "jsonl"] {
            try_parse(&["detect", "t", "--progress", progress]).unwrap();
        }
    }

    #[test]
    fn detect_help_lists_the_phase_four_surface() {
        let mut command = runtime_command();
        let detect = command.find_subcommand_mut("detect").unwrap();
        let rendered = detect.render_help().to_string();
        for fragment in [
            "[TEXT]",
            "--file",
            "--detach",
            "--format",
            "--include-input",
            "--save",
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
