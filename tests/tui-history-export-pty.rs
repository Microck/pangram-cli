//! Compiled History export contracts through a real native pseudo-terminal.
//! These tests seed certified SQLite history, drive the public keyboard flow,
//! and inspect the same terminal and export bytes a person or shell receives.

#![cfg(feature = "dev-tools")]

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use microck_pangram_cli::domain::{
    AnalysisId, AnalysisInput, AnalysisStatus, CheckKind, CheckStatus, SaveState, Sha256Hash,
    SubmissionOutcome, TextInput, TextOrigin, UtcTimestamp,
};
use microck_pangram_cli::history::{HistoryStore, InputKind, StoredAnalysis, StoredCheck};
use portable_pty::{CommandBuilder, ExitStatus, NativePtySystem, PtySize, PtySystem};

const SCREEN_TIMEOUT: Duration = Duration::from_secs(5);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const ANALYSIS_ID: &str = "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8b01";
const RETAINED_TEXT: &str = "private retained export text must stay off the TUI";
const SEGMENT_TEXT: &str = "private segment evidence must stay off the TUI";
const SYNTHETIC_KEY: &str = "synthetic-export-pty-key";
const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";

struct Isolated {
    root: tempfile::TempDir,
    config: PathBuf,
    data: PathBuf,
}

impl Isolated {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("temporary root");
        let config_home = root.path().join("config");
        let data = root.path().join("data");
        std::fs::create_dir_all(&config_home).expect("create config directory");
        std::fs::create_dir_all(&data).expect("create data directory");
        let config = config_home.join("pangram.toml");
        std::fs::write(
            &config,
            "config_version = 1\n\n[history]\nenabled = false\n\n[updates]\ncheck_on_tui_start = false\n",
        )
        .expect("write isolated TUI configuration");

        seed_history(&data);
        Self { root, config, data }
    }

    fn command(&self) -> CommandBuilder {
        let home = self.root.path().join("home");
        let cache = self.root.path().join("cache");
        std::fs::create_dir_all(&home).expect("create home directory");
        std::fs::create_dir_all(&cache).expect("create cache directory");

        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_pangram"));
        command.env_clear();
        for (key, value) in [
            ("HOME", home.as_os_str()),
            ("USERPROFILE", home.as_os_str()),
            (
                "XDG_CONFIG_HOME",
                self.config.parent().expect("config parent").as_os_str(),
            ),
            ("XDG_DATA_HOME", self.data.as_os_str()),
            ("XDG_CACHE_HOME", cache.as_os_str()),
            (
                "APPDATA",
                self.config.parent().expect("config parent").as_os_str(),
            ),
            ("LOCALAPPDATA", self.data.as_os_str()),
            ("PANGRAM_CONFIG", self.config.as_os_str()),
            ("PANGRAM_DATA_DIR", self.data.as_os_str()),
            ("PANGRAM_API_KEY", OsStr::new(SYNTHETIC_KEY)),
            ("TERM", OsStr::new("xterm-256color")),
            ("CI", OsStr::new("true")),
            ("LANG", OsStr::new("C.UTF-8")),
            ("LC_ALL", OsStr::new("C.UTF-8")),
            ("NO_COLOR", OsStr::new("1")),
        ] {
            command.env(key, value);
        }
        for key in ["SYSTEMROOT", "WINDIR", "COMSPEC"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(OsString::from(key), value);
            }
        }
        command
    }
}

struct PtyOutcome {
    status: ExitStatus,
    transcript: Vec<u8>,
}

