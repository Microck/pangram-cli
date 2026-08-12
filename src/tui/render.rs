//! Deterministic terminal rendering for the pure TUI state machine.
//!
//! This one-way projection performs no I/O and never changes `AppState`.
//! Text cues keep focus and availability legible without color.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::history_render;
use super::model::{
    AppState, Focus, IntroFrequency, MIN_HEIGHT, MIN_WIDTH, MotionLevel, Overlay, Route, WIDE_WIDTH,
};
use super::result_lines::{analysis_status_label, sanitize_single_line};
use super::result_viewport::visible_analysis_result_lines;

const ROUTE_RAIL_WIDTH: u16 = 17;
const INSPECTOR_WIDTH: u16 = 30;
const COMMAND_BAR_HEIGHT: u16 = 2;
const NARROW_TABS_HEIGHT: u16 = 2;
const NARROW_INSPECTOR_HEIGHT: u16 = 7;

/// Draws the current application state into the terminal frame.
pub(super) fn render(frame: &mut Frame<'_>, state: &AppState) {
    let area = frame.area();
    if area.width >= WIDE_WIDTH {
        render_wide(frame, area, state);
    } else {
        render_narrow(frame, area, state);
    }

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_resize_overlay(frame, area);
    } else if let Some(overlay) = &state.overlay {
        render_overlay(frame, area, state, overlay);
    }
}

fn render_wide(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let [body, command] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(COMMAND_BAR_HEIGHT)]).areas(area);
    let [
        rail,
        rail_separator,
        workspace,
        inspector_separator,
        inspector,
    ] = Layout::horizontal([
        Constraint::Length(ROUTE_RAIL_WIDTH),
        Constraint::Length(1),
        Constraint::Min(44),
        Constraint::Length(1),
        Constraint::Length(INSPECTOR_WIDTH),
    ])
    .areas(body);

    render_route_rail(frame, rail, state);
    render_vertical_separator(frame, rail_separator);
    render_workspace(frame, workspace, state);
    render_vertical_separator(frame, inspector_separator);
    render_inspector(frame, inspector, state);
    render_command_bar(frame, command, state);
}

fn render_narrow(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let [tabs, workspace, inspector, command] = Layout::vertical([
        Constraint::Length(NARROW_TABS_HEIGHT),
        Constraint::Min(1),
        Constraint::Length(NARROW_INSPECTOR_HEIGHT),
        Constraint::Length(COMMAND_BAR_HEIGHT),
    ])
    .areas(area);

    render_route_tabs(frame, tabs, state);
    render_workspace(frame, workspace, state);
    render_inspector(frame, inspector, state);
    render_command_bar(frame, command, state);
}

fn render_route_rail(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let mut lines = vec![
        Line::styled("Pangram", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw(""),
    ];
    for route in Route::ALL {
        lines.push(Line::raw(route_label(route, state)));
    }
    frame.render_widget(Paragraph::new(lines), inset(area, 1, 1));
}

fn render_route_tabs(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let mut spans = Vec::new();
    for (index, route) in Route::ALL.into_iter().enumerate() {
        if index != 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::raw(route_label(route, state)));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::BOTTOM)),
        area,
    );
}

fn route_label(route: Route, state: &AppState) -> String {
    let selected = state.route == route;
    let focused = state.focus == Focus::Routes && selected;
    format!(
        "{}{}{}{}",
        if focused { "> " } else { "  " },
        if selected { "[" } else { " " },
        route.name(),
        if selected { "]" } else { " " },
    )
}

fn render_vertical_separator(frame: &mut Frame<'_>, area: Rect) {
    let buffer = frame.buffer_mut();
    for y in area.y..area.y.saturating_add(area.height) {
        buffer[(area.x, y)].set_symbol("|");
    }
}

fn render_workspace(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    match state.route {
        Route::Analyze => render_analyze(frame, area, state),
        Route::Active => render_active(frame, area, state),
        Route::History => history_render::render_history(frame, area, state),
        Route::Settings => render_settings(frame, area, state),
    }
}

