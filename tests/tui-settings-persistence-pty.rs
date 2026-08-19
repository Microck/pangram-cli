//! Settings persistence through the compiled binary and a real native PTY.
//!
//! The test changes the keymap through visible TUI controls, verifies the
//! stored value through a fresh noninteractive CLI process, then proves that a
//! fresh TUI process starts with the Vim behavior it loaded from that file.

#![cfg(feature = "dev-tools")]

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use serde_json::Value;

const SCREEN_TIMEOUT: Duration = Duration::from_secs(5);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const SYNTHETIC_KEY: &str = "pangram_synthetic_tui_settings_key_abcdef0123456789_NOT_A_REAL_KEY";

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

fn isolated_env(root: &Path) -> Vec<(OsString, OsString)> {
    let home = root.join("home");
    let config_home = root.join("config");
    let data_home = root.join("data");
    let cache_home = root.join("cache");
    for directory in [&home, &config_home, &data_home, &cache_home] {
        std::fs::create_dir_all(directory).unwrap();
    }

    let mut environment = vec![
        (OsString::from("HOME"), home.clone().into_os_string()),
        (OsString::from("USERPROFILE"), home.into_os_string()),
        (
            OsString::from("XDG_CONFIG_HOME"),
            config_home.clone().into_os_string(),
        ),
        (
            OsString::from("XDG_DATA_HOME"),
            data_home.clone().into_os_string(),
        ),
        (
            OsString::from("XDG_CACHE_HOME"),
            cache_home.into_os_string(),
        ),
        (
            OsString::from("APPDATA"),
            config_home.clone().into_os_string(),
        ),
        (
            OsString::from("LOCALAPPDATA"),
            data_home.clone().into_os_string(),
        ),
        (
            OsString::from("PANGRAM_CONFIG"),
            config_home.join("pangram.toml").into_os_string(),
        ),
        (
            OsString::from("PANGRAM_DATA_DIR"),
            data_home.into_os_string(),
        ),
        (
            OsString::from("PANGRAM_API_KEY"),
            OsString::from(SYNTHETIC_KEY),
        ),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("CI"), OsString::from("true")),
        (OsString::from("LANG"), OsString::from("C.UTF-8")),
        (OsString::from("LC_ALL"), OsString::from("C.UTF-8")),
        (OsString::from("NO_COLOR"), OsString::from("1")),
    ];

    // CreateProcess needs these platform bootstrap values on some Windows
    // hosts. They contain no credentials or Pangram state.
    for key in ["SYSTEMROOT", "WINDIR", "COMSPEC"] {
        if let Some(value) = std::env::var_os(key) {
            environment.push((OsString::from(key), value));
        }
    }
    environment
}

fn compiled_cli_output(root: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pangram"));
    command.env_clear();
    command.envs(isolated_env(root));
    command.args(args).stdin(Stdio::null());
    command.output().expect("run compiled pangram CLI")
}

fn success_data(output: &Output, expected_command: &str) -> Value {
    assert!(
        output.status.success(),
        "{expected_command} failed with {:?}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{expected_command} wrote stderr on success:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{expected_command} did not emit one JSON envelope: {error}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(envelope["schema_version"], "1");
    assert_eq!(envelope["command"], expected_command);
    envelope["data"].clone()
}

fn assert_config_value(root: &Path, expected: &str) {
    let output = compiled_cli_output(root, &["config", "get", "tui.keymap"]);
    let data = success_data(&output, "config_get");
    assert_eq!(data["key"], "tui.keymap");
    assert_eq!(data["value"], expected);
}

fn pty_command(root: &Path) -> CommandBuilder {
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_pangram"));
    command.env_clear();
    for (key, value) in isolated_env(root) {
        command.env(key, value);
    }
    command
}

fn screen_contains(transcript: &[u8], expected: &str) -> bool {
    let mut terminal = vt100::Parser::new(40, 120, 0);
    terminal.process(transcript);
    terminal.screen().contents().contains(expected)
}

fn screen_contains_setting(transcript: &[u8], label: &str, value: &str) -> bool {
    let mut terminal = vt100::Parser::new(40, 120, 0);
    terminal.process(transcript);
    terminal.screen().contents().lines().any(|line| {
        let line = line.trim();
        line.contains(label) && line.ends_with(value)
    })
}

fn assert_screen_text(receiver: &Receiver<Vec<u8>>, transcript: &mut Vec<u8>, expected: &str) {
    assert!(
        receive_until(receiver, transcript, SCREEN_TIMEOUT, |bytes| {
            screen_contains(bytes, expected)
        }),
        "TUI did not render {expected:?} after the interaction:\n{}",
        String::from_utf8_lossy(transcript)
    );
}

fn write_keys(writer: &mut dyn Write, keys: &[u8], context: &str) {
    writer.write_all(keys).unwrap_or_else(|error| {
        panic!("failed to send {context}: {error}");
    });
    writer.flush().unwrap_or_else(|error| {
        panic!("failed to flush {context}: {error}");
    });
}

fn run_tui(root: &Path, interact: impl FnOnce(&Receiver<Vec<u8>>, &mut Vec<u8>, &mut dyn Write)) {
    let pair = NativePtySystem::default()
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open native pseudo-terminal");
    // portable-pty attaches the child stdin, stdout, and stderr to the slave,
    // so bare dispatch sees the required all-streams interactive launch.
    let mut child = pair
        .slave
        .spawn_command(pty_command(root))
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

    let mut killer = child.clone_killer();
    let mut transcript = Vec::new();
    let interaction = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_screen_text(&output_rx, &mut transcript, "Text composer");
        interact(&output_rx, &mut transcript, writer.as_mut());
    }));
    if interaction.is_err() {
        let _ = killer.kill();
    }

    let (status_tx, status_rx) = mpsc::channel();
    let wait_thread = std::thread::spawn(move || {
        let _ = status_tx.send(child.wait());
    });
    let status = match status_rx.recv_timeout(EXIT_TIMEOUT) {
        Ok(status) => status.expect("wait for TUI exit"),
        Err(error) => {
            let _ = killer.kill();
            panic!("TUI did not exit through the Quit action: {error}");
        }
    };
    wait_thread.join().expect("join child waiter");
    drop(writer);
    reader_thread.join().expect("join PTY reader");
    while let Ok(chunk) = output_rx.try_recv() {
        transcript.extend_from_slice(&chunk);
    }

    if let Err(payload) = interaction {
        std::panic::resume_unwind(payload);
    }

    assert_eq!(status.exit_code(), 0, "the Quit action is a normal exit");
    for (sequence, behavior) in [
        (b"\x1b[?1049h".as_slice(), "enters the alternate screen"),
        (b"\x1b[?1049l".as_slice(), "restores the primary screen"),
        (b"\x1b[?25h".as_slice(), "restores cursor visibility"),
    ] {
        assert!(contains_bytes(&transcript, sequence), "the TUI {behavior}");
    }
    assert!(
        !contains_bytes(&transcript, SYNTHETIC_KEY.as_bytes()),
        "the synthetic credential stays out of the terminal"
    );
}

