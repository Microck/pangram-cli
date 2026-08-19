use super::*;

use std::str::FromStr as _;

use crate::analysis::TextAnalysisMode;
use crate::domain::{
    AnalysisInputKind, AnalysisStatus, AnalysisSummary, CheckKind, OrderedChecks, SaveState,
    UtcTimestamp,
};
use crate::history::HistoryExportFormat;
use crate::tui::history::{ExportRequest, HistoryLoadResult, PendingOperation};

pub(super) fn ready_state() -> AppState {
    AppState::new(
        TerminalSize {
            columns: WIDE_WIDTH,
            rows: 40,
        },
        StartupState {
            settings: SettingsDraft {
                credential_present: true,
                update_preference: Some(false),
                ..SettingsDraft::default()
            },
            keymap: Keymap::Regular,
            ..StartupState::default()
        },
    )
}

pub(super) fn history_state(items: Vec<AnalysisSummary>) -> AppState {
    let state = ready_state();
    let request = state.history.load_request();
    let transition = reduce(
        state,
        AppEvent::HistoryLoaded {
            request,
            result: Ok(history_load(items)),
        },
    );
    assert!(transition.effects.is_empty());
    transition.state
}

fn history_load(page: Vec<AnalysisSummary>) -> HistoryLoadResult {
    let unfinished = page
        .iter()
        .filter(|summary| {
            matches!(
                summary.status,
                AnalysisStatus::Queued | AnalysisStatus::Running
            )
        })
        .cloned()
        .collect();
    HistoryLoadResult { page, unfinished }
}

pub(super) fn history_id(index: u8) -> AnalysisId {
    format!("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a{index:02x}")
        .parse()
        .expect("canonical fixture ID")
}

pub(super) fn history_summary(index: u8) -> AnalysisSummary {
    history_summary_with_status(index, AnalysisStatus::Succeeded)
}

fn history_summary_with_status(index: u8, status: AnalysisStatus) -> AnalysisSummary {
    AnalysisSummary {
        id: history_id(index),
        status,
        checks: OrderedChecks::new([CheckKind::AiDetection]).expect("one check"),
        save_state: SaveState::SavedHistory,
        input_kind: AnalysisInputKind::Text,
        display_name: Some(format!("record-{index}")),
        created_at: UtcTimestamp::from_str("2026-08-01T10:00:00Z").expect("canonical timestamp"),
    }
}

#[test]
fn saved_active_rows_survive_page_omission_and_exact_terminal_evidence_removes_one() {
    let state = ready_state();
    let initial_request = state.history.load_request();
    let loaded = reduce(
        state,
        AppEvent::HistoryLoaded {
            request: initial_request,
            result: Ok(history_load(vec![
                history_summary_with_status(1, AnalysisStatus::Queued),
                history_summary_with_status(2, AnalysisStatus::Running),
            ])),
        },
    );
    assert_eq!(
        loaded.state.active.status(history_id(1)),
        Some(AnalysisStatus::Queued)
    );
    assert_eq!(
        loaded.state.active.status(history_id(2)),
        Some(AnalysisStatus::Running)
    );

    let changed = reduce(loaded.state, AppEvent::HistoryChanged);
    let [Effect::LoadHistory(filtered_request)] = changed.effects.as_slice() else {
        panic!("history change must request a reload")
    };
    let omitted = reduce(
        changed.state,
        AppEvent::HistoryLoaded {
            request: filtered_request.clone(),
            result: Ok(HistoryLoadResult {
                page: vec![history_summary_with_status(2, AnalysisStatus::Running)],
                unfinished: vec![
                    history_summary_with_status(1, AnalysisStatus::Queued),
                    history_summary_with_status(2, AnalysisStatus::Running),
                ],
            }),
        },
    );
    assert_eq!(
        omitted.state.active.status(history_id(1)),
        Some(AnalysisStatus::Queued)
    );

    let changed = reduce(omitted.state, AppEvent::HistoryChanged);
    let [Effect::LoadHistory(terminal_request)] = changed.effects.as_slice() else {
        panic!("history change must request a reload")
    };
    let terminal = reduce(
        changed.state,
        AppEvent::HistoryLoaded {
            request: terminal_request.clone(),
            result: Ok(HistoryLoadResult {
                page: vec![history_summary(1)],
                unfinished: vec![history_summary_with_status(2, AnalysisStatus::Running)],
            }),
        },
    );
    assert_eq!(terminal.state.active.status(history_id(1)), None);
    assert_eq!(
        terminal.state.active.status(history_id(2)),
        Some(AnalysisStatus::Running)
    );
}

