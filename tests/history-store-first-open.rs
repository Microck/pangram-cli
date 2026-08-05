//! Real SQLite first-open serialization tests for the history contract.
//!
//! These tests start from an absent database path and exercise both threads
//! and independent operating-system processes. No mock or pre-initialized
//! database participates in either race.

#![forbid(unsafe_code)]

use std::path::Path;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

use microck_pangram_cli::history::{HistoryErrorCode, HistoryStore};

const THREADS: usize = 16;
const PROCESSES: usize = 8;

fn create_protected_empty_database(root: &Path) -> std::path::PathBuf {
    let history = root.join("history");
    std::fs::create_dir(&history).expect("create history directory");
    let database = history.join("pangram-history.db");
    std::fs::File::create(&database).expect("create empty database file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&history, std::fs::Permissions::from_mode(0o700))
            .expect("protect history directory");
        std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600))
            .expect("protect empty database");
    }
    database
}

fn assert_exact_initialized_store(root: &Path) {
    let store = HistoryStore::open(root).expect("reopen exact initialized store");
    assert_eq!(store.user_version().expect("read user_version"), 1);
    let tables = store
        .with_connection(|connection| {
            connection
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type = 'table' \
                     AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )
                .expect("prepare catalog query")
                .query_map([], |row| row.get::<_, String>(0))
                .expect("query catalog")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect catalog")
        })
        .expect("read exact catalog");
    assert_eq!(
        tables,
        [
            "analyses",
            "analysis_checks",
            "analysis_search",
            "analysis_search_config",
            "analysis_search_content",
            "analysis_search_data",
            "analysis_search_docsize",
            "analysis_search_idx",
            "bulk_collections",
            "upstream_tasks",
        ]
    );
    drop(store);

    let history = root.join("history");
    let database = history.join("pangram-history.db");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            std::fs::metadata(&history)
                .expect("history directory metadata")
                .permissions()
                .mode()
                & 0o7777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&database)
                .expect("database metadata")
                .permissions()
                .mode()
                & 0o7777,
            0o600
        );
    }
    for suffix in ["-journal", "-wal", "-shm"] {
        assert!(
            !database
                .with_file_name(format!("pangram-history.db{suffix}"))
                .exists(),
            "completed first-open race left a partial `{suffix}` artifact"
        );
    }
}

#[test]
fn concurrent_threads_initialize_one_exact_schema_from_an_absent_path() {
    let root = tempfile::tempdir().expect("temporary data directory");
    let barrier = Arc::new(Barrier::new(THREADS));
    let mut handles = Vec::with_capacity(THREADS);
    for _ in 0..THREADS {
        let barrier = Arc::clone(&barrier);
        let root = root.path().to_path_buf();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let store = HistoryStore::open(&root).expect("racing thread opens store");
            assert_eq!(store.user_version().expect("read version"), 1);
        }));
    }
    for handle in handles {
        handle.join().expect("racing thread completes");
    }
    assert_exact_initialized_store(root.path());
}

#[cfg(unix)]
#[test]
fn concurrent_first_open_never_exposes_an_insecure_history_directory() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().expect("temporary data directory");
    let history = root.path().join("history");
    let barrier = Arc::new(Barrier::new(THREADS + 1));
    let complete = Arc::new(AtomicBool::new(false));
    let insecure = Arc::new(AtomicBool::new(false));
    let observer = {
        let barrier = Arc::clone(&barrier);
        let complete = Arc::clone(&complete);
        let insecure = Arc::clone(&insecure);
        let history = history.clone();
        std::thread::spawn(move || {
            barrier.wait();
            while !complete.load(Ordering::Acquire) {
                match std::fs::symlink_metadata(&history) {
                    Ok(metadata) => {
                        if metadata.file_type().is_symlink()
                            || !metadata.is_dir()
                            || metadata.permissions().mode() & 0o7777 != 0o700
                        {
                            insecure.store(true, Ordering::Release);
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("observe history directory: {error}"),
                }
                std::thread::yield_now();
            }
        })
    };

    let mut handles = Vec::with_capacity(THREADS);
    for _ in 0..THREADS {
        let barrier = Arc::clone(&barrier);
        let root = root.path().to_path_buf();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            HistoryStore::open(&root).expect("simultaneous opener succeeds")
        }));
    }
    for handle in handles {
        drop(handle.join().expect("simultaneous opener completes"));
    }
    complete.store(true, Ordering::Release);
    observer.join().expect("permission observer completes");

    assert!(
        !insecure.load(Ordering::Acquire),
        "the history directory was visible before owner-only mode was established"
    );
    assert_exact_initialized_store(root.path());
}

#[cfg(unix)]
#[test]
fn a_preexisting_history_symlink_is_rejected_without_touching_its_target() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let root = tempfile::tempdir().expect("temporary data directory");
    let target = tempfile::tempdir().expect("symlink target");
    std::fs::set_permissions(target.path(), std::fs::Permissions::from_mode(0o700))
        .expect("protect target");
    symlink(target.path(), root.path().join("history")).expect("create hostile history symlink");

    let error = HistoryStore::open(root.path()).expect_err("history symlink must fail closed");
    assert_eq!(error.code(), HistoryErrorCode::InsecureHistoryPermissions);
    assert!(
        !target.path().join("pangram-history.db").exists(),
        "the rejected symlink target must remain untouched"
    );
}

