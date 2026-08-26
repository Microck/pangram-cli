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
    let mut count = 0;
    for line in lines {
        visit_wrapped_rows(line, width, |_| {
            count += 1;
            true
        });
    }
    count
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
        let completed = visit_wrapped_rows(line, width, |row| {
            if range.contains(&row_index) {
                visible.push((row_index, Line::raw(row.concat())));
            }
            row_index += 1;
            row_index < range.end
        });
        if !completed {
            return visible;
        }
    }
    visible
}

fn visit_wrapped_rows<'a>(
    line: &'a Line<'_>,
    width: usize,
    mut visit: impl FnMut(&[&'a str]) -> bool,
) -> bool {
    let mut row = Vec::new();
    let mut used = 0_usize;
    let mut last_whitespace = None;

    for grapheme in line.styled_graphemes(Style::default()) {
        let symbol = grapheme.symbol;
        let symbol_width = Span::raw(symbol).width();
        while !row.is_empty() && used.saturating_add(symbol_width) > width {
            let split = last_whitespace.map_or(row.len(), |index| index + 1);
            if !visit(&row[..split]) {
                return false;
            }
            row.drain(..split);
            used = row.iter().map(|symbol| Span::raw(*symbol).width()).sum();
            last_whitespace = row
                .iter()
                .rposition(|symbol| symbol.chars().all(char::is_whitespace));
        }
        row.push(symbol);
        used = used.saturating_add(symbol_width);
        if symbol.chars().all(char::is_whitespace) {
            last_whitespace = Some(row.len() - 1);
        }
    }

    if row.is_empty() {
        visit(&[])
    } else {
        visit(&row)
    }
}

fn result_width(state: &AppState) -> usize {
    let frame = ratatui::layout::Rect::new(0, 0, state.terminal.columns, state.terminal.rows);
    let workspace = super::render::screen_areas(frame, 0, state.route).workspace;
    usize::from(super::render::workspace_content_area(workspace).width)
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

    #[test]
    fn wrapping_keeps_a_word_whole_when_it_fits_on_the_next_row() {
        let mut rows = Vec::new();
        visit_wrapped_rows(&Line::raw("evidence TAIL_SENTINEL"), 16, |row| {
            rows.push(row.concat());
            true
        });

        assert_eq!(rows, ["evidence ", "TAIL_SENTINEL"]);
    }
}