#[test]
fn complete_unfinished_projection_reconciles_rows_outside_the_visible_page() {
    let state = ready_state();
    let initial_request = state.history.load_request();
    let loaded = reduce(
        state,
        AppEvent::HistoryLoaded {
            request: initial_request,
            result: Ok(HistoryLoadResult {
                page: vec![history_summary(3)],
                unfinished: vec![
                    history_summary_with_status(1, AnalysisStatus::Queued),
                    history_summary_with_status(2, AnalysisStatus::Running),
                ],
            }),
        },
    );
    assert_eq!(loaded.state.history.showing_count(), 1);
    assert_eq!(
        loaded.state.active.status(history_id(1)),
        Some(AnalysisStatus::Queued)
    );
    assert_eq!(
        loaded.state.active.status(history_id(2)),
        Some(AnalysisStatus::Running)
    );

    let changed = reduce(loaded.state, AppEvent::HistoryChanged);
    let [Effect::LoadHistory(request)] = changed.effects.as_slice() else {
        panic!("history change must request a reload")
    };
    let reconciled = reduce(
        changed.state,
        AppEvent::HistoryLoaded {
            request: request.clone(),
            result: Ok(HistoryLoadResult {
                page: vec![history_summary(4)],
                unfinished: vec![history_summary_with_status(2, AnalysisStatus::Running)],
            }),
        },
    );

    assert_eq!(reconciled.state.active.status(history_id(1)), None);
    assert_eq!(
        reconciled.state.active.status(history_id(2)),
        Some(AnalysisStatus::Running)
    );
    assert_eq!(reconciled.state.history.selected_id(), Some(history_id(4)));
}

#[test]
fn successful_history_delete_removes_only_its_exact_active_identity() {
    let mut state = history_state(vec![
        history_summary_with_status(1, AnalysisStatus::Queued),
        history_summary_with_status(2, AnalysisStatus::Running),
    ]);
    state.route = Route::History;
    state.focus = Focus::HistoryDelete;

    let opened = reduce(state, AppEvent::Key(KeyInput::Enter));
    let requested = reduce(opened.state, AppEvent::Key(KeyInput::Character('y')));
    assert!(matches!(
        requested.effects.as_slice(),
        [Effect::DeleteHistory(id)] if *id == history_id(1)
    ));
    let deleted = reduce(
        requested.state,
        AppEvent::HistoryDeleted {
            analysis_id: history_id(1),
            result: Ok(()),
        },
    );

    assert_eq!(deleted.state.active.status(history_id(1)), None);
    assert_eq!(
        deleted.state.active.status(history_id(2)),
        Some(AnalysisStatus::Running)
    );
}

#[test]
fn saved_unfinished_rows_do_not_block_a_new_in_session_submission() {
    let mut state = history_state(vec![history_summary_with_status(
        1,
        AnalysisStatus::Running,
    )]);
    state.route = Route::Analyze;
    state.focus = Focus::Submit;
    state.composer = TextField::from_value("new local submission".to_owned());

    let submitted = reduce(state, AppEvent::Key(KeyInput::Enter));

    assert!(matches!(
        submitted.effects.as_slice(),
        [Effect::SubmitText { .. }]
    ));
    assert!(submitted.state.analysis.submitting);
    assert_eq!(
        submitted.state.active.status(history_id(1)),
        Some(AnalysisStatus::Running)
    );
}

