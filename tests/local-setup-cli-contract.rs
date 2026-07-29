// Phase 1 tests pin the observable contract for commands not yet compiled.
// They fail with the Phase 0 planned-command argument error until implemented.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use tempfile::TempDir;

/// A credential distinct and long enough that any output appearance is a leak
/// and whose trailing characters prove status masking.
const SYNTHETIC_API_KEY: &str =
    "pangram_synthetic_contract_test_key_0000000000000000_NOT_A_REAL_KEY";

fn pangram() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pangram"))
}

/// Runs `pangram` with all credential, config, and data state rooted in one
/// temporary directory. Each call builds a fresh child `Command` with its own
/// environment, so repeated invocations never share argv, variables, or files.
struct Isolated {
    // Kept alive so the directories outlive every child process.
    _root: TempDir,
    env: Vec<(String, String)>,
    explicit_config: PathBuf,
    platform_config_dir: PathBuf,
}

impl Isolated {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let xdg_config = root.path().join("xdg-config");
        let xdg_data = root.path().join("xdg-data");
        let explicit_config = root.path().join("explicit").join("pangram.toml");
        let data_dir = root.path().join("data");
        for directory in [
            &home,
            &xdg_config,
            &xdg_data,
            explicit_config.parent().unwrap(),
            &data_dir,
        ] {
            fs::create_dir_all(directory).unwrap();
        }

        let env = [
            ("HOME", home.to_str().unwrap()),
            ("XDG_CONFIG_HOME", xdg_config.to_str().unwrap()),
            ("XDG_DATA_HOME", xdg_data.to_str().unwrap()),
            ("PANGRAM_CONFIG", explicit_config.to_str().unwrap()),
            ("PANGRAM_DATA_DIR", data_dir.to_str().unwrap()),
            ("CI", "true"),
            ("TERM", "dumb"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();

        Self {
            _root: root,
            env,
            explicit_config,
            platform_config_dir: xdg_config.join("pangram"),
        }
    }

    /// Builds one private child `Command`. The credential override is cleared
    /// by default; a test sets it deliberately only on the child that needs it.
    fn command(&self) -> Command {
        let mut command = pangram();
        command.env_remove("PANGRAM_API_KEY");
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }

    fn output(&self, args: &[&str]) -> std::process::Output {
        self.run(args, &[], None)
    }

    /// Builds one child with optional extra env and piped stdin, and runs it.
    fn run(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
        input: Option<&str>,
    ) -> std::process::Output {
        let mut command = self.command();
        command.args(args);
        for (key, value) in env {
            command.env(key, value);
        }

        let Some(input) = input else {
            return command
                .stdin(Stdio::null())
                .output()
                .expect("failed to run pangram");
        };
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn pangram");
        // A child that rejects its arguments closes the pipe before reading;
        // only a genuine write error should abort the test.
        if let Err(error) = child.stdin.as_mut().unwrap().write_all(input.as_bytes()) {
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe,
                "stdin: {error}"
            );
        }
        child.wait_with_output().expect("failed to await pangram")
    }

    fn credentials_file(&self) -> PathBuf {
        self.platform_config_dir.join("credentials.toml")
    }
}

/// Parses one canonical envelope, enforcing the one-of data/error invariant.
fn envelope(output: &std::process::Output, context: &str) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "{context}: stdout is not one JSON envelope: {error}\nstdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(envelope["schema_version"], "1", "{context}");
    assert_ne!(
        envelope.get("data").is_some(),
        envelope.get("error").is_some(),
        "{context}: envelope must hold exactly one of data/error: {envelope}"
    );
    envelope
}

