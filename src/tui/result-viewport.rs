//! ID-owned navigation for the shared Analyze and History result projection.

use std::ops::Range;

use ratatui::style::Style;
use ratatui::text::{Line, Span, StyledGrapheme};

use crate::domain::{Analysis, AnalysisId};
use crate::output::CanonicalError;

use super::model::{AppState, Focus, KeyInput, Keymap, Route};
use super::render::{focus_marker, muted_style, primary_style};
use super::result_lines::{ResultPresentation, analysis_result_lines};

const PAGE_LINES: usize = 6;
/// Every result row reserves the focus-marker gutter, focused or not, so text
/// does not shift when the selection moves.
const GUTTER: usize = focus_marker(false).len();

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
    let lines = analysis_result_lines(
        analysis,
        ResultPresentation::from_state(state),
        row_width(width),
    );
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
    presentation: ResultPresentation,
    width: usize,
    capacity: usize,
) -> Vec<Line<'static>> {
    let color_mode = presentation.color_mode;
    let lines = analysis_result_lines(analysis, presentation, row_width(width));
    let row_count = wrapped_row_count(&lines, width);
    let (range, selected) = viewport.window(analysis.id, row_count, capacity.max(1));
    let mut visible = Vec::with_capacity(range.len() + 1);
    // The pager row is context, not a target: the selected row already owns
    // the focus marker, so a second marker here would read as two selections.
    visible.push(Line::styled(
        format!(
            "{}Result rows {}-{} of {}",
            focus_marker(false),
            range.start.saturating_add(1),
            range.end,
            row_count,
        ),
        muted_style(color_mode),
    ));
    for (index, mut line) in visible_wrapped_rows(&lines, width, range.clone()) {
        let marker = focused && index == selected;
        // Ratatui's word wrapper renders a whitespace-only line as two rows,
        // which would push the page tail off screen. A blank row carries no
        // text, so it keeps only its marker.
        if line.spans.iter().all(|span| span.content.trim().is_empty()) {
            line = if marker {
                Line::styled(">", primary_style(color_mode))
            } else {
                Line::default()
            };
            visible.push(line);
            continue;
        }
        line.spans.insert(
            0,
            Span::styled(
                focus_marker(marker),
                if marker {
                    primary_style(color_mode)
                } else {
                    Style::default()
                },
            ),
        );
        visible.push(line);
    }
    visible
}

/// Cells one result row may use once the marker gutter is reserved.
fn row_width(paragraph_width: usize) -> usize {
    paragraph_width.saturating_sub(GUTTER).max(1)
}

fn wrapped_row_count(lines: &[Line<'_>], paragraph_width: usize) -> usize {
    let width = row_width(paragraph_width);
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
    let width = row_width(paragraph_width);
    let mut visible = Vec::with_capacity(range.len());
    let mut row_index = 0;

    for line in lines {
        let completed = visit_wrapped_rows(line, width, |row| {
            if range.contains(&row_index) {
                visible.push((row_index, row_line(row)));
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

/// Rebuilds one physical row from styled graphemes, merging neighbours that
/// share a style so the row keeps the projection's semantic colors.
fn row_line(row: &[StyledGrapheme<'_>]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for grapheme in row {
        match spans.last_mut() {
            Some(span) if span.style == grapheme.style => {
                span.content.to_mut().push_str(grapheme.symbol);
            }
            _ => spans.push(Span::styled(grapheme.symbol.to_owned(), grapheme.style)),
        }
    }
    Line::from(spans)
}

/// Splits one logical line into physical rows at whole words where possible.
/// Continuation rows hang under the line's leading indent so indented
/// evidence stays visually attached to its label row.
fn visit_wrapped_rows<'a>(
    line: &'a Line<'_>,
    width: usize,
    mut visit: impl FnMut(&[StyledGrapheme<'a>]) -> bool,
) -> bool {
    let is_whitespace =
        |grapheme: &StyledGrapheme<'_>| grapheme.symbol.chars().all(char::is_whitespace);
    let graphemes: Vec<StyledGrapheme<'a>> = line.styled_graphemes(Style::default()).collect();
    // The indent is capped so an over-indented line still leaves room for
    // text; without the cap a continuation row could hold no content at all.
    let indent = graphemes
        .iter()
        .take_while(|grapheme| is_whitespace(grapheme))
        .count()
        .min(width / 2);
    let indent_cell = StyledGrapheme {
        symbol: " ",
        style: Style::default(),
    };

    // Invariant: `row[..indent]` is always indent, real on the first row and
    // synthetic afterwards, so word breaks only ever happen after it.
    let mut row: Vec<StyledGrapheme<'a>> = Vec::new();
    let mut used = 0_usize;
    let mut last_whitespace = None;

    for grapheme in graphemes {
        let symbol_width = Span::raw(grapheme.symbol).width();
        while row.len() > indent && used.saturating_add(symbol_width) > width {
            let split = last_whitespace.map_or(row.len(), |index| index + 1);
            if !visit(&row[..split]) {
                return false;
            }
            row.drain(..split);
            row.splice(0..0, std::iter::repeat_n(indent_cell.clone(), indent));
            used = row.iter().map(|g| Span::raw(g.symbol).width()).sum();
            last_whitespace = row
                .iter()
                .skip(indent)
                .rposition(is_whitespace)
                .map(|index| index + indent);
        }
        let breakable = is_whitespace(&grapheme);
        row.push(grapheme);
        used = used.saturating_add(symbol_width);
        if breakable && row.len() > indent {
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

    fn rows_of(line: &Line<'_>, width: usize) -> Vec<String> {
        let mut rows = Vec::new();
        visit_wrapped_rows(line, width, |row| {
            rows.push(row.iter().map(|g| g.symbol).collect::<String>());
            true
        });
        rows
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
        assert_eq!(
            rows_of(&Line::raw("evidence TAIL_SENTINEL"), 16),
            ["evidence ", "TAIL_SENTINEL"]
        );
    }

    #[test]
    fn continuation_rows_hang_under_the_leading_indent() {
        let rows = rows_of(&Line::raw("   alpha beta gamma delta"), 12);

        assert_eq!(rows, ["   alpha ", "   beta ", "   gamma ", "   delta"]);
    }

    #[test]
    fn hanging_indent_still_splits_an_overlong_token() {
        let rows = rows_of(&Line::raw("   abcdefghijklmnop"), 10);

        assert_eq!(rows.concat().replace(' ', ""), "abcdefghijklmnop");
        assert!(rows.iter().all(|row| Span::raw(row.as_str()).width() <= 10));
        assert!(rows.iter().all(|row| row.starts_with("   ")));
    }

    #[test]
    fn physical_rows_keep_span_styles() {
        let line = Line::from(vec![
            Span::styled(
                "bold ",
                Style::default().add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::raw("plain text that wraps"),
        ]);
        let rows = visible_wrapped_rows(&[line], 14, 0..3)
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>();

        assert_eq!(rows[0].spans[0].content, "bold ");
        assert!(
            rows[0].spans[0]
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
        assert_eq!(rows[0].spans[1].content, "plain ");
        assert_eq!(rows[0].spans[1].style, Style::default());
    }
}