fn seed_history(data: &Path) {
    let id = AnalysisId::from_str(ANALYSIS_ID).expect("canonical seed ID");
    let timestamp =
        UtcTimestamp::from_str("2026-08-10T12:00:00Z").expect("canonical seed timestamp");
    let input_sha256 = Sha256Hash::digest(RETAINED_TEXT);
    let input = AnalysisInput::Text(
        TextInput::new(
            TextOrigin::Literal,
            None,
            input_sha256,
            u64::try_from(RETAINED_TEXT.len()).expect("seed byte count"),
            u64::try_from(RETAINED_TEXT.split_whitespace().count()).expect("seed word count"),
            Some(RETAINED_TEXT.to_owned()),
        )
        .expect("canonical retained input"),
    );
    let result_json = serde_json::json!({
        "classification": "mixed",
        "headline": "Mixed",
        "prediction": "Mixed",
        "fraction_ai": 0.5,
        "fraction_ai_assisted": 0.0,
        "fraction_human": 0.5,
        "num_ai_segments": 1,
        "num_ai_assisted_segments": 0,
        "num_human_segments": 0,
        "segments": [{
            "text": SEGMENT_TEXT,
            "label": "AI",
            "ai_assistance_score": 0.8,
            "confidence": "high",
            "start_index": 0,
            "end_index": SEGMENT_TEXT.len(),
            "word_count": SEGMENT_TEXT.split_whitespace().count(),
            "token_length": 8,
            "humanizer_score": 0.2,
            "is_humanized": false
        }],
        "dashboard_link": "https://dashboard.example/private-result"
    })
    .to_string();
    let record = StoredAnalysis {
        id,
        bulk: None,
        caller_id: None,
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        save_state: SaveState::SavedManual,
        input_kind: InputKind::Text,
        input_sha256,
        display_name: None,
        input_json: serde_json::to_string(&input).expect("serialize retained input"),
        result_json: Some(result_json.clone()),
        error_json: None,
        upstream_version: Some("4.0".to_owned()),
        retry_of: None,
        rerun_of: None,
        submitted_at: Some(timestamp),
        created_at: timestamp,
        updated_at: timestamp,
        completed_at: Some(timestamp),
        search_input_text: Some(RETAINED_TEXT.to_owned()),
        search_filename: None,
        search_headline: Some("Mixed".to_owned()),
        search_source_urls: None,
    };
    let check = StoredCheck {
        analysis_id: id,
        check_index: 0,
        check_kind: CheckKind::AiDetection,
        status: CheckStatus::Succeeded,
        result_json: Some(result_json),
        error_json: None,
    };
    let mut store = HistoryStore::open(data).expect("open real history store");
    store
        .save_analysis_complete(&record, &[check], &[])
        .expect("save certified retained-content record");
    store
        .canonical_analysis(&id, true)
        .expect("seed round-trips through canonical certification");
}

fn run_in_pty(
    isolated: &Isolated,
    interact: impl FnOnce(&Receiver<Vec<u8>>, &mut Vec<u8>, &mut dyn Write),
) -> PtyOutcome {
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
        .spawn_command(isolated.command())
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
        interact(&output_rx, &mut transcript, &mut *writer);
    }));
    let mut killer = child.clone_killer();
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
            panic!("TUI did not exit within {EXIT_TIMEOUT:?}: {error}");
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

    PtyOutcome { status, transcript }
}

fn receive_until(
    receiver: &Receiver<Vec<u8>>,
    transcript: &mut Vec<u8>,
    predicate: impl Fn(&[u8]) -> bool,
) -> bool {
    let deadline = Instant::now() + SCREEN_TIMEOUT;
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

fn assert_screen(
    receiver: &Receiver<Vec<u8>>,
    transcript: &mut Vec<u8>,
    predicate: impl Fn(&str) -> bool,
    description: &str,
) {
    assert!(
        receive_until(receiver, transcript, |bytes| {
            predicate(&screen_contents(bytes))
        }),
        "TUI did not render {description}:\n{}",
        String::from_utf8_lossy(transcript)
    );
}

fn write_keys(writer: &mut dyn Write, keys: &[u8], description: &str) {
    writer.write_all(keys).unwrap_or_else(|error| {
        panic!("failed to send {description}: {error}");
    });
    writer.flush().unwrap_or_else(|error| {
        panic!("failed to flush {description}: {error}");
    });
}

fn enter_history_export(
    receiver: &Receiver<Vec<u8>>,
    transcript: &mut Vec<u8>,
    writer: &mut dyn Write,
) {
    assert_screen(
        receiver,
        transcript,
        |screen| screen.contains("Text composer"),
        "the initial Analyze composer",
    );

    // Composer -> Routes, then Analyze -> History. Waiting for the certified
    // row also proves that the initial asynchronous load has released its
    // one-operation gate before Export is activated.
    let mut keys = b"\x1b[Z".repeat(6);
    keys.extend_from_slice(&b"\x1b[C".repeat(2));
    write_keys(writer, &keys, "navigate to local History");
    assert_screen(
        receiver,
        transcript,
        |screen| screen.contains("Local Pangram CLI history") && screen.contains("Showing 1"),
        "the certified local History row",
    );

    // Routes -> Search -> Status -> Check -> List -> Rerun -> Export.
    write_keys(writer, &b"\t".repeat(6), "focus the History Export action");
    assert_screen(
        receiver,
        transcript,
        |screen| {
            screen.contains("Rerun") && screen.contains("> Export") && screen.contains("Delete")
        },
        "the focused History Export action",
    );
    write_keys(writer, b"\r", "open the History export overlay");
    assert_screen(
        receiver,
        transcript,
        |screen| {
            screen.contains("Export local history")
                && screen.contains("Format  JSONL")
                && screen.contains("Content  redacted")
                && screen.contains("> Action  cancel <")
        },
        "the default-cancel JSONL redacted export overlay",
    );
}

fn finish_with_quit(writer: &mut dyn Write) {
    // Export -> Delete -> Quit. This proves cancellation returned to the
    // interactive focus model instead of silently ending the session.
    write_keys(writer, b"\t\t\r", "activate the focusable Quit action");
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|candidate| candidate == needle)
}

