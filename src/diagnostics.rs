//! Sanitized local diagnostics for `pangram doctor`.
//!
//! This module owns the Phase 1 non-billable diagnostic contract. It runs
//! four ordered checks (`configuration`, `credentials`, `data_directory`,
//! `runtime`), never performs a Pangram network request, and never creates
//! or mutates local state. It does not own billable validation; the analysis
//! module remains the sole owner of submission, polling, and upstream
//! contract validation.
//!
//! All messages are sanitized: they name configuration keys, permission
//! bits, and safe paths only. They never include credential material,
//! submitted content, or raw environment dumps.

use std::fs;

use thiserror::Error;

use crate::config::{ConfigError, ConfigService};
use crate::output::{DoctorCheck, DoctorCheckStatus, DoctorStatus, OutputValidationError};

/// The closed, ordered Phase 1 check names.
pub const CHECK_NAMES: &[&str] = &["configuration", "credentials", "data_directory", "runtime"];

const API_KEY_GUIDANCE: &str = "https://www.pangram.com/apikey";

/// Diagnostics could not construct a typed report. This is not a local
/// health failure; unhealthy checks are reported as `fail` in the returned
/// `DoctorStatus` instead.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DiagnosticsError {
    #[error("could not construct diagnostic report: {0}")]
    OutputValidation(#[from] OutputValidationError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticsContext {
    ci: bool,
}

impl DiagnosticsContext {
    pub const fn new(ci: bool) -> Self {
        Self { ci }
    }

    pub fn from_environment() -> Self {
        Self::new(std::env::var_os("CI").is_some_and(|value| !value.is_empty()))
    }
}

/// Runs the ordered Phase 1 diagnostics against the provided configuration
/// service and returns the typed report. Local health failures are encoded
/// inside the report; an error is returned only when a check object cannot
/// be constructed at all.
pub fn run(
    service: &ConfigService,
    context: DiagnosticsContext,
) -> Result<DoctorStatus, DiagnosticsError> {
    let checks = vec![
        check_configuration(service),
        check_credentials(service),
        check_data_directory(service),
        check_runtime(context),
    ];
    Ok(DoctorStatus::new(checks))
}

/// Effective strict configuration loads without error.
fn check_configuration(service: &ConfigService) -> DoctorCheck {
    match service.effective() {
        Ok(_) => DoctorCheck::new("configuration", DoctorCheckStatus::Pass, None)
            .expect("the configuration check name is non-empty"),
        Err(error) => DoctorCheck::new(
            "configuration",
            DoctorCheckStatus::Fail,
            Some(sanitize_config_error(&error)),
        )
        .expect("the configuration check name is non-empty"),
    }
}

/// Credential resolution reports a source, a missing-key warning, or a
/// stored-credential failure. Never emits the key or its suffix.
///
/// The stored credential's health is probed independently of resolution:
/// `resolve()` prefers `PANGRAM_API_KEY` over the file and returns before
/// touching disk, so an exposed/unreadable `credentials.toml` would otherwise
/// be masked by a healthy environment key. `doctor` must still diagnose the
/// persistent store, so `read()` is run whenever the credentials file exists
/// and any error (including insecure permissions) drives `fail` regardless of
/// which source is effective.
fn check_credentials(service: &ConfigService) -> DoctorCheck {
    let credentials = service.credentials();
    match credentials.resolve(service.overrides()) {
        Ok(resolution) => {
            // Probe the persistent store directly: `read()` fails closed on a
            // lookup error other than NotFound, insecure permissions, or
            // malformed content, so an exposed key cannot hide behind a
            // working environment override.
            if let Err(error) = credentials.read() {
                return DoctorCheck::new(
                    "credentials",
                    DoctorCheckStatus::Fail,
                    Some(sanitize_config_error(&error)),
                )
                .expect("the credentials check name is non-empty");
            }
            if resolution.is_configured() {
                let source = match resolution.source() {
                    crate::config::CredentialSource::Environment => "environment (PANGRAM_API_KEY)",
                    crate::config::CredentialSource::Stored => "stored credentials",
                    crate::config::CredentialSource::None => {
                        unreachable!("is_configured() is true, so the source cannot be None")
                    }
                };
                DoctorCheck::new(
                    "credentials",
                    DoctorCheckStatus::Pass,
                    Some(format!("credential source: {source}")),
                )
                .expect("the credentials check name is non-empty")
            } else {
                DoctorCheck::new(
                    "credentials",
                    DoctorCheckStatus::Warn,
                    Some(format!(
                        "no API key is configured; obtain one at {API_KEY_GUIDANCE}"
                    )),
                )
                .expect("the credentials check name is non-empty")
            }
        }
        Err(error) => DoctorCheck::new(
            "credentials",
            DoctorCheckStatus::Fail,
            Some(sanitize_config_error(&error)),
        )
        .expect("the credentials check name is non-empty"),
    }
}

