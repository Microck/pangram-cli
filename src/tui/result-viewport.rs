//! ID-owned navigation for the shared Analyze and History result projection.

use std::ops::Range;

use ratatui::text::{Line, Span};

use crate::domain::{Analysis, AnalysisId};
use crate::output::CanonicalError;

use super::model::{AppState, Focus, KeyInput, Keymap, Route};
use super::result_lines::{analysis_result_line_count, analysis_result_lines};

const PAGE_LINES: usize = 6;

#[derive(Clone, Copy)]
pub(super) enum ResultMove {
    Previous,
    Next,
    PageUp,
    PageDown,
    First,
    Last,
}

#[derive(Clone, Default)]
pub(super) struct ResultViewport {
    analysis_id: Option<AnalysisId>,
    selected_line: usize,
}

impl ResultViewport {
    pub(super) fn reset(&mut self, analysis_id: AnalysisId) {
        self.analysis_id = Some(analysis_id);
        self.selected_line = 0;
    }

    fn navigate(&mut self, analysis_id: AnalysisId, line_count: usize, movement: ResultMove) {
        if self.analysis_id != Some(analysis_id) {
            self.reset(analysis_id);
        }
        let last = line_count.saturating_sub(1);
        self.selected_line = match movement {
            ResultMove::Previous => self.selected_line.saturating_sub(1),
            ResultMove::Next => self.selected_line.saturating_add(1).min(last),
            ResultMove::PageUp => self.selected_line.saturating_sub(PAGE_LINES),
            ResultMove::PageDown => self.selected_line.saturating_add(PAGE_LINES).min(last),
            ResultMove::First => 0,
            ResultMove::Last => last,
        };
    }

    fn window(
        &self,
        analysis_id: AnalysisId,
        line_count: usize,
        capacity: usize,
    ) -> (Range<usize>, usize) {
        let selected = if self.analysis_id == Some(analysis_id) {
            self.selected_line
        } else {
            0
        }
        .min(line_count.saturating_sub(1));
        let start = selected
            .saturating_add(1)
            .saturating_sub(capacity)
            .min(line_count.saturating_sub(capacity));
        (start..(start + capacity).min(line_count), selected)
    }
}

pub(super) fn reduce_key(state: &mut AppState, key: KeyInput) -> bool {
    if state.focus != Focus::Result {
        return false;
    }
    let movement = match key {
        KeyInput::Up => ResultMove::Previous,
        KeyInput::Down => ResultMove::Next,
        KeyInput::Home => ResultMove::First,
        KeyInput::End => ResultMove::Last,
        KeyInput::PageUp => ResultMove::PageUp,
        KeyInput::PageDown => ResultMove::PageDown,
        KeyInput::Character('k') if state.keymap == Keymap::Vim => ResultMove::Previous,
        KeyInput::Character('j') if state.keymap == Keymap::Vim => ResultMove::Next,
        KeyInput::CtrlU if state.keymap == Keymap::Vim => ResultMove::PageUp,
        KeyInput::CtrlD if state.keymap == Keymap::Vim => ResultMove::PageDown,
        _ => return false,
    };
    navigate(state, movement);
    true
}

pub(super) fn navigate(state: &mut AppState, movement: ResultMove) {
    let analysis = match state.route {
        Route::Analyze => state.analysis.current.as_ref(),
        Route::History => state.history.selected_detail(),
        Route::Active | Route::Settings => None,
    };
    let Some(analysis) = analysis else {
        return;
    };
    state
        .result_viewport
        .navigate(analysis.id, analysis_result_line_count(analysis), movement);
}

/// Projects a navigable page without discarding any canonical result lines.
pub(super) fn visible_analysis_result_lines(
    analysis: &Analysis<CanonicalError>,
    viewport: &ResultViewport,
    focused: bool,
    capacity: usize,
) -> Vec<Line<'static>> {
    let lines = analysis_result_lines(analysis);
    debug_assert_eq!(analysis_result_line_count(analysis), lines.len());
    let (range, selected) = viewport.window(analysis.id, lines.len(), capacity.max(1));
    let mut visible = Vec::with_capacity(range.len() + 1);
    visible.push(Line::raw(format!(
        "{}Result lines {}-{} of {}",
        if focused { "> " } else { "  " },
        range.start.saturating_add(1),
        range.end,
        lines.len(),
    )));
    for (index, mut line) in lines
        .into_iter()
        .enumerate()
        .skip(range.start)
        .take(range.len())
    {
        line.spans.insert(
            0,
            Span::raw(if focused && index == selected {
                "> "
            } else {
                "  "
            }),
        );
        visible.push(line);
    }
    visible
}