fn byte_position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|candidate| candidate == needle)
}

fn byte_rposition(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|candidate| candidate == needle)
}

/// Cursor patches can replace spaces instead of re-emitting them. Removing
/// terminal controls and ASCII whitespace checks every transient screen for a
/// private phrase without assuming one particular Ratatui repaint strategy.
fn transcript_contains_rendered_text(transcript: &[u8], expected: &str) -> bool {
    let visible = visible_ascii(transcript);
    let expected = expected
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    visible
        .windows(expected.len())
        .any(|candidate| candidate == expected)
}

fn visible_ascii(transcript: &[u8]) -> Vec<u8> {
    let mut visible = Vec::with_capacity(transcript.len());
    let mut index = 0;
    while index < transcript.len() {
        if transcript[index] == b'\x1b' {
            index += 1;
            match transcript.get(index) {
                Some(b'[') => {
                    index += 1;
                    while let Some(byte) = transcript.get(index) {
                        index += 1;
                        if (0x40..=0x7e).contains(byte) {
                            break;
                        }
                    }
                }
                Some(_) => index += 1,
                None => {}
            }
        } else {
            let byte = transcript[index];
            index += 1;
            if byte.is_ascii_graphic() {
                visible.push(byte);
            }
        }
    }
    visible
}

fn assert_no_private_terminal_cells(transcript: &[u8]) {
    for private in [RETAINED_TEXT, SEGMENT_TEXT, SYNTHETIC_KEY] {
        assert!(
            !transcript_contains_rendered_text(transcript, private),
            "private value entered rendered terminal cells: {private:?}"
        );
    }
}

fn jsonl_prefix() -> Vec<u8> {
    format!("{{\"id\":\"{ANALYSIS_ID}\"").into_bytes()
}

fn assert_normal_exit_without_export(outcome: &PtyOutcome) {
    assert_eq!(outcome.status.exit_code(), 0, "Quit is a normal exit");
    assert!(
        !contains_bytes(&outcome.transcript, &jsonl_prefix()),
        "a cancelled overlay must not emit a JSONL document"
    );
    assert!(
        contains_bytes(&outcome.transcript, LEAVE_ALTERNATE_SCREEN),
        "normal Quit restores the primary screen"
    );
    assert!(
        contains_bytes(&outcome.transcript, SHOW_CURSOR),
        "normal Quit restores cursor visibility"
    );
    assert_no_private_terminal_cells(&outcome.transcript);
}

#[test]
fn default_cancel_emits_no_export_document_and_history_stays_interactive() {
    let isolated = Isolated::new();
    let outcome = run_in_pty(&isolated, |receiver, transcript, writer| {
        enter_history_export(receiver, transcript, writer);
        write_keys(writer, b"\r", "accept the default Cancel action");
        assert_screen(
            receiver,
            transcript,
            |screen| {
                screen.contains("Local Pangram CLI history")
                    && !screen.contains("Export local history")
            },
            "History after cancelling export",
        );
        finish_with_quit(writer);
    });

    assert_normal_exit_without_export(&outcome);
}

