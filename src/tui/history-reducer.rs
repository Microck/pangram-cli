//! Pure transitions for the History route.
//!
//! This module keeps every history action behind one pending-operation gate.
//! Completion events must carry the exact request or analysis ID that owns
//! that gate, so stale worker messages cannot release newer work.

use crate::domain::{AnalysisId, AnalysisSummary};
use crate::output::CanonicalError;

use super::history::{
    ExportResolution, HistoryExportField, HistoryLoadRequest, PendingOperation, RedactedAnalysis,
    SelectionMove,
};
use super::model::{AppState, Effect, Focus, KeyInput, Overlay, Route};

pub(super) const fn focus_order() -> &'static [Focus] {
    &[
        Focus::Routes,
        Focus::HistorySearch,
        Focus::HistoryStatusFilter,
        Focus::HistoryCheckFilter,
        Focus::HistoryList,
        Focus::HistoryRerun,
        Focus::HistoryExport,
        Focus::HistoryDelete,
        Focus::Quit,
    ]
}

pub(super) fn history_changed(state: &mut AppState, effects: &mut Vec<Effect>) {
    request_reload(state, effects);
}

pub(super) fn complete_load(
    state: &mut AppState,
    request: HistoryLoadRequest,
    result: Result<Vec<AnalysisSummary>, CanonicalError>,
    effects: &mut Vec<Effect>,
) {
    let operation = PendingOperation::Reload(request);
    if !state.history.finish_pending(&operation) {
        return;
    }
    if state.history.take_reload_dirty() {
        request_reload(state, effects);
        return;
    }
    match result {
        Ok(items) => {
            state.history.reload(items);
        }
        Err(error) => history_error(state, error),
    }
}

pub(super) fn complete_detail(
    state: &mut AppState,
    analysis_id: AnalysisId,
    result: Result<RedactedAnalysis, CanonicalError>,
    effects: &mut Vec<Effect>,
) {
    if !state
        .history
        .finish_pending(&PendingOperation::Detail(analysis_id))
    {
        return;
    }
    if reload_if_dirty(state, effects) {
        return;
    }
    match result {
        Ok(detail) if detail.analysis().id == analysis_id => {
            state.history.load_detail(detail);
            state.notice = None;
        }
        Ok(_) => {}
        Err(error) => history_error(state, error),
    }
}

pub(super) fn complete_delete(
    state: &mut AppState,
    analysis_id: AnalysisId,
    result: Result<(), CanonicalError>,
    effects: &mut Vec<Effect>,
) {
    if !state
        .history
        .finish_pending(&PendingOperation::Delete(analysis_id))
    {
        return;
    }
    match result {
        Ok(()) => {
            // The row remains visible until this committed completion. A
            // fresh certified page, rather than local removal, now decides
            // what is visible and preserves deterministic selection rules.
            state.history.take_reload_dirty();
            request_reload(state, effects);
        }
        Err(error) => {
            history_error(state, error);
            reload_if_dirty(state, effects);
        }
    }
}

pub(super) fn complete_rerun(
    state: &mut AppState,
    analysis_id: AnalysisId,
    result: Result<(), CanonicalError>,
    effects: &mut Vec<Effect>,
) {
    let operation = PendingOperation::Rerun(analysis_id);
    if state.history.pending() != Some(&operation) {
        return;
    }
    match result {
        Ok(()) => {
            // Request preparation has finished, but credentials, submission,
            // polling, and optional persistence still run in the same worker.
            // Keep both the History gate and the global analysis gate closed
            // until that worker emits its terminal analysis event.
            state.analysis.submitting = true;
            state.route = Route::Active;
            state.focus = Focus::ActiveList;
            state.notice = Some("Rerun started.".to_owned());
        }
        Err(error) => {
            state.history.finish_pending(&operation);
            state.analysis.submitting = false;
            history_error(state, error);
            reload_if_dirty(state, effects);
        }
    }
}

pub(super) fn complete_rerun_analysis(state: &mut AppState, effects: &mut Vec<Effect>) {
    let Some(PendingOperation::Rerun(analysis_id)) = state.history.pending().cloned() else {
        return;
    };
    state
        .history
        .finish_pending(&PendingOperation::Rerun(analysis_id));
    reload_if_dirty(state, effects);
}

pub(super) fn edit_search(state: &mut AppState, key: KeyInput) -> bool {
    let query = state.history.draft_query_mut();
    match key {
        // Printable Vim navigation characters are literal search input while
        // the search field owns focus.
        KeyInput::Character(character) if !character.is_control() => query.push(character),
        KeyInput::Backspace => {
            query.pop();
        }
        KeyInput::Delete | KeyInput::Left | KeyInput::Right | KeyInput::Home | KeyInput::End => {}
        KeyInput::Escape
        | KeyInput::Enter
        | KeyInput::Tab
        | KeyInput::BackTab
        | KeyInput::Up
        | KeyInput::Down => return false,
        _ => return true,
    }
    true
}