fn regular_open_settings(
    receiver: &Receiver<Vec<u8>>,
    transcript: &mut Vec<u8>,
    writer: &mut dyn Write,
) {
    // Composer -> Routes through the documented regular-keymap BackTab, then
    // Analyze -> Settings through three Right arrows.
    let mut keys = b"\x1b[Z".repeat(6);
    keys.extend_from_slice(&b"\x1b[C".repeat(3));
    write_keys(writer, &keys, "regular navigation to Settings");
    assert_screen_text(receiver, transcript, "Diagnostics");
}

#[test]
fn keymap_change_persists_across_cli_and_tui_processes() {
    let isolated = tempfile::tempdir().unwrap();

    // Resolve the update preference before the TUI launch so this test only
    // exercises the Settings journey. The synthetic environment credential
    // similarly avoids credential onboarding without writing a secret.
    let setup = compiled_cli_output(
        isolated.path(),
        &["config", "set", "updates.check_on_tui_start", "false"],
    );
    assert_eq!(success_data(&setup, "config_set")["ok"], true);
    assert_config_value(isolated.path(), "regular");

    run_tui(isolated.path(), |receiver, transcript, writer| {
        regular_open_settings(receiver, transcript, writer);

        // Routes -> Authentication -> History -> Intro -> Keymap, then change
        // the keymap through the focused Settings control.
        let mut keys = b"\x1b[B".repeat(4);
        keys.push(b'\r');
        write_keys(writer, &keys, "change the Settings keymap to Vim");
        assert!(
            receive_until(receiver, transcript, SCREEN_TIMEOUT, |bytes| {
                screen_contains_setting(bytes, "Keymap", "Vim")
            }),
            "TUI did not render the Vim keymap after the interaction:\n{}",
            String::from_utf8_lossy(transcript)
        );

        // Keymap -> Motion -> Updates -> Quit, using the normal focusable exit.
        write_keys(writer, b"\t\t\t\r", "activate the Quit action");
    });

    // This is a new compiled process reading the effective configuration, not
    // an in-process config object or a hand-parsed implementation detail.
    assert_config_value(isolated.path(), "vim");

    run_tui(isolated.path(), |receiver, transcript, writer| {
        // BackTab remains available in Vim mode. Once Routes is focused, the
        // persisted Vim `l` binding must move from Analyze to Settings.
        let mut keys = b"\x1b[Z".repeat(6);
        keys.extend_from_slice(b"lll");
        write_keys(writer, &keys, "use the persisted Vim route navigation");
        assert!(
            receive_until(receiver, transcript, SCREEN_TIMEOUT, |bytes| {
                screen_contains_setting(bytes, "Keymap", "Vim")
            }),
            "TUI did not render the persisted Vim keymap:\n{}",
            String::from_utf8_lossy(transcript)
        );

        // `G` is another restart-visible Vim binding and focuses normal Quit.
        write_keys(writer, b"G\r", "use Vim to activate the Quit action");
    });
}