pub(super) fn assert_pending_rerun(
    state: &AppState,
    original_id: AnalysisId,
    analysis_id: Option<AnalysisId>,
) {
    assert!(matches!(
        state.history.pending(),
        Some(PendingOperation::Rerun {
            original_id: pending_original,
            analysis_id: pending_analysis,
        }) if *pending_original == original_id && *pending_analysis == analysis_id
    ));
}

#[test]
fn vim_printable_navigation_keys_edit_the_composer() {
    let mut state = ready_state();
    state.keymap = Keymap::Vim;
    for character in ['h', 'j', 'k', 'l'] {
        state = reduce(state, AppEvent::Key(KeyInput::Character(character))).state;
    }
    assert_eq!(state.composer.value(), "hjkl");
    assert_eq!(state.focus, Focus::Composer);
}

#[test]
fn composer_navigation_keys_edit_at_the_cursor() {
    let mut state = ready_state();
    state.composer = TextField::from_value("alpha".to_owned());

    for key in [
        KeyInput::Home,
        KeyInput::Right,
        KeyInput::Delete,
        KeyInput::End,
        KeyInput::Left,
        KeyInput::Backspace,
        KeyInput::Character('X'),
    ] {
        state = reduce(state, AppEvent::Key(key)).state;
    }

    assert_eq!(state.composer.value(), "apXa");
    assert_eq!(state.focus, Focus::Composer);
}

#[test]
fn credential_entry_uses_the_same_cursor_editing_behavior() {
    let mut state = AppState::new(TerminalSize::default(), StartupState::default());
    for key in [
        KeyInput::Character('a'),
        KeyInput::Character('b'),
        KeyInput::Home,
        KeyInput::Right,
        KeyInput::Delete,
        KeyInput::Character('c'),
        KeyInput::Enter,
    ] {
        let transition = reduce(state, AppEvent::Key(key));
        if let [Effect::StoreCredential { credential }] = transition.effects.as_slice() {
            assert_eq!(credential.as_str(), "ac");
            return;
        }
        state = transition.state;
    }

    panic!("credential entry did not request persistence");
}

#[test]
fn phase_seven_check_controls_select_text_work_without_submitting() {
    let mut state = ready_state();
    state.public_link = true;
    state.focus = Focus::CheckPlagiarism;
    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(transition.effects.is_empty());
    assert_eq!(transition.state.text_mode, TextAnalysisMode::Plagiarism);
    assert!(!transition.state.public_link);
    assert!(!transition.state.public_link_available());
    assert_eq!(transition.state.billing_estimate().1, 5);

    let mut state = transition.state;
    state.focus = Focus::PublicLink;
    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(!transition.state.public_link);

    let mut state = transition.state;
    state.focus = Focus::Composer;
    let transition = reduce(state, AppEvent::Key(KeyInput::Tab));
    assert_eq!(transition.state.focus, Focus::ManualSave);

    let mut state = transition.state;
    state.focus = Focus::CheckBoth;
    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(transition.effects.is_empty());
    assert_eq!(transition.state.text_mode, TextAnalysisMode::Combined);
    assert!(transition.state.public_link_available());
    assert_eq!(transition.state.billing_estimate().1, 6);

    let mut state = transition.state;
    state.focus = Focus::CheckAi;
    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(transition.effects.is_empty());
    assert_eq!(transition.state.text_mode, TextAnalysisMode::Detection);
    assert_eq!(transition.state.billing_estimate().1, 1);

    let mut state = transition.state;
    state.focus = Focus::InputFiles;
    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(transition.effects.is_empty());
}