/// Asserts the canonical success envelope for `command` and returns `data`.
fn success_data(output: &std::process::Output, command: &str) -> Value {
    let context = format!("{command} success");
    let envelope = envelope(output, &context);
    assert_eq!(envelope["command"], command, "{context}");
    assert!(
        output.status.success(),
        "{context}: exit status {:?}",
        output.status
    );
    assert!(
        output.stderr.is_empty(),
        "{context}: stderr must be empty on success: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    envelope["data"].clone()
}

/// Asserts the canonical failure envelope for `command` at `exit` with the
/// stable code and category, returning the error object.
fn failure_error(
    output: &std::process::Output,
    command: &str,
    exit: i32,
    code: &str,
    category: &str,
) -> Value {
    let context = format!("{command} failure");
    let envelope = envelope(output, &context);
    assert_eq!(envelope["command"], command, "{context}");
    assert_eq!(output.status.code(), Some(exit), "{context}");
    let error = &envelope["error"];
    assert_eq!(error["code"], code, "{context}");
    assert_eq!(error["category"], category, "{context}");
    assert!(error["message"].is_string(), "{context}");
    error.clone()
}

/// Asserts the credential never appears in either output stream.
fn assert_key_never_leaks(output: &std::process::Output, context: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains(SYNTHETIC_API_KEY),
        "{context}: key leaked to stdout"
    );
    assert!(
        !stderr.contains(SYNTHETIC_API_KEY),
        "{context}: key leaked to stderr"
    );
}

#[test]
fn config_path_returns_the_resolved_path() {
    let isolated = Isolated::new();
    let output = isolated.output(&["config", "path"]);

    let data = success_data(&output, "config_path");
    let reported = data["path"].as_str().expect("config_path data.path");
    assert_eq!(Path::new(reported), isolated.explicit_config.as_path());
}

#[test]
fn config_rejects_unknown_keys_strictly() {
    let isolated = Isolated::new();
    fs::write(
        &isolated.explicit_config,
        "config_version = 1\nunknown_key = true\n",
    )
    .unwrap();
    let output = isolated.output(&["config", "list"]);

    let error = failure_error(&output, "config_list", 7, "invalid_config", "local_config");
    assert!(
        error["message"].as_str().unwrap().contains("unknown_key"),
        "strict rejection names the unknown key: {error}"
    );
}

#[test]
fn config_set_rejects_credential_keys() {
    let isolated = Isolated::new();
    let output = isolated.output(&["config", "set", "credentials.api_key", SYNTHETIC_API_KEY]);

    failure_error(&output, "config_set", 7, "invalid_config", "local_config");
    assert_key_never_leaks(&output, "config set credential rejection");
}

#[test]
fn config_set_get_and_list_roundtrip_a_non_secret_value() {
    let isolated = Isolated::new();

    let set_output = isolated.output(&["config", "set", "tui.intro", "off"]);
    assert_eq!(success_data(&set_output, "config_set")["ok"], true);

    let get = success_data(
        &isolated.output(&["config", "get", "tui.intro"]),
        "config_get",
    );
    assert_eq!(get["key"], "tui.intro");
    assert_eq!(get["value"], "off");

    let list = success_data(&isolated.output(&["config", "list"]), "config_list");
    assert_eq!(list["config"]["tui"]["intro"], "off");
}

#[test]
fn config_get_on_an_absent_file_returns_effective_documented_defaults() {
    let isolated = Isolated::new();
    // The explicit config file is never written: every key resolves purely
    // from built-in defaults. `get` must be typed and match `list`.
    assert!(
        !isolated.explicit_config.exists(),
        "precheck: no config file exists"
    );

    let cases: [(&str, Value); 5] = [
        ("tui.intro", Value::String("once".into())),
        ("tui.keymap", Value::String("regular".into())),
        ("tui.motion", Value::String("full".into())),
        ("history.enabled", Value::Bool(false)),
        ("network.max_requests_per_second", serde_json::json!(5.0)),
    ];

    let list = success_data(&isolated.output(&["config", "list"]), "config_list");
    let list_config = &list["config"];

    for (key, expected) in cases {
        let get = success_data(&isolated.output(&["config", "get", key]), "config_get");
        assert_eq!(get["key"], key);
        assert_eq!(
            get["value"], expected,
            "{key} resolves to its documented default, not a sentinel"
        );
        assert_ne!(
            get["value"], "(unset)",
            "{key} must never surface the undocumented sentinel"
        );

        let (section, leaf) = key.split_once('.').unwrap();
        assert_eq!(
            get["value"], list_config[section][leaf],
            "{key} agrees with the config list projection"
        );
    }

    // The number values stay typed numbers through the JSON envelope.
    let rate = success_data(
        &isolated.output(&["config", "get", "network.max_requests_per_second"]),
        "config_get",
    );
    assert!(
        rate["value"].is_f64() || rate["value"].is_i64() || rate["value"].is_u64(),
        "the rate ceiling stays a typed number, not a string: {rate}"
    );
    let enabled = success_data(
        &isolated.output(&["config", "get", "history.enabled"]),
        "config_get",
    );
    assert!(
        enabled["value"].is_boolean(),
        "history.enabled stays a typed bool, not a string: {enabled}"
    );
}

