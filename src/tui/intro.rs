//! Intro policy, timing, and one-time state persistence.
//!
//! The artwork and generated frame tables are intentionally absent until the
//! logo-use gate is satisfied. This module owns only the control plane that is
//! independent of that artwork: launch eligibility, policy resolution, frame
//! timing, skip-key consumption, and the `intro_seen` marker.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossterm::event::KeyCode;
use serde::Deserialize;

use super::model::{IntroFrequency, MIN_HEIGHT, MIN_WIDTH, MotionLevel, TerminalSize};

pub(crate) const STATE_FILE_NAME: &str = "tui-state.json";
const FRAME_DURATION_MILLIS: u64 = 50;
const INTRO_DURATION_MILLIS: u64 = 2_800;
const _: () = assert!(INTRO_DURATION_MILLIS.is_multiple_of(FRAME_DURATION_MILLIS));
pub(crate) const FRAME_DURATION: Duration = Duration::from_millis(FRAME_DURATION_MILLIS);
pub(crate) const INTRO_DURATION: Duration = Duration::from_millis(INTRO_DURATION_MILLIS);
pub(crate) const FRAME_COUNT: usize = (INTRO_DURATION_MILLIS / FRAME_DURATION_MILLIS) as usize;

const STATE_SCHEMA_VERSION: &str = "1";
const SEEN_STATE_BYTES: &[u8] = b"{\n  \"schema_version\": \"1\",\n  \"intro_seen\": true\n}\n";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// The exact marker location below the resolved Pangram data directory.
pub(crate) fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STATE_FILE_NAME)
}

/// A safe diagnostic for an intro-state failure.
///
/// These values deliberately discard paths, operating-system error strings,
/// and file contents. The TUI can show them without leaking local details and
/// can continue opening Analyze because intro state is never a startup gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntroDiagnostic {
    Read,
    Invalid,
    Write,
}

impl IntroDiagnostic {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Read => "could not read local TUI intro state; treating the intro as unseen",
            Self::Invalid => "local TUI intro state is invalid; treating the intro as unseen",
            Self::Write => "could not record local TUI intro state; Analyze remains available",
        }
    }
}

impl std::fmt::Display for IntroDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

/// The usable part of the marker read plus an optional non-blocking warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LoadedIntroState {
    seen: bool,
    diagnostic: Option<IntroDiagnostic>,
}

impl LoadedIntroState {
    pub(crate) const fn seen(self) -> bool {
        self.seen
    }

    pub(crate) const fn diagnostic(self) -> Option<IntroDiagnostic> {
        self.diagnostic
    }