#[test]
fn selected_text_mode_is_carried_by_the_single_submit_effect() {
    let mut state = ready_state();
    state.text_mode = TextAnalysisMode::Combined;
    state.composer = TextField::from_value("one billable request".to_owned());
    state.focus = Focus::Submit;

    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));

    assert!(matches!(
        transition.effects.as_slice(),
        [Effect::SubmitText {
            mode: TextAnalysisMode::Combined,
            ..
        }]
    ));
}

#[test]
fn below_minimum_resize_preserves_route_and_composer() {
    let mut state = ready_state();
    state = reduce(state, AppEvent::Key(KeyInput::Character('x'))).state;
    state.route = Route::History;
    state.focus = first_focus(Route::History);
    let transition = reduce(
        state,
        AppEvent::Resize(TerminalSize {
            columns: 79,
            rows: 23,
        }),
    );
    assert_eq!(transition.state.layout(), ResponsiveLayout::ResizeRequired);
    assert_eq!(transition.state.route, Route::History);
    assert_eq!(transition.state.composer.value(), "x");
}

#[test]
fn ctrl_c_requests_interrupt_exit() {
    let transition = reduce(ready_state(), AppEvent::Key(KeyInput::CtrlC));
    assert!(matches!(transition.effects.as_slice(), [Effect::Exit(130)]));
}

#[test]
fn in_place_reducer_updates_state_without_replacing_it() {
    let mut state = ready_state();

    let effects = reduce_in_place(&mut state, AppEvent::Key(KeyInput::Character('x')));

    assert!(effects.is_empty());
    assert_eq!(state.composer.value(), "x");
}

#[test]
fn focusable_quit_requests_normal_exit() {
    let mut state = ready_state();
    state.focus = Focus::Quit;
    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(matches!(transition.effects.as_slice(), [Effect::Exit(0)]));
}

#[test]
fn onboarding_orders_credential_before_update_preference() {
    let state = AppState::new(TerminalSize::default(), StartupState::default());
    assert!(matches!(state.overlay, Some(Overlay::Credential(_))));

    let transition = reduce(state, AppEvent::Key(KeyInput::Escape));
    assert!(matches!(
        transition.state.overlay,
        Some(Overlay::UpdatePreference { choice: true })
    ));

    let transition = reduce(transition.state, AppEvent::Key(KeyInput::Character('n')));
    assert!(matches!(
        transition.effects.as_slice(),
        [Effect::StoreUpdatePreference(false)]
    ));
    let transition = reduce(
        transition.state,
        AppEvent::SettingStored {
            setting: StoredSetting::UpdatePreference(false),
            result: Ok(()),
        },
    );
    assert!(transition.state.overlay.is_none());
}

#[test]
fn update_preference_escape_returns_to_credential_setup_only_when_missing() {
    let state = AppState::new(TerminalSize::default(), StartupState::default());
    let update = reduce(state, AppEvent::Key(KeyInput::Escape));
    assert!(matches!(
        update.state.overlay,
        Some(Overlay::UpdatePreference { .. })
    ));

    let credential = reduce(update.state, AppEvent::Key(KeyInput::Escape));
    assert!(matches!(
        credential.state.overlay,
        Some(Overlay::Credential(_))
    ));
    assert!(credential.effects.is_empty());

    let configured = AppState::new(
        TerminalSize::default(),
        StartupState {
            settings: SettingsDraft {
                credential_present: true,
                update_preference: None,
                ..SettingsDraft::default()
            },
            keymap: Keymap::Regular,
            ..StartupState::default()
        },
    );
    assert!(matches!(
        configured.overlay,
        Some(Overlay::UpdatePreference { .. })
    ));

    let stayed = reduce(configured, AppEvent::Key(KeyInput::Escape));
    assert!(matches!(
        stayed.state.overlay,
        Some(Overlay::UpdatePreference { .. })
    ));
    assert!(stayed.effects.is_empty());
}

