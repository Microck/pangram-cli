//! Phase 1 contract tests for the `diagnostics` module.
//!
//! Each test builds its own temporary filesystem root and passes paths
//! explicitly; no test mutates process-global environment. The only
//! environment interaction is the `runtime` check reading `CI`, which is
//! tested twice (present and absent) and then restored to the caller's
//! original value.

use std::fs;
use std::path::PathBuf;

use microck_pangram_cli::config::{ConfigOverrides, ConfigService, Paths};
use microck_pangram_cli::diagnostics::{CHECK_NAMES, DiagnosticsContext, run};
use microck_pangram_cli::output::DoctorCheckStatus;
use tempfile::TempDir;

const SYNTHETIC_KEY: &str =
    "pangram_synthetic_diagnostics_test_key_0123456789abcdef_NOT_A_REAL_KEY";

struct Layout {
    _root: TempDir,
    platform_config_dir: PathBuf,
    platform_data_dir: PathBuf,
}

impl Layout {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let platform_config_dir = root.path().join("xdg-config").join("pangram");
        let platform_data_dir = root.path().join("xdg-data").join("pangram");
        fs::create_dir_all(&platform_config_dir).unwrap();
        fs::create_dir_all(&platform_data_dir).unwrap();
        Self {
            _root: root,
            platform_config_dir,
            platform_data_dir,
        }
    }

    fn paths(&self) -> Paths {
        Paths::for_test(
            self.platform_config_dir.clone(),
            self.platform_data_dir.clone(),
        )
    }

    fn overrides(&self) -> ConfigOverrides {
        ConfigOverrides::default()
            .with_config_file(self.paths().config_file().to_string_lossy().into_owned())
            .with_data_dir(self.paths().data_dir().to_string_lossy().into_owned())
    }

    fn service(&self) -> ConfigService {
        ConfigService::for_test(self.paths(), self.overrides())
    }

    fn credentials_file(&self) -> PathBuf {
        self.platform_config_dir.join("credentials.toml")
    }

    fn data_dir(&self) -> PathBuf {
        self.platform_data_dir.clone()
    }
}

#[cfg(unix)]
fn set_mode(path: &PathBuf, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// True when running as root without adding a `libc` dev-dependency. uid 0
/// bypasses DAC permission bits, so the permission-denial tests would
/// report a false green under it and must be skipped instead.
#[cfg(unix)]
fn running_as_root() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim() == "0")
        .unwrap_or(false)
}

/// Restores a directory mode on drop so a panicking assertion cannot leave a
/// temp directory unreadable for cleanup. Coupled with `running_as_root`, it
/// keeps the permission-denial tests hermetic.
#[cfg(unix)]
struct ModeRestore<'a> {
    path: &'a PathBuf,
    restore: u32,
}

#[cfg(unix)]
impl Drop for ModeRestore<'_> {
    fn drop(&mut self) {
        set_mode(self.path, self.restore);
    }
}

#[test]
fn clean_missing_key_report_has_exact_order_and_warn() {
    let layout = Layout::new();
    let service = layout.service();
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    let names: Vec<&str> = report.checks().iter().map(|check| check.name()).collect();
    assert_eq!(names, CHECK_NAMES, "check order must match the contract");

    let credentials = report
        .checks()
        .iter()
        .find(|check| check.name() == "credentials")
        .unwrap();
    assert_eq!(credentials.status(), DoctorCheckStatus::Warn);
    assert!(
        credentials
            .message()
            .unwrap()
            .contains("https://www.pangram.com/apikey"),
        "missing key guidance must mention the API key URL"
    );
}