/// The data directory exists and is a readable directory, is absent and
/// therefore lazily creatable, or exists in a bad state. Never creates or
/// mutates the directory.
///
/// Lookup errors are never conflated with absence: when the metadata call
/// fails for any reason other than `NotFound` the check reports `fail` (which
/// drives exit 7), matching the normative doctor contract. When the path is a
/// directory, a non-mutating `read_dir` open proves the invoking user can
/// actually read it before `pass` is reported.
fn check_data_directory(service: &ConfigService) -> DoctorCheck {
    let path = service.paths().data_dir();
    let display = sanitize_text(&path.display().to_string());
    match fs::metadata(path) {
        Ok(_) if path.is_dir() => {
            if fs::read_dir(path).is_err() {
                return DoctorCheck::new(
                    "data_directory",
                    DoctorCheckStatus::Fail,
                    Some(format!("data directory is not readable: {display}")),
                )
                .expect("the data_directory check name is non-empty");
            }
            DoctorCheck::new(
                "data_directory",
                DoctorCheckStatus::Pass,
                Some(format!("data directory is ready: {display}")),
            )
            .expect("the data_directory check name is non-empty")
        }
        Ok(_) => DoctorCheck::new(
            "data_directory",
            DoctorCheckStatus::Fail,
            Some(format!("path exists but is not a directory: {display}")),
        )
        .expect("the data_directory check name is non-empty"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DoctorCheck::new(
            "data_directory",
            DoctorCheckStatus::Warn,
            Some(format!("data directory does not exist yet: {display}")),
        )
        .expect("the data_directory check name is non-empty"),
        Err(_) => DoctorCheck::new(
            "data_directory",
            DoctorCheckStatus::Fail,
            Some(format!("cannot access data directory: {display}")),
        )
        .expect("the data_directory check name is non-empty"),
    }
}

/// Replaces every Unicode control character with U+FFFD so a user-controlled
/// path interpolated at the projection boundary cannot inject terminal
/// control sequences or forge additional `doctor` lines.
///
/// `char::is_control` covers the ASCII C0 range (newline, carriage return,
/// tab, and the ANSI `ESC` byte), `DEL`, and the C1 range (U+0080 through
/// U+009F, including the CSI introducer some terminals still honor).
/// Printable text, including the ordinary space and all non-control Unicode,
/// passes through unchanged.
fn sanitize_text(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { '\u{FFFD}' } else { ch })
        .collect()
}

/// Package version, target platform, and CI state. Always passes.
fn check_runtime(context: DiagnosticsContext) -> DoctorCheck {
    let message = format!(
        "pangram {} on {}-{} (ci={})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        context.ci
    );
    DoctorCheck::new("runtime", DoctorCheckStatus::Pass, Some(message))
        .expect("the runtime check name is non-empty")
}

/// Renders a configuration or credential error without exposing secrets.
/// ConfigError messages are already sanitized by construction (they name
/// keys, paths, and permission bits only), but we render through the type
/// rather than a raw `format!("{error}")` on arbitrary types to make the
/// boundary explicit.
fn sanitize_config_error(error: &ConfigError) -> String {
    sanitize_text(&error.to_string())
}