#[test]
fn config_get_updates_preference_before_onboarding_reports_no_configured_value() {
    let isolated = Isolated::new();
    // `updates.check_on_tui_start` has no built-in default. Before onboarding
    // the list projection omits the whole updates section, and `get` reports
    // "not configured" without asserting a meaning the model does not have.
    let list = success_data(&isolated.output(&["config", "list"]), "config_list");
    assert!(
        list["config"].get("updates").is_none(),
        "pre-onboarding updates section is omitted from the list projection: {list}"
    );

    let get = success_data(
        &isolated.output(&["config", "get", "updates.check_on_tui_start"]),
        "config_get",
    );
    assert_eq!(get["key"], "updates.check_on_tui_start");
    // `null` honestly encodes "no value configured yet"; the key never
    // asserts a default, and no sentinel wording reaches any projection.
    assert!(
        get["value"].is_null(),
        "the no-default key reports null, not a sentinel or invented default: {get}"
    );
    assert_ne!(get["value"], "(unset)");
}

#[test]
fn auth_set_api_key_stdin_persists_with_unix_0600() {
    let isolated = Isolated::new();
    let key_line = format!("{SYNTHETIC_API_KEY}\n");
    let output = isolated.run(&["auth", "set", "--api-key-stdin"], &[], Some(&key_line));

    assert_eq!(success_data(&output, "auth_set")["ok"], true);
    assert_key_never_leaks(&output, "auth set --api-key-stdin");

    let credentials = isolated.credentials_file();
    let contents = fs::read_to_string(&credentials)
        .expect("stored credentials must live in the platform config directory");
    assert!(contents.contains("credentials_version = 1"), "{contents}");
    assert!(
        contents.contains(SYNTHETIC_API_KEY),
        "the key is persisted: {contents}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&credentials).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "credentials.toml must be owner-only");
    }

    // The credential store must not be relocated by PANGRAM_CONFIG: it lives
    // under the platform config directory, never beside the explicit file.
    assert_ne!(credentials, isolated.explicit_config);
}

#[test]
fn auth_set_api_key_stdin_accepts_exactly_one_line() {
    let isolated = Isolated::new();
    let two_lines = format!("{SYNTHETIC_API_KEY}\nsecond-line-rejected\n");
    let output = isolated.run(&["auth", "set", "--api-key-stdin"], &[], Some(&two_lines));

    failure_error(&output, "auth_set", 2, "unsupported_input", "usage");
    assert_key_never_leaks(&output, "auth set --api-key-stdin extra line");
    assert!(
        !isolated.credentials_file().exists(),
        "a rejected multi-line credential must not be persisted"
    );
}

#[test]
fn auth_status_masks_a_stored_key() {
    let isolated = Isolated::new();
    let key_line = format!("{SYNTHETIC_API_KEY}\n");
    isolated.run(&["auth", "set", "--api-key-stdin"], &[], Some(&key_line));

    let output = isolated.output(&["auth", "status"]);
    let data = success_data(&output, "auth_status");
    assert_eq!(data["configured"], true);
    assert_eq!(data["source"], "stored");

    let suffix = data["masked_suffix"]
        .as_str()
        .expect("masked_suffix present");
    assert!(suffix.len() <= 8, "suffix is contract-bounded: {suffix:?}");
    assert!(
        SYNTHETIC_API_KEY.ends_with(suffix),
        "masked suffix {suffix:?} is the key's trailing {n} characters",
        n = suffix.len()
    );
    assert_key_never_leaks(&output, "auth status");
}

