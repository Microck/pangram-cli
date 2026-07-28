use std::ffi::OsString;

use clap::Command;

pub mod grammar;

pub use grammar::{
    ArgumentGroupSpec, ArgumentKind, ArgumentSpec, Availability, CommandKind, CommandSpec,
    FULL_GRAMMAR, GrammarSpec, Phase, full_grammar_reference,
};

/// Builds only the Phase 0 runtime surface. Later phases add command entries
/// here only when their compiled behavior and contract tests land together.
pub fn runtime_command() -> Command {
    Command::new(FULL_GRAMMAR.name)
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .version(env!("CARGO_PKG_VERSION"))
        .override_usage(FULL_GRAMMAR.name)
}

/// Parses a caller-supplied argv without exiting the process.
///
/// A bare invocation displays the same help as `--help`, but keeps the
/// successful exit status required by the Phase 0 executable contract.
pub fn try_run_from<I, T>(arguments: I) -> Result<(), clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut arguments: Vec<OsString> = arguments.into_iter().map(Into::into).collect();
    if arguments.is_empty() {
        arguments.push(FULL_GRAMMAR.name.into());
    }
    if arguments.len() == 1 {
        arguments.push("--help".into());
    }

    runtime_command().try_get_matches_from(arguments).map(drop)
}

/// Runs the process-facing CLI and returns a portable process status.
///
/// Clap owns whether help goes to stdout or usage errors go to stderr. This
/// function renders that output but never exits, so guarded callers remain in
/// control of process lifetime.
pub fn run() -> u8 {
    match try_run_from(std::env::args_os()) {
        Ok(()) => 0,
        Err(error) => {
            let exit_code = u8::try_from(error.exit_code()).unwrap_or(1);
            if error.print().is_ok() { exit_code } else { 1 }
        }
    }
}