pub(super) fn reduce_key(state: &mut AppState, key: KeyInput, effects: &mut Vec<Effect>) -> bool {
    if state.route != Route::History {
        return false;
    }
    match (state.focus, key) {
        (Focus::HistorySearch, KeyInput::Enter) => {
            state.history.apply_query();
            request_reload(state, effects);
        }
        (Focus::HistoryStatusFilter, KeyInput::Enter) => {
            state.history.cycle_status_filter();
            request_reload(state, effects);
        }
        (Focus::HistoryCheckFilter, KeyInput::Enter) => {
            state.history.cycle_check_filter();
            request_reload(state, effects);
        }
        (Focus::HistoryList, KeyInput::Up) => {
            state.history.move_selection(SelectionMove::Previous);
        }
        (Focus::HistoryList, KeyInput::Down) => {
            state.history.move_selection(SelectionMove::Next);
        }
        (Focus::HistoryList, KeyInput::Character('k'))
            if state.keymap == super::model::Keymap::Vim =>
        {
            state.history.move_selection(SelectionMove::Previous);
        }
        (Focus::HistoryList, KeyInput::Character('j'))
            if state.keymap == super::model::Keymap::Vim =>
        {
            state.history.move_selection(SelectionMove::Next);
        }
        (Focus::HistoryList, KeyInput::Home) => {
            state.history.move_selection(SelectionMove::First);
        }
        (Focus::HistoryList, KeyInput::End) => {
            state.history.move_selection(SelectionMove::Last);
        }
        (Focus::HistoryList, KeyInput::Enter) => load_selected_detail(state, effects),
        (Focus::HistoryRerun, KeyInput::Enter) => rerun_selected(state, effects),
        (Focus::HistoryExport, KeyInput::Enter) => {
            if state.history.pending().is_none() {
                state.history.reset_export_choices();
                state.overlay = Some(Overlay::HistoryExport {
                    field: HistoryExportField::Action,
                });
            }
        }
        (Focus::HistoryDelete, KeyInput::Enter) => {
            if state.history.pending().is_none() {
                if let Some(analysis_id) = state.history.selected_id() {
                    state.overlay = Some(Overlay::ConfirmHistoryDelete {
                        analysis_id,
                        confirm: false,
                    });
                }
            }
        }
        _ => return false,
    }
    true
}

pub(super) fn reduce_overlay(
    state: &mut AppState,
    key: KeyInput,
    effects: &mut Vec<Effect>,
) -> bool {
    enum HistoryOverlay {
        Delete(AnalysisId, bool),
        Export(HistoryExportField),
        FullExport(super::history::ExportRequest, bool),
    }

    // Copy only the small history payload. Cloning the whole overlay would
    // copy and then zeroize an in-progress credential for every keystroke.
    let overlay = match state.overlay.as_ref() {
        Some(Overlay::ConfirmHistoryDelete {
            analysis_id,
            confirm,
        }) => HistoryOverlay::Delete(*analysis_id, *confirm),
        Some(Overlay::HistoryExport { field }) => HistoryOverlay::Export(*field),
        Some(Overlay::ConfirmFullHistoryExport { request, confirm }) => {
            HistoryOverlay::FullExport(*request, *confirm)
        }
        _ => return false,
    };
    match overlay {
        HistoryOverlay::Delete(analysis_id, confirm) => {
            reduce_delete_overlay(state, key, analysis_id, confirm, effects)
        }
        HistoryOverlay::Export(field) => reduce_export_overlay(state, key, field, effects),
        HistoryOverlay::FullExport(request, confirm) => {
            reduce_full_export_overlay(state, key, request, confirm, effects)
        }
    }
    true
}

fn request_reload(state: &mut AppState, effects: &mut Vec<Effect>) {
    if state.history.pending().is_some() {
        state.history.mark_reload_dirty();
        return;
    }
    let request = state.history.load_request();
    if state
        .history
        .start_pending(PendingOperation::Reload(request.clone()))
    {
        effects.push(Effect::LoadHistory(request));
    }
}

fn reload_if_dirty(state: &mut AppState, effects: &mut Vec<Effect>) -> bool {
    if !state.history.take_reload_dirty() {
        return false;
    }
    request_reload(state, effects);
    true
}

fn load_selected_detail(state: &mut AppState, effects: &mut Vec<Effect>) {
    let Some(analysis_id) = state.history.selected_id() else {
        return;
    };
    if state
        .history
        .start_pending(PendingOperation::Detail(analysis_id))
    {
        effects.push(Effect::LoadHistoryDetail(analysis_id));
    }
}

