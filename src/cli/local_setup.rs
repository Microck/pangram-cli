//! Phase 1 CLI execution for `auth`, `config`, and `doctor`.
//!
//! The adapter is deliberately thin: it parses matched flags, merges flag
//! overrides over the environment, and calls the shared configuration and
//! diagnostics modules. All TOML, path-resolution, permission, and
//! atomic-write protocol logic lives in `crate::config` and
//! `crate::diagnostics`; this module owns only stream decisions (interactive
//! prompting, stdin/stdout), canonical error mapping, and envelope printing.
//!
//! Privacy invariants enforced here:
//! - stdin key material is handled through a zeroizing buffer and never
//!   crosses any error, debug, or log surface
//! - error mapping uses only the already-sanitized `ConfigError` messages
//! - failures print exactly one JSON envelope to stdout and keep stderr empty

use std::io::{BufRead as _, Read as _, Write};

use clap::ArgMatches;
use secrecy::ExposeSecret as _;
use zeroize::Zeroizing;

use crate::config::{
    ConfigError, ConfigOverrides, ConfigService, CredentialService, CredentialSource,
};
use crate::diagnostics::{self, DiagnosticsContext, DiagnosticsError};
use crate::output::{
    AuthSource, AuthStatus, CanonicalError, CommandData, CommandEnvelope, ConfigGetStatus,
    ConfigListStatus, ConfigPathStatus, DoctorCheckStatus, DoctorStatus, EnvelopeMeta, ErrorCode,
    ExitCode, MutationAcknowledgement, Recovery, ResolvedCommand,
};

use super::StreamTty;

/// One executed Phase 1 command before the process renders it. A `None`
/// envelope means the result could not be honestly rendered (typed
/// construction failure or a projection that replaced the envelope): the
/// process layer exits 1 without printing rather than fabricating a payload.
pub(crate) struct PhaseOneOutcome {
    pub(crate) exit_code: u8,
    pub(crate) envelope: Option<CommandEnvelope>,
}

impl PhaseOneOutcome {
    fn success(data: CommandData) -> Self {
        Self {
            exit_code: ExitCode::Success.as_u8(),
            envelope: Some(CommandEnvelope::success(data, EnvelopeMeta::default())),
        }
    }

    fn failure(command: ResolvedCommand, error: CanonicalError) -> Self {
        let exit_code = ExitCode::for_error(error.category()).as_u8();
        Self {
            exit_code,
            envelope: Some(CommandEnvelope::failure(
                command,
                error,
                EnvelopeMeta::default(),
            )),
        }
    }

    /// A construction failure that cannot honestly be rendered.
    fn internal() -> Self {
        Self {
            exit_code: ExitCode::GeneralFailure.as_u8(),
            envelope: None,
        }
    }
}

/// Routes a matched Phase 1 command. Planned commands graduate only with
/// their compiled behavior, so anything else is unreachable in this binary.
pub(crate) fn dispatch(
    matches: &ArgMatches,
    config_flag: Option<&str>,
    data_dir_flag: Option<&str>,
    streams: &dyn StreamTty,
) -> PhaseOneOutcome {
    match matches.subcommand() {
        Some(("auth", sub)) => auth(sub, config_flag, data_dir_flag, streams),
        Some(("config", sub)) => config(sub, config_flag, data_dir_flag),
        Some(("doctor", sub)) => doctor(sub, config_flag, data_dir_flag),
        _ => PhaseOneOutcome::internal(),
    }
}

/// Builds the shared service with flags overriding environment values. Path
/// resolution failures surface as their own local-config envelope per command.
///
/// The error is boxed: `PhaseOneOutcome` carries a full envelope and is far
/// larger than the happy-path `ConfigService`, so an unboxed `Result` would
/// needlessly inflate this hot return type.
fn service(
    command: ResolvedCommand,
    config_flag: Option<&str>,
    data_dir_flag: Option<&str>,
) -> Result<ConfigService, Box<PhaseOneOutcome>> {
    let mut flags = ConfigOverrides::default();
    if let Some(config) = config_flag {
        flags = flags.with_config_file(config);
    }
    if let Some(data_dir) = data_dir_flag {
        flags = flags.with_data_dir(data_dir);
    }
    let overrides = ConfigOverrides::merge(flags, ConfigOverrides::from_environment());
    ConfigService::new(&overrides)
        .map_err(|error| Box::new(PhaseOneOutcome::failure(command, config_error(error))))
}

