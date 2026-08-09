//! Compiled TUI process contracts through a real native pseudo-terminal.
//! Each process receives an isolated environment and the same bytes a terminal
//! sends; no renderer seam or semantic test adapter can make these lifecycle
//! assertions pass while the visible terminal is broken.

#![cfg(feature = "dev-tools")]

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

const START_TIMEOUT: Duration = Duration::from_secs(5);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

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

fn screen_contents(transcript: &[u8]) -> String {
    let mut terminal = vt100::Parser::new(40, 120, 0);
    terminal.process(transcript);
    terminal.screen().contents()
}

fn isolated_command(root: &std::path::Path) -> CommandBuilder {
    let home = root.join("home");
    let data = root.join("data");
    let config = root.join("config.toml");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&data).unwrap();

    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_pangram"));
    command.env_clear();
    for (key, value) in [
        ("HOME", home.as_os_str()),
        ("USERPROFILE", home.as_os_str()),
        ("XDG_CONFIG_HOME", home.as_os_str()),
        ("XDG_DATA_HOME", home.as_os_str()),
        ("PANGRAM_CONFIG", config.as_os_str()),
        ("PANGRAM_DATA_DIR", data.as_os_str()),
        ("TERM", std::ffi::OsStr::new("xterm-256color")),
        ("CI", std::ffi::OsStr::new("true")),
        ("LANG", std::ffi::OsStr::new("C.UTF-8")),
        ("LC_ALL", std::ffi::OsStr::new("C.UTF-8")),
        ("NO_COLOR", std::ffi::OsStr::new("1")),
    ] {
        command.env(key, value);
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

#[test]
fn bare_all_tty_launches_analyze_and_ctrl_c_restores_the_terminal() {
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
        .spawn_command(isolated_command(isolated.path()))
        .expect("spawn compiled pangram in the pseudo-terminal");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");
    let mut writer = pair.master.take_writer().expect("take PTY writer");
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
    let launched = receive_until(&output_rx, &mut transcript, START_TIMEOUT, |bytes| {
        let visible = screen_contents(bytes);
        ["Analyze", "Active", "History", "Settings"]
            .iter()
            .all(|label| visible.contains(label))
    });
    if !launched {
        let _ = child.kill();
        let _ = child.wait();
        drop(writer);
        let _ = reader_thread.join();
        panic!(
            "bare all-TTY launch did not reach the four-route TUI:\n{}",
            String::from_utf8_lossy(&transcript)
        );
    }

    writer.write_all(&[0x03]).expect("send Ctrl+C");
    writer.flush().expect("flush Ctrl+C");
    let mut killer = child.clone_killer();
    let (status_tx, status_rx) = mpsc::channel();
    let wait_thread = std::thread::spawn(move || {
        let _ = status_tx.send(child.wait());
    });
    let status = match status_rx.recv_timeout(EXIT_TIMEOUT) {
        Ok(status) => status,
        Err(error) => {
            let _ = killer.kill();
            let _ = wait_thread.join();
            drop(writer);
            let _ = reader_thread.join();
            panic!("TUI did not exit after Ctrl+C: {error}");
        }
    };
    wait_thread.join().expect("join child waiter");
    drop(writer);
    reader_thread.join().expect("join PTY reader");
    while let Ok(chunk) = output_rx.try_recv() {
        transcript.extend_from_slice(&chunk);
    }

    let status = status.expect("wait for TUI exit");
    assert_eq!(status.exit_code(), 130, "Ctrl+C uses the interruption exit");
    assert!(
        transcript
            .windows(b"\x1b[?1049h".len())
            .any(|bytes| bytes == b"\x1b[?1049h"),
        "the TUI enters the alternate screen"
    );
    assert!(
        transcript
            .windows(b"\x1b[?1049l".len())
            .any(|bytes| bytes == b"\x1b[?1049l"),
        "the TUI restores the primary screen"
    );
    assert!(
        transcript
            .windows(b"\x1b[?25h".len())
            .any(|bytes| bytes == b"\x1b[?25h"),
        "the TUI restores cursor visibility"
    );
}

#[test]
fn ci_suppressed_all_tty_launch_and_clean_quit_leave_intro_unseen() {
    let isolated = tempfile::tempdir().unwrap();
    let marker = isolated.path().join("data/tui-state.json");
    let mut command = isolated_command(isolated.path());
    // This synthetic value only resolves credential onboarding. The test
    // never submits analysis or sends the value outside the child process.
    command.env("PANGRAM_API_KEY", "synthetic-pty-key");
    std::fs::write(
        isolated.path().join("config.toml"),
        "config_version = 1\n\n[updates]\ncheck_on_tui_start = false\n",
    )
    .expect("preconfigure the non-secret update preference");
    assert!(!marker.exists(), "the isolated launch starts unseen");

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
        .spawn_command(command)
        .expect("spawn compiled pangram in the pseudo-terminal");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");
    let mut writer = pair.master.take_writer().expect("take PTY writer");
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
    let launched = receive_until(&output_rx, &mut transcript, START_TIMEOUT, |bytes| {
        let visible = screen_contents(bytes);
        ["Analyze", "Active", "History", "Settings"]
            .iter()
            .all(|label| visible.contains(label))
    });
    if !launched {
        let _ = child.kill();
        let _ = child.wait();
        drop(writer);
        let _ = reader_thread.join();
        panic!(
            "CI-suppressed all-TTY launch did not reach the TUI:\n{}",
            String::from_utf8_lossy(&transcript)
        );
    }
    assert!(
        !marker.exists(),
        "CI suppression must not consume intro state during startup"
    );

    // Composer -> Public link -> Manual save -> Submit -> Quit.
    transcript.clear();
    writer
        .write_all(b"\t\t\t\t")
        .expect("focus the normal Quit action");
    writer.flush().expect("flush Quit navigation");
    let quit_focused = receive_until(&output_rx, &mut transcript, START_TIMEOUT, |bytes| {
        screen_contents(bytes).contains("> [Enter] Quit <")
    });
    if !quit_focused {
        let _ = child.kill();
        let _ = child.wait();
        drop(writer);
        let _ = reader_thread.join();
        panic!(
            "the normal Quit action did not receive focus:\n{}",
            String::from_utf8_lossy(&transcript)
        );
    }

    writer.write_all(b"\r").expect("activate Quit");
    writer.flush().expect("flush Quit");
    let mut killer = child.clone_killer();
    let (status_tx, status_rx) = mpsc::channel();
    let wait_thread = std::thread::spawn(move || {
        let _ = status_tx.send(child.wait());
    });
    let status = match status_rx.recv_timeout(EXIT_TIMEOUT) {
        Ok(status) => status,
        Err(error) => {
            let _ = killer.kill();
            let _ = wait_thread.join();
            drop(writer);
            let _ = reader_thread.join();
            panic!("TUI did not exit through the Quit action: {error}");
        }
    };
    wait_thread.join().expect("join child waiter");
    drop(writer);
    reader_thread.join().expect("join PTY reader");

    let status = status.expect("wait for TUI exit");
    assert_eq!(status.exit_code(), 0, "the Quit action is a normal exit");
    assert!(
        !marker.exists(),
        "CI suppression must leave the once-only intro unconsumed"
    );
}
