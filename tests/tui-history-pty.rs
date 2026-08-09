//! Compiled History journey through a real PTY, certified SQLite records, and
//! the real loopback Pangram protocol fixture. The test crosses every adapter
//! boundary that can otherwise drift: keyboard focus, literal search, closed
//! filters, redacted detail, destructive confirmation, billable rerun,
//! automatic retention, and terminal restoration.

#![cfg(feature = "dev-tools")]

#[path = "support/protocol_loopback/mod.rs"]
mod fixture;

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::Path;
use std::str::FromStr as _;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use fixture::{ProtocolFixture, SYNTHETIC_KEY, Step, TASK_ID, pangram4_success};
use microck_pangram_cli::domain::{
    Analysis, AnalysisId, AnalysisInput, AnalysisStatus, CheckKind, CheckStatus, SaveState,
    Sha256Hash, SubmissionOutcome, TextInput, TextOrigin, UtcTimestamp,
};
use microck_pangram_cli::history::{HistoryStore, InputKind, StoredAnalysis, StoredCheck};
use microck_pangram_cli::output::CanonicalError;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

const SCREEN_TIMEOUT: Duration = Duration::from_secs(5);
const ANALYSIS_TIMEOUT: Duration = Duration::from_secs(10);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const TARGET_ID: &str = "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a01";
const PLAGIARISM_ID: &str = "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a02";
const NONMATCH_ID: &str = "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a03";
const RERUN_TEXT: &str = "needle private retained words stay hidden from terminal";

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|candidate| candidate == needle)
}

fn screen_contains(transcript: &[u8], expected: &str) -> bool {
    let mut terminal = vt100::Parser::new(40, 120, 0);
    terminal.process(transcript);
    terminal.screen().contents().contains(expected)
}

/// Checks the complete byte history for text that may have appeared on an
/// intermediate screen. Spaces can be cursor movements, so both sides are
/// compared without ASCII whitespace after control sequences are removed.
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

