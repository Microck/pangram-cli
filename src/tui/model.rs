//! Pure state and transition logic for the terminal adapter.
//!
//! It owns no I/O: runtimes feed [`AppEvent`] values into [`reduce`] and
//! execute the returned [`Effect`] values.

use std::fmt;

use zeroize::Zeroizing;

use crate::analysis::AnalysisProgress;
use crate::domain::{Analysis, AnalysisId, AnalysisSummary, text_billable_units};
use crate::output::CanonicalError;

use super::active::ActiveState;
pub(crate) use super::history::HistoryExportField;
use super::history::{ExportRequest, HistoryLoadRequest, HistoryState, RedactedAnalysis};
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
}

#[derive(Clone)]
pub struct AppState {
    pub route: Route,
    pub focus: Focus,
    pub composer: TextField,
    pub public_link: bool,
    pub manual_save: bool,
    pub terminal: TerminalSize,
    pub overlay: Option<Overlay>,
    pub keymap: Keymap,
    pub notice: Option<String>,
    pub analysis: AnalysisView,
    pub(crate) active: ActiveState,
    pub history: HistoryState,
    pub settings: SettingsDraft,
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
            public_link: false,
            manual_save: false,
            terminal,
            overlay,
            keymap: startup.keymap,
            notice: None,
            analysis: AnalysisView::default(),
            active: ActiveState::default(),
            history: HistoryState::initial_loading(),
            settings: startup.settings,
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

    pub fn word_count(&self) -> u64 {
        u64::try_from(self.composer.value().split_whitespace().count()).unwrap_or(u64::MAX)
    }