/// SQLite URI parsing must never reinterpret the already-protected history
/// path. Run the actual open in a child process so the process-local current
/// directory can safely make a relative data path begin with `file:`.
#[cfg(unix)]
#[test]
fn uri_like_data_path_opens_only_the_literal_protected_database() {
    use std::os::unix::fs::PermissionsExt;

    let workspace = tempfile::tempdir().expect("temporary URI-path workspace");
    let redirected = workspace.path().join("redirected-target.db");
    std::fs::write(&redirected, b"redirect sentinel").expect("seed redirected target");
    let literal_data = workspace
        .path()
        .join("file:redirect?mode=memory#literal-fragment");
    std::fs::create_dir(&literal_data).expect("create literal URI-like data directory");
    std::fs::set_permissions(&literal_data, std::fs::Permissions::from_mode(0o700))
        .expect("protect literal URI-like data directory");

    let binary = std::env::current_exe().expect("current test binary");
    let output = std::process::Command::new(binary)
        .args(["uri_like_path_process_entry", "--exact", "--ignored"])
        .current_dir(workspace.path())
        .env(
            "PANGRAM_URI_LIKE_DATA",
            "file:redirect?mode=memory#literal-fragment",
        )
        .output()
        .expect("spawn literal URI-path opener");
    assert!(
        output.status.success(),
        "URI-path child failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let literal_history = literal_data.join("history");
    assert!(
        literal_history.join("pangram-history.db").is_file(),
        "the exact literal protected database is created"
    );
    for sidecar in ["pangram-history.db-wal", "pangram-history.db-shm"] {
        assert!(
            !literal_history.join(sidecar).exists(),
            "no literal {sidecar} survives the closed store"
        );
    }
    assert_eq!(
        std::fs::read(&redirected).expect("redirected target remains"),
        b"redirect sentinel",
        "URI-looking path bytes never redirect SQLite"
    );
    assert!(
        !workspace.path().join("redirect").exists(),
        "no URI-derived redirect path appears"
    );
}

#[cfg(unix)]
#[test]
#[ignore = "child entry point spawned by the URI-path parent"]
fn uri_like_path_process_entry() {
    let data = std::env::var_os("PANGRAM_URI_LIKE_DATA").expect("URI-like data path environment");
    let store = HistoryStore::open(Path::new(&data)).expect("open literal URI-like history path");
    assert_eq!(store.user_version().expect("read version"), 1);
}

#[test]
fn concurrent_processes_initialize_one_exact_schema_from_an_absent_path() {
    let root = tempfile::tempdir().expect("temporary data directory");
    let binary = std::env::current_exe().expect("current test binary");
    let mut children = Vec::with_capacity(PROCESSES);
    for process in 0..PROCESSES {
        children.push(
            std::process::Command::new(&binary)
                .args(["first_open_process_entry", "--exact", "--ignored"])
                .env("PANGRAM_FIRST_OPEN_ROOT", root.path())
                .env(
                    "PANGRAM_FIRST_OPEN_MARKER",
                    root.path().join(format!("process-{process}.marker")),
                )
                .spawn()
                .expect("spawn first-open contender"),
        );
    }
    for (process, child) in children.into_iter().enumerate() {
        let output = child.wait_with_output().expect("wait for contender");
        assert!(
            output.status.success(),
            "first-open process {process} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join(format!("process-{process}.marker")))
                .expect("read success marker"),
            "ok"
        );
    }
    assert_exact_initialized_store(root.path());
}

#[test]
#[ignore = "child entry point spawned by the process-race parent"]
fn first_open_process_entry() {
    let root = std::env::var_os("PANGRAM_FIRST_OPEN_ROOT").expect("root environment");
    let marker = std::env::var_os("PANGRAM_FIRST_OPEN_MARKER").expect("marker environment");
    let store = HistoryStore::open(Path::new(&root)).expect("process opens store");
    assert_eq!(store.user_version().expect("read version"), 1);
    std::fs::write(marker, b"ok").expect("write success marker");
}

#[test]
fn a_protected_empty_file_left_before_schema_creation_is_initialized() {
    let root = tempfile::tempdir().expect("temporary data directory");
    create_protected_empty_database(root.path());

    assert_exact_initialized_store(root.path());
}

#[test]
fn an_opener_waits_for_an_interrupted_initializer_then_builds_exact_v1() {
    let root = tempfile::tempdir().expect("temporary data directory");
    let database = create_protected_empty_database(root.path());
    let holder = rusqlite::Connection::open(&database).expect("open lock holder");
    holder
        .execute_batch("BEGIN IMMEDIATE; CREATE TABLE partial_schema (value TEXT);")
        .expect("hold an uncommitted partial schema");

    let opener_root = root.path().to_path_buf();
    let opener = std::thread::spawn(move || {
        HistoryStore::open(&opener_root).expect("waiting opener recovers after rollback")
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    holder
        .execute_batch("ROLLBACK")
        .expect("interrupt initializer by rolling back");
    drop(holder);
    drop(opener.join().expect("waiting opener completes"));

    assert_exact_initialized_store(root.path());
}

#[test]
fn user_version_zero_with_a_schema_object_is_rejected_without_mutation() {
    let root = tempfile::tempdir().expect("temporary data directory");
    let history = root.path().join("history");
    std::fs::create_dir(&history).expect("create history directory");
    let database = history.join("pangram-history.db");
    {
        let connection = rusqlite::Connection::open(&database).expect("open fixture");
        connection
            .execute_batch("CREATE TABLE unexpected (value TEXT);")
            .expect("create incompatible object");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&history, std::fs::Permissions::from_mode(0o700))
            .expect("protect history directory");
        std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600))
            .expect("protect database");
    }
    let before = std::fs::read(&database).expect("read fixture bytes");

    let error = HistoryStore::open(root.path()).expect_err("incompatible zero version fails");
    assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
    assert_eq!(
        std::fs::read(&database).expect("reread fixture bytes"),
        before,
        "failed open must preserve incompatible database bytes"
    );
}
