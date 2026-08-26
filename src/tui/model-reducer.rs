//! In-place transitions for the terminal application model.
//!
//! Keeping reducer mechanics separate from the state and event contract makes
//! the model's ownership boundary visible without splitting each route into a
//! trait or duplicating shared navigation rules.

use super::*;
use crate::analysis::TextAnalysisMode;

pub(crate) fn reduce_in_place(state: &mut AppState, event: AppEvent) -> Vec<Effect> {
    let mut effects = Vec::new();
    match event {
        AppEvent::Resize(size) => state.terminal = size,
        AppEvent::Paste(text) => {
            if state.layout() != ResponsiveLayout::ResizeRequired
                && state.route == Route::Analyze
                && state.focus == Focus::Composer
                && state.overlay.is_none()
            {
                state.composer.insert_text(&text);
            }
        }
        AppEvent::AnalysisAccepted(analysis) => {
            if !state.history.accepts_analysis_event(analysis.id) {
                return effects;
            }
            state.analysis.submitting = false;
            state.analysis.failure = None;
            state.active.accept(&analysis);
            state.analysis.current = Some(analysis);
        }
        AppEvent::AnalysisProgress(progress) => {
            if !state.history.accepts_analysis_event(progress.analysis_id)
                || !state.active.progress(progress.analysis_id)
            {
                return effects;
            }
            state.analysis.submitting = false;
            state.analysis.progress = Some(progress);
        }
        AppEvent::AnalysisFinished(analysis) => {
            if !state.history.accepts_analysis_event(analysis.id) {
                return effects;
            }
            let analysis_id = analysis.id;
            state.analysis.submitting = false;
            state.analysis.progress = None;
            state.analysis.failure = None;
            state.active.remove(analysis.id);
            state.analysis.current = Some(analysis);
            crate::tui::history_reducer::complete_rerun_analysis(state, analysis_id, &mut effects);
            if state.route == Route::Analyze {
                state.result_viewport.reset(analysis_id);
                state.focus = Focus::Result;
            }
        }
        AppEvent::AnalysisFailed(failure) => {
            if !state.history.accepts_analysis_event(failure.analysis_id) {
                return effects;
            }
            let analysis_id = failure.analysis_id;
            state.analysis.submitting = false;
            state.analysis.progress = None;
            state.active.remove(failure.analysis_id);
            state.analysis.failure = Some(failure);
            crate::tui::history_reducer::complete_rerun_analysis(state, analysis_id, &mut effects);
        }
        AppEvent::HistoryChanged => {
            crate::tui::history_reducer::history_changed(state, &mut effects)
        }
        AppEvent::HistoryLoaded { request, result } => {
            crate::tui::history_reducer::complete_load(state, request, result, &mut effects)
        }
        AppEvent::HistoryDetailLoaded {
            analysis_id,
            result,
        } => crate::tui::history_reducer::complete_detail(state, analysis_id, result, &mut effects),
        AppEvent::HistoryDeleted {
            analysis_id,
            result,
        } => crate::tui::history_reducer::complete_delete(state, analysis_id, result, &mut effects),
        AppEvent::HistoryRerunPrepared {
            analysis_id,
            result,
        } => crate::tui::history_reducer::complete_rerun(state, analysis_id, result, &mut effects),
        AppEvent::Notice(notice) => state.notice = Some(notice),
        AppEvent::SettingStored { setting, result } => {
            state.setting_write_pending = false;
            match result {
                Ok(()) => match setting {
                    StoredSetting::Credential => {
                        state.settings.credential_present = true;
                        state.notice = None;
                        advance_onboarding(state);
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
            }
        }
        AppEvent::Pointer(intent) => {
            if state.layout() != ResponsiveLayout::ResizeRequired {
                reduce_pointer(state, intent, &mut effects);
            }
        }
        AppEvent::Key(KeyInput::CtrlC) => effects.push(Effect::Exit(130)),
        AppEvent::Key(key) => {
            if state.layout() != ResponsiveLayout::ResizeRequired {
                reduce_key(state, key, &mut effects);
            }
        }
    }
    effects
}

fn reduce_pointer(state: &mut AppState, intent: PointerIntent, effects: &mut Vec<Effect>) {
    match intent {
        PointerIntent::Route(route) => {
            state.route = route;
            state.focus = Focus::Routes;
        }
        PointerIntent::Focus(focus) => state.focus = focus,
        PointerIntent::Activate(focus) => {
            state.focus = focus;
            reduce_key(state, KeyInput::Enter, effects);
        }
        PointerIntent::ActiveRow(index) => {
            state.route = Route::Active;
            state.focus = Focus::ActiveList;
            state.active.select_visible(index);
        }
        PointerIntent::HistoryRow(index) => {
            state.route = Route::History;
            state.focus = Focus::HistoryList;
            let selection_changed = state.history.select_visible(index);
            if selection_changed || state.history.selected_detail().is_none() {
                crate::tui::history_reducer::load_selected_detail(state, effects);
            }
        }
        PointerIntent::HistoryExportField(field) => {
            if let Some(Overlay::HistoryExport { field: active }) = state.overlay.as_mut() {
                *active = field;
                reduce_key(state, KeyInput::Enter, effects);
            }
        }
        PointerIntent::Scroll { focus, direction } => {
            state.focus = focus;
            reduce_key(
                state,
                match direction {
                    PointerDirection::Previous => KeyInput::Up,
                    PointerDirection::Next => KeyInput::Down,
                },
                effects,
            );
        }
        PointerIntent::Key(key) => reduce_key(state, key, effects),
    }
}

fn reduce_key(state: &mut AppState, key: KeyInput, effects: &mut Vec<Effect>) {
    if crate::tui::history_reducer::reduce_overlay(state, key, effects) {
        return;
    }
    if reduce_overlay(state, key, effects) {
        return;
    }
    if reduce_text_field(state, key) {
        return;
    }
    if crate::tui::history_reducer::reduce_key(state, key, effects) {
        return;
    }
    if crate::tui::result_viewport::reduce_key(state, key) {
        return;
    }
    if crate::tui::active::reduce_key(state, key) {
        return;
    }

    state.vim_prefix_g = match (state.keymap, state.vim_prefix_g, key) {
        (Keymap::Vim, true, KeyInput::Character('g')) => {
            if state.focus == Focus::Result {
                crate::tui::result_viewport::navigate(state, ResultMove::First);
            } else {
                state.focus = first_focus(state.route);
            }
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
            if state.focus == Focus::Result {
                crate::tui::result_viewport::navigate(state, ResultMove::Last);
            } else {
                state.focus = Focus::Quit;
            }
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
                if !state.setting_write_pending {
                    state.setting_write_pending = true;
                    effects.push(Effect::StoreCredential {
                        credential: entry.take(),
                    });
                }
            }
            _ => {
                edit_value(&mut entry.value, &mut entry.cursor, key);
            }
        },
        Overlay::UpdatePreference { choice } => match key {
            KeyInput::Escape if !state.settings.credential_present => {
                state.overlay = Some(Overlay::Credential(CredentialEntry::default()));
            }
            KeyInput::Character('y' | 'Y') => request_setting(
                &mut state.setting_write_pending,
                effects,
                Effect::StoreUpdatePreference(true),
            ),
            KeyInput::Character('n' | 'N') => request_setting(
                &mut state.setting_write_pending,
                effects,
                Effect::StoreUpdatePreference(false),
            ),
            KeyInput::Left | KeyInput::Right | KeyInput::Up | KeyInput::Down => *choice = !*choice,
            KeyInput::Enter => request_setting(
                &mut state.setting_write_pending,
                effects,
                Effect::StoreUpdatePreference(*choice),
            ),
            _ => {}
        },
        Overlay::HistoryConsent => match key {
            KeyInput::Character('y' | 'Y') | KeyInput::Enter => {
                request_setting(
                    &mut state.setting_write_pending,
                    effects,
                    Effect::StoreHistory(true),
                );
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
        state.composer.insert_text("\n");
        return true;
    }
    let field = match state.focus {
        Focus::Composer if state.route == Route::Analyze => Some(&mut state.composer),
        Focus::HistorySearch if state.route == Route::History => {
            return crate::tui::history_reducer::edit_search(state, key);
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
        Focus::CheckAi => state.text_mode = TextAnalysisMode::Detection,
        Focus::CheckPlagiarism => {
            state.text_mode = TextAnalysisMode::Plagiarism;
            state.public_link = false;
        }
        Focus::CheckBoth => state.text_mode = TextAnalysisMode::Combined,
        Focus::InputText | Focus::InputFiles => {}
        Focus::PublicLink if state.public_link_available() => {
            state.public_link = !state.public_link;
        }
        Focus::PublicLink => {}
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
                    mode: state.text_mode,
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
                request_setting(
                    &mut state.setting_write_pending,
                    effects,
                    Effect::StoreHistory(false),
                );
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
            request_setting(
                &mut state.setting_write_pending,
                effects,
                Effect::StoreIntro(intro),
            );
        }
        Focus::SettingsKeymap => {
            let keymap = match state.keymap {
                Keymap::Regular => Keymap::Vim,
                Keymap::Vim => Keymap::Regular,
            };
            request_setting(
                &mut state.setting_write_pending,
                effects,
                Effect::StoreKeymap(keymap),
            );
        }
        Focus::SettingsMotion => {
            let motion = match state.settings.motion {
                MotionLevel::Full => MotionLevel::Reduced,
                MotionLevel::Reduced => MotionLevel::Off,
                MotionLevel::Off => MotionLevel::Full,
            };
            request_setting(
                &mut state.setting_write_pending,
                effects,
                Effect::StoreMotion(motion),
            );
        }
        Focus::SettingsUpdates => request_setting(
            &mut state.setting_write_pending,
            effects,
            Effect::StoreUpdatePreference(!state.settings.update_preference.unwrap_or(false)),
        ),
        Focus::ActiveList if state.active.is_empty() => {
            state.route = Route::Analyze;
            state.focus = Focus::Composer;
        }
        Focus::Quit => effects.push(Effect::Exit(0)),
        Focus::Routes
        | Focus::Composer
        | Focus::Result
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

fn request_setting(pending: &mut bool, effects: &mut Vec<Effect>, effect: Effect) {
    if !*pending {
        *pending = true;
        effects.push(effect);
    }
}

pub(super) fn first_focus(route: Route) -> Focus {
    match route {
        Route::Analyze => Focus::Composer,
        Route::Active => Focus::ActiveList,
        Route::History => Focus::HistorySearch,
        Route::Settings => Focus::SettingsAuthentication,
    }
}

fn focus_order(state: &AppState) -> &'static [Focus] {
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
    const ANALYZE_PLAGIARISM: &[Focus] = &[
        Focus::Routes,
        Focus::CheckAi,
        Focus::CheckPlagiarism,
        Focus::CheckBoth,
        Focus::InputText,
        Focus::InputFiles,
        Focus::Composer,
        Focus::ManualSave,
        Focus::Submit,
        Focus::Quit,
    ];
    const ANALYZE_RESULT: &[Focus] = &[Focus::Routes, Focus::Result, Focus::Submit, Focus::Quit];
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
    match state.route {
        Route::Analyze if state.analysis.current.is_some() => ANALYZE_RESULT,
        Route::Analyze if !state.public_link_available() => ANALYZE_PLAGIARISM,
        Route::Analyze => ANALYZE,
        Route::Active => ACTIVE,
        Route::History => {
            crate::tui::history_reducer::focus_order(state.history.selected_detail().is_some())
        }
        Route::Settings => SETTINGS,
    }
}

fn move_focus(state: &mut AppState, offset: isize) {
    let order = focus_order(state);
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