fn auth(
    matches: &ArgMatches,
    config_flag: Option<&str>,
    data_dir_flag: Option<&str>,
    streams: &dyn StreamTty,
) -> PhaseOneOutcome {
    match matches.subcommand() {
        Some(("set", sub)) => auth_set(sub, config_flag, data_dir_flag),
        Some(("status", _)) => auth_status(config_flag, data_dir_flag),
        Some(("logout", sub)) => auth_logout(sub, config_flag, data_dir_flag, streams),
        // Bare `auth`: prompt for a masked key only on a fully interactive
        // terminal that is not running under CI. The local-setup contract
        // defines `CI` as disabling interactive behavior, so a job that
        // happens to allocate TTYs for all three streams still falls back to
        // the typed `auth status` report instead of blocking on a masked
        // prompt that no human will ever answer.
        _ if streams.all_interactive() && !is_ci() => auth_guided(config_flag, data_dir_flag),
        _ => auth_status(config_flag, data_dir_flag),
    }
}

/// True when the `CI` environment variable is set to a non-empty value,
/// matching the shared diagnostics definition of "running under CI".
fn is_ci() -> bool {
    std::env::var_os("CI").is_some_and(|value| !value.is_empty())
}

/// `pangram auth` on a fully interactive terminal: one masked read from the
/// controlling terminal, stored without any billable validation request.
fn auth_guided(config_flag: Option<&str>, data_dir_flag: Option<&str>) -> PhaseOneOutcome {
    let service = match service(ResolvedCommand::AuthSet, config_flag, data_dir_flag) {
        Ok(service) => service,
        Err(outcome) => return *outcome,
    };
    let key = match CredentialService::prompt_masked("Pangram API key: ") {
        Ok(key) => key,
        Err(error) => {
            return PhaseOneOutcome::failure(ResolvedCommand::AuthSet, config_error(error));
        }
    };
    match service.credentials().store(key.expose_secret()) {
        Ok(()) => PhaseOneOutcome::success(CommandData::AuthSet(MutationAcknowledgement::new())),
        Err(error) => PhaseOneOutcome::failure(ResolvedCommand::AuthSet, config_error(error)),
    }
}

/// `pangram auth set ...`: `--api-key` or `--api-key-stdin`, enforced as one
/// exclusive required group by Clap before this code runs.
fn auth_set(
    matches: &ArgMatches,
    config_flag: Option<&str>,
    data_dir_flag: Option<&str>,
) -> PhaseOneOutcome {
    let service = match service(ResolvedCommand::AuthSet, config_flag, data_dir_flag) {
        Ok(service) => service,
        Err(outcome) => return *outcome,
    };

    let key: Zeroizing<String> = if matches.get_flag("api-key-stdin") {
        match read_stdin_key() {
            Ok(key) => key,
            Err(error) => {
                return PhaseOneOutcome::failure(ResolvedCommand::AuthSet, *error);
            }
        }
    } else {
        // The group guarantees the flag; only non-UTF-8 argv can miss it.
        match matches.get_one::<String>("api-key") {
            Some(value) => Zeroizing::new(value.clone()),
            None => {
                return PhaseOneOutcome::failure(
                    ResolvedCommand::AuthSet,
                    usage_error(
                        ErrorCode::UnsupportedInput,
                        "a --api-key value must be valid UTF-8 text",
                    ),
                );
            }
        }
    };

    match service.credentials().store(&key) {
        Ok(()) => PhaseOneOutcome::success(CommandData::AuthSet(MutationAcknowledgement::new())),
        Err(error) => PhaseOneOutcome::failure(ResolvedCommand::AuthSet, config_error(error)),
    }
}