#[test]
fn empty_submit_is_local_validation_only() {
    let mut state = ready_state();
    state.focus = Focus::Submit;
    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(transition.effects.is_empty());
    assert_eq!(
        transition.state.notice.as_deref(),
        Some("Enter text before submitting.")
    );
    assert!(!transition.state.analysis.submitting);
}

#[test]
fn repeated_submit_cannot_duplicate_a_billable_request() {
    let mut state = ready_state();
    state.composer = TextField::from_value("one billable request".to_owned());
    state.focus = Focus::Submit;

    let first = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(matches!(
        first.effects.as_slice(),
        [Effect::SubmitText {
            automatic_save: false,
            ..
        }]
    ));
    let repeated = reduce(first.state, AppEvent::Key(KeyInput::Enter));
    assert!(repeated.effects.is_empty());
    assert_eq!(
        repeated.state.notice.as_deref(),
        Some("An analysis is already in progress.")
    );
}

#[test]
fn keymap_changes_only_after_persistence_succeeds() {
    let mut state = ready_state();
    state.route = Route::Settings;
    state.focus = Focus::SettingsKeymap;

    let requested = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert_eq!(requested.state.keymap, Keymap::Regular);
    assert!(matches!(
        requested.effects.as_slice(),
        [Effect::StoreKeymap(Keymap::Vim)]
    ));

    let stored = reduce(
        requested.state,
        AppEvent::SettingStored {
            setting: StoredSetting::Keymap(Keymap::Vim),
            result: Ok(()),
        },
    );
    assert_eq!(stored.state.keymap, Keymap::Vim);
}

#[test]
fn repeated_setting_activation_is_coalesced_until_persistence_finishes() {
    let mut state = ready_state();
    state.route = Route::Settings;
    state.focus = Focus::SettingsKeymap;

    let requested = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(matches!(
        requested.effects.as_slice(),
        [Effect::StoreKeymap(Keymap::Vim)]
    ));

    let repeated = reduce(requested.state, AppEvent::Key(KeyInput::Enter));
    assert!(repeated.effects.is_empty());
    assert_eq!(repeated.state.keymap, Keymap::Regular);

    let stored = reduce(
        repeated.state,
        AppEvent::SettingStored {
            setting: StoredSetting::Keymap(Keymap::Vim),
            result: Ok(()),
        },
    );
    assert_eq!(stored.state.keymap, Keymap::Vim);

    let next = reduce(stored.state, AppEvent::Key(KeyInput::Enter));
    assert!(matches!(
        next.effects.as_slice(),
        [Effect::StoreKeymap(Keymap::Regular)]
    ));
}

#[test]
fn enabling_history_requires_confirmation_and_a_committed_write() {
    let mut state = ready_state();
    state.route = Route::Settings;
    state.focus = Focus::SettingsHistory;

    let warned = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(warned.effects.is_empty());
    assert!(matches!(
        warned.state.overlay,
        Some(Overlay::HistoryConsent)
    ));
    assert!(!warned.state.settings.history_enabled);

    let requested = reduce(warned.state, AppEvent::Key(KeyInput::Character('y')));
    assert!(matches!(
        requested.effects.as_slice(),
        [Effect::StoreHistory(true)]
    ));
    assert!(!requested.state.settings.history_enabled);

    let stored = reduce(
        requested.state,
        AppEvent::SettingStored {
            setting: StoredSetting::History(true),
            result: Ok(()),
        },
    );
    assert!(stored.state.settings.history_enabled);
    assert!(stored.state.overlay.is_none());
}

#[test]
fn submit_becomes_new_analysis_after_a_failure() {
    let mut state = ready_state();
    state.composer = TextField::from_value("old input".to_owned());
    state.focus = Focus::Submit;
    state.analysis.failure = Some(AnalysisFailure {
        analysis_id: AnalysisId::new(),
        error: CanonicalError::new(crate::output::ErrorCode::NetworkUnavailable, "offline")
            .expect("valid canonical error"),
    });
    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(transition.effects.is_empty());
    assert!(transition.state.composer.value().is_empty());
    assert!(transition.state.analysis.failure.is_none());
}

