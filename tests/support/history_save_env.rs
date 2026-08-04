//! Shared harness for the compiled-binary history-save loopback suites
//! (dev-tools only): the isolated credential/config/data invocation
//! environment, the real-SQLite saved-database handle, and the envelope /
//! no-leak / row assertions. Split out so the persistence and the
//! failure/reconciliation semantics suites each stay under the source-size
//! hygiene threshold while both exercise the exact same store.
//!
//! Test-support code consumed through `#[path]` by more than one test crate;
//! a few helpers are unused in each, so the whole module allows that rather
//! than peppering `cfg` gates.

#![allow(dead_code)]

#[path = "protocol_loopback/mod.rs"]
pub mod fixture;

use std::process::Command;

use rusqlite::Connection;
use serde_json::Value;
use tempfile::TempDir;

use fixture::{ProtocolFixture, SYNTHETIC_KEY, Step, TASK_ID, pangram4_success};

pub const KEY_FRAGMENT: &str = "synthetic_key_0000";

pub fn pangram() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pangram"))
}

/// An isolated invocation: credential, config, and data state rooted in one
/// temporary directory, with `CI` set (never interactive) and a synthetic
/// key. Unlike the pure-protocol suites this one keeps a handle on the data
/// directory so the real SQLite store can be asserted after the run.
pub struct Isolated {
    _root: TempDir,
    env: Vec<(String, String)>,
    pub data: std::path::PathBuf,
}

impl Isolated {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let config = root.path().join("config.toml");
        let data = root.path().join("data");
        for directory in [&home, &data] {
            std::fs::create_dir_all(directory).unwrap();
        }
        let env = [
            ("HOME", home.to_str().unwrap()),
            ("XDG_CONFIG_HOME", home.to_str().unwrap()),
            ("XDG_DATA_HOME", home.to_str().unwrap()),
            ("PANGRAM_CONFIG", config.to_str().unwrap()),
            ("PANGRAM_DATA_DIR", data.to_str().unwrap()),
            ("CI", "true"),
            ("TERM", "dumb"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
        Self {
            _root: root,
            env,
            data,
        }
    }

    pub fn command(&self, endpoint: &str) -> Command {
        let mut command = pangram();
        command
            .env("PANGRAM_API_KEY", SYNTHETIC_KEY)
            .env("PANGRAM_DETECT_ENDPOINT", endpoint);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }

    /// Writes `config set history.enabled true` through the compiled binary
    /// so the persisted shape is exactly what the config surface produces.
    /// The ADR 0004 first-enable warning may appear on stderr; that is the
    /// intended transition acknowledgment, not a failure.
    pub fn enable_history(&self) {
        let output = self
            .command("http://127.0.0.1:1")
            .args(["config", "set", "history.enabled", "true"])
            .output()
            .expect("run config set");
        assert_eq!(output.status.code(), Some(0), "enable history");
    }

    /// Runs a `config set` invocation directly so a test can assert its
    /// exact stdout/stderr/exit (used by the ADR 0004 warning transitions).
    pub fn config_set(&self, key: &str, value: &str) -> std::process::Output {
        self.command("http://127.0.0.1:1")
            .args(["config", "set", key, value])
            .output()
            .expect("run config set")
    }

    /// The history directory Packet C would create lazily on first open.
    pub fn history_directory(&self) -> std::path::PathBuf {
        self.data.join("history")
    }

    pub fn database_path(&self) -> std::path::PathBuf {
        self.history_directory().join("pangram-history.db")
    }

    /// Opens the real saved database with the plain rusqlite test handle.
    /// The store has already closed it, so WAL+SHM are checkpointed away.
    pub fn open_database(&self) -> Connection {
        Connection::open(self.database_path()).expect("open saved history database")
    }
}

pub fn stdout_envelope(output: &std::process::Output) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    serde_json::from_str(stdout.trim_end())
        .unwrap_or_else(|error| panic!("stdout is one JSON envelope: {error}\nstdout: {stdout}"))
}

pub fn assert_no_leak(output: &std::process::Output) {
    for surface in [
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ] {
        assert!(
            !surface.contains(SYNTHETIC_KEY) && !surface.contains(KEY_FRAGMENT),
            "the credential or its header must never appear in any output: {surface}"
        );
        assert!(
            !surface.to_ascii_lowercase().contains("x-api-key"),
            "auth header names stay out of output"
        );
    }
}

pub fn stderr_text(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// Ownership mode of a path's parent directory, for the owner-only proof.
#[cfg(unix)]
pub fn parent_dir_mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path.parent().expect("a parent exists"))
        .expect("parent directory exists")
        .permissions()
        .mode()
        & 0o777
}

/// One analyzer row from the saved database: (id, status,
/// submission_outcome, save_state, input_type, input_json,
/// result_json, error_json, completed_at).
#[allow(clippy::type_complexity)]
pub fn analyses_rows(
    connection: &Connection,
) -> Vec<(
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
)> {
    let mut statement = connection
        .prepare(
            "SELECT id, status, submission_outcome, save_state, input_type, input_json,
                    result_json, error_json, completed_at
             FROM analyses ORDER BY created_at, id",
        )
        .expect("prepare analyses query");
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })
        .expect("query analyses")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect analyses")
}

/// One upstream_tasks row: (analysis_id, check_kind, upstream_task_id, last_stage).
pub fn task_rows(connection: &Connection) -> Vec<(String, String, String, Option<String>)> {
    let mut statement = connection
        .prepare("SELECT analysis_id, check_kind, upstream_task_id, last_stage FROM upstream_tasks")
        .expect("prepare tasks query");
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("query tasks")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect tasks")
}

pub fn search_payload(connection: &Connection) -> Vec<(String, Option<String>, Option<String>)> {
    let mut statement = connection
        .prepare("SELECT analysis_id, input_text, headline FROM analysis_search")
        .expect("prepare search query");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query search")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect search")
}

/// One `bulk_collections` row: (id, upstream_bulk_id, status, counters).
#[allow(clippy::type_complexity)]
pub fn bulk_collection_rows(
    connection: &Connection,
) -> Vec<(String, Option<String>, String, (i64, i64, i64, i64))> {
    let mut statement = connection
        .prepare(
            "SELECT id, upstream_bulk_id, status, total_items, accepted, succeeded, failed
             FROM bulk_collections ORDER BY created_at, id",
        )
        .expect("prepare bulk query");
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                (row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?),
            ))
        })
        .expect("query bulk")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect bulk")
}

/// The Unix file mode of the database, for the owner-only proof.
#[cfg(unix)]
pub fn database_mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("database exists")
        .permissions()
        .mode()
        & 0o777
}

/// A completed literal detection dedicated to save-flow assertions.
pub async fn completed_fixture(text: &'static str) -> ProtocolFixture {
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    fixture
}

/// A policed data directory whose owner-only protection cannot be verified:
/// an existing `history` path that is a regular file. The store fails closed
/// before ever opening SQLite, so no database appears alongside it.
#[cfg(unix)]
pub fn poison_data_dir(isolated: &Isolated) {
    std::fs::write(isolated.history_directory(), b"not a directory").unwrap();
}