/// Reads all of stdin as UTF-8 and keeps exactly one non-empty line through
/// the credential service's own validator. The whole buffer stays zeroized.
///
/// The error is boxed so this small happy-path return type is not inflated by
/// the comparatively large `CanonicalError`.
fn read_stdin_key() -> Result<Zeroizing<String>, Box<CanonicalError>> {
    let mut buffer = Zeroizing::new(String::new());
    if std::io::stdin().lock().read_to_string(&mut buffer).is_err() {
        return Err(Box::new(usage_error(
            ErrorCode::UnsupportedInput,
            "stdin must be valid UTF-8 text",
        )));
    }
    let key = CredentialService::read_stdin_line(&buffer)
        .map(str::to_owned)
        .map(Zeroizing::new)
        .map_err(|_| {
            Box::new(usage_error(
                ErrorCode::UnsupportedInput,
                "--api-key-stdin accepts exactly one non-empty line",
            ))
        })?;
    Ok(key)
}

/// `pangram auth status` (and noninteractive bare `auth`): the masked source
/// and suffix. Local and non-billable by construction.
fn auth_status(config_flag: Option<&str>, data_dir_flag: Option<&str>) -> PhaseOneOutcome {
    let service = match service(ResolvedCommand::AuthStatus, config_flag, data_dir_flag) {
        Ok(service) => service,
        Err(outcome) => return *outcome,
    };
    match service.credentials().status(service.overrides()) {
        Ok((source, suffix)) => {
            let status = AuthStatus::new(
                source != CredentialSource::None,
                auth_source(source),
                suffix,
            );
            match status {
                Ok(status) => PhaseOneOutcome::success(CommandData::AuthStatus(status)),
                Err(_) => PhaseOneOutcome::internal(),
            }
        }
        Err(error) => PhaseOneOutcome::failure(ResolvedCommand::AuthStatus, config_error(error)),
    }
}

/// `pangram auth logout [--yes]`: removes only the stored key. An
/// environment credential is unaffected and stays active afterwards.
fn auth_logout(
    matches: &ArgMatches,
    config_flag: Option<&str>,
    data_dir_flag: Option<&str>,
    streams: &dyn StreamTty,
) -> PhaseOneOutcome {
    let service = match service(ResolvedCommand::AuthLogout, config_flag, data_dir_flag) {
        Ok(service) => service,
        Err(outcome) => return *outcome,
    };

    // The local-setup contract defines `CI` as noninteractive, so a job that
    // allocates TTYs for all three streams must still never block on the
    // confirmation prompt: under CI without `--yes` the command takes the
    // noninteractive usage-error path instead of `confirm_logout`.
    let interactive = streams.all_interactive() && !is_ci();

    if matches.get_flag("yes") || (interactive && confirm_logout(streams)) {
        return match service.credentials().remove() {
            Ok(()) => {
                PhaseOneOutcome::success(CommandData::AuthLogout(MutationAcknowledgement::new()))
            }
            Err(error) => {
                PhaseOneOutcome::failure(ResolvedCommand::AuthLogout, config_error(error))
            }
        };
    }

    if interactive {
        // Interactive decline: not a mutation, and not an error.
        return PhaseOneOutcome::success(CommandData::AuthLogout(MutationAcknowledgement::new()));
    }

    PhaseOneOutcome::failure(
        ResolvedCommand::AuthLogout,
        usage_error_with_recovery(
            ErrorCode::UnsupportedCombination,
            "without --yes, `pangram auth logout` requires an interactive terminal to confirm",
            "Re-run with --yes to remove the stored key noninteractively.",
            "pangram auth logout --yes",
        ),
    )
}

/// One plaintext confirmation read for interactive logout without `--yes`.
/// Because this path exists only when every stream is a TTY, blocking input
/// cannot hang a pipeline. The prompt goes to stderr so stdout stays a pure
/// envelope stream even in the interactive flow.
fn confirm_logout(streams: &dyn StreamTty) -> bool {
    if !streams.all_interactive() {
        return false;
    }
    eprint!("Remove the stored Pangram API key from this machine? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    match std::io::stdin().lock().read_line(&mut answer) {
        Ok(_) => matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"),
        Err(_) => false,
    }
}