#[test]
fn confirmed_redacted_jsonl_restores_terminal_before_exporting_certified_record() {
    let isolated = Isolated::new();
    let outcome = run_in_pty(&isolated, |receiver, transcript, writer| {
        enter_history_export(receiver, transcript, writer);
        write_keys(writer, b"\x1b[C", "change the export action to Export");
        assert_screen(
            receiver,
            transcript,
            |screen| screen.contains("> Action  export <"),
            "the confirmed redacted export action",
        );
        write_keys(writer, b"\r", "confirm redacted JSONL export");
    });

    assert_eq!(outcome.status.exit_code(), 0, "certified export succeeds");
    let enter = byte_position(&outcome.transcript, ENTER_ALTERNATE_SCREEN)
        .expect("TUI enters the alternate screen");
    let leave = byte_position(&outcome.transcript, LEAVE_ALTERNATE_SCREEN)
        .expect("TUI restores the primary screen");
    let prefix = jsonl_prefix();
    let json_start = byte_position(&outcome.transcript, &prefix)
        .expect("confirmed export writes the certified JSONL record");
    let cursor = byte_rposition(&outcome.transcript[..json_start], SHOW_CURSOR)
        .expect("TUI restores cursor visibility before exporting");
    assert!(
        enter < cursor && cursor < leave,
        "cursor restoration must precede primary-screen restoration"
    );
    assert!(
        enter < leave && leave < json_start,
        "LeaveAlternateScreen must precede the first JSONL byte"
    );
    assert_eq!(
        outcome
            .transcript
            .windows(prefix.len())
            .filter(|candidate| *candidate == prefix.as_slice())
            .count(),
        1,
        "one certified history record produces one JSONL document"
    );

    let json_end = outcome.transcript[json_start..]
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
        .map_or(outcome.transcript.len(), |offset| json_start + offset);
    let exported: serde_json::Value =
        serde_json::from_slice(&outcome.transcript[json_start..json_end])
            .expect("export emits one valid JSONL object");
    assert_eq!(exported["id"], ANALYSIS_ID);
    assert_eq!(exported["status"], "succeeded");
    assert_eq!(exported["checks"][0]["kind"], "ai_detection");
    assert!(
        exported["input"].get("text").is_none(),
        "redacted JSONL omits retained input text"
    );
    assert_eq!(
        exported["checks"][0]["result"]["segments"][0]["text"], "",
        "redacted JSONL keeps segment structure without evidence text"
    );
    assert!(
        exported["checks"][0]["result"]
            .get("dashboard_link")
            .is_none(),
        "redacted JSONL omits the private dashboard URL"
    );
    assert_no_private_terminal_cells(&outcome.transcript);
}

#[test]
fn full_content_requires_second_confirmation_whose_bare_enter_cancels() {
    let isolated = Isolated::new();
    let outcome = run_in_pty(&isolated, |receiver, transcript, writer| {
        enter_history_export(receiver, transcript, writer);

        // Action -> Content, then select full content. Returning to Action and
        // choosing Export must open a second, independently default-cancelled
        // confirmation instead of ending the terminal session.
        write_keys(
            writer,
            b"\x1b[Z\x1b[C",
            "select full retained export content",
        );
        assert_screen(
            receiver,
            transcript,
            |screen| screen.contains("> Content  full retained content <"),
            "the full retained content choice",
        );
        write_keys(
            writer,
            b"\t\x1b[C\r",
            "request the full-content JSONL export",
        );
        assert_screen(
            receiver,
            transcript,
            |screen| {
                screen.contains("Export full retained content")
                    && screen.contains("> Cancel   right Export full content")
            },
            "the second default-cancel full-content confirmation",
        );

        write_keys(
            writer,
            b"\r",
            "accept the second confirmation's default Cancel action",
        );
        assert_screen(
            receiver,
            transcript,
            |screen| {
                screen.contains("Local Pangram CLI history")
                    && !screen.contains("Export full retained content")
            },
            "interactive History after cancelling full export",
        );
        finish_with_quit(writer);
    });

    assert_normal_exit_without_export(&outcome);
}