    pub fn billable_units(&self) -> u64 {
        text_billable_units(self.word_count())
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

pub enum AppEvent {
    Key(KeyInput),
    Resize(TerminalSize),
    AnalysisAccepted(Analysis<CanonicalError>),
    AnalysisProgress(AnalysisProgress),
    AnalysisFinished(Analysis<CanonicalError>),
    AnalysisFailed(AnalysisFailure),
    HistoryChanged,
    HistoryLoaded {
        request: HistoryLoadRequest,
        result: Result<Vec<AnalysisSummary>, CanonicalError>,
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

pub struct Transition {
    pub state: AppState,
    pub effects: Vec<Effect>,
}

pub fn reduce(mut state: AppState, event: AppEvent) -> Transition {
    let mut effects = Vec::new();
    match event {
        AppEvent::Resize(size) => state.terminal = size,
        AppEvent::AnalysisAccepted(analysis) => {
            if !state.history.accepts_analysis_event(analysis.id) {
                return Transition { state, effects };
            }
            state.analysis.submitting = false;
            state.analysis.failure = None;
            state.analysis.current = Some(analysis.clone());
            state.active.accept(&analysis);
        }
        AppEvent::AnalysisProgress(progress) => {
            if !state.history.accepts_analysis_event(progress.analysis_id) {
                return Transition { state, effects };
            }
            if !state.active.progress(progress.analysis_id) {
                return Transition { state, effects };
            }
            state.analysis.submitting = false;
            state.analysis.progress = Some(progress);
        }
        AppEvent::AnalysisFinished(analysis) => {
            if !state.history.accepts_analysis_event(analysis.id) {
                return Transition { state, effects };
            }
            let analysis_id = analysis.id;
            state.analysis.submitting = false;
            state.analysis.progress = None;
            state.analysis.failure = None;
            state.active.remove(analysis.id);
            state.analysis.current = Some(analysis);
            super::history_reducer::complete_rerun_analysis(&mut state, analysis_id, &mut effects);
        }
        AppEvent::AnalysisFailed(failure) => {
            if !state.history.accepts_analysis_event(failure.analysis_id) {
                return Transition { state, effects };
            }
            let analysis_id = failure.analysis_id;
            state.analysis.submitting = false;
            state.analysis.progress = None;
            state.active.remove(failure.analysis_id);
            state.analysis.failure = Some(failure);
            super::history_reducer::complete_rerun_analysis(&mut state, analysis_id, &mut effects);
        }
        AppEvent::HistoryChanged => {
            super::history_reducer::history_changed(&mut state, &mut effects)
        }
        AppEvent::HistoryLoaded { request, result } => {
            super::history_reducer::complete_load(&mut state, request, result, &mut effects)
        }
        AppEvent::HistoryDetailLoaded {
            analysis_id,
            result,
        } => super::history_reducer::complete_detail(&mut state, analysis_id, result, &mut effects),
        AppEvent::HistoryDeleted {
            analysis_id,
            result,
        } => super::history_reducer::complete_delete(&mut state, analysis_id, result, &mut effects),
        AppEvent::HistoryRerunPrepared {
            analysis_id,
            result,
        } => super::history_reducer::complete_rerun(&mut state, analysis_id, result, &mut effects),
        AppEvent::Notice(notice) => state.notice = Some(notice),
        AppEvent::SettingStored { setting, result } => match result {
            Ok(()) => match setting {
                StoredSetting::Credential => {
                    state.settings.credential_present = true;
                    state.notice = None;
                    advance_onboarding(&mut state);
                }
                StoredSetting::UpdatePreference(choice) => {
                    state.settings.update_preference = Some(choice);
                    state.overlay = None;
                    state.notice = None;
                }
                StoredSetting::History(enabled) => {
                    state.settings.history_enabled = enabled;
                    state.overlay = None;
                    state.notice = None;
                }
                StoredSetting::Intro(intro) => {
                    state.settings.intro = intro;
                    state.notice = None;
                }
                StoredSetting::Keymap(keymap) => {
                    state.keymap = keymap;
                    state.notice = None;
                }
                StoredSetting::Motion(motion) => {
                    state.settings.motion = motion;
                    state.notice = None;
                }
            },
            Err(error) => state.notice = Some(error.message().to_owned()),
        },
        AppEvent::Key(KeyInput::CtrlC) => effects.push(Effect::Exit(130)),
        AppEvent::Key(key) => {
            if state.layout() != ResponsiveLayout::ResizeRequired {
                reduce_key(&mut state, key, &mut effects);
            }
        }
    }
    Transition { state, effects }
}

fn reduce_key(state: &mut AppState, key: KeyInput, effects: &mut Vec<Effect>) {
    if super::history_reducer::reduce_overlay(state, key, effects) {
        return;
    }
    if reduce_overlay(state, key, effects) {
        return;
    }
    if reduce_text_field(state, key) {
        return;
    }
    if super::history_reducer::reduce_key(state, key, effects) {
        return;
    }

    state.vim_prefix_g = match (state.keymap, state.vim_prefix_g, key) {
        (Keymap::Vim, true, KeyInput::Character('g')) => {
            state.focus = first_focus(state.route);
            false
        }
        (Keymap::Vim, false, KeyInput::Character('g')) => true,
        _ => false,
    };
    if state.vim_prefix_g {
        return;
    }

    match key {
        KeyInput::Character('?') => state.overlay = Some(Overlay::Help),
        KeyInput::Character('/') if state.route == Route::History => {
            state.focus = Focus::HistorySearch;
        }
        KeyInput::Character('h') if state.keymap == Keymap::Vim => move_route_or_focus(state, -1),
        KeyInput::Character('l') if state.keymap == Keymap::Vim => move_route_or_focus(state, 1),
        KeyInput::Character('k') if state.keymap == Keymap::Vim => move_focus(state, -1),
        KeyInput::Character('j') if state.keymap == Keymap::Vim => move_focus(state, 1),
        KeyInput::Character('G') if state.keymap == Keymap::Vim => {
            state.focus = Focus::Quit;
        }
        KeyInput::Character('n') | KeyInput::CtrlD if state.keymap == Keymap::Vim => {
            move_focus(state, 1);
        }
        KeyInput::Character('N') | KeyInput::CtrlU if state.keymap == Keymap::Vim => {
            move_focus(state, -1);
        }
        KeyInput::Left => move_route_or_focus(state, -1),
        KeyInput::Right => move_route_or_focus(state, 1),
        KeyInput::Up | KeyInput::BackTab => move_focus(state, -1),
        KeyInput::Down | KeyInput::Tab => move_focus(state, 1),
        KeyInput::Home => state.focus = first_focus(state.route),
        KeyInput::End => state.focus = Focus::Quit,
        KeyInput::PageUp => move_focus(state, -1),
        KeyInput::PageDown => move_focus(state, 1),
        KeyInput::Enter => activate_focus(state, effects),
        KeyInput::Escape => state.focus = Focus::Routes,
        KeyInput::Character(_)
        | KeyInput::Backspace
        | KeyInput::Delete
        | KeyInput::CtrlC
        | KeyInput::CtrlU
        | KeyInput::CtrlD => {}
    }
}

fn reduce_overlay(state: &mut AppState, key: KeyInput, effects: &mut Vec<Effect>) -> bool {
    let Some(overlay) = state.overlay.as_mut() else {
        return false;
    };
    match overlay {
        Overlay::Credential(entry) => match key {
            KeyInput::Escape => advance_onboarding(state),
            KeyInput::Enter if entry.value().trim().is_empty() => {
                state.notice = Some("Enter an API key or press Escape to skip.".to_owned());
            }
            KeyInput::Enter => {
                effects.push(Effect::StoreCredential {
                    credential: entry.take(),
                });
            }
            _ => {
                edit_value(&mut entry.value, &mut entry.cursor, key);
            }
        },
        Overlay::UpdatePreference { choice } => match key {
            KeyInput::Escape => {
                state.overlay = Some(Overlay::Credential(CredentialEntry::default()));
            }
            KeyInput::Character('y' | 'Y') => effects.push(Effect::StoreUpdatePreference(true)),
            KeyInput::Character('n' | 'N') => effects.push(Effect::StoreUpdatePreference(false)),
            KeyInput::Left | KeyInput::Right | KeyInput::Up | KeyInput::Down => *choice = !*choice,
            KeyInput::Enter => {
                let selected = *choice;
                effects.push(Effect::StoreUpdatePreference(selected));
            }
            _ => {}
        },
        Overlay::HistoryConsent => match key {
            KeyInput::Character('y' | 'Y') | KeyInput::Enter => {
                effects.push(Effect::StoreHistory(true));
            }
            KeyInput::Character('n' | 'N') | KeyInput::Escape => state.overlay = None,
            _ => {}
        },
        Overlay::Help => {
            if matches!(key, KeyInput::Escape | KeyInput::Character('?')) {
                state.overlay = None;
            }
        }
        Overlay::ConfirmHistoryDelete { .. }
        | Overlay::HistoryExport { .. }
        | Overlay::ConfirmFullHistoryExport { .. } => {}
    }
    true
}

fn advance_onboarding(state: &mut AppState) {
    state.overlay = if state.settings.update_preference.is_none() {
        Some(Overlay::UpdatePreference { choice: true })
    } else {
        None
    };
}

fn reduce_text_field(state: &mut AppState, key: KeyInput) -> bool {
    if state.focus == Focus::Composer && key == KeyInput::Enter {
        state.composer.insert('\n');
        return true;
    }
    let field = match state.focus {
        Focus::Composer if state.route == Route::Analyze => Some(&mut state.composer),
        Focus::HistorySearch if state.route == Route::History => {
            return super::history_reducer::edit_search(state, key);
        }
        _ => None,
    };
    let Some(field) = field else {
        return false;
    };
    field.edit(key)
}

fn activate_focus(state: &mut AppState, effects: &mut Vec<Effect>) {
    match state.focus {
        Focus::CheckAi
        | Focus::CheckPlagiarism
        | Focus::CheckBoth
        | Focus::InputText
        | Focus::InputFiles => {}
        Focus::PublicLink => state.public_link = !state.public_link,
        Focus::ManualSave => state.manual_save = !state.manual_save,
        Focus::Submit => {
            if state.analysis.submitting || state.active.has_session() {
                state.notice = Some(ANALYSIS_IN_PROGRESS_NOTICE.to_owned());
            } else if state.analysis.current.is_some()
                || state.analysis.progress.is_some()
                || state.analysis.failure.is_some()
            {
                state.composer = TextField::default();
                state.public_link = false;
                state.manual_save = false;
                state.analysis = AnalysisView::default();
                state.notice = None;
            } else if state.composer.value().trim().is_empty() {
                state.notice = Some("Enter text before submitting.".to_owned());
            } else {
                state.notice = None;
                state.analysis.submitting = true;
                effects.push(Effect::SubmitText {
                    text: state.composer.value().to_owned(),
                    public_link: state.public_link,
                    save: state.manual_save,
                    automatic_save: state.settings.history_enabled,
                });
            }
        }
        Focus::SettingsAuthentication => {
            state.overlay = Some(Overlay::Credential(CredentialEntry::default()));
        }
        Focus::SettingsHistory => {
            if state.settings.history_enabled {
                effects.push(Effect::StoreHistory(false));
            } else {
                state.overlay = Some(Overlay::HistoryConsent);
            }
        }
        Focus::SettingsIntro => {
            let intro = match state.settings.intro {
                IntroFrequency::Once => IntroFrequency::Always,
                IntroFrequency::Always => IntroFrequency::Off,
                IntroFrequency::Off => IntroFrequency::Once,
            };
            effects.push(Effect::StoreIntro(intro));
        }
        Focus::SettingsKeymap => {
            let keymap = match state.keymap {
                Keymap::Regular => Keymap::Vim,
                Keymap::Vim => Keymap::Regular,
            };
            effects.push(Effect::StoreKeymap(keymap));
        }
        Focus::SettingsMotion => {
            let motion = match state.settings.motion {
                MotionLevel::Full => MotionLevel::Reduced,
                MotionLevel::Reduced => MotionLevel::Off,
                MotionLevel::Off => MotionLevel::Full,
            };
            effects.push(Effect::StoreMotion(motion));
        }
        Focus::SettingsUpdates => effects.push(Effect::StoreUpdatePreference(
            !state.settings.update_preference.unwrap_or(false),
        )),
        Focus::Quit => effects.push(Effect::Exit(0)),
        Focus::Routes
        | Focus::Composer
        | Focus::ActiveList
        | Focus::HistorySearch
        | Focus::HistoryStatusFilter
        | Focus::HistoryCheckFilter
        | Focus::HistoryList
        | Focus::HistoryRerun
        | Focus::HistoryExport
        | Focus::HistoryDelete => {}
    }
}

fn first_focus(route: Route) -> Focus {
    match route {
        Route::Analyze => Focus::Composer,
        Route::Active => Focus::ActiveList,
        Route::History => Focus::HistorySearch,
        Route::Settings => Focus::SettingsAuthentication,
    }
}

fn focus_order(route: Route) -> &'static [Focus] {
    const ANALYZE: &[Focus] = &[
        Focus::Routes,
        Focus::CheckAi,
        Focus::CheckPlagiarism,
        Focus::CheckBoth,
        Focus::InputText,
        Focus::InputFiles,
        Focus::Composer,
        Focus::PublicLink,
        Focus::ManualSave,
        Focus::Submit,
        Focus::Quit,
    ];
    const ACTIVE: &[Focus] = &[Focus::Routes, Focus::ActiveList, Focus::Quit];
    const SETTINGS: &[Focus] = &[
        Focus::Routes,
        Focus::SettingsAuthentication,
        Focus::SettingsHistory,
        Focus::SettingsIntro,
        Focus::SettingsKeymap,
        Focus::SettingsMotion,
        Focus::SettingsUpdates,
        Focus::Quit,
    ];
    match route {
        Route::Analyze => ANALYZE,
        Route::Active => ACTIVE,
        Route::History => super::history_reducer::focus_order(),
        Route::Settings => SETTINGS,
    }
}

fn move_focus(state: &mut AppState, offset: isize) {
    let order = focus_order(state.route);
    let current = order
        .iter()
        .position(|focus| *focus == state.focus)
        .unwrap_or(0);
    let next = current.saturating_add_signed(offset).min(order.len() - 1);
    state.focus = order[next];
}

fn move_route_or_focus(state: &mut AppState, offset: isize) {
    if state.focus != Focus::Routes {
        move_focus(state, offset);
        return;
    }
    let current = Route::ALL
        .iter()
        .position(|route| *route == state.route)
        .unwrap_or(0);
    let next = current
        .saturating_add_signed(offset)
        .min(Route::ALL.len() - 1);
    state.route = Route::ALL[next];
}

#[cfg(test)]
#[path = "model-tests.rs"]
mod tests;