#[cfg(unix)]
#[test]
fn exposed_stored_key_the_environment_credential_does_not_mask_it() {
    if running_as_root() {
        return;
    }
    // Precedence regression: resolve() prefers the environment key and would
    // otherwise report pass. An insecure stored file must still be diagnosed
    // because its exposure is independent of the effective source.
    let layout = Layout::new();
    microck_pangram_cli::config::CredentialService::new(layout.credentials_file())
        .store(SYNTHETIC_KEY)
        .unwrap();
    set_mode(&layout.credentials_file(), 0o644);

    let overrides = ConfigOverrides::default()
        .with_config_file(layout.paths().config_file().to_string_lossy().into_owned())
        .with_data_dir(layout.paths().data_dir().to_string_lossy().into_owned())
        .with_env_api_key("pangram_synthetic_env_key_0000000000000000_NOT_A_REAL_KEY");
    let service = ConfigService::for_test(layout.paths(), overrides);
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    let credentials = report
        .checks()
        .iter()
        .find(|check| check.name() == "credentials")
        .unwrap();
    assert_eq!(
        credentials.status(),
        DoctorCheckStatus::Fail,
        "an insecure stored key must fail doctor even with a healthy env key: {credentials:?}"
    );
    assert!(
        report.has_fail(),
        "an insecure stored key must drive exit 7 regardless of env precedence"
    );
    set_mode(&layout.credentials_file(), 0o600);
}

#[test]
fn malformed_config_causes_configuration_fail() {
    let layout = Layout::new();
    let config_file = layout.paths().config_file().to_owned();
    fs::write(&config_file, "config_version = [broken\n").unwrap();
    let service = layout.service();
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    let configuration = report
        .checks()
        .iter()
        .find(|check| check.name() == "configuration")
        .unwrap();
    assert_eq!(configuration.status(), DoctorCheckStatus::Fail);
    let message = configuration.message().unwrap();
    assert!(
        message.contains("could not parse TOML") || message.contains("invalid"),
        "configuration failure should be sanitized: {message}"
    );
}

#[cfg(unix)]
#[test]
fn insecure_credential_permissions_cause_credentials_fail() {
    let layout = Layout::new();
    microck_pangram_cli::config::CredentialService::new(layout.credentials_file())
        .store(SYNTHETIC_KEY)
        .unwrap();
    set_mode(&layout.credentials_file(), 0o644);

    let service = layout.service();
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    let credentials_check = report
        .checks()
        .iter()
        .find(|check| check.name() == "credentials")
        .unwrap();
    assert_eq!(credentials_check.status(), DoctorCheckStatus::Fail);
    let message = credentials_check.message().unwrap();
    assert!(
        message.contains("owner-only permissions"),
        "insecure permissions should be named: {message}"
    );
    assert!(
        !message.contains(SYNTHETIC_KEY),
        "key leaked into credentials failure message"
    );
}

#[test]
fn absent_data_directory_warns_lazily() {
    let layout = Layout::new();
    // Remove the directory after the layout helper created it.
    fs::remove_dir_all(layout.data_dir()).unwrap();
    let service = layout.service();
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    let data_dir = report
        .checks()
        .iter()
        .find(|check| check.name() == "data_directory")
        .unwrap();
    assert_eq!(data_dir.status(), DoctorCheckStatus::Warn);
    assert!(
        data_dir.message().unwrap().contains("does not exist yet"),
        "absent data directory should warn about lazy creation: {data_dir:?}"
    );
}

#[test]
fn path_is_file_causes_data_directory_fail() {
    let layout = Layout::new();
    fs::remove_dir_all(layout.data_dir()).unwrap();
    fs::write(layout.data_dir(), b"not a directory").unwrap();
    let service = layout.service();
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    let data_dir = report
        .checks()
        .iter()
        .find(|check| check.name() == "data_directory")
        .unwrap();
    assert_eq!(data_dir.status(), DoctorCheckStatus::Fail);
    assert!(
        data_dir.message().unwrap().contains("not a directory"),
        "file path should fail the data directory check: {data_dir:?}"
    );
}