#[test]
fn history_filter_changes_coalesce_behind_the_exact_pending_reload() {
    let mut state = ready_state();
    state.route = Route::History;
    state.focus = Focus::HistoryStatusFilter;
    let first_request = state.history.load_request();

    let status_changed = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(status_changed.effects.is_empty());
    assert_eq!(
        status_changed.state.history.load_request().status,
        Some(AnalysisStatus::Queued)
    );

    let mut state = status_changed.state;
    state.focus = Focus::HistoryCheckFilter;
    let check_changed = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(check_changed.effects.is_empty());

    let superseded = reduce(
        check_changed.state,
        AppEvent::HistoryLoaded {
            request: first_request,
            result: Ok(history_load(vec![history_summary(1)])),
        },
    );
    assert_eq!(superseded.state.history.showing_count(), 0);
    let latest_request = match superseded.effects.as_slice() {
        [Effect::LoadHistory(request)] => request.clone(),
        _ => panic!("expected one coalesced reload"),
    };
    assert_eq!(latest_request.status, Some(AnalysisStatus::Queued));
    assert_eq!(latest_request.check, Some(CheckKind::AiDetection));

    let changed_again = reduce(superseded.state, AppEvent::HistoryChanged);
    assert!(changed_again.effects.is_empty());
    let superseded_again = reduce(
        changed_again.state,
        AppEvent::HistoryLoaded {
            request: latest_request,
            result: Ok(history_load(vec![history_summary(2)])),
        },
    );
    assert_eq!(superseded_again.state.history.showing_count(), 0);
    assert!(matches!(
        superseded_again.effects.as_slice(),
        [Effect::LoadHistory(_)]
    ));
}

#[test]
fn a_mismatched_history_completion_cannot_release_the_operation_gate() {
    let state = ready_state();
    let pending = state.history.pending().cloned().expect("startup reload");
    let mut wrong_request = state.history.load_request();
    wrong_request.query = Some("different".to_owned());

    let transition = reduce(
        state,
        AppEvent::HistoryLoaded {
            request: wrong_request,
            result: Ok(history_load(Vec::new())),
        },
    );
    assert!(transition.effects.is_empty());
    assert_eq!(transition.state.history.pending(), Some(&pending));
}

#[test]
fn vim_search_characters_are_literal_and_bare_d_never_deletes() {
    let mut state = history_state(vec![history_summary(1)]);
    state.route = Route::History;
    state.focus = Focus::HistorySearch;
    state.keymap = Keymap::Vim;
    for character in ['h', 'j', 'k', 'l', 'd'] {
        state = reduce(state, AppEvent::Key(KeyInput::Character(character))).state;
    }
    assert_eq!(state.history.draft_query(), "hjkld");

    let searched = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(matches!(
        searched.effects.as_slice(),
        [Effect::LoadHistory(request)] if request.query.as_deref() == Some("hjkld")
    ));

    let mut state = history_state(vec![history_summary(1)]);
    state.route = Route::History;
    state.focus = Focus::HistoryDelete;
    let untouched = reduce(state, AppEvent::Key(KeyInput::Character('d')));
    assert!(untouched.effects.is_empty());
    assert!(untouched.state.overlay.is_none());
    assert_eq!(untouched.state.history.showing_count(), 1);
}

#[test]
fn history_search_navigation_keys_edit_at_the_cursor() {
    let mut state = history_state(vec![history_summary(1)]);
    state.route = Route::History;
    state.focus = Focus::HistorySearch;
    for key in [
        KeyInput::Character('f'),
        KeyInput::Character('o'),
        KeyInput::Character('x'),
        KeyInput::Home,
        KeyInput::Right,
        KeyInput::Delete,
        KeyInput::Character('i'),
        KeyInput::End,
        KeyInput::Left,
        KeyInput::Backspace,
        KeyInput::Character('i'),
    ] {
        state = reduce(state, AppEvent::Key(key)).state;
    }
    assert_eq!(state.history.draft_query(), "fix");
}

