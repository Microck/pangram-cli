//! Positive compiled-TUI analysis through a real pseudo-terminal and the
//! real loopback Pangram 4 protocol fixture. The test crosses the terminal,
//! reducer, adapter, shared analyzer, HTTP, and result-rendering boundaries;
//! no semantic terminal protocol or mocked collaborator can satisfy it.

#![cfg(feature = "dev-tools")]

#[path = "support/protocol_loopback/mod.rs"]
mod fixture;

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use fixture::{
    ProtocolFixture, SYNTHETIC_KEY, Step, TASK_ID, pangram4_success, plagiarism_success,
};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

const SCREEN_TIMEOUT: Duration = Duration::from_secs(5);
const ANALYSIS_TIMEOUT: Duration = Duration::from_secs(10);
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

fn screen_contains(transcript: &[u8], expected: &str) -> bool {
    let mut terminal = vt100::Parser::new(40, 120, 0);
    terminal.process(transcript);
    terminal.screen().contents().contains(expected)
}

fn isolated_command(root: &std::path::Path, endpoint: &str) -> CommandBuilder {
    let home = root.join("home");
    let config_home = root.join("config");
    let data_home = root.join("data");
    let cache_home = root.join("cache");
    let config = config_home.join("pangram.toml");
    for directory in [&home, &config_home, &data_home, &cache_home] {
        std::fs::create_dir_all(directory).unwrap();
    }

    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_pangram-test-driver"));
    command.arg(endpoint);
    command.env_clear();
    for (key, value) in [
        ("HOME", home.as_os_str()),
        ("USERPROFILE", home.as_os_str()),
        ("XDG_CONFIG_HOME", config_home.as_os_str()),
        ("XDG_DATA_HOME", data_home.as_os_str()),
        ("XDG_CACHE_HOME", cache_home.as_os_str()),
        ("PANGRAM_CONFIG", config.as_os_str()),
        ("PANGRAM_DATA_DIR", data_home.as_os_str()),
        ("PANGRAM_API_KEY", OsStr::new(SYNTHETIC_KEY)),
        ("TERM", OsStr::new("xterm-256color")),
        ("CI", OsStr::new("true")),
        ("LANG", OsStr::new("C.UTF-8")),
        ("LC_ALL", OsStr::new("C.UTF-8")),
        ("NO_COLOR", OsStr::new("1")),
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

fn assert_visible(
    receiver: &Receiver<Vec<u8>>,
    transcript: &mut Vec<u8>,
    timeout: Duration,
    expected: &str,
) {
    assert!(
        receive_until(receiver, transcript, timeout, |bytes| {
            screen_contains(bytes, expected)
        }),
        "TUI did not render {expected:?}:\n{}",
        String::from_utf8_lossy(transcript)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn all_tty_combined_analysis_reaches_both_shared_analyzer_routes_once() {
    let text = "This synthetic TUI sentence\tremains human written\n?j today";
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(serde_json::json!({
        "task_id": TASK_ID,
        "stage": "STAGE_PREPROCESSING"
    })));
    fixture.on_poll(Step::Json(serde_json::json!({
        "task_id": TASK_ID,
        "stage": "STAGE_PREPROCESSING"
    })));
    fixture.on_poll(Step::Json(pangram4_success(text)));
    fixture.on_plagiarism(Step::Json(plagiarism_success()));

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
        .spawn_command(isolated_command(isolated.path(), fixture.base_url()))
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
    // Every assertion and terminal write after spawning runs inside this
    // unwind boundary. A failed contract must still reach the common kill,
    // reap, writer-drop, and reader-join path below.
    let interaction = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Wait for the complete overlay body, not its early title bytes. The
        // PTY reader may deliver one frame in several chunks.
        assert_visible(
            &output_rx,
            &mut transcript,
            SCREEN_TIMEOUT,
            "Automatic checks: on",
        );
        writer.write_all(b"n").expect("decline update checks");
        writer.flush().expect("flush onboarding choice");
        assert!(
            receive_until(&output_rx, &mut transcript, SCREEN_TIMEOUT, |bytes| {
                let mut terminal = vt100::Parser::new(40, 120, 0);
                terminal.process(bytes);
                let screen = terminal.screen().contents();
                screen.contains("composer") && !screen.contains("Automatic checks:")
            }),
            "update preference did not persist before composer input:\n{}",
            String::from_utf8_lossy(&transcript)
        );
        assert!(
            transcript
                .windows(b"\x1b[?2004h".len())
                .any(|bytes| bytes == b"\x1b[?2004h"),
            "the TUI enables bracketed paste before accepting composer input"
        );

        // A terminal emulator delivers the complete bracketed-paste frame as
        // one input burst. Keeping the delimiters and payload together also
        // prevents the PTY transport from splitting crossterm's parse unit.
        let mut paste = Vec::with_capacity(text.len() + 12);
        paste.extend_from_slice(b"\x1b[200~");
        paste.extend_from_slice(text.as_bytes());
        paste.extend_from_slice(b"\x1b[201~");
        writer
            .write_all(&paste)
            .expect("paste bracketed analysis text");
        writer.flush().expect("flush bracketed analysis text");
        assert_visible(
            &output_rx,
            &mut transcript,
            SCREEN_TIMEOUT,
            "This synthetic TUI sentence",
        );

        // Composer -> Input files -> Input text -> Both. Select the combined
        // check, wait for that state transition, then traverse the documented
        // regular-keymap order to Submit. The visible boundaries keep PTY
        // input bursts from overtaking one another in crossterm's event queue.
        writer
            .write_all(b"\x1b[Z\x1b[Z\x1b[Z\r")
            .expect("select Both");
        writer.flush().expect("flush combined-check selection");
        assert_visible(&output_rx, &mut transcript, SCREEN_TIMEOUT, ">*Both");
        writer.write_all(b"\t\t\t\t\t\t").expect("focus Submit");
        writer.flush().expect("flush focus navigation");
        writer.write_all(b"\r").expect("activate Submit");
        writer.flush().expect("flush Submit");

        // Accepted combined work must enter Active before its first progress
        // event, and later progress must advance that same session-owned row.
        writer
            .write_all(b"\x1b[<0;3;6M")
            .expect("click the Active route");
        writer.flush().expect("flush Active click");
        assert_visible(
            &output_rx,
            &mut transcript,
            ANALYSIS_TIMEOUT,
            "this session",
        );
        writer
            .write_all(b"\x1b[<0;3;4M")
            .expect("click the Analyze route");
        writer.flush().expect("flush Analyze click");

        assert!(
            receive_until(&output_rx, &mut transcript, ANALYSIS_TIMEOUT, |bytes| {
                screen_contains(bytes, "Classification:")
                    && screen_contains(bytes, "Plagiarism: not detected")
            }),
            "completed combined result was not visibly rendered:\n{}",
            String::from_utf8_lossy(&transcript)
        );

        assert_eq!(fixture.post_count(), 2, "each selected check submits once");
        assert_eq!(
            fixture.get_count(),
            3,
            "the TUI observes each scripted task state exactly once"
        );
        let requests = fixture.requests();
        assert_eq!(
            requests.len(),
            5,
            "two POSTs and three polls reach the fixture"
        );
        let submit = requests
            .iter()
            .find(|request| request.path == "/task")
            .expect("AI submission request");
        assert_eq!(submit.method, "POST");
        assert_eq!(submit.path, "/task");
        assert!(submit.header_equals("x-api-key", SYNTHETIC_KEY));
        let body = submit.body_json();
        assert_eq!(body["text"], text);
        assert_eq!(body["model"], "pangram-4");
        assert_eq!(body["public_dashboard_link"], false);
        let poll = requests
            .iter()
            .find(|request| request.path == format!("/task/{TASK_ID}"))
            .expect("AI poll request");
        assert_eq!(poll.method, "GET");
        let plagiarism = requests
            .iter()
            .find(|request| request.path == "/plagiarism")
            .expect("plagiarism submission request");
        assert_eq!(plagiarism.method, "POST");
        assert!(plagiarism.header_equals("x-api-key", SYNTHETIC_KEY));
        assert_eq!(plagiarism.body_json(), serde_json::json!({"text": text}));

        // Completed results focus the scrollable evidence first. Traverse the
        // focusable New analysis action before reaching Quit without a shortcut.
        writer.write_all(b"\t\t").expect("focus Quit");
        writer.flush().expect("flush Quit navigation");
        writer.write_all(b"\r").expect("activate Quit");
        writer.flush().expect("flush Quit");
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
        .unwrap_or_else(|error| panic!("TUI did not exit through the Quit action: {error}"))
        .expect("wait for TUI exit");
    assert_eq!(status.exit_code(), 0, "the Quit action is a normal exit");
    for (sequence, behavior) in [
        (b"\x1b[?1049h".as_slice(), "enters the alternate screen"),
        (b"\x1b[?2004h".as_slice(), "enables bracketed paste"),
        (b"\x1b[?2004l".as_slice(), "disables bracketed paste"),
        (b"\x1b[?1049l".as_slice(), "restores the primary screen"),
        (b"\x1b[?25h".as_slice(), "restores cursor visibility"),
    ] {
        assert!(
            transcript
                .windows(sequence.len())
                .any(|bytes| bytes == sequence),
            "the TUI {behavior}"
        );
    }
    let visible = String::from_utf8_lossy(&transcript);
    assert!(!visible.contains(SYNTHETIC_KEY), "the API key stays secret");
    assert!(
        !visible.to_ascii_lowercase().contains("x-api-key"),
        "the auth header name stays out of the terminal"
    );

    fixture.shutdown().await;
}