fn write_keys(writer: &mut dyn Write, keys: &[u8], context: &str) {
    writer.write_all(keys).unwrap_or_else(|error| {
        panic!("failed to send {context}: {error}");
    });
    writer.flush().unwrap_or_else(|error| {
        panic!("failed to flush {context}: {error}");
    });
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

struct Isolated {
    root: tempfile::TempDir,
    config: std::path::PathBuf,
    data: std::path::PathBuf,
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
            "config_version = 1\n\n[history]\nenabled = true\n\n[updates]\ncheck_on_tui_start = false\n",
        )
        .expect("configure automatic history and resolve update onboarding");
        Self { root, config, data }
    }

    fn command(&self, endpoint: &str) -> CommandBuilder {
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
            ("PANGRAM_DETECT_ENDPOINT", OsStr::new(endpoint)),
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

fn ai_result() -> String {
    serde_json::json!({
        "classification": "human",
        "headline": "Human",
        "prediction": "Human",
        "fraction_ai": 0.0,
        "fraction_ai_assisted": 0.0,
        "fraction_human": 1.0,
        "num_ai_segments": 0,
        "num_ai_assisted_segments": 0,
        "num_human_segments": 1,
        "segments": []
    })
    .to_string()
}

fn plagiarism_result() -> String {
    serde_json::json!({
        "plagiarism_detected": false,
        "total_sentences": 1,
        "plagiarized_sentence_count": 0,
        "percent_plagiarized": 0.0,
        "matches": []
    })
    .to_string()
}

fn seed_analysis(
    store: &mut HistoryStore,
    id: &str,
    created_at: &str,
    text: &str,
    display_name: &str,
    check_kind: CheckKind,
) -> AnalysisId {
    let id = AnalysisId::from_str(id).expect("canonical seed ID");
    let timestamp = UtcTimestamp::from_str(created_at).expect("canonical seed timestamp");
    let input = AnalysisInput::Text(
        TextInput::new(
            TextOrigin::File,
            Some(display_name.to_owned()),
            Sha256Hash::digest(text),
            u64::try_from(text.len()).expect("seed byte count"),
            u64::try_from(text.split_whitespace().count()).expect("seed word count"),
            Some(text.to_owned()),
        )
        .expect("canonical retained text"),
    );
    let result_json = match check_kind {
        CheckKind::AiDetection => ai_result(),
        CheckKind::Plagiarism => plagiarism_result(),
    };
    let record = StoredAnalysis {
        id,
        bulk: None,
        caller_id: None,
        status: AnalysisStatus::Succeeded,
        submission_outcome: SubmissionOutcome::Terminal,
        save_state: SaveState::SavedManual,
        input_kind: InputKind::Text,
        input_sha256: Sha256Hash::digest(text),
        display_name: Some(display_name.to_owned()),
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
        search_input_text: Some(text.to_owned()),
        search_filename: Some(display_name.to_owned()),
        search_headline: (check_kind == CheckKind::AiDetection).then(|| "Human".to_owned()),
        search_source_urls: None,
    };
    let check = StoredCheck {
        analysis_id: id,
        check_index: 0,
        check_kind,
        status: CheckStatus::Succeeded,
        result_json: Some(result_json),
        error_json: None,
    };
    store
        .save_analysis_complete(&record, &[check], &[])
        .expect("save certified terminal seed");
    id
}

fn seed_history(data: &Path) -> AnalysisId {
    let mut store = HistoryStore::open(data).expect("open real history store");
    // The newest record deliberately does not match the search. If Enter did
    // not apply the literal query, the journey would select and rerun this
    // record, making the exact request assertion fail.
    seed_analysis(
        &mut store,
        NONMATCH_ID,
        "2026-08-10T12:00:00Z",
        "ordinary private words that must not be selected",
        "newest nonmatching record",
        CheckKind::AiDetection,
    );
    let target = seed_analysis(
        &mut store,
        TARGET_ID,
        "2026-08-10T11:00:00Z",
        RERUN_TEXT,
        "searched AI target",
        CheckKind::AiDetection,
    );
    // This second literal search match disappears only when the closed check
    // filter moves from all to AI detection.
    seed_analysis(
        &mut store,
        PLAGIARISM_ID,
        "2026-08-10T10:00:00Z",
        "needle plagiarism record exercises the closed check filter",
        "searched plagiarism record",
        CheckKind::Plagiarism,
    );
    target
}

fn saved_rerun(data: &Path, original: AnalysisId) -> Option<Analysis<CanonicalError>> {
    let store = HistoryStore::open_existing(data).ok()??;
    store
        .list(10, 0)
        .ok()?
        .into_iter()
        .filter_map(|hit| store.canonical_analysis(&hit.analysis_id, true).ok())
        .find(|analysis| analysis.rerun_of() == Some(original))
}

#[tokio::test(flavor = "multi_thread")]
async fn history_search_filter_detail_cancel_delete_and_rerun_cross_real_boundaries() {
    let isolated = Isolated::new();
    let original_id = seed_history(&isolated.data);
    let fixture = ProtocolFixture::start().await;
    fixture.on_submit(Step::Json(serde_json::json!({"task_id": TASK_ID})));
    fixture.on_poll(Step::Json(pangram4_success(RERUN_TEXT)));

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
        .spawn_command(isolated.command(fixture.base_url()))
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
        assert_screen_text(&output_rx, &mut transcript, "Text composer");

        // Analyze Composer -> Routes, then Analyze -> History.
        let mut keys = b"\x1b[Z".repeat(6);
        keys.extend_from_slice(&b"\x1b[C".repeat(2));
        write_keys(&mut writer, &keys, "navigate to local History");
        assert_screen_text(&output_rx, &mut transcript, "Local Pangram CLI history");
        assert_screen_text(&output_rx, &mut transcript, "Showing 3");

        // Routes -> Search. Enter applies the literal query. The three status
        // activations reach succeeded through the closed cycle, and the check
        // activation narrows all -> AI detection. Pending loads may coalesce,
        // but the last certified page must contain only the labelled target.
        write_keys(&mut writer, b"\tneedle\r", "apply literal History search");
        write_keys(
            &mut writer,
            b"\t\r\r\r\t\r",
            "apply succeeded and AI detection filters",
        );
        assert!(
            receive_until(&output_rx, &mut transcript, SCREEN_TIMEOUT, |bytes| {
                screen_contains(bytes, "searched AI target")
                    && screen_contains(bytes, "Check filter: AI detection")
                    && screen_contains(bytes, "Status filter: succeeded")
            }),
            "the final literal search and closed filters did not select the AI target:\n{}",
            String::from_utf8_lossy(&transcript)
        );

        write_keys(&mut writer, b"\t\r", "load selected redacted detail");
        assert_screen_text(
            &output_rx,
            &mut transcript,
            "Selected detail - retained input redacted",
        );
        assert_screen_text(
            &output_rx,
            &mut transcript,
            "Retained input content: redacted",
        );
        assert!(
            !transcript_contains_rendered_text(&transcript, RERUN_TEXT),
            "retained plaintext must never enter rendered terminal cells"
        );

        // List -> Rerun -> Export -> Delete. Enter opens the destructive
        // overlay with Cancel selected; the second Enter must not mutate.
        write_keys(&mut writer, b"\t\t\t\r", "open selected-record deletion");
        assert_screen_text(&output_rx, &mut transcript, "Delete local history record");
        assert_screen_text(&output_rx, &mut transcript, "> [Enter] Cancel <");
        write_keys(&mut writer, b"\r", "accept the default cancel action");
        assert!(
            HistoryStore::open_existing(&isolated.data)
                .expect("reopen history after cancel")
                .expect("seeded history still exists")
                .canonical_analysis(&original_id, true)
                .is_ok(),
            "default-cancel deletion must preserve the selected row"
        );

        // Delete -> Export -> Rerun. This one Enter is the explicit billable
        // request. The runtime must rebuild the retained input privately and
        // finish through the shared Analyzer before automatic history saves.
        write_keys(
            &mut writer,
            b"\x1b[Z\x1b[Z\r",
            "activate the selected billable rerun",
        );
        assert!(
            receive_until(&output_rx, &mut transcript, ANALYSIS_TIMEOUT, |_| {
                saved_rerun(&isolated.data, original_id).is_some()
            },),
            "the rerun did not finish and commit automatic history:\n{}",
            String::from_utf8_lossy(&transcript)
        );

        // A successful rerun moves to Active with ActiveList focused. One Tab
        // reaches the documented Quit action without a hidden shortcut.
        write_keys(&mut writer, b"\t\r", "activate the focusable Quit action");
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
        !transcript_contains_rendered_text(&transcript, RERUN_TEXT),
        "retained plaintext stays out of the complete terminal transcript"
    );
    assert!(
        !contains_bytes(&transcript, SYNTHETIC_KEY.as_bytes()),
        "the API key stays secret"
    );

    assert_eq!(fixture.post_count(), 1, "the rerun submits exactly once");
    assert_eq!(fixture.get_count(), 1, "the rerun polls exactly once");
    let requests = fixture.requests();
    assert_eq!(requests.len(), 2, "one POST and one GET reach the fixture");
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/task");
    assert!(requests[0].header_equals("x-api-key", SYNTHETIC_KEY));
    let body = requests[0].body_json();
    assert_eq!(body["text"], RERUN_TEXT);
    assert_eq!(body["model"], "pangram-4");
    assert_eq!(body["public_dashboard_link"], false);
    assert_eq!(requests[1].method, "GET");
    assert_eq!(requests[1].path, format!("/task/{TASK_ID}"));

    let rerun = saved_rerun(&isolated.data, original_id)
        .expect("automatic history contains the completed rerun");
    assert_ne!(
        rerun.id, original_id,
        "a rerun receives fresh local identity"
    );
    assert_eq!(rerun.rerun_of(), Some(original_id));
    assert_eq!(rerun.save_state, SaveState::SavedHistory);
    assert_eq!(
        rerun.input().and_then(|input| match input {
            AnalysisInput::Text(text) => text.text.as_deref(),
            AnalysisInput::File(_) => None,
        }),
        Some(RERUN_TEXT),
        "automatic history retains the exact rerun input"
    );

    fixture.shutdown().await;
}