#[cfg(unix)]
#[test]
fn unreadable_data_directory_fails_instead_of_passing() {
    if running_as_root() {
        // uid 0 bypasses DAC permission bits, so 0o000 cannot deny access and
        // this scenario would report a false green; skip instead.
        return;
    }
    let layout = Layout::new();
    let dir = layout.data_dir();
    // 0o000 strips read/execute so a directory open is denied; `is_dir`
    // metadata may still succeed, so this proves the readability probe wires
    // `fail` and drives the exit-7 path. The guard restores the mode even if
    // an assertion panics, so cleanup never races an unreadable directory.
    let _restore = ModeRestore {
        path: &dir,
        restore: 0o755,
    };
    set_mode(&dir, 0o000);
    let service = layout.service();
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    let data_dir = report
        .checks()
        .iter()
        .find(|check| check.name() == "data_directory")
        .unwrap();
    assert_eq!(
        data_dir.status(),
        DoctorCheckStatus::Fail,
        "an unreadable directory must fail closed: {data_dir:?}"
    );
    assert!(
        report.has_fail(),
        "unreadable directory must surface via has_fail for exit 7"
    );
}

#[cfg(unix)]
#[test]
fn unsearchable_credentials_parent_fails_instead_of_reporting_absent() {
    if running_as_root() {
        // uid 0 bypasses DAC permission bits; see the unreadable-directory
        // scenario above for why this must skip rather than guess.
        return;
    }
    let layout = Layout::new();
    // 0o000 on the config parent makes `metadata(credentials.toml)` fail with
    // PermissionDenied rather than NotFound; the read must therefore fail
    // closed as `Fail` instead of suppressing to the "no key" `Warn`.
    let _restore = ModeRestore {
        path: &layout.platform_config_dir,
        restore: 0o755,
    };
    set_mode(&layout.platform_config_dir, 0o000);
    let service = layout.service();
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    let credentials = report
        .checks()
        .iter()
        .find(|check| check.name() == "credentials")
        .unwrap();
    assert_eq!(
        credentials.status(),
        DoctorCheckStatus::Fail,
        "an unsearchable credential parent must fail closed, not report absent: {credentials:?}"
    );
}

#[cfg(unix)]
#[test]
fn control_bytes_in_data_dir_path_never_reach_the_message() {
    let layout = Layout::new();
    // A path containing an ANSI escape introducer, a newline, a carriage
    // return, and a tab. `PathBuf` cannot always host raw control bytes on a
    // real filesystem portably, so a directory is created at the control path
    // only when the platform permits; otherwise the message is built from the
    // path string that was provided without creation.
    let control_path = tempfile::tempdir()
        .unwrap()
        .path()
        .join("pangr\u{001B}[2J\nan\r\t-cli");
    let overrides = ConfigOverrides::default()
        .with_config_file(layout.paths().config_file().to_string_lossy().into_owned())
        .with_data_dir(control_path.to_string_lossy().into_owned());
    let paths = Paths::for_test(layout.platform_config_dir.clone(), control_path.clone());
    let service = ConfigService::for_test(paths, overrides);
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    for check in report.checks() {
        if let Some(message) = check.message() {
            assert!(
                !message.bytes().any(|b| b == 0x1B),
                "escape byte leaked into check message: {message:?}"
            );
            assert!(
                !message.contains('\n') && !message.contains('\r') && !message.contains('\t'),
                "control whitespace leaked into check message: {message:?}"
            );
        }
    }
}

#[test]
fn runtime_reports_ci_true() {
    let layout = Layout::new();
    let service = layout.service();

    let report = run(&service, DiagnosticsContext::new(true)).unwrap();

    let runtime = report
        .checks()
        .iter()
        .find(|check| check.name() == "runtime")
        .unwrap();
    assert_eq!(runtime.status(), DoctorCheckStatus::Pass);
    let message = runtime.message().unwrap();
    assert!(
        message.contains("ci=true"),
        "CI flag must be reported when set: {message}"
    );
    assert!(
        message.contains(env!("CARGO_PKG_VERSION")),
        "runtime message must name the package version: {message}"
    );
    assert!(
        message.contains(std::env::consts::OS),
        "runtime message must name the OS: {message}"
    );
    assert!(
        message.contains(std::env::consts::ARCH),
        "runtime message must name the architecture: {message}"
    );
}