fn render_analyze(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if try_render_analysis_state(frame, area, state) {
        return;
    }

    let [choices, composer] =
        Layout::vertical([Constraint::Length(9), Constraint::Min(3)]).areas(inset(area, 1, 0));
    let choice_lines = vec![
        heading("Analyze"),
        Line::raw("Checks"),
        selectable_line(state.focus == Focus::CheckAi, true, "AI detection", true),
        selectable_line(
            state.focus == Focus::CheckPlagiarism,
            false,
            "Plagiarism",
            false,
        ),
        selectable_line(state.focus == Focus::CheckBoth, false, "Both", false),
        Line::raw("Input"),
        selectable_line(state.focus == Focus::InputText, true, "Text", true),
        selectable_line(state.focus == Focus::InputFiles, false, "Files", false),
        Line::raw(""),
    ];
    frame.render_widget(Paragraph::new(choice_lines), choices);

    let composer_title = if state.focus == Focus::Composer {
        "> Text composer"
    } else {
        "Text composer"
    };
    let composer_text = sanitize_multiline(state.composer.value());
    let cursor_prefix = state.composer.before_cursor();
    let cursor_row = cursor_prefix.matches('\n').count();
    let cursor_column = Line::raw(crate::output::sanitize_terminal(
        cursor_prefix.rsplit('\n').next().unwrap_or_default(),
    ))
    .width();
    let composer_inner = inset(composer, 1, 1);
    // The edit position owns the derived viewport. Keeping it out of AppState
    // prevents scroll state from drifting after paste, resize, or cursor edits.
    let scroll_x =
        cursor_column.saturating_sub(usize::from(composer_inner.width).saturating_sub(1));
    let scroll_y = cursor_row.saturating_sub(usize::from(composer_inner.height).saturating_sub(1));
    let content = if composer_text.is_empty() {
        Text::from("Type or paste text here")
    } else {
        Text::from(composer_text)
    };
    frame.render_widget(
        Paragraph::new(content)
            .scroll((
                u16::try_from(scroll_y).unwrap_or(u16::MAX),
                u16::try_from(scroll_x).unwrap_or(u16::MAX),
            ))
            .block(Block::default().title(composer_title).borders(Borders::ALL)),
        composer,
    );
    if state.focus == Focus::Composer
        && state.overlay.is_none()
        && frame.area().width >= MIN_WIDTH
        && frame.area().height >= MIN_HEIGHT
        && composer_inner.width > 0
        && composer_inner.height > 0
    {
        frame.set_cursor_position((
            composer_inner.x + u16::try_from(cursor_column - scroll_x).unwrap_or(u16::MAX),
            composer_inner.y + u16::try_from(cursor_row - scroll_y).unwrap_or(u16::MAX),
        ));
    }
}

fn try_render_analysis_state(frame: &mut Frame<'_>, area: Rect, state: &AppState) -> bool {
    if !state.analysis.submitting
        && state.analysis.failure.is_none()
        && state.analysis.progress.is_none()
        && state.analysis.current.is_none()
    {
        return false;
    }
    let mut lines = vec![heading("Analyze"), Line::raw("")];
    if state.analysis.submitting {
        lines.push(Line::raw("Submitting analysis"));
        lines.push(Line::raw("The request is being sent once."));
    } else if let Some(failure) = &state.analysis.failure {
        lines.push(Line::raw("Analysis failed"));
        lines.push(Line::raw(format!("Analysis: {}", failure.analysis_id)));
        lines.push(Line::raw(format!(
            "Error: {}",
            sanitize_single_line(failure.error.message())
        )));
    } else if let Some(progress) = &state.analysis.progress {
        lines.push(Line::raw("Analysis in progress"));
        lines.push(Line::raw(format!("Analysis: {}", progress.analysis_id)));
        lines.push(Line::raw(format!(
            "Stage: {}",
            sanitize_single_line(progress.last_stage.as_str())
        )));
    } else if let Some(analysis) = &state.analysis.current {
        lines.extend(visible_analysis_result_lines(
            analysis,
            &state.result_viewport,
            state.focus == Focus::Result,
            usize::from(area.height.saturating_sub(5)),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(action_line(state.focus == Focus::Submit, "New analysis"));
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inset(area, 1, 0),
    );
    true
}

fn selectable_line(focused: bool, selected: bool, label: &str, available: bool) -> Line<'static> {
    Line::raw(format!(
        "{}[{}] {label} - {}",
        if focused { "> " } else { "  " },
        if selected { "x" } else { " " },
        if available {
            "available"
        } else {
            "unavailable"
        },
    ))
}

fn render_active(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let mut content = if state.active.is_empty() {
        vec![
            heading("Active"),
            Line::raw(""),
            Line::raw(format!(
                "{}No unfinished analyses.",
                if state.focus == Focus::ActiveList {
                    "> "
                } else {
                    "  "
                }
            )),
            Line::raw("Submit text from Analyze to start one."),
        ]
    } else {
        vec![
            heading("Active"),
            Line::raw(""),
            Line::raw(format!("{} unfinished analysis(es)", state.active.len())),
        ]
    };
    for row in state.active.visible_rows() {
        content.push(Line::raw(format!(
            "{}{} - {} - {}",
            if state.active.selected_id() == Some(row.id) && state.focus == Focus::ActiveList {
                "> "
            } else {
                "  "
            },
            row.id,
            analysis_status_label(row.status),
            row.source.label(),
        )));
    }
    frame.render_widget(
        Paragraph::new(content).wrap(Wrap { trim: false }),
        inset(area, 1, 0),
    );
}

fn render_settings(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let lines = vec![
        heading("Settings"),
        Line::raw(""),
        setting_line(
            state.focus == Focus::SettingsAuthentication,
            "Authentication",
            if state.settings.credential_present {
                "configured"
            } else {
                "not configured"
            },
        ),
        setting_line(
            state.focus == Focus::SettingsHistory,
            "History",
            if state.settings.history_enabled {
                "enabled"
            } else {
                "disabled"
            },
        ),
        setting_line(
            state.focus == Focus::SettingsIntro,
            "Intro frequency",
            intro_label(state.settings.intro),
        ),
        setting_line(
            state.focus == Focus::SettingsKeymap,
            "Keymap",
            state.keymap.name(),
        ),
        setting_line(
            state.focus == Focus::SettingsMotion,
            "Motion",
            motion_label(state.settings.motion),
        ),
        setting_line(
            state.focus == Focus::SettingsUpdates,
            "Updates",
            match state.settings.update_preference {
                Some(true) => "enabled",
                Some(false) => "disabled",
                None => "not set",
            },
        ),
        Line::raw("Diagnostics: available"),
    ];
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inset(area, 1, 0),
    );
}

