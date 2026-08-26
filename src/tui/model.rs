//! Pure state and transition logic for the terminal adapter.
//!
//! It owns no I/O: runtimes feed [`AppEvent`] values into [`reduce_in_place`]
//! and execute the returned [`Effect`] values.

use std::fmt;

use zeroize::Zeroizing;

use crate::analysis::{AnalysisProgress, TextAnalysisMode};
use crate::domain::{Analysis, AnalysisId, text_billable_units};
use crate::output::CanonicalError;

use super::active::ActiveState;
pub(crate) use super::history::HistoryExportField;
use super::history::{
    ExportRequest, HistoryLoadRequest, HistoryLoadResult, HistoryState, RedactedAnalysis,
};
use super::result_viewport::{ResultMove, ResultViewport};
pub use super::text_field::TextField;
use super::text_field::edit_value;

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;
pub const WIDE_WIDTH: u16 = 120;
pub(super) const ANALYSIS_IN_PROGRESS_NOTICE: &str = "An analysis is already in progress.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Route {
    #[default]
    Analyze,
    Active,
    History,
    Settings,
}

impl Route {
    pub const ALL: [Self; 4] = [Self::Analyze, Self::Active, Self::History, Self::Settings];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Analyze => "Analyze",
            Self::Active => "Active",
            Self::History => "History",
            Self::Settings => "Settings",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Keymap {
    #[default]
    Regular,
    Vim,
}

impl Keymap {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Regular => "Regular",
            Self::Vim => "Vim",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IntroFrequency {
    #[default]
    Once,
    Always,
    Off,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionLevel {
    #[default]
    Full,
    Reduced,
    Off,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsDraft {
    pub credential_present: bool,
    pub history_enabled: bool,
    pub intro: IntroFrequency,
    pub motion: MotionLevel,
    pub update_preference: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub columns: u16,
    pub rows: u16,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            columns: WIDE_WIDTH,
            rows: 40,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponsiveLayout {
    ResizeRequired,
    Narrow,
    Wide,
}

/// Color capability resolved once at the terminal boundary.
///
/// Keeping this in state makes rendering deterministic and lets `NO_COLOR`
/// use the same projection as truecolor and ANSI terminals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorMode {
    None,
    Ansi,
    #[default]
    TrueColor,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    Routes,
    CheckAi,
    CheckPlagiarism,
    CheckBoth,
    InputText,
    InputFiles,
    #[default]
    Composer,
    PublicLink,
    ManualSave,
    Submit,
    Result,
    ActiveList,
    HistorySearch,
    HistoryStatusFilter,
    HistoryCheckFilter,
    HistoryList,
    HistoryRerun,
    HistoryExport,
    HistoryDelete,
    SettingsAuthentication,
    SettingsHistory,
    SettingsIntro,
    SettingsKeymap,
    SettingsMotion,
    SettingsUpdates,
    Quit,
}

#[derive(Clone, PartialEq, Eq)]
pub struct CredentialEntry {
    value: Zeroizing<String>,
    cursor: usize,
}

impl Default for CredentialEntry {
    fn default() -> Self {
        Self {
            value: Zeroizing::new(String::new()),
            cursor: 0,
        }
    }
}

impl fmt::Debug for CredentialEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialEntry")
            .field("value", &"[REDACTED]")
            .field("characters", &self.value.chars().count())
            .finish()
    }
}

impl CredentialEntry {
    #[cfg(test)]
    pub fn from_value(value: String) -> Self {
        let cursor = value.len();
        Self {
            value: Zeroizing::new(value),
            cursor,
        }
    }

    pub fn value(&self) -> &str {
        self.value.as_str()
    }