#[test]
fn auth_status_reports_none_without_any_credential() {
    let isolated = Isolated::new();
    let output = isolated.output(&["auth", "status"]);

    let data = success_data(&output, "auth_status");
    assert_eq!(data["configured"], false);
    assert_eq!(data["source"], "none");
    assert!(
        data.get("masked_suffix").is_none(),
        "no suffix without a key"
    );
}

#[test]
fn environment_api_key_overrides_stored_key_in_auth_status() {
    let isolated = Isolated::new();
    let key_line = format!("{SYNTHETIC_API_KEY}\n");
    isolated.run(&["auth", "set", "--api-key-stdin"], &[], Some(&key_line));

    let env_key = "pangram_synthetic_env_override_key_1111111111111111_NOT_REAL";
    let env = [("PANGRAM_API_KEY", env_key)];
    let output = isolated.run(&["auth", "status"], &env, None);

    let data = success_data(&output, "auth_status");
    assert_eq!(data["configured"], true);
    assert_eq!(data["source"], "environment");
    let suffix = data["masked_suffix"].as_str().unwrap();
    assert!(
        env_key.ends_with(suffix),
        "environment suffix wins: {suffix:?}"
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(env_key),
        "environment key leaked"
    );
}

#[test]
fn auth_logout_removes_stored_credentials_and_reports_environment_override() {
    let isolated = Isolated::new();
    let key_line = format!("{SYNTHETIC_API_KEY}\n");
    isolated.run(&["auth", "set", "--api-key-stdin"], &[], Some(&key_line));
    assert!(isolated.credentials_file().exists());

    let output = isolated.output(&["auth", "logout", "--yes"]);
    assert_eq!(success_data(&output, "auth_logout")["ok"], true);
    assert!(
        !isolated.credentials_file().exists(),
        "logout removes the stored credential file"
    );

    let status = isolated.output(&["auth", "status"]);
    let data = success_data(&status, "auth_status");
    assert_eq!(data["configured"], false);
    assert_eq!(data["source"], "none");
}

#[test]
fn bare_auth_does_not_prompt_noninteractively() {
    let isolated = Isolated::new();
    let output = isolated.output(&["auth"]);

    // Without a controlling terminal, bare `auth` must terminate instead of
    // prompting: the contract offers no noninteractive prompt.
    assert!(
        matches!(output.status.code(), Some(0) | Some(4)),
        "bare auth hung or exited unexpectedly: {:?}",
        output.status
    );
    assert_eq!(envelope(&output, "bare auth")["command"], "auth_status");
}

#[test]
fn doctor_returns_typed_local_results_without_a_key() {
    let isolated = Isolated::new();
    let output = isolated.output(&["doctor"]);

    let data = success_data(&output, "doctor");
    let checks = data["checks"].as_array().expect("doctor checks array");
    assert!(
        !checks.is_empty(),
        "doctor reports at least one local check"
    );
    for check in checks {
        assert!(check["name"].is_string(), "check name: {check}");
        assert!(
            matches!(check["status"].as_str().unwrap(), "pass" | "warn" | "fail"),
            "check status is closed: {check}"
        );
    }
}

#[test]
fn doctor_never_requires_an_api_key_or_network() {
    let isolated = Isolated::new();
    let proxy = [
        ("HTTPS_PROXY", "http://127.0.0.1:1"),
        ("HTTP_PROXY", "http://127.0.0.1:1"),
    ];
    let data = success_data(
        &isolated.run(&["doctor", "--format", "json"], &proxy, None),
        "doctor",
    );
    // A dead proxy and missing key may warn, but no local check may hard-fail,
    // because doctor never validates credentials against Pangram.
    for check in data["checks"].as_array().unwrap() {
        assert_ne!(
            check["status"], "fail",
            "a local check failed without cause: {check}"
        );
    }
}

