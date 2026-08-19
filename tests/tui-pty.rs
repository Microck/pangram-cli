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

fn screen_has_rgb(transcript: &[u8], expected: (u8, u8, u8)) -> bool {
    let mut terminal = vt100::Parser::new(40, 120, 0);
    terminal.process(transcript);
    (0..40).any(|row| {
        (0..120).any(|column| {
            terminal.screen().cell(row, column).is_some_and(|cell| {
                cell.fgcolor() == vt100::Color::Rgb(expected.0, expected.1, expected.2)
            })
        })
    })
}

fn screen_contains_setting(transcript: &[u8], label: &str, value: &str) -> bool {
    screen_contents(transcript).lines().any(|line| {
        let line = line.trim();
        line.contains(label) && line.ends_with(value)
    })
}

fn isolated_command(root: &std::path::Path, ci: bool, no_color: bool) -> CommandBuilder {
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
        ("LANG", std::ffi::OsStr::new("C.UTF-8")),
        ("LC_ALL", std::ffi::OsStr::new("C.UTF-8")),
    ] {
        command.env(key, value);
    }
    if ci {
        command.env("CI", "true");
    }
    if no_color {
        command.env("NO_COLOR", "1");
    } else {
        command.env("COLORTERM", "truecolor");
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
        .spawn_command(isolated_command(isolated.path(), true, true))
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
    let interaction = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let launched = receive_until(&output_rx, &mut transcript, START_TIMEOUT, |bytes| {
            let visible = screen_contents(bytes);
            ["Analyze", "Active", "History", "Settings"]
                .iter()
                .all(|label| visible.contains(label))
        });
        assert!(
            launched,
            "bare all-TTY launch did not reach the four-route TUI:\n{}",
            String::from_utf8_lossy(&transcript)
        );

        writer.write_all(&[0x03]).expect("send Ctrl+C");
        writer.flush().expect("flush Ctrl+C");
    }));
    let mut killer = child.clone_killer();
    if interaction.is_err() {
        let _ = killer.kill();
    }
    let (status_tx, status_rx) = mpsc::channel();
    let wait_thread = std::thread::spawn(move || {
        let _ = status_tx.send(child.wait());
    });
    let status = status_rx.recv_timeout(EXIT_TIMEOUT);
    if status.is_err() {
        let _ = killer.kill();
    }
    let wait_join = wait_thread.join();
    drop(writer);
    let reader_join = reader_thread.join();
    while let Ok(chunk) = output_rx.try_recv() {
        transcript.extend_from_slice(&chunk);
    }
    if let Err(payload) = interaction {
        std::panic::resume_unwind(payload);
    }

    wait_join.expect("join child waiter");
    reader_join.expect("join PTY reader");
    let status = status
        .unwrap_or_else(|error| panic!("TUI did not exit after Ctrl+C: {error}"))
        .expect("wait for TUI exit");
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
fn ci_suppressed_mouse_route_and_clean_quit_leave_intro_unseen() {
    let isolated = tempfile::tempdir().unwrap();
    let data_dir = isolated.path().join("data");
    let mut command = isolated_command(isolated.path(), true, true);
    // This synthetic value only resolves credential onboarding. The test
    // never submits analysis or sends the value outside the child process.
    command.env("PANGRAM_API_KEY", "synthetic-pty-key");
    std::fs::write(
        isolated.path().join("config.toml"),
        "config_version = 1\n\n[updates]\ncheck_on_tui_start = false\n",
    )
    .expect("preconfigure the non-secret update preference");
    assert_eq!(
        std::fs::read_dir(&data_dir).unwrap().count(),
        0,
        "the production-selected data directory starts empty"
    );

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
    let interaction = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let launched = receive_until(&output_rx, &mut transcript, START_TIMEOUT, |bytes| {
            let visible = screen_contents(bytes);
            ["Analyze", "Active", "History", "Settings"]
                .iter()
                .all(|label| visible.contains(label))
        });
        assert!(
            launched,
            "CI-suppressed all-TTY launch did not reach the TUI:\n{}",
            String::from_utf8_lossy(&transcript)
        );
        assert!(
            std::fs::read_dir(&data_dir).unwrap().next().is_none(),
            "CI suppression must not write state during startup"
        );

        // SGR mouse coordinates are one-based on the wire. Click the visible
        // Settings route at terminal cell (2, 9), then prove the same terminal
        // stream can activate the focused command-bar action with a click.
        writer
            .write_all(b"\x1b[<0;3;10M")
            .expect("click the Settings route");
        writer.flush().expect("flush Settings click");
        let settings_open = receive_until(&output_rx, &mut transcript, START_TIMEOUT, |bytes| {
            screen_contains_setting(bytes, "Keymap", "Regular")
        });
        assert!(
            settings_open,
            "the Settings mouse target did not activate:\n{}",
            String::from_utf8_lossy(&transcript)
        );

        // End focuses Quit without depending on the route's focus count.
        writer
            .write_all(b"\x1b[F")
            .expect("focus the normal Quit action");
        writer.flush().expect("flush Quit navigation");
        let quit_focused = receive_until(&output_rx, &mut transcript, START_TIMEOUT, |bytes| {
            screen_contents(bytes).contains("enter  quit")
        });
        assert!(
            quit_focused,
            "the normal Quit action did not receive focus:\n{}",
            String::from_utf8_lossy(&transcript)
        );

        writer
            // The wide command bar begins after the route rail. This lands
            // inside the visible `enter  quit` target, not the preceding Help
            // control. SGR coordinates are one-based.
            .write_all(b"\x1b[<0;68;39M")
            .expect("click the focused command-bar action");
        writer.flush().expect("flush Quit click");
    }));
    let mut killer = child.clone_killer();
    if interaction.is_err() {
        let _ = killer.kill();
    }
    let (status_tx, status_rx) = mpsc::channel();
    let wait_thread = std::thread::spawn(move || {
        let _ = status_tx.send(child.wait());
    });
    let status = status_rx.recv_timeout(EXIT_TIMEOUT);
    if status.is_err() {
        let _ = killer.kill();
    }
    let wait_join = wait_thread.join();
    drop(writer);
    let reader_join = reader_thread.join();
    if let Err(payload) = interaction {
        std::panic::resume_unwind(payload);
    }

    wait_join.expect("join child waiter");
    reader_join.expect("join PTY reader");
    let status = status
        .unwrap_or_else(|error| panic!("TUI did not exit through the Quit action: {error}"))
        .expect("wait for TUI exit");
    assert_eq!(status.exit_code(), 0, "the Quit action is a normal exit");
    // This path must stay negative: CI makes the launch ineligible, so the
    // runtime does not consume a generated intro plan.
    // Assert against the production-selected data root instead of duplicating
    // the private marker filename. Directory emptiness is stronger: any intro
    // state write fails this contract, even if production changes its path.
    assert!(
        std::fs::read_dir(&data_dir).unwrap().next().is_none(),
        "CI suppression must leave the production data directory unchanged"
    );
}

