use super::*;

use std::str::FromStr as _;

use crate::domain::{
    AnalysisInputKind, AnalysisStatus, AnalysisSummary, CheckKind, OrderedChecks, SaveState,
    UtcTimestamp,
};
use crate::history::HistoryExportFormat;
use crate::tui::history::{ExportRequest, PendingOperation};

fn ready_state() -> AppState {
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
        },
    )
}

fn history_state(items: Vec<AnalysisSummary>) -> AppState {
    let state = ready_state();
    let request = state.history.load_request();
    let transition = reduce(
        state,
        AppEvent::HistoryLoaded {
            request,
            result: Ok(items),
        },
    );
    assert!(transition.effects.is_empty());
    transition.state
}

fn history_id(index: u8) -> AnalysisId {
    format!("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a{index:02x}")
        .parse()
        .expect("canonical fixture ID")
}

fn history_summary(index: u8) -> AnalysisSummary {
    AnalysisSummary {
        id: history_id(index),
        status: AnalysisStatus::Succeeded,
        checks: OrderedChecks::new([CheckKind::AiDetection]).expect("one check"),
        save_state: SaveState::SavedHistory,
        input_kind: AnalysisInputKind::Text,
        display_name: Some(format!("record-{index}")),
        created_at: UtcTimestamp::from_str("2026-08-01T10:00:00Z").expect("canonical timestamp"),
    }
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
fn unavailable_controls_cannot_submit_work() {
    let mut state = ready_state();
    state.focus = Focus::CheckPlagiarism;
    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(transition.effects.is_empty());

    let mut state = transition.state;
    state.focus = Focus::InputFiles;
    let transition = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(transition.effects.is_empty());
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
            result: Ok(vec![history_summary(1)]),
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
            result: Ok(vec![history_summary(2)]),
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
            result: Ok(Vec::new()),
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
fn rerun_gate_stays_closed_until_the_analysis_finishes() {
    let mut state = history_state(vec![history_summary(1)]);
    state.route = Route::History;
    state.focus = Focus::HistoryRerun;
    let analysis_id = history_id(1);

    let requested = reduce(state, AppEvent::Key(KeyInput::Enter));
    assert!(matches!(
        requested.effects.as_slice(),
        [Effect::PrepareHistoryRerun {
            analysis_id: requested_id,
            automatic_save: false,
        }] if *requested_id == analysis_id
    ));
    assert_eq!(
        requested.state.history.pending(),
        Some(&PendingOperation::Rerun(analysis_id))
    );
    assert!(requested.state.analysis.submitting);

    let wrong = reduce(
        requested.state,
        AppEvent::HistoryRerunPrepared {
            analysis_id: history_id(2),
            result: Ok(()),
        },
    );
    assert_eq!(
        wrong.state.history.pending(),
        Some(&PendingOperation::Rerun(analysis_id))
    );
    let completed = reduce(
        wrong.state,
        AppEvent::HistoryRerunPrepared {
            analysis_id,
            result: Ok(()),
        },
    );
    assert_eq!(completed.state.route, Route::Active);
    assert_eq!(completed.state.focus, Focus::ActiveList);
    assert_eq!(completed.state.notice.as_deref(), Some("Rerun started."));
    assert!(completed.state.analysis.submitting);
    assert_eq!(
        completed.state.history.pending(),
        Some(&PendingOperation::Rerun(analysis_id))
    );

    let mut returned = completed.state;
    returned.route = Route::History;
    returned.focus = Focus::HistoryRerun;
    let duplicate = reduce(returned, AppEvent::Key(KeyInput::Enter));
    assert!(duplicate.effects.is_empty());
    assert_eq!(
        duplicate.state.history.pending(),
        Some(&PendingOperation::Rerun(analysis_id))
    );

    let failed = reduce(
        duplicate.state,
        AppEvent::AnalysisFailed(AnalysisFailure {
            analysis_id: AnalysisId::new(),
            error: CanonicalError::new(crate::output::ErrorCode::NetworkUnavailable, "offline")
                .expect("valid canonical error"),
        }),
    );
    assert!(!failed.state.analysis.submitting);
    assert!(failed.state.history.pending().is_none());
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
            result: Ok(vec![history_summary(1)]),
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
            result: Ok(vec![history_summary(1)]),
        },
    );
    assert_eq!(
        loaded.state.notice.as_deref(),
        Some("History deletion committed, but cleanup failed")
    );
}
