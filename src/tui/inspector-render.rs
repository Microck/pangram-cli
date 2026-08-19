//! Inspector projection for wide and narrow TUI layouts.

use ratatui::text::{Line, Span};

use super::{
    action_style, analysis_action_label, analysis_can_reset, analyze_inspector_rows, control_style,
    heading, history_render, muted_style, primary_style,
};
use crate::tui::model::{AppState, ColorMode, Focus, Route};
use crate::tui::result_lines::sanitize_single_line;

pub(super) fn lines(state: &AppState, narrow: bool) -> Vec<Line<'static>> {
    let mut lines = match state.route {
        Route::Analyze => analyze_lines(state, narrow),
        Route::Active if narrow => vec![Line::raw(format!(
            "Session  {} active  |  ephemeral",
            state.active.len()
        ))],
        Route::Active => vec![
            heading(state.color_mode, "Session"),
            Line::raw(""),
            Line::raw(format!("{} active", state.active.len())),
            Line::styled(
                "Ephemeral work is process-local.",
                muted_style(state.color_mode),
            ),
        ],
        Route::History => history_render::inspector_lines(state, narrow),
        Route::Settings => Vec::new(),
    };
    if let Some(notice) = &state.notice {
        if !narrow && !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::raw(format!(
            "Notice: {}",
            sanitize_single_line(notice)
        )));
    }
    lines
}

fn analyze_lines(state: &AppState, narrow: bool) -> Vec<Line<'static>> {
    if narrow && !analysis_can_reset(state) {
        return Vec::new();
    }
    let (word_count, units) = state.billing_estimate();
    let estimate = estimate_label(word_count, units);
    let submit_label = analysis_action_label(state);
    let rows = analyze_inspector_rows(!narrow);
    let mut lines = vec![Line::raw(""); usize::from(rows.height)];
    if narrow {
        lines[usize::from(rows.public_link)] = inline_toggle_line(state);
        lines[usize::from(rows.words)] =
            Line::raw(format!("  Words {word_count}  |  Estimate {estimate}"));
    } else {
        lines[0] = heading(state.color_mode, "Inspector");
        lines[usize::from(rows.public_link)] = public_link_line(state);
        lines[usize::from(rows.manual_save)] = toggle_line(
            state.color_mode,
            state.focus == Focus::ManualSave,
            state.manual_save,
            "Manual save",
        );
        lines[usize::from(rows.words)] = Line::raw(format!("  Words {word_count}"));
        lines[usize::from(rows.estimate)] = Line::raw(format!("  Estimate {estimate}"));
    }
    lines[usize::from(rows.submit)] =
        action_line(state.color_mode, state.focus == Focus::Submit, submit_label);
    lines
}

fn toggle_line(
    color_mode: ColorMode,
    focused: bool,
    enabled: bool,
    label: &'static str,
) -> Line<'static> {
    Line::from(toggle_spans(color_mode, focused, enabled, label))
}

fn public_link_line(state: &AppState) -> Line<'static> {
    if state.public_link_available() {
        toggle_line(
            state.color_mode,
            state.focus == Focus::PublicLink,
            state.public_link,
            "Public link",
        )
    } else {
        Line::styled("  Public link n/a", muted_style(state.color_mode))
    }
}

pub(super) fn public_link_spans(state: &AppState) -> Vec<Span<'static>> {
    if state.public_link_available() {
        toggle_spans(
            state.color_mode,
            state.focus == Focus::PublicLink,
            state.public_link,
            "Public link",
        )
    } else {
        vec![Span::styled(
            "  Public link n/a ",
            muted_style(state.color_mode),
        )]
    }
}

fn inline_toggle_line(state: &AppState) -> Line<'static> {
    let mut spans = if state.public_link_available() {
        toggle_spans(
            state.color_mode,
            state.focus == Focus::PublicLink,
            state.public_link,
            "Public link",
        )
    } else {
        vec![Span::styled(
            "  Public link n/a",
            muted_style(state.color_mode),
        )]
    };
    spans.push(Span::raw("   "));
    spans.extend(toggle_spans(
        state.color_mode,
        state.focus == Focus::ManualSave,
        state.manual_save,
        "Manual save",
    ));
    Line::from(spans)
}

pub(super) fn toggle_spans(
    color_mode: ColorMode,
    focused: bool,
    enabled: bool,
    label: &'static str,
) -> Vec<Span<'static>> {
    vec![
        Span::styled(if focused { ">" } else { " " }, primary_style(color_mode)),
        Span::styled(
            format!(" {label} {} ", if enabled { "on" } else { "off" }),
            control_style(color_mode, focused, enabled, true),
        ),
    ]
}

pub(super) fn action_line(color_mode: ColorMode, focused: bool, label: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(if focused { ">" } else { " " }, primary_style(color_mode)),
        Span::styled(format!(" {label} "), action_style(color_mode)),
    ])
}

pub(super) fn estimate_label(word_count: u64, units: u64) -> String {
    if word_count == 0 {
        "-".to_owned()
    } else {
        format!("{units} {}", if units == 1 { "unit" } else { "units" })
    }
}