#[test]
fn delete_and_export_confirmations_default_to_cancel() {
    let mut state = history_state(vec![history_summary(1)]);
    state.route = Route::History;
    state.focus = Focus::HistoryDelete;
    let opened = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(matches!(
        opened.state.overlay,
        Some(Overlay::ConfirmHistoryDelete { confirm: false, .. })
    ));
    let cancelled = reduce(opened.state, AppEvent::Key(KeyInput::Enter));
    assert!(cancelled.effects.is_empty());
    assert!(cancelled.state.overlay.is_none());

    let mut state = cancelled.state;
    state.focus = Focus::HistoryExport;
    let opened = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(matches!(
        opened.state.overlay,
        Some(Overlay::HistoryExport {
            field: HistoryExportField::Action
        })
    ));
    let cancelled = reduce(opened.state, AppEvent::Key(KeyInput::Enter));
    assert!(cancelled.effects.is_empty());
    assert!(cancelled.state.overlay.is_none());
}

#[test]
fn full_history_export_requires_a_second_cancel_default_confirmation() {
    let mut state = history_state(vec![history_summary(1)]);
    state.route = Route::History;
    state.focus = Focus::HistoryExport;
    state = reduce(state, AppEvent::Key(KeyInput::Enter)).state;
    state = reduce(state, AppEvent::Key(KeyInput::Up)).state;
    state = reduce(state, AppEvent::Key(KeyInput::Right)).state;
    state = reduce(state, AppEvent::Key(KeyInput::Down)).state;
    state = reduce(state, AppEvent::Key(KeyInput::Right)).state;

    let warned = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(matches!(
        warned.state.overlay,
        Some(Overlay::ConfirmFullHistoryExport {
            request: ExportRequest {
                format: HistoryExportFormat::Jsonl,
                redact_content: false,
            },
            confirm: false,
        })
    ));
    let cancelled = reduce(warned.state, AppEvent::Key(KeyInput::Enter));
    assert!(cancelled.effects.is_empty());
    assert!(cancelled.state.overlay.is_none());
}

#[test]
fn history_reload_cannot_consume_a_confirmed_delete_or_export() {
    let mut state = history_state(vec![history_summary(1)]);
    state.route = Route::History;
    state.focus = Focus::HistoryDelete;
    let opened = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(matches!(
        opened.state.overlay,
        Some(Overlay::ConfirmHistoryDelete { confirm: false, .. })
    ));

    let changed = reduce(opened.state, AppEvent::HistoryChanged);
    let [Effect::LoadHistory(request)] = changed.effects.as_slice() else {
        panic!("history change must start one reload")
    };
    let confirmed_while_busy = reduce(changed.state, AppEvent::Key(KeyInput::Character('y')));
    assert!(confirmed_while_busy.effects.is_empty());
    assert!(matches!(
        confirmed_while_busy.state.overlay,
        Some(Overlay::ConfirmHistoryDelete { .. })
    ));

    let reloaded = reduce(
        confirmed_while_busy.state,
        AppEvent::HistoryLoaded {
            request: request.clone(),
            result: Ok(history_load(vec![history_summary(1)])),
        },
    );
    let confirmed = reduce(reloaded.state, AppEvent::Key(KeyInput::Character('y')));
    assert!(matches!(
        confirmed.effects.as_slice(),
        [Effect::DeleteHistory(analysis_id)] if *analysis_id == history_id(1)
    ));

    let mut state = history_state(vec![history_summary(1)]);
    state.route = Route::History;
    state.overlay = Some(Overlay::HistoryExport {
        field: HistoryExportField::Action,
    });
    state.history.export_choices_mut().toggle_action();
    let changed = reduce(state, AppEvent::HistoryChanged);
    let confirmed_while_busy = reduce(changed.state, AppEvent::Key(KeyInput::Enter));
    assert!(confirmed_while_busy.effects.is_empty());
    assert!(matches!(
        confirmed_while_busy.state.overlay,
        Some(Overlay::HistoryExport { .. })
    ));
}