    fn take(&mut self) -> Zeroizing<String> {
        self.cursor = 0;
        std::mem::replace(&mut self.value, Zeroizing::new(String::new()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    Credential(CredentialEntry),
    UpdatePreference {
        choice: bool,
    },
    HistoryConsent,
    ConfirmHistoryDelete {
        analysis_id: AnalysisId,
        confirm: bool,
    },
    HistoryExport {
        field: HistoryExportField,
    },
    ConfirmFullHistoryExport {
        request: ExportRequest,
        confirm: bool,
    },
    Help,
}

#[derive(Clone)]
pub struct AnalysisFailure {
    pub analysis_id: AnalysisId,
    pub error: CanonicalError,
}

#[derive(Clone, Default)]
pub struct AnalysisView {
    pub submitting: bool,
    pub current: Option<Analysis<CanonicalError>>,
    pub progress: Option<AnalysisProgress>,
    pub failure: Option<AnalysisFailure>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartupState {
    pub settings: SettingsDraft,
    pub keymap: Keymap,
    pub color_mode: ColorMode,
}

#[derive(Clone)]
pub struct AppState {
    pub route: Route,
    pub focus: Focus,
    pub composer: TextField,
    pub text_mode: TextAnalysisMode,
    pub color_mode: ColorMode,
    pub public_link: bool,
    pub manual_save: bool,
    pub terminal: TerminalSize,
    pub overlay: Option<Overlay>,
    pub keymap: Keymap,
    pub notice: Option<String>,
    pub analysis: AnalysisView,
    pub(crate) result_viewport: ResultViewport,
    pub(crate) active: ActiveState,
    pub history: HistoryState,
    pub settings: SettingsDraft,
    pub(crate) setting_write_pending: bool,
    vim_prefix_g: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(TerminalSize::default(), StartupState::default())
    }
}

impl AppState {
    pub fn new(terminal: TerminalSize, startup: StartupState) -> Self {
        let overlay = if !startup.settings.credential_present {
            Some(Overlay::Credential(CredentialEntry::default()))
        } else if startup.settings.update_preference.is_none() {
            Some(Overlay::UpdatePreference { choice: true })
        } else {
            None
        };
        Self {
            route: Route::Analyze,
            focus: Focus::Composer,
            composer: TextField::default(),
            text_mode: TextAnalysisMode::Detection,
            color_mode: startup.color_mode,
            public_link: false,
            manual_save: false,
            terminal,
            overlay,
            keymap: startup.keymap,
            notice: None,
            analysis: AnalysisView::default(),
            result_viewport: ResultViewport::default(),
            active: ActiveState::default(),
            history: HistoryState::initial_loading(),
            settings: startup.settings,
            setting_write_pending: false,
            vim_prefix_g: false,
        }
    }

    pub const fn layout(&self) -> ResponsiveLayout {
        if self.terminal.columns < MIN_WIDTH || self.terminal.rows < MIN_HEIGHT {
            ResponsiveLayout::ResizeRequired
        } else if self.terminal.columns < WIDE_WIDTH {
            ResponsiveLayout::Narrow
        } else {
            ResponsiveLayout::Wide
        }
    }

    pub fn billing_estimate(&self) -> (u64, u64) {
        let word_count =
            u64::try_from(self.composer.value().split_whitespace().count()).unwrap_or(u64::MAX);
        let units = self
            .text_mode
            .billable_units(text_billable_units(word_count));
        (word_count, units)
    }

    pub const fn public_link_available(&self) -> bool {
        !matches!(self.text_mode, TextAnalysisMode::Plagiarism)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyInput {
    Character(char),
    Up,
    Down,
    Left,
    Right,
    Tab,
    BackTab,
    Enter,
    Escape,
    Home,
    End,
    PageUp,
    PageDown,
    Backspace,
    Delete,
    CtrlC,
    CtrlU,
    CtrlD,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerDirection {
    Previous,
    Next,
}

/// A terminal coordinate resolved into an application action at the adapter
/// boundary. Raw mouse positions never enter the reducer or persistent state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerIntent {
    Route(Route),
    Focus(Focus),
    Activate(Focus),
    ActiveRow(usize),
    HistoryRow(usize),
    HistoryExportField(HistoryExportField),
    Scroll {
        focus: Focus,
        direction: PointerDirection,
    },
    Key(KeyInput),
}

pub enum AppEvent {
    Key(KeyInput),
    Pointer(PointerIntent),
    Paste(String),
    Resize(TerminalSize),
    AnalysisAccepted(Analysis<CanonicalError>),
    AnalysisProgress(AnalysisProgress),
    AnalysisFinished(Analysis<CanonicalError>),
    AnalysisFailed(AnalysisFailure),
    HistoryChanged,
    HistoryLoaded {
        request: HistoryLoadRequest,
        result: Result<HistoryLoadResult, CanonicalError>,
    },
    HistoryDetailLoaded {
        analysis_id: AnalysisId,
        result: Result<RedactedAnalysis, CanonicalError>,
    },
    HistoryDeleted {
        analysis_id: AnalysisId,
        result: Result<(), CanonicalError>,
    },
    HistoryRerunPrepared {
        analysis_id: AnalysisId,
        result: Result<AnalysisId, CanonicalError>,
    },
    Notice(String),
    SettingStored {
        setting: StoredSetting,
        result: Result<(), CanonicalError>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredSetting {
    Credential,
    UpdatePreference(bool),
    History(bool),
    Intro(IntroFrequency),
    Keymap(Keymap),
    Motion(MotionLevel),
}

pub enum Effect {
    SubmitText {
        text: String,
        mode: TextAnalysisMode,
        public_link: bool,
        save: bool,
        automatic_save: bool,
    },
    StoreCredential {
        credential: Zeroizing<String>,
    },
    StoreUpdatePreference(bool),
    StoreHistory(bool),
    StoreIntro(IntroFrequency),
    StoreKeymap(Keymap),
    StoreMotion(MotionLevel),
    LoadHistory(HistoryLoadRequest),
    LoadHistoryDetail(AnalysisId),
    DeleteHistory(AnalysisId),
    PrepareHistoryRerun {
        analysis_id: AnalysisId,
        automatic_save: bool,
    },
    ExportHistory(ExportRequest),
    Exit(u8),
}

#[cfg(test)]
pub struct Transition {
    pub state: AppState,
    pub effects: Vec<Effect>,
}

#[path = "model-reducer.rs"]
mod reducer;

pub(crate) use reducer::reduce_in_place;

#[cfg(test)]
use reducer::first_focus;

#[cfg(test)]
pub fn reduce(mut state: AppState, event: AppEvent) -> Transition {
    let effects = reduce_in_place(&mut state, event);
    Transition { state, effects }
}

#[cfg(test)]
#[path = "model-paste-tests.rs"]
mod paste_tests;

#[cfg(test)]
#[path = "model-tests.rs"]
mod tests;

#[cfg(test)]
#[path = "model-rerun-tests.rs"]
mod rerun_tests;
