//! Process adapters for MCP serving, client setup, and embedded guidance.
//!
//! This module keeps Clap and process I/O out of the MCP module. The latter
//! owns protocol, installer, and embedded-resource behavior behind narrow
//! interfaces; the CLI only translates parsed arguments and renders bytes.

use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command};

use crate::mcp::install::{ClientTarget, InstallAction, InstallRequest, Installer};
use crate::output::{
    CanonicalError, CommandData, CommandEnvelope, EnvelopeMeta, ErrorCode, ExitCode,
    McpClientStatus, McpStatus, ResolvedCommand,
};

fn installer_command(name: &'static str, about: &'static str) -> Command {
    Command::new(name)
        .about(about)
        .arg(
            Arg::new("target")
                .long("target")
                .value_name("CLIENT")
                .value_parser(clap::value_parser!(ClientTarget))
                .action(ArgAction::Append)
                .help("Select one client; may be repeated"),
        )
        .arg(
            Arg::new("all")
                .long("all")
                .action(ArgAction::SetTrue)
                .help("Select every supported client"),
        )
        .arg(
            Arg::new("server-name")
                .long("server-name")
                .value_name("NAME")
                .default_value("pangram")
                .help("Exact MCP server entry name owned by this operation"),
        )
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .action(ArgAction::SetTrue)
                .help("Report exact planned changes without writing"),
        )
        .group(
            ArgGroup::new("target-selection")
                .args(["target", "all"])
                .required(true)
                .multiple(false),
        )
}

pub(crate) fn command() -> Command {
    Command::new("mcp")
        .about("Run and configure the Pangram MCP server")
        .subcommand_required(false)
        .arg_required_else_help(false)
        .arg(
            Arg::new("history")
                .long("history")
                .action(ArgAction::SetTrue)
                .help("Expose local history read tools"),
        )
        .arg(
            Arg::new("allow-history-mutations")
                .long("allow-history-mutations")
                .action(ArgAction::SetTrue)
                .help("Expose history mutations and allow saved MCP analyses"),
        )
        .arg(
            Arg::new("allow-config-mutations")
                .long("allow-config-mutations")
                .action(ArgAction::SetTrue)
                .help("Expose the restricted configuration mutation tool"),
        )
        .arg(
            Arg::new("allow-public-links")
                .long("allow-public-links")
                .action(ArgAction::SetTrue)
                .help("Allow tools to request public Pangram dashboard links"),
        )
        .arg(
            Arg::new("allow-file-root")
                .long("allow-file-root")
                .value_name("PATH")
                .action(ArgAction::Append)
                .help("Approve an absolute file root; may be repeated"),
        )
        .subcommand(installer_command(
            "install",
            "Add the Pangram MCP server to selected client configurations",
        ))
        .subcommand(installer_command(
            "uninstall",
            "Remove Pangram-owned entries from selected client configurations",
        ))
        .subcommand(
            Command::new("status")
                .about("Inspect Pangram MCP client configuration status")
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_name("FORMAT")
                        .value_parser(["json", "pretty"])
                        .help("Render canonical JSON or a readable status list"),
                ),
        )
}

pub(crate) fn agent_command() -> Command {
    Command::new("agent").about("Print compact Pangram MCP guidance as Markdown")
}

pub(crate) fn skills_command() -> Command {
    Command::new("skills")
        .about("Inspect Pangram's embedded agent skill")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("list").about("List embedded Pangram skills as Markdown"))
        .subcommand(
            Command::new("get")
                .about("Print an embedded skill as Markdown")
                .arg(Arg::new("SKILL").required(true).value_parser(["pangram"]))
                .arg(
                    Arg::new("full")
                        .long("full")
                        .action(ArgAction::SetTrue)
                        .help("Print the complete skill instead of compact guidance"),
                ),
        )
        .subcommand(
            Command::new("path")
                .about("Print the stable embedded locator for a skill")
                .arg(
                    Arg::new("SKILL")
                        .default_value("pangram")
                        .value_parser(["pangram"]),
                ),
        )
}

pub(crate) fn serve_options(arguments: &ArgMatches) -> crate::mcp::McpOptions {
    crate::mcp::McpOptions {
        history: arguments.get_flag("history"),
        allow_history_mutations: arguments.get_flag("allow-history-mutations"),
        allow_config_mutations: arguments.get_flag("allow-config-mutations"),
        allow_public_links: arguments.get_flag("allow-public-links"),
        allow_file_roots: arguments
            .get_many::<String>("allow-file-root")
            .into_iter()
            .flatten()
            .map(Into::into)
            .collect(),
    }
}

/// Executes an installer/status leaf and renders its complete process output.
/// The runtime calls this only in process mode, preserving `try_run_from` as
/// a parse-only interface with no filesystem or stdout side effects.
pub(crate) fn execute(arguments: &ArgMatches, global: crate::cli::detect::GlobalFlags) -> u8 {
    match arguments.subcommand() {
        Some(("install", leaf)) => mutation(leaf, InstallAction::Install, global.error_format_text),
        Some(("uninstall", leaf)) => {
            mutation(leaf, InstallAction::Uninstall, global.error_format_text)
        }
        Some(("status", leaf)) => status(leaf, global.error_format_text),
        _ => ExitCode::GeneralFailure.as_u8(),
    }
}