fn config(
    matches: &ArgMatches,
    config_flag: Option<&str>,
    data_dir_flag: Option<&str>,
) -> PhaseOneOutcome {
    // `config` requires a subcommand (Clap owns that usage error), so the
    // resolved command is always one of these four.
    let (sub, command) = match matches.subcommand() {
        Some(("list", sub)) => (sub, ResolvedCommand::ConfigList),
        Some(("get", sub)) => (sub, ResolvedCommand::ConfigGet),
        Some(("set", sub)) => (sub, ResolvedCommand::ConfigSet),
        Some(("path", sub)) => (sub, ResolvedCommand::ConfigPath),
        _ => return PhaseOneOutcome::internal(),
    };
    let service = match service(command, config_flag, data_dir_flag) {
        Ok(service) => service,
        Err(outcome) => return *outcome,
    };
    match command {
        ResolvedCommand::ConfigList => match service.store().list_as_nested() {
            Ok(nested) => {
                // `preserve_order` gives nested values insertion order; the
                // top-level projection re-keys into the sorted BTreeMap the
                // envelope type requires.
                let config: std::collections::BTreeMap<String, serde_json::Value> =
                    nested.into_iter().collect();
                PhaseOneOutcome::success(CommandData::ConfigList(ConfigListStatus::new(config)))
            }
            Err(error) => PhaseOneOutcome::failure(command, config_error(error)),
        },
        ResolvedCommand::ConfigGet => {
            // Clap required-ness makes the value present; only non-UTF-8 argv
            // can miss the typed getter.
            let Some(key) = sub.get_one::<String>("KEY") else {
                return PhaseOneOutcome::failure(
                    command,
                    usage_error(ErrorCode::UnsupportedInput, "KEY must be valid UTF-8 text"),
                );
            };
            match service.get(key) {
                Ok(value) => match ConfigGetStatus::new(key, value.to_json()) {
                    Ok(value) => PhaseOneOutcome::success(CommandData::ConfigGet(value)),
                    Err(_) => PhaseOneOutcome::internal(),
                },
                Err(error) => PhaseOneOutcome::failure(command, config_error(error)),
            }
        }
        ResolvedCommand::ConfigSet => {
            let (Some(key), Some(value)) =
                (sub.get_one::<String>("KEY"), sub.get_one::<String>("VALUE"))
            else {
                return PhaseOneOutcome::failure(
                    command,
                    usage_error(
                        ErrorCode::UnsupportedInput,
                        "KEY and VALUE must be valid UTF-8 text",
                    ),
                );
            };
            match service.set(key, value) {
                Ok(_) => {
                    PhaseOneOutcome::success(CommandData::ConfigSet(MutationAcknowledgement::new()))
                }
                Err(error) => PhaseOneOutcome::failure(command, config_error(error)),
            }
        }
        ResolvedCommand::ConfigPath => {
            match ConfigPathStatus::new(
                service.paths().config_file().to_string_lossy().into_owned(),
            ) {
                Ok(value) => PhaseOneOutcome::success(CommandData::ConfigPath(value)),
                Err(_) => PhaseOneOutcome::internal(),
            }
        }
        _ => PhaseOneOutcome::internal(),
    }
}

fn doctor(
    matches: &ArgMatches,
    config_flag: Option<&str>,
    data_dir_flag: Option<&str>,
) -> PhaseOneOutcome {
    let service = match service(ResolvedCommand::Doctor, config_flag, data_dir_flag) {
        Ok(service) => service,
        Err(outcome) => return *outcome,
    };
    let report = match diagnostics::run(&service, DiagnosticsContext::from_environment()) {
        Ok(report) => report,
        Err(DiagnosticsError::OutputValidation(_)) => {
            // Reserved for impossible output-construction failures (14.9.1).
            return PhaseOneOutcome::internal();
        }
    };
    // Health-derived exit: any `fail` maps to the canonical local-state code
    // even though the payloads stay canonical reports. `warn`-only reports
    // remain a success. The report itself is unaffected by the exit code.
    let health_exit = if report.has_fail() {
        ExitCode::LocalState
    } else {
        ExitCode::Success
    };
    let format = matches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("json");
    match format {
        "pretty" => {
            // The envelope is consumed: the pretty projection replaces it. A
            // write or flush failure cannot honestly be rendered, so the
            // process exits with a general failure (1) instead of reporting a
            // healthy-looking exit code; health exit never overwrites that.
            let mut stdout = std::io::stdout().lock();
            match render_doctor_pretty(&report, &mut stdout) {
                Ok(()) => PhaseOneOutcome {
                    exit_code: health_exit.as_u8(),
                    envelope: None,
                },
                Err(_) => PhaseOneOutcome::internal(),
            }
        }
        // Clap's value_parser already rejected every other spelling. The JSON
        // path keeps the canonical success envelope (`data`, never `error`)
        // while adopting the health-derived exit code.
        _ => PhaseOneOutcome {
            exit_code: health_exit.as_u8(),
            envelope: Some(CommandEnvelope::success(
                CommandData::Doctor(report),
                EnvelopeMeta::default(),
            )),
        },
    }
}