#[test]
fn eligible_truecolor_intro_renders_and_skip_records_once_state() {
    let isolated = tempfile::tempdir().unwrap();
    let data_dir = isolated.path().join("data");
    let mut command = isolated_command(isolated.path(), false, false);
    command.env("PANGRAM_API_KEY", "synthetic-intro-pty-key");
    std::fs::write(
        isolated.path().join("config.toml"),
        "config_version = 1\n\n[updates]\ncheck_on_tui_start = false\n",
    )
    .expect("preconfigure the non-secret update preference");

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
    let interaction = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let fox_rendered = receive_until(&output_rx, &mut transcript, START_TIMEOUT, |bytes| {
            let visible = screen_contents(bytes);
            !visible.contains("Analyze")
                && visible
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .count()
                    > 200
        });
        assert!(
            fox_rendered,
            "eligible launch did not render the generated fox:\n{}",
            String::from_utf8_lossy(&transcript)
        );
        assert!(
            screen_has_rgb(&transcript, (255, 97, 6)),
            "truecolor playback did not render Pangram orange"
        );

        writer.write_all(b"\r").expect("skip intro with Enter");
        writer.flush().expect("flush intro skip");
        let analyze_open = receive_until(&output_rx, &mut transcript, START_TIMEOUT, |bytes| {
            ["Analyze", "Active", "History", "Settings"]
                .iter()
                .all(|label| screen_contents(bytes).contains(label))
        });
        assert!(analyze_open, "skip did not open Analyze");

        let marker = data_dir.join("tui-state.json");
        let deadline = Instant::now() + Duration::from_secs(1);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read_to_string(marker).unwrap(),
            "{\n  \"schema_version\": \"1\",\n  \"intro_seen\": true\n}\n"
        );

        writer.write_all(&[0x03]).expect("send Ctrl+C");
        writer.flush().expect("flush Ctrl+C");
    }));
    let mut killer = child.clone_killer();
    if interaction.is_err() {
        let _ = killer.kill();
    }
    let (status_tx, status_rx) = mpsc::channel();
    let wait_thread = std::thread::spawn(move || {
        let _ = status_tx.send(child.wait());
    });
    let status = status_rx.recv_timeout(EXIT_TIMEOUT);
    if status.is_err() {
        let _ = killer.kill();
    }
    wait_thread.join().expect("join child waiter");
    drop(writer);
    reader_thread.join().expect("join PTY reader");
    while let Ok(chunk) = output_rx.try_recv() {
        transcript.extend_from_slice(&chunk);
    }
    if let Err(payload) = interaction {
        std::panic::resume_unwind(payload);
    }

    let status = status
        .unwrap_or_else(|error| panic!("TUI did not exit after intro skip: {error}"))
        .expect("wait for TUI exit");
    assert_eq!(status.exit_code(), 130);
    assert!(
        transcript
            .windows(b"\x1b[?1049l".len())
            .any(|bytes| bytes == b"\x1b[?1049l"),
        "the TUI restores the primary screen after intro playback"
    );
}