fn rerun_selected(state: &mut AppState, effects: &mut Vec<Effect>) {
    if state.analysis.submitting || !state.active.is_empty() {
        state.notice = Some("An analysis is already in progress.".to_owned());
        return;
    }
    let Some(analysis_id) = state.history.selected_id() else {
        return;
    };
    if state
        .history
        .start_pending(PendingOperation::Rerun(analysis_id))
    {
        // Close the shared analysis gate before private preflight begins. The
        // history operation remains pending through the terminal worker event.
        state.analysis.submitting = true;
        effects.push(Effect::PrepareHistoryRerun {
            analysis_id,
            automatic_save: state.settings.history_enabled,
        });
    }
}

fn reduce_delete_overlay(
    state: &mut AppState,
    key: KeyInput,
    analysis_id: AnalysisId,
    confirm: bool,
    effects: &mut Vec<Effect>,
) {
    match key {
        KeyInput::Character('y' | 'Y') => start_delete(state, analysis_id, effects),
        KeyInput::Character('n' | 'N') | KeyInput::Escape => state.overlay = None,
        KeyInput::Left | KeyInput::Right | KeyInput::Up | KeyInput::Down => {
            state.overlay = Some(Overlay::ConfirmHistoryDelete {
                analysis_id,
                confirm: !confirm,
            });
        }
        KeyInput::Enter if confirm => start_delete(state, analysis_id, effects),
        KeyInput::Enter => state.overlay = None,
        _ => {}
    }
}

fn start_delete(state: &mut AppState, analysis_id: AnalysisId, effects: &mut Vec<Effect>) {
    if state
        .history
        .start_pending(PendingOperation::Delete(analysis_id))
    {
        state.overlay = None;
        effects.push(Effect::DeleteHistory(analysis_id));
    }
}

fn reduce_export_overlay(
    state: &mut AppState,
    key: KeyInput,
    field: HistoryExportField,
    effects: &mut Vec<Effect>,
) {
    match key {
        KeyInput::Escape => state.overlay = None,
        KeyInput::Up | KeyInput::BackTab => {
            set_export_field(state, previous_export_field(field));
        }
        KeyInput::Down | KeyInput::Tab => set_export_field(state, next_export_field(field)),
        KeyInput::Left | KeyInput::Right => change_export_choice(state, field),
        KeyInput::Enter if field != HistoryExportField::Action => {
            change_export_choice(state, field);
        }
        KeyInput::Enter => match state.history.export_choices().resolve() {
            ExportResolution::Cancel => state.overlay = None,
            ExportResolution::Ready(request) => start_export(state, request, effects),
            ExportResolution::ConfirmFull(request) => {
                state.overlay = Some(Overlay::ConfirmFullHistoryExport {
                    request,
                    confirm: false,
                });
            }
        },
        _ => {}
    }
}

fn set_export_field(state: &mut AppState, field: HistoryExportField) {
    state.overlay = Some(Overlay::HistoryExport { field });
}

fn previous_export_field(field: HistoryExportField) -> HistoryExportField {
    match field {
        HistoryExportField::Format => HistoryExportField::Format,
        HistoryExportField::Content => HistoryExportField::Format,
        HistoryExportField::Action => HistoryExportField::Content,
    }
}

fn next_export_field(field: HistoryExportField) -> HistoryExportField {
    match field {
        HistoryExportField::Format => HistoryExportField::Content,
        HistoryExportField::Content | HistoryExportField::Action => HistoryExportField::Action,
    }
}

fn change_export_choice(state: &mut AppState, field: HistoryExportField) {
    let choices = state.history.export_choices_mut();
    match field {
        HistoryExportField::Format => choices.cycle_format(),
        HistoryExportField::Content => choices.toggle_content(),
        HistoryExportField::Action => choices.toggle_action(),
    }
}

fn reduce_full_export_overlay(
    state: &mut AppState,
    key: KeyInput,
    request: super::history::ExportRequest,
    confirm: bool,
    effects: &mut Vec<Effect>,
) {
    match key {
        KeyInput::Character('y' | 'Y') => start_export(state, request, effects),
        KeyInput::Character('n' | 'N') | KeyInput::Escape => state.overlay = None,
        KeyInput::Left | KeyInput::Right | KeyInput::Up | KeyInput::Down => {
            state.overlay = Some(Overlay::ConfirmFullHistoryExport {
                request,
                confirm: !confirm,
            });
        }
        KeyInput::Enter if confirm => start_export(state, request, effects),
        KeyInput::Enter => state.overlay = None,
        _ => {}
    }
}

fn start_export(
    state: &mut AppState,
    request: super::history::ExportRequest,
    effects: &mut Vec<Effect>,
) {
    if state
        .history
        .start_pending(PendingOperation::Export(request))
    {
        state.overlay = None;
        effects.push(Effect::ExportHistory(request));
    }
}

fn history_error(state: &mut AppState, error: CanonicalError) {
    state.notice = Some(format!("History unavailable: {}", error.message()));
}