fn setting_line(focused: bool, label: &str, value: &str) -> Line<'static> {
    Line::raw(format!(
        "{}{label}: {value}",
        if focused { "> " } else { "  " }
    ))
}

fn intro_label(value: IntroFrequency) -> &'static str {
    match value {
        IntroFrequency::Once => "once",
        IntroFrequency::Always => "always",
        IntroFrequency::Off => "off",
    }
}

fn motion_label(value: MotionLevel) -> &'static str {
    match value {
        MotionLevel::Full => "full",
        MotionLevel::Reduced => "reduced",
        MotionLevel::Off => "off",
    }
}

fn render_inspector(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let mut lines = match state.route {
        Route::Analyze => {
            let word_count = state.word_count();
            vec![
                heading("Inspector"),
                toggle_line(
                    state.focus == Focus::PublicLink,
                    state.public_link,
                    "Public link",
                ),
                toggle_line(
                    state.focus == Focus::ManualSave,
                    state.manual_save,
                    "Manual save",
                ),
                Line::raw(format!("Words: {word_count}")),
                Line::raw(format!(
                    "Billable estimate: {} unit(s)",
                    state.billable_units()
                )),
                action_line(state.focus == Focus::Submit, "Submit"),
            ]
        }
        Route::Active => vec![
            heading("Session"),
            Line::raw(format!("Active: {}", state.active.len())),
            Line::raw("Only this process owns ephemeral work."),
        ],
        Route::History => {
            history_render::inspector_lines(state, area.height <= NARROW_INSPECTOR_HEIGHT)
        }
        Route::Settings => vec![
            heading("Configuration"),
            Line::raw("Authentication and local behavior only."),
            Line::raw("No endpoint or theme override."),
        ],
    };
    if let Some(notice) = &state.notice {
        lines.push(Line::raw(format!(
            "Notice: {}",
            sanitize_single_line(notice)
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inset(area, 1, 1),
    );
}

fn toggle_line(focused: bool, enabled: bool, label: &str) -> Line<'static> {
    Line::raw(format!(
        "{}[{}] {label}: {}",
        if focused { "> " } else { "  " },
        if enabled { "x" } else { " " },
        if enabled { "on" } else { "off" },
    ))
}

fn action_line(focused: bool, label: &str) -> Line<'static> {
    Line::raw(format!(
        "{}[Enter] {label}{}",
        if focused { "> " } else { "  " },
        if focused { " <" } else { "" },
    ))
}

fn render_command_bar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let mut actions = vec!["[Tab] Next", "[Shift+Tab] Previous", "[?] Help"];
    match state.route {
        Route::Analyze => actions.push("[Enter] Activate"),
        Route::History => actions.push("[/] Search"),
        Route::Settings => actions.push("[Enter] Change"),
        Route::Active => {}
    }
    if state.focus == Focus::Result {
        actions.push("[Arrows/Page] Result");
    }
    let quit = if state.focus == Focus::Quit {
        "> [Enter] Quit <"
    } else {
        "[Enter] Quit"
    };
    actions.push(quit);
    frame.render_widget(
        Paragraph::new(actions.join("   "))
            .block(Block::default().borders(Borders::TOP))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_overlay(frame: &mut Frame<'_>, area: Rect, state: &AppState, overlay: &Overlay) {
    let overlay_area = centered(area, 56, 9);
    let (title, lines) = match overlay {
        Overlay::Credential(entry) => (
            "Credential setup",
            vec![
                Line::raw("Enter a Pangram API key, or skip for now."),
                Line::raw(""),
                Line::raw(if entry.value().is_empty() {
                    "API key: [empty]"
                } else {
                    "API key: ******** (masked)"
                }),
                Line::raw(""),
                Line::raw("[Enter] Save   [Esc] Skip"),
            ],
        ),
        Overlay::UpdatePreference { choice } => (
            "Update checks",
            vec![
                Line::raw("Choose whether Pangram may check for updates."),
                Line::raw(""),
                Line::raw(format!(
                    "Automatic checks: {}",
                    if *choice { "on" } else { "off" }
                )),
                Line::raw(""),
                Line::raw("[Enter] Continue   [Esc] Back"),
            ],
        ),
        Overlay::HistoryConsent => (
            "Enable local history",
            vec![
                Line::raw("History stores full input and results in plaintext."),
                Line::raw("Records remain until you delete them."),
                Line::raw(""),
                Line::raw("[Y/Enter] Enable   [N/Esc] Cancel"),
            ],
        ),
        Overlay::Help => (
            "Help",
            vec![
                Line::raw("Arrows or Tab move focus. Enter activates."),
                Line::raw("Printable keys edit the active text field."),
                Line::raw("? opens help. Esc closes an overlay."),
                Line::raw(""),
                Line::raw("Focus Quit in the command bar for a normal exit."),
            ],
        ),
        Overlay::ConfirmHistoryDelete { .. }
        | Overlay::HistoryExport { .. }
        | Overlay::ConfirmFullHistoryExport { .. } => history_render::overlay_lines(state, overlay)
            .expect("history overlay variants have a renderer"),
    };
    frame.render_widget(Clear, overlay_area);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().title(title).borders(Borders::ALL)),
        overlay_area,
    );
}