/// The pretty projection is a plain-text rendering of the same typed checks:
/// the closed order is preserved, statuses print with a stable marker, and
/// message text remains exactly what the diagnostics module produced (already
/// sanitized; no upstream text appears in Phase 1).
///
/// Writing is injected so the process layer can surface failures: any write
/// or flush error aborts the render and propagates to the caller, which maps
/// it to a general failure rather than reporting false success.
fn render_doctor_pretty(report: &DoctorStatus, out: &mut dyn Write) -> std::io::Result<()> {
    for check in report.checks() {
        let marker = match check.status() {
            DoctorCheckStatus::Pass => "pass",
            DoctorCheckStatus::Warn => "warn",
            DoctorCheckStatus::Fail => "fail",
        };
        let line = match check.message() {
            Some(message) => format!("{marker:4} {}: {message}", check.name()),
            None => format!("{marker:4} {}", check.name()),
        };
        writeln!(out, "{line}")?;
    }
    out.flush()
}

fn auth_source(source: CredentialSource) -> AuthSource {
    match source {
        CredentialSource::None => AuthSource::None,
        CredentialSource::Environment => AuthSource::Environment,
        CredentialSource::Stored => AuthSource::Stored,
    }
}

/// Maps a sanitized configuration or credential error to its stable code.
/// Messages are already safe by construction of `ConfigError`; the adapter
/// never adds argv or key material of its own.
fn config_error(error: ConfigError) -> CanonicalError {
    let code = match &error {
        ConfigError::InsecurePermissions | ConfigError::RestrictionFailed => {
            ErrorCode::InsecureConfigPermissions
        }
        _ => ErrorCode::InvalidConfig,
    };
    CanonicalError::new(code, error.to_string())
        // The only invalidated message path is an empty string, which
        // ConfigError never produces; the fallback message stays fixed.
        .unwrap_or_else(|_| {
            CanonicalError::new(code, "local configuration is invalid")
                .expect("the fixed fallback message is non-empty")
        })
}

fn usage_error(code: ErrorCode, message: &str) -> CanonicalError {
    CanonicalError::new(code, message).expect("usage messages are non-empty")
}

fn usage_error_with_recovery(
    code: ErrorCode,
    message: &str,
    recovery_message: &str,
    recovery_command: &str,
) -> CanonicalError {
    let recovery = Recovery::new(recovery_message)
        .and_then(|recovery| recovery.with_command(recovery_command))
        .expect("fixed recovery text is non-empty");
    usage_error(code, message)
        .with_recovery(recovery)
        .expect("recovery is valid for usage codes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::DoctorCheck;

    fn report() -> DoctorStatus {
        DoctorStatus::new(vec![
            DoctorCheck::new("configuration", DoctorCheckStatus::Pass, None).unwrap(),
            DoctorCheck::new(
                "credentials",
                DoctorCheckStatus::Warn,
                Some("no key stored".to_owned()),
            )
            .unwrap(),
        ])
    }

    /// A writer that always fails, used to prove write errors are not
    /// swallowed by the pretty projection.
    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "synthetic write failure",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "synthetic flush failure",
            ))
        }
    }

    #[test]
    fn pretty_doctor_write_failure_propagates_instead_of_reporting_success() {
        let mut out = FailingWriter;
        let result = render_doctor_pretty(&report(), &mut out);
        assert!(
            result.is_err(),
            "a failing writer must surface an error, not report success"
        );
    }

    #[test]
    fn pretty_doctor_preserves_closed_check_order_and_markers() {
        let mut out: Vec<u8> = Vec::new();
        render_doctor_pretty(&report(), &mut out).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(
            rendered,
            "pass configuration\nwarn credentials: no key stored\n"
        );
    }
}