#[test]
fn successful_history_reload_preserves_an_existing_notice() {
    let mut state = history_state(vec![history_summary(1)]);
    state.notice = Some("History deletion committed, but cleanup failed".to_owned());
    let changed = reduce(state, AppEvent::HistoryChanged);
    let [Effect::LoadHistory(request)] = changed.effects.as_slice() else {
        panic!("history change must start one reload")
    };
    let loaded = reduce(
        changed.state,
        AppEvent::HistoryLoaded {
            request: request.clone(),
            result: Ok(history_load(vec![history_summary(1)])),
        },
    );
    assert_eq!(
        loaded.state.notice.as_deref(),
        Some("History deletion committed, but cleanup failed")
    );
}

#[test]
fn pointer_actions_share_keyboard_selection_and_submission_gates() {
    let selected = reduce(
        ready_state(),
        AppEvent::Pointer(PointerIntent::Activate(Focus::CheckPlagiarism)),
    );
    assert_eq!(selected.state.text_mode, TextAnalysisMode::Plagiarism);
    assert!(selected.effects.is_empty());

    let empty_submit = reduce(
        selected.state,
        AppEvent::Pointer(PointerIntent::Activate(Focus::Submit)),
    );
    assert!(empty_submit.effects.is_empty());
    assert_eq!(
        empty_submit.state.notice.as_deref(),
        Some("Enter text before submitting.")
    );

    let mut pending = empty_submit.state;
    pending.composer = TextField::from_value("one billable request".to_owned());
    pending.analysis.submitting = true;
    let duplicate = reduce(
        pending,
        AppEvent::Pointer(PointerIntent::Activate(Focus::Submit)),
    );
    assert!(duplicate.effects.is_empty());
    assert_eq!(
        duplicate.state.notice.as_deref(),
        Some(ANALYSIS_IN_PROGRESS_NOTICE)
    );

    let mut export = ready_state();
    export.overlay = Some(Overlay::HistoryExport {
        field: HistoryExportField::Action,
    });
    let format = reduce(
        export,
        AppEvent::Pointer(PointerIntent::HistoryExportField(
            HistoryExportField::Format,
        )),
    );
    assert!(matches!(
        format.state.overlay,
        Some(Overlay::HistoryExport {
            field: HistoryExportField::Format
        })
    ));
    assert_eq!(
        format.state.history.export_choices().format(),
        HistoryExportFormat::Markdown
    );
}

#[test]
fn pointer_history_row_uses_the_same_detail_operation_gate() {
    let mut state = history_state(vec![history_summary(1), history_summary(2)]);
    state.route = Route::History;

    let selected = reduce(state, AppEvent::Pointer(PointerIntent::HistoryRow(1)));

    assert_eq!(selected.state.focus, Focus::HistoryList);
    assert_eq!(selected.state.history.selected_id(), Some(history_id(2)));
    assert!(matches!(
        selected.effects.as_slice(),
        [Effect::LoadHistoryDetail(analysis_id)] if *analysis_id == history_id(2)
    ));

    let duplicate = reduce(
        selected.state,
        AppEvent::Pointer(PointerIntent::HistoryRow(1)),
    );
    assert!(duplicate.effects.is_empty());
}

#[test]
fn empty_active_analyze_action_returns_to_the_composer_without_an_effect() {
    let mut state = ready_state();
    state.route = Route::Active;
    state.focus = Focus::ActiveList;

    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));

    assert_eq!(transition.state.route, Route::Analyze);
    assert_eq!(transition.state.focus, Focus::Composer);
    assert!(transition.effects.is_empty());
}