    const fn unseen(diagnostic: Option<IntroDiagnostic>) -> Self {
        Self {
            seen: false,
            diagnostic,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateWire {
    schema_version: String,
    intro_seen: bool,
}

/// Reads the marker without making intro state a startup dependency.
///
/// Only `NotFound` is cleanly unseen. Other read errors produce a sanitized
/// warning, while malformed or out-of-schema JSON produces the distinct
/// invalid-state warning. Both failures still resolve to unseen.
pub(crate) fn load_state(data_dir: &Path) -> LoadedIntroState {
    let bytes = match fs::read(state_path(data_dir)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LoadedIntroState::unseen(None);
        }
        Err(_) => {
            return LoadedIntroState::unseen(Some(IntroDiagnostic::Read));
        }
    };

    match serde_json::from_slice::<StateWire>(&bytes) {
        Ok(state) if state.schema_version == STATE_SCHEMA_VERSION && state.intro_seen => {
            LoadedIntroState {
                seen: true,
                diagnostic: None,
            }
        }
        Ok(_) | Err(_) => LoadedIntroState::unseen(Some(IntroDiagnostic::Invalid)),
    }
}

/// Atomically records the canonical schema-major-1 seen marker.
///
/// Content is written to a unique sibling, synced, and renamed into place.
/// Any failed stage removes its temporary file. Directory sync is best effort
/// on Unix, matching the repository's other atomic local-state writers.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn mark_seen(data_dir: &Path) -> Result<(), IntroDiagnostic> {
    let path = state_path(data_dir);
    let temporary = temporary_path(data_dir);

    let write_result = (|| -> std::io::Result<()> {
        fs::create_dir_all(data_dir)?;
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(SEEN_STATE_BYTES)?;
            file.sync_all()?;
        }
        publish(&temporary, &path)?;
        #[cfg(unix)]
        if let Ok(directory) = fs::File::open(data_dir) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(IntroDiagnostic::Write);
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish(temporary: &Path, path: &Path) -> std::io::Result<()> {
    fs::rename(temporary, path)
}

#[cfg(windows)]
fn publish(temporary: &Path, path: &Path) -> std::io::Result<()> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let temporary: Vec<u16> = temporary
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    // Unlike std::fs::rename on Windows, MoveFileExW can atomically replace a
    // stale or invalid marker. WRITE_THROUGH also waits for the move to reach
    // storage after the staged file itself has been synced.
    let moved = unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            path.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn temporary_path(data_dir: &Path) -> PathBuf {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    data_dir.join(format!(
        ".{STATE_FILE_NAME}.{}-{elapsed}-{sequence}.tmp",
        std::process::id()
    ))
}

/// Terminal and process properties resolved once for this launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LaunchCapabilities {
    all_streams_tty: bool,
    ci: bool,
    term_is_dumb: bool,
    terminal_size: TerminalSize,
}

impl LaunchCapabilities {
    pub(crate) fn new(
        stdin_tty: bool,
        stdout_tty: bool,
        stderr_tty: bool,
        ci: bool,
        term: Option<&str>,
        terminal_size: TerminalSize,
    ) -> Self {
        Self {
            all_streams_tty: stdin_tty && stdout_tty && stderr_tty,
            ci,
            term_is_dumb: term == Some("dumb"),
            terminal_size,
        }
    }

    pub(crate) const fn eligible(self) -> bool {
        self.all_streams_tty
            && !self.ci
            && !self.term_is_dumb
            && self.terminal_size.columns >= MIN_WIDTH
            && self.terminal_size.rows >= MIN_HEIGHT
    }
}

/// The artwork-independent work, if any, for this launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntroPlan {
    Suppressed,
    FullMotion,
    ReducedMark,
}

/// Resolves frequency and presentation without consuming one-time state.
pub(crate) const fn plan_intro(
    frequency: IntroFrequency,
    motion: MotionLevel,
    seen: bool,
    capabilities: LaunchCapabilities,
) -> IntroPlan {
    if !capabilities.eligible()
        || matches!(frequency, IntroFrequency::Off)
        || matches!(motion, MotionLevel::Off)
        || (matches!(frequency, IntroFrequency::Once) && seen)
    {
        return IntroPlan::Suppressed;
    }

    match motion {
        MotionLevel::Full => IntroPlan::FullMotion,
        MotionLevel::Reduced => IntroPlan::ReducedMark,
        MotionLevel::Off => IntroPlan::Suppressed,
    }
}

/// A point where an offered intro has actually resolved for the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum IntroResolution {
    Completed,
    Skipped,
    ReducedRendered,
}

/// Whether resolving this plan consumes the `once` marker.
///
/// Keeping the plan and resolution together prevents an ineligible or
/// below-minimum launch from consuming state merely because startup ran.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const fn should_mark_seen(
    frequency: IntroFrequency,
    plan: IntroPlan,
    resolution: IntroResolution,
) -> bool {
    if !matches!(frequency, IntroFrequency::Once) {
        return false;
    }
    matches!(
        (plan, resolution),
        (
            IntroPlan::FullMotion,
            IntroResolution::Completed | IntroResolution::Skipped
        ) | (IntroPlan::ReducedMark, IntroResolution::ReducedRendered)
    )
}

/// The frame due for a monotonic elapsed time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum FrameSelection {
    Frame(usize),
    Complete,
}

/// Selects directly from elapsed time, skipping every stale intermediate.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn select_frame(elapsed: Duration) -> FrameSelection {
    if elapsed >= INTRO_DURATION {
        return FrameSelection::Complete;
    }
    let index = elapsed.as_millis() / FRAME_DURATION.as_millis();
    debug_assert!(index < FRAME_COUNT as u128);
    FrameSelection::Frame(index as usize)
}

