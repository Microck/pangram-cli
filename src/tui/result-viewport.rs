//! ID-owned navigation for the shared Analyze and History result projection.

use std::ops::Range;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::domain::{Analysis, AnalysisId};
use crate::output::CanonicalError;

use super::model::{AppState, Focus, KeyInput, Keymap, Route};
use super::result_lines::analysis_result_lines;

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
    selected_row: usize,
}

impl ResultViewport {
    pub(super) fn reset(&mut self, analysis_id: AnalysisId) {
        self.analysis_id = Some(analysis_id);
        self.selected_row = 0;
    }

    fn navigate(&mut self, analysis_id: AnalysisId, row_count: usize, movement: ResultMove) {
        if self.analysis_id != Some(analysis_id) {
            self.reset(analysis_id);
        }
        let last = row_count.saturating_sub(1);
        self.selected_row = match movement {
            ResultMove::Previous => self.selected_row.saturating_sub(1),
            ResultMove::Next => self.selected_row.saturating_add(1).min(last),
            ResultMove::PageUp => self.selected_row.saturating_sub(PAGE_LINES),
            ResultMove::PageDown => self.selected_row.saturating_add(PAGE_LINES).min(last),
            ResultMove::First => 0,
            ResultMove::Last => last,
        };
    }

    fn window(
        &self,
        analysis_id: AnalysisId,
        row_count: usize,
        capacity: usize,
    ) -> (Range<usize>, usize) {
        let selected = if self.analysis_id == Some(analysis_id) {
            self.selected_row
        } else {
            0
        }
        .min(row_count.saturating_sub(1));
        let start = selected
            .saturating_add(1)
            .saturating_sub(capacity)
            .min(row_count.saturating_sub(capacity));
        (start..(start + capacity).min(row_count), selected)
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
    let width = result_width(state);
    let lines = analysis_result_lines(analysis);
    let row_count = wrapped_row_count(&lines, width);
    state
        .result_viewport
        .navigate(analysis.id, row_count, movement);
}

/// Projects a navigable page without discarding any canonical result lines.
pub(super) fn visible_analysis_result_lines(
    analysis: &Analysis<CanonicalError>,
    viewport: &ResultViewport,
    focused: bool,
    width: usize,
    capacity: usize,
) -> Vec<Line<'static>> {
    let lines = analysis_result_lines(analysis);
    let row_count = wrapped_row_count(&lines, width);
    let (range, selected) = viewport.window(analysis.id, row_count, capacity.max(1));
    let mut visible = Vec::with_capacity(range.len() + 1);
    visible.push(Line::raw(format!(
        "{}Result rows {}-{} of {}",
        if focused { "> " } else { "  " },
        range.start.saturating_add(1),
        range.end,
        row_count,
    )));
    for (index, mut line) in visible_wrapped_rows(&lines, width, range.clone()) {
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

fn wrapped_row_count(lines: &[Line<'_>], paragraph_width: usize) -> usize {
    let width = paragraph_width.saturating_sub(2).max(1);
    lines
        .iter()
        .map(|line| {
            let mut rows = 1;
            let mut used = 0_usize;
            for grapheme in line.styled_graphemes(Style::default()) {
                let grapheme_width = Span::raw(grapheme.symbol).width();
                if used > 0 && used.saturating_add(grapheme_width) > width {
                    rows += 1;
                    used = 0;
                }
                used = used.saturating_add(grapheme_width);
            }
            rows
        })
        .sum()
}

fn visible_wrapped_rows(
    lines: &[Line<'_>],
    paragraph_width: usize,
    range: Range<usize>,
) -> Vec<(usize, Line<'static>)> {
    let width = paragraph_width.saturating_sub(2).max(1);
    let mut visible = Vec::with_capacity(range.len());
    let mut row_index = 0;

    for line in lines {
        let mut row = String::new();
        let mut used = 0_usize;
        for grapheme in line.styled_graphemes(Style::default()) {
            let grapheme_width = Span::raw(grapheme.symbol).width();
            if used > 0 && used.saturating_add(grapheme_width) > width {
                if range.contains(&row_index) {
                    visible.push((row_index, Line::raw(std::mem::take(&mut row))));
                }
                row_index += 1;
                used = 0;
            }
            if range.contains(&row_index) {
                row.push_str(grapheme.symbol);
            }
            used = used.saturating_add(grapheme_width);
        }
        if range.contains(&row_index) {
            visible.push((row_index, Line::raw(row)));
        }
        row_index += 1;
        if row_index >= range.end {
            break;
        }
    }
    visible
}

fn result_width(state: &AppState) -> usize {
    let columns = usize::from(state.terminal.columns);
    match (state.layout(), state.route) {
        (super::model::ResponsiveLayout::Wide, Route::Analyze) => columns.saturating_sub(51),
        (super::model::ResponsiveLayout::Wide, Route::History) => columns.saturating_sub(50),
        (_, Route::Analyze) => columns.saturating_sub(2),
        (_, Route::History) => columns.saturating_sub(1),
        (_, Route::Active | Route::Settings) => columns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rows: &[Line<'_>]) -> String {
        rows.iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn physical_rows_preserve_wide_extended_graphemes_without_clipping() {
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}";
        let original = format!("A\u{6f22}{family}B\u{6f22}{family}C");

        let lines = [Line::raw(original.clone())];
        let row_count = wrapped_row_count(&lines, 7);
        let rows = visible_wrapped_rows(&lines, 7, 0..row_count)
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>();

        assert_eq!(text(&rows), original);
        assert!(rows.iter().all(|line| line.width() <= 5));
        assert_eq!(text(&rows).matches(family).count(), 2);
    }
}
