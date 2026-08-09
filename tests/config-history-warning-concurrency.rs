//! Compiled-process contract for the durable-plaintext enable warning.
//!
//! The storage unit tests prove lock acquisition and transition ordering. This
//! target proves the thin CLI adapter emits that storage decision exactly once
//! across real processes while preserving the canonical stdout envelope.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Barrier;

use serde_json::Value;
use tempfile::TempDir;

const HISTORY_PLAINTEXT_WARNING: &str = "warning: history stores submitted content and results \
unencrypted as plaintext in the local data directory\n";

fn pangram() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pangram"))
}

struct Isolated {
    _root: TempDir,
    env: Vec<(String, String)>,
    explicit_config: PathBuf,
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
        }
    }

    fn output(&self, args: &[&str]) -> std::process::Output {
        let mut command = pangram();
        command
            .env_remove("PANGRAM_API_KEY")
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .args(args)
            .stdin(Stdio::null());
        command.output().expect("failed to run pangram")
    }
}

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

fn config_set_success_stderr(output: &std::process::Output) -> String {
    let envelope = envelope(output, "config_set success");
    assert_eq!(envelope["command"], "config_set");
    assert_eq!(envelope["data"]["ok"], true);
    assert!(output.status.success(), "config_set exit: {output:?}");
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn concurrent_history_enable_processes_warn_exactly_once() {
    let isolated = Isolated::new();
    let started = Barrier::new(3);

    let outputs = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let isolated = &isolated;
                scope.spawn(|| {
                    started.wait();
                    isolated.output(&["config", "set", "history.enabled", "true"])
                })
            })
            .collect();
        started.wait();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });

    let warnings: Vec<String> = outputs.iter().map(config_set_success_stderr).collect();
    assert_eq!(
        warnings
            .iter()
            .filter(|stderr| stderr.as_str() == HISTORY_PLAINTEXT_WARNING)
            .count(),
        1,
        "exactly one process owns the persisted false-to-true transition: {warnings:?}"
    );
    assert_eq!(
        warnings.iter().filter(|stderr| stderr.is_empty()).count(),
        1,
        "the idempotent process stays silent: {warnings:?}"
    );

    let get = isolated.output(&["config", "get", "history.enabled"]);
    let envelope = envelope(&get, "config_get success");
    assert!(get.status.success());
    assert!(get.stderr.is_empty());
    assert_eq!(envelope["command"], "config_get");
    assert_eq!(envelope["data"]["value"], true);
    assert!(isolated.explicit_config.exists());

    let disable = isolated.output(&["config", "set", "history.enabled", "false"]);
    assert!(config_set_success_stderr(&disable).is_empty());

    let reenable = isolated.output(&["config", "set", "history.enabled", "true"]);
    assert_eq!(
        config_set_success_stderr(&reenable),
        HISTORY_PLAINTEXT_WARNING,
        "a later persisted false-to-true transition owns a new warning"
    );
}