/// Whether an intro key becomes a consumed skip event or remains routable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum IntroKeyDisposition {
    ConsumeAndSkip,
    RouteNormally,
}

#[cfg_attr(not(test), allow(dead_code))]
impl IntroKeyDisposition {
    pub(crate) const fn consumed(self) -> bool {
        matches!(self, Self::ConsumeAndSkip)
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const fn classify_key(code: KeyCode) -> IntroKeyDisposition {
    match code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') => IntroKeyDisposition::ConsumeAndSkip,
        _ => IntroKeyDisposition::RouteNormally,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eligible_capabilities(size: TerminalSize) -> LaunchCapabilities {
        LaunchCapabilities::new(true, true, true, false, Some("xterm-256color"), size)
    }

    fn minimum_size() -> TerminalSize {
        TerminalSize {
            columns: MIN_WIDTH,
            rows: MIN_HEIGHT,
        }
    }

    #[test]
    fn marker_path_and_bytes_match_the_closed_schema() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(state_path(root.path()), root.path().join("tui-state.json"));

        mark_seen(root.path()).unwrap();
        assert_eq!(fs::read(state_path(root.path())).unwrap(), SEEN_STATE_BYTES);
        assert!(load_state(root.path()).seen());

        for invalid in [
            br#"{"schema_version":"2","intro_seen":true}"#.as_slice(),
            br#"{"schema_version":"1","intro_seen":false}"#.as_slice(),
            br#"{"schema_version":"1","intro_seen":true,"extra":1}"#.as_slice(),
        ] {
            fs::write(state_path(root.path()), invalid).unwrap();
            let loaded = load_state(root.path());
            assert!(!loaded.seen());
            assert_eq!(loaded.diagnostic(), Some(IntroDiagnostic::Invalid));
        }
    }

    #[test]
    fn missing_invalid_and_unreadable_state_are_nonblocking() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            load_state(root.path()),
            LoadedIntroState::unseen(None),
            "a missing marker is normal unseen state"
        );

        fs::write(state_path(root.path()), b"not json").unwrap();
        assert_eq!(
            load_state(root.path()),
            LoadedIntroState::unseen(Some(IntroDiagnostic::Invalid))
        );

