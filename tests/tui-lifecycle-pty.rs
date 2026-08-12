//! Release-blocking terminal restoration contracts through the compiled binary
//! and a real native pseudo-terminal. These tests inspect the terminal bytes
//! emitted at the process boundary, not an in-process renderer or fake guard.

#![cfg(feature = "dev-tools")]

use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, ExitStatus, NativePtySystem, PtySize, PtySystem};

const START_TIMEOUT: Duration = Duration::from_secs(5);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
const ENABLE_BRACKETED_PASTE: &[u8] = b"\x1b[?2004h";
const DISABLE_BRACKETED_PASTE: &[u8] = b"\x1b[?2004l";
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";

struct PtyOutcome {
    status: ExitStatus,
    transcript: Vec<u8>,
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|candidate| candidate == needle)
}

fn receive_until(
    receiver: &Receiver<Vec<u8>>,
    transcript: &mut Vec<u8>,
    timeout: Duration,
    predicate: impl Fn(&[u8]) -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if predicate(transcript) {
            return true;
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        match receiver.recv_timeout(remaining) {
            Ok(chunk) => transcript.extend_from_slice(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                return predicate(transcript);
            }
        }
    }
}

fn isolated_command(root: &Path, injection: Option<&str>) -> CommandBuilder {
    let home = root.join("home");
    let config_home = root.join("config");
    let data_home = root.join("data");
    let cache_home = root.join("cache");
    let config = config_home.join("pangram.toml");
    for directory in [&home, &config_home, &data_home, &cache_home] {
        std::fs::create_dir_all(directory).unwrap();
    }

    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_pangram"));
    command.env_clear();
    for (key, value) in [
        ("HOME", home.as_os_str()),
        ("USERPROFILE", home.as_os_str()),
        ("XDG_CONFIG_HOME", config_home.as_os_str()),
        ("XDG_DATA_HOME", data_home.as_os_str()),
        ("XDG_CACHE_HOME", cache_home.as_os_str()),
        ("PANGRAM_CONFIG", config.as_os_str()),
        ("PANGRAM_DATA_DIR", data_home.as_os_str()),
        ("TERM", OsStr::new("xterm-256color")),
        ("CI", OsStr::new("true")),
        ("LANG", OsStr::new("C.UTF-8")),
        ("LC_ALL", OsStr::new("C.UTF-8")),
        ("NO_COLOR", OsStr::new("1")),
    ] {
        command.env(key, value);
    }
    if let Some(injection) = injection {
        command.env(injection, "1");
    }

    // CreateProcess needs these platform bootstrap values on some Windows
    // hosts. They contain no credentials or Pangram state.
    for key in ["SYSTEMROOT", "WINDIR", "COMSPEC"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

fn run_in_pty(
    injection: Option<&str>,
    after_entry: impl FnOnce(u32) -> Result<(), String>,
) -> PtyOutcome {
    let isolated = tempfile::tempdir().unwrap();
    let pair = NativePtySystem::default()
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open native pseudo-terminal");
    let mut child = pair
        .slave
        .spawn_command(isolated_command(isolated.path(), injection))
        .expect("spawn compiled pangram in the pseudo-terminal");
    drop(pair.slave);

    let process_id = child.process_id().expect("PTY child has a process ID");
    let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");
    let writer = pair.master.take_writer().expect("take PTY writer");
    let (output_tx, output_rx) = mpsc::channel();
    let reader_thread = std::thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if output_tx.send(buffer[..read].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut transcript = Vec::new();
    let entered = receive_until(&output_rx, &mut transcript, START_TIMEOUT, |bytes| {
        contains_bytes(bytes, ENTER_ALTERNATE_SCREEN)
    });
    let trigger_error = entered
        .then(|| after_entry(process_id))
        .and_then(Result::err);

    let mut killer = child.clone_killer();
    if !entered || trigger_error.is_some() {
        let _ = killer.kill();
    }
    let (status_tx, status_rx) = mpsc::channel();
    let wait_thread = std::thread::spawn(move || {
        let _ = status_tx.send(child.wait());
    });
    let status_result = status_rx.recv_timeout(EXIT_TIMEOUT);
    if status_result.is_err() {
        let _ = killer.kill();
    }
    wait_thread.join().expect("join child waiter");
    drop(writer);
    reader_thread.join().expect("join PTY reader");
    while let Ok(chunk) = output_rx.try_recv() {
        transcript.extend_from_slice(&chunk);
    }

    assert!(
        entered,
        "TUI did not enter the alternate screen before exiting:\n{}",
        String::from_utf8_lossy(&transcript)
    );
    if let Some(error) = trigger_error {
        panic!("failed to trigger the lifecycle exit: {error}");
    }
    let status = status_result
        .unwrap_or_else(|error| {
            panic!(
                "TUI did not exit within {EXIT_TIMEOUT:?}: {error}\n{}",
                String::from_utf8_lossy(&transcript)
            )
        })
        .expect("wait for TUI exit");
    PtyOutcome { status, transcript }
}

fn assert_terminal_restored(outcome: &PtyOutcome) {
    let enter = outcome
        .transcript
        .windows(ENTER_ALTERNATE_SCREEN.len())
        .position(|bytes| bytes == ENTER_ALTERNATE_SCREEN)
        .expect("the TUI enters the alternate screen");
    let cursor = outcome
        .transcript
        .windows(SHOW_CURSOR.len())
        .rposition(|bytes| bytes == SHOW_CURSOR)
        .expect("the TUI restores cursor visibility");
    let enable_paste = outcome
        .transcript
        .windows(ENABLE_BRACKETED_PASTE.len())
        .position(|bytes| bytes == ENABLE_BRACKETED_PASTE)
        .expect("the TUI enables bracketed paste");
    let disable_paste = outcome
        .transcript
        .windows(DISABLE_BRACKETED_PASTE.len())
        .position(|bytes| bytes == DISABLE_BRACKETED_PASTE)
        .expect("the TUI disables bracketed paste");
    let leave = outcome
        .transcript
        .windows(LEAVE_ALTERNATE_SCREEN.len())
        .position(|bytes| bytes == LEAVE_ALTERNATE_SCREEN)
        .expect("the TUI restores the primary screen");

    assert!(
        enter < enable_paste
            && enable_paste < disable_paste
            && disable_paste < cursor
            && cursor < leave,
        "terminal restoration must follow alternate-screen entry"
    );
}

#[cfg(unix)]
#[test]
fn sigterm_restores_the_terminal_and_returns_the_termination_exit_intent() {
    let outcome = run_in_pty(None, |process_id| {
        let status = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(process_id.to_string())
            .status()
            .map_err(|error| format!("native kill command failed: {error}"))?;
        if !status.success() {
            return Err(format!("native kill command returned {status}"));
        }
        Ok(())
    });

    assert_terminal_restored(&outcome);
    assert_eq!(
        outcome.status.exit_code(),
        1,
        "SIGTERM returns exit intent 1"
    );
    assert_eq!(
        outcome.status.signal(),
        None,
        "SIGTERM is handled instead of terminating the process directly"
    );
}

#[test]
fn terminal_owner_unwind_panic_restores_the_terminal_before_exit() {
    let outcome = run_in_pty(Some("PANGRAM_TUI_TEST_PANIC_AFTER_ENTER"), |_| Ok(()));

    assert_terminal_restored(&outcome);
    assert_eq!(
        outcome.status.exit_code(),
        101,
        "an owner-thread unwind retains Rust's panic exit intent"
    );
    assert_eq!(outcome.status.signal(), None, "the panic unwinds normally");
    assert!(
        contains_bytes(&outcome.transcript, b"panicked"),
        "the injected owner-thread panic reaches the process panic hook"
    );
}

#[test]
fn handled_tui_io_failure_restores_the_terminal_and_returns_general_failure() {
    let outcome = run_in_pty(Some("PANGRAM_TUI_TEST_IO_ERROR_AFTER_ENTER"), |_| Ok(()));

    assert_terminal_restored(&outcome);
    assert_eq!(
        outcome.status.exit_code(),
        1,
        "handled terminal I/O failure returns general failure"
    );
    assert_eq!(
        outcome.status.signal(),
        None,
        "handled terminal I/O failure returns through main"
    );
    assert!(
        contains_bytes(&outcome.transcript, b"pangram: terminal interface failed:"),
        "handled terminal I/O failure emits the sanitized TUI diagnostic"
    );
}