#[test]
fn auth_set_api_key_help_warns_about_argv_exposure() {
    let output = pangram().args(["auth", "set", "--help"]).output().unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(
        help.contains("argv may be visible in process listings and shell history"),
        "help must warn about argv exposure:\n{help}"
    );
    assert!(
        help.contains("--api-key-stdin"),
        "help presents the stdin alternative:\n{help}"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn auth_set_stores_an_api_key_passed_as_a_value() {
    let isolated = Isolated::new();
    let output = isolated.output(&["auth", "set", "--api-key", SYNTHETIC_API_KEY]);

    assert_eq!(success_data(&output, "auth_set")["ok"], true);
    assert_key_never_leaks(&output, "auth set --api-key");

    let status = success_data(&isolated.output(&["auth", "status"]), "auth_status");
    assert_eq!(status["configured"], true);
    assert_eq!(status["source"], "stored");
}

#[test]
fn auth_set_requires_exactly_one_key_source() {
    let isolated = Isolated::new();
    for args in [
        &["auth", "set"][..],
        &["auth", "set", "--api-key", "x", "--api-key-stdin"][..],
    ] {
        let output = isolated.output(args);
        // Argument-shape failures are Clap-owned usage errors on stderr.
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
    }
    assert!(
        !isolated.credentials_file().exists(),
        "rejected invocations store nothing"
    );
}

#[test]
fn auth_logout_without_yes_is_a_usage_error_noninteractively() {
    let isolated = Isolated::new();
    let key_line = format!("{SYNTHETIC_API_KEY}\n");
    isolated.run(&["auth", "set", "--api-key-stdin"], &[], Some(&key_line));

    let output = isolated.output(&["auth", "logout"]);
    let error = failure_error(
        &output,
        "auth_logout",
        2,
        "unsupported_combination",
        "usage",
    );

    // The envelope recovers the caller with the explicit noninteractive flag.
    assert_eq!(
        error["recovery"]["command"], "pangram auth logout --yes",
        "recovery pins the noninteractive escape: {error}"
    );
    assert!(
        isolated.credentials_file().exists(),
        "a refused logout removes nothing"
    );
}

#[test]
fn config_global_flag_overrides_the_pangram_config_environment_variable() {
    let isolated = Isolated::new();
    let flag_path = isolated.explicit_config.with_file_name("flag-wins.toml");
    let flag = flag_path.to_str().unwrap().to_owned();
    let args = ["--config", flag.as_str(), "config", "path"];

    let data = success_data(&isolated.output(&args), "config_path");
    assert_eq!(
        Path::new(data["path"].as_str().unwrap()),
        flag_path.as_path(),
        "the flag wins over PANGRAM_CONFIG"
    );

    // The combined precedence chain still resolves: flag over environment.
    let env_only = success_data(&isolated.output(&["config", "path"]), "config_path");
    assert_eq!(
        Path::new(env_only["path"].as_str().unwrap()),
        isolated.explicit_config.as_path()
    );
}

#[test]
fn auth_logout_keeps_the_environment_credential_active() {
    let isolated = Isolated::new();
    let key_line = format!("{SYNTHETIC_API_KEY}\n");
    isolated.run(&["auth", "set", "--api-key-stdin"], &[], Some(&key_line));

    let env_key = "pangram_synthetic_env_keeps_active_2222222222222222_NOT_REAL";
    let env = [("PANGRAM_API_KEY", env_key)];

    let logout = isolated.run(&["auth", "logout", "--yes"], &env, None);
    assert_eq!(success_data(&logout, "auth_logout")["ok"], true);
    assert!(
        !isolated.credentials_file().exists(),
        "logout removes the stored credential file"
    );

    // The environment override is untouched by a stored logout.
    let status = isolated.run(&["auth", "status"], &env, None);
    let data = success_data(&status, "auth_status");
    assert_eq!(data["configured"], true);
    assert_eq!(data["source"], "environment");
    assert!(
        env_key.ends_with(data["masked_suffix"].as_str().unwrap()),
        "the environment key still masks in auth status"
    );

    let later_data = success_data(&isolated.output(&["auth", "status"]), "auth_status");
    assert_eq!(later_data["configured"], false);
    assert_eq!(later_data["source"], "none");
}

#[test]
fn doctor_exits_7_when_any_check_fails_but_still_emits_the_complete_json_report() {
    let isolated = Isolated::new();
    // An invalid config forces the `configuration` check to `fail` entirely
    // locally: no network, no credential validation, no filesystem mutation.
    fs::write(
        &isolated.explicit_config,
        "config_version = 1\nunknown_key = true\n",
    )
    .unwrap();

    let output = isolated.output(&["doctor", "--format", "json"]);

    assert_eq!(
        output.status.code(),
        Some(7),
        "a failing check maps to the local-state exit code 7, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The envelope remains a canonical success envelope (data, never error)
    // even though the process exits non-zero, and it carries every check.
    let context = "doctor failing-check JSON";
    let envelope = envelope(&output, context);
    assert_eq!(envelope["command"], "doctor", "{context}");
    assert!(
        envelope.get("data").is_some() && envelope.get("error").is_none(),
        "{context}: failing checks stay a typed data payload, not an error envelope: {envelope}"
    );

    let checks = envelope["data"]["checks"]
        .as_array()
        .expect("doctor checks array");
    let names: Vec<&str> = checks.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["configuration", "credentials", "data_directory", "runtime"],
        "{context}: the complete ordered checks are retained even at exit 7"
    );
    assert!(
        checks.iter().any(|c| c["status"] == "fail"),
        "{context}: at least one check failed to justify exit 7"
    );
}

#[test]
fn doctor_exits_0_when_no_check_fails_in_json() {
    let isolated = Isolated::new();
    let output = isolated.output(&["doctor", "--format", "json"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "pass/warn-only doctor exits 0"
    );
    let data = success_data(&output, "doctor");
    for check in data["checks"].as_array().unwrap() {
        assert_ne!(check["status"], "fail", "precheck: no check fails: {check}");
    }
}

#[test]
fn doctor_pretty_exits_7_when_any_check_fails_and_still_renders_every_line() {
    let isolated = Isolated::new();
    fs::write(
        &isolated.explicit_config,
        "config_version = 1\nunknown_key = true\n",
    )
    .unwrap();

    let output = isolated.output(&["doctor", "--format", "pretty"]);

    assert_eq!(
        output.status.code(),
        Some(7),
        "pretty doctor maps a failing check to exit 7, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The pretty projection still prints the complete ordered checks.
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut cursor = 0;
    for name in ["configuration", "credentials", "data_directory", "runtime"] {
        let position = stdout[cursor..]
            .find(name)
            .unwrap_or_else(|| panic!("check {name} present in pretty output:\n{stdout}"));
        cursor += position + name.len();
    }
    assert!(
        stdout.lines().any(|line| line.starts_with("fail")),
        "a failing line is rendered:\n{stdout}"
    );
}

#[test]
fn doctor_pretty_exits_0_when_no_check_fails() {
    let isolated = Isolated::new();
    let output = isolated.output(&["doctor", "--format", "pretty"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "pretty doctor exits 0 when no check fails"
    );
}

#[test]
fn doctor_format_pretty_renders_the_ordered_checks_to_stdout() {
    let isolated = Isolated::new();
    let output = isolated.output(&["doctor", "--format", "pretty"]);

    assert!(
        output.status.success(),
        "pretty doctor exit: {:?}",
        output.status
    );
    let stdout = String::from_utf8(output.stdout).unwrap();

    // The pretty projection is not a JSON envelope.
    assert!(
        serde_json::from_str::<Value>(&stdout).is_err(),
        "pretty output must not parse as the JSON envelope:\n{stdout}"
    );

    // The closed Phase 1 check names appear in their contract order.
    let names = ["configuration", "credentials", "data_directory", "runtime"];
    let mut cursor = 0;
    for name in names {
        let position = stdout[cursor..]
            .find(name)
            .unwrap_or_else(|| panic!("check {name} present in:\n{stdout}"));
        cursor += position + name.len();
    }

    // Every line carries one of the closed status markers.
    for line in stdout.lines() {
        let marker = line.split_whitespace().next().unwrap_or("");
        assert!(
            matches!(marker, "pass" | "warn" | "fail"),
            "unexpected status marker in line: {line:?}"
        );
    }

    assert!(
        output.stderr.is_empty(),
        "pretty doctor keeps stderr empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