        fs::remove_file(state_path(root.path())).unwrap();
        fs::create_dir(state_path(root.path())).unwrap();
        let unreadable = load_state(root.path());
        assert_eq!(
            unreadable,
            LoadedIntroState::unseen(Some(IntroDiagnostic::Read))
        );
        assert!(
            !unreadable
                .diagnostic()
                .unwrap()
                .message()
                .contains(root.path().to_string_lossy().as_ref())
        );
    }

    #[test]
    fn failed_publish_cleans_the_sibling_temporary_file() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(state_path(root.path())).unwrap();

        assert_eq!(mark_seen(root.path()), Err(IntroDiagnostic::Write));
        let entries: Vec<_> = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], STATE_FILE_NAME);
    }

    #[test]
    fn frequency_motion_and_capabilities_resolve_exactly() {
        let eligible = eligible_capabilities(minimum_size());
        let cases = [
            (
                IntroFrequency::Off,
                MotionLevel::Full,
                false,
                IntroPlan::Suppressed,
            ),
            (
                IntroFrequency::Once,
                MotionLevel::Full,
                false,
                IntroPlan::FullMotion,
            ),
            (
                IntroFrequency::Once,
                MotionLevel::Full,
                true,
                IntroPlan::Suppressed,
            ),
            (
                IntroFrequency::Once,
                MotionLevel::Reduced,
                false,
                IntroPlan::ReducedMark,
            ),
            (
                IntroFrequency::Once,
                MotionLevel::Off,
                false,
                IntroPlan::Suppressed,
            ),
            (
                IntroFrequency::Always,
                MotionLevel::Full,
                true,
                IntroPlan::FullMotion,
            ),
            (
                IntroFrequency::Always,
                MotionLevel::Reduced,
                true,
                IntroPlan::ReducedMark,
            ),
            (
                IntroFrequency::Always,
                MotionLevel::Off,
                false,
                IntroPlan::Suppressed,
            ),
        ];
        for (frequency, motion, seen, expected) in cases {
            assert_eq!(plan_intro(frequency, motion, seen, eligible), expected);
        }

        for ineligible in [
            LaunchCapabilities::new(false, true, true, false, Some("xterm"), minimum_size()),
            LaunchCapabilities::new(true, false, true, false, Some("xterm"), minimum_size()),
            LaunchCapabilities::new(true, true, false, false, Some("xterm"), minimum_size()),
            LaunchCapabilities::new(true, true, true, true, Some("xterm"), minimum_size()),
            LaunchCapabilities::new(true, true, true, false, Some("dumb"), minimum_size()),
        ] {
            assert_eq!(
                plan_intro(IntroFrequency::Always, MotionLevel::Full, false, ineligible),
                IntroPlan::Suppressed
            );
        }
    }

    #[test]
    fn below_minimum_launch_does_not_consume_once_state() {
        for size in [
            TerminalSize {
                columns: MIN_WIDTH - 1,
                rows: MIN_HEIGHT,
            },
            TerminalSize {
                columns: MIN_WIDTH,
                rows: MIN_HEIGHT - 1,
            },
        ] {
            let plan = plan_intro(
                IntroFrequency::Once,
                MotionLevel::Full,
                false,
                eligible_capabilities(size),
            );
            assert_eq!(plan, IntroPlan::Suppressed);
            assert!(!should_mark_seen(
                IntroFrequency::Once,
                plan,
                IntroResolution::Completed
            ));
        }
    }

    #[test]
    fn once_is_recorded_only_after_the_matching_resolution() {
        assert!(should_mark_seen(
            IntroFrequency::Once,
            IntroPlan::FullMotion,
            IntroResolution::Completed
        ));
        assert!(should_mark_seen(
            IntroFrequency::Once,
            IntroPlan::FullMotion,
            IntroResolution::Skipped
        ));
        assert!(should_mark_seen(
            IntroFrequency::Once,
            IntroPlan::ReducedMark,
            IntroResolution::ReducedRendered
        ));
        assert!(!should_mark_seen(
            IntroFrequency::Once,
            IntroPlan::ReducedMark,
            IntroResolution::Completed
        ));
        assert!(!should_mark_seen(
            IntroFrequency::Always,
            IntroPlan::FullMotion,
            IntroResolution::Completed
        ));
    }

    #[test]
    fn elapsed_time_selects_boundaries_and_skips_stale_frames() {
        assert_eq!(
            INTRO_DURATION,
            FRAME_DURATION * u32::try_from(FRAME_COUNT).expect("frame count fits u32"),
            "the frame count must stay derived from the sequence timing"
        );
        assert_eq!(select_frame(Duration::ZERO), FrameSelection::Frame(0));
        assert_eq!(
            select_frame(Duration::from_millis(49)),
            FrameSelection::Frame(0)
        );
        assert_eq!(
            select_frame(Duration::from_millis(50)),
            FrameSelection::Frame(1)
        );
        assert_eq!(
            select_frame(Duration::from_millis(175)),
            FrameSelection::Frame(3)
        );
        assert_eq!(
            select_frame(INTRO_DURATION - Duration::from_millis(1)),
            FrameSelection::Frame(FRAME_COUNT - 1)
        );
        assert_eq!(select_frame(INTRO_DURATION), FrameSelection::Complete);
        assert_eq!(
            select_frame(Duration::from_secs(60)),
            FrameSelection::Complete
        );
    }

    #[test]
    fn only_escape_enter_and_space_are_consumed_skip_keys() {
        for key in [KeyCode::Esc, KeyCode::Enter, KeyCode::Char(' ')] {
            assert_eq!(classify_key(key), IntroKeyDisposition::ConsumeAndSkip);
            assert!(classify_key(key).consumed());
        }
        for key in [KeyCode::Char('q'), KeyCode::Tab, KeyCode::Backspace] {
            assert_eq!(classify_key(key), IntroKeyDisposition::RouteNormally);
            assert!(!classify_key(key).consumed());
        }
    }
}