fn render_resize_overlay(frame: &mut Frame<'_>, area: Rect) {
    let overlay_area = centered(area, 48, 7);
    frame.render_widget(Clear, overlay_area);
    frame.render_widget(
        Paragraph::new(vec![
            heading("Terminal too small"),
            Line::raw(""),
            Line::raw(format!("Resize to at least {MIN_WIDTH}x{MIN_HEIGHT}.")),
            Line::raw(format!("Current size: {}x{}", area.width, area.height)),
        ])
        .wrap(Wrap { trim: false })
        .block(Block::default().title("Resize").borders(Borders::ALL)),
        overlay_area,
    );
}

fn heading(text: &'static str) -> Line<'static> {
    Line::styled(text, Style::default().add_modifier(Modifier::BOLD))
}

fn inset(area: Rect, horizontal: u16, vertical: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(horizontal),
        y: area.y.saturating_add(vertical),
        width: area.width.saturating_sub(horizontal.saturating_mul(2)),
        height: area.height.saturating_sub(vertical.saturating_mul(2)),
    }
}

fn centered(area: Rect, preferred_width: u16, preferred_height: u16) -> Rect {
    let width = preferred_width.min(area.width.saturating_sub(2).max(1));
    let height = preferred_height.min(area.height.saturating_sub(2).max(1));
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn sanitize_multiline(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for (index, line) in value.split('\n').enumerate() {
        if index != 0 {
            output.push('\n');
        }
        output.push_str(&crate::output::sanitize_terminal(line));
    }
    output
}

#[cfg(test)]
#[path = "render-composer-tests.rs"]
mod composer_tests;

#[cfg(test)]
#[path = "render-test-support.rs"]
mod test_support;

#[cfg(test)]
#[path = "render-tests.rs"]
mod tests;