fn mutation(arguments: &ArgMatches, action: InstallAction, text_errors: Option<bool>) -> u8 {
    let command = match action {
        InstallAction::Install => ResolvedCommand::McpInstall,
        InstallAction::Uninstall => ResolvedCommand::McpUninstall,
    };
    let targets = selected_targets(arguments);
    let request = match InstallRequest::new(
        action,
        targets,
        arguments
            .get_one::<String>("server-name")
            .expect("Clap supplies the server-name default"),
        arguments.get_flag("dry-run"),
    ) {
        Ok(request) => request,
        Err(error) => return render_install_error(command, error, text_errors == Some(true)),
    };
    let installer = match Installer::from_process() {
        Ok(installer) => installer,
        Err(error) => return render_install_error(command, error, text_errors == Some(true)),
    };
    let report = match installer.apply(request) {
        Ok(report) => report,
        Err(error) => return render_install_error(command, error, text_errors == Some(true)),
    };
    let report = match report.to_output() {
        Ok(report) => report,
        Err(_) => return ExitCode::GeneralFailure.as_u8(),
    };
    let data = match action {
        InstallAction::Install => CommandData::McpInstall(report),
        InstallAction::Uninstall => CommandData::McpUninstall(report),
    };
    render_envelope(CommandEnvelope::success(data, EnvelopeMeta::default()), 0)
}

fn status(arguments: &ArgMatches, error_format_text: Option<bool>) -> u8 {
    let pretty = arguments
        .get_one::<String>("format")
        .is_some_and(|format| format == "pretty");
    let installer = match Installer::from_process() {
        Ok(installer) => installer,
        Err(error) => {
            return render_install_error(
                ResolvedCommand::McpStatus,
                error,
                error_format_text.unwrap_or(pretty),
            );
        }
    };
    let statuses = match installer.status(ClientTarget::ALL, "pangram") {
        Ok(statuses) => statuses,
        Err(error) => {
            return render_install_error(
                ResolvedCommand::McpStatus,
                error,
                error_format_text.unwrap_or(pretty),
            );
        }
    };
    let clients = statuses
        .iter()
        .map(|status| {
            let path = match status.path() {
                Some(path) => Some(path.to_str().ok_or(())?.to_owned()),
                None => None,
            };
            McpClientStatus::new(status.target().as_str(), status.installed(), path).map_err(|_| ())
        })
        .collect::<Result<Vec<_>, _>>();
    let clients = match clients {
        Ok(clients) => clients,
        Err(_) => return ExitCode::GeneralFailure.as_u8(),
    };
    let report = McpStatus::new(clients);
    let envelope =
        CommandEnvelope::success(CommandData::McpStatus(report), EnvelopeMeta::default());
    if pretty {
        return render_envelope_as(envelope, crate::output::OutputFormat::Pretty, 0);
    }
    render_envelope(envelope, 0)
}

fn selected_targets(arguments: &ArgMatches) -> Vec<ClientTarget> {
    if arguments.get_flag("all") {
        return ClientTarget::ALL.to_vec();
    }
    arguments
        .get_many::<ClientTarget>("target")
        .expect("Clap requires a target selection")
        .copied()
        .collect()
}

fn render_install_error(
    command: ResolvedCommand,
    error: impl std::fmt::Display,
    pretty: bool,
) -> u8 {
    let error =
        CanonicalError::new(ErrorCode::InvalidConfig, error.to_string()).unwrap_or_else(|_| {
            CanonicalError::new(
                ErrorCode::InvalidConfig,
                "MCP client configuration is invalid",
            )
            .expect("fixed message")
        });
    let exit = ExitCode::for_error(error.category()).as_u8();
    if pretty {
        return render_stderr_line(error.message(), exit);
    }
    render_envelope(
        CommandEnvelope::failure(command, error, EnvelopeMeta::default()),
        exit,
    )
}

fn render_envelope(envelope: CommandEnvelope, success_exit: u8) -> u8 {
    render_envelope_as(envelope, crate::output::OutputFormat::Json, success_exit)
}

fn render_envelope_as(
    envelope: CommandEnvelope,
    format: crate::output::OutputFormat,
    success_exit: u8,
) -> u8 {
    let mut stdout = std::io::stdout().lock();
    if crate::output::render(
        format,
        crate::output::ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
        &mut stdout,
    )
    .is_ok()
    {
        success_exit
    } else {
        ExitCode::GeneralFailure.as_u8()
    }
}

pub(crate) fn render_stderr_line(message: &str, intended_exit: u8) -> u8 {
    use std::io::Write as _;
    let mut stderr = std::io::stderr().lock();
    if writeln!(stderr, "{message}")
        .and_then(|_| stderr.flush())
        .is_ok()
    {
        intended_exit
    } else {
        ExitCode::GeneralFailure.as_u8()
    }
}