#[test]
fn runtime_reports_ci_false() {
    let layout = Layout::new();
    let service = layout.service();

    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    let runtime = report
        .checks()
        .iter()
        .find(|check| check.name() == "runtime")
        .unwrap();
    let message = runtime.message().unwrap();
    assert!(
        message.contains("ci=false"),
        "CI flag must report false when absent: {message}"
    );
}

#[test]
fn report_serializes_without_key_material() {
    let layout = Layout::new();
    microck_pangram_cli::config::CredentialService::new(layout.credentials_file())
        .store(SYNTHETIC_KEY)
        .unwrap();
    let service = layout.service();
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    let json = serde_json::to_string(&report).unwrap();
    assert!(
        !json.contains(SYNTHETIC_KEY),
        "key leaked into JSON serialization: {json}"
    );

    let debug = format!("{report:?}");
    assert!(
        !debug.contains(SYNTHETIC_KEY),
        "key leaked into Debug: {debug}"
    );
}

// The credentials failure is induced by a Unix file mode (`0o644`), which the
// persistence layer reads as insecure. Windows enforces owner-only ACLs rather
// than modes, so this scenario is Unix-only and matches the gated pattern used
// by `insecure_credential_permissions_cause_credentials_fail` above.
#[cfg(unix)]
#[test]
fn report_is_returned_even_when_checks_fail() {
    let layout = Layout::new();
    let config_file = layout.paths().config_file().to_owned();
    fs::write(&config_file, "config_version = [broken\n").unwrap();
    microck_pangram_cli::config::CredentialService::new(layout.credentials_file())
        .store(SYNTHETIC_KEY)
        .unwrap();
    set_mode(&layout.credentials_file(), 0o644);
    fs::remove_dir_all(layout.data_dir()).unwrap();
    fs::write(layout.data_dir(), b"not a directory").unwrap();

    let service = layout.service();
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();

    assert_eq!(
        report.checks()[0].status(),
        DoctorCheckStatus::Fail,
        "configuration should fail"
    );
    assert_eq!(
        report.checks()[1].status(),
        DoctorCheckStatus::Fail,
        "credentials should fail"
    );
    assert_eq!(
        report.checks()[2].status(),
        DoctorCheckStatus::Fail,
        "data directory should fail"
    );
    assert_eq!(
        report.checks()[3].status(),
        DoctorCheckStatus::Pass,
        "runtime should still pass"
    );

    // The exit-code predicate the CLI relies on: unhealthy local state is
    // visible on the typed report so the adapter exits 7 without reparsing.
    assert!(
        report.has_fail(),
        "a report carrying any `fail` check must surface via has_fail"
    );
}

#[test]
fn pass_and_warn_only_reports_do_not_trigger_the_failing_exit() {
    let layout = Layout::new();
    let service = layout.service();
    let report = run(&service, DiagnosticsContext::new(false)).unwrap();
    assert!(
        !report.has_fail(),
        "a pass/warn-only report must not trigger the exit-7 path: {:?}",
        report
            .checks()
            .iter()
            .map(|check| (check.name(), check.status()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn diagnostics_source_contains_no_forbidden_network_patterns() {
    let source = include_str!("../src/diagnostics.rs");
    for pattern in [
        "TcpStream",
        "TcpListener",
        "reqwest",
        "ureq",
        "hyper",
        "dns",
    ] {
        assert!(
            !source.contains(pattern),
            "diagnostics module must not contain network API pattern `{pattern}`"
        );
    }
}
