//! Deterministic terminal rendering for the pure TUI state machine.
//!
//! This one-way projection performs no I/O and never changes `AppState`.
//! Color establishes hierarchy while ASCII cues preserve every state when
//! color is disabled.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap};

use super::history_render;
use super::model::{
    AppState, ColorMode, Focus, IntroFrequency, MIN_HEIGHT, MIN_WIDTH, MotionLevel, Route,
    WIDE_WIDTH,
};
use super::result_lines::{analysis_status_label, sanitize_single_line};
use super::result_viewport::visible_analysis_result_lines;
use crate::analysis::TextAnalysisMode;

#[path = "inspector-render.rs"]
mod inspector_render;
#[path = "overlay-render.rs"]
pub(super) mod overlay_render;
#[path = "render-layout.rs"]
mod render_layout;
#[path = "render-style.rs"]
mod render_style;

pub(super) use render_style::{
    action_style, base_style, body_style, canvas_color, control_style, element_style,
    fade_from_black, muted_style, panel_style, primary_style, route_style, separator_style,
};

pub(super) use render_layout::{
    ScreenAreas, action_width, active_empty_action_area, active_empty_state_area,
    analyze_composer_area, analyze_dock_area, analyze_empty_area, analyze_inspector_rows,
    analyze_rows, analyze_toolbar_area, analyze_toolbar_areas, centered, command_controls_area,
    inset, labeled_control_width, screen_areas, selector_width, settings_control_width,
    settings_label_width, settings_rows, shortcut_width, toggle_width, unavailable_toggle_width,
    workspace_content_area,
};

pub(super) fn inspector_lines(state: &AppState, narrow: bool) -> Vec<Line<'static>> {
    inspector_render::lines(state, narrow)
}

/// Draws the current application state into the terminal frame.
pub(super) fn render(frame: &mut Frame<'_>, state: &AppState) {
    let area = frame.area();
    if state.color_mode != ColorMode::None {
        // Paint the whole frame so empty cells and rendered text share the
        // same dark neutral canvas. Later widgets inherit this base style.
        frame.render_widget(Block::default().style(base_style(state.color_mode)), area);
    }
    if area.width >= WIDE_WIDTH {
        let areas = screen_areas(area, 0, state.route);
        render_wide(frame, state, areas);
    } else {
        let inspector_lines = inspector_lines(state, true);
        let inspector_height = u16::try_from(inspector_lines.len()).unwrap_or(u16::MAX);
        let areas = screen_areas(area, inspector_height, state.route);
        render_narrow(frame, state, areas, inspector_lines);
    }

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_resize_overlay(frame, area, state.color_mode);
    } else if let Some(overlay) = &state.overlay {
        overlay_render::render(frame, area, state, overlay);
    }
}

/// Renders the real application frame, then blends its semantic colors from
/// the base canvas. Symbols and layout are already final during the fade.
pub(super) fn render_faded(frame: &mut Frame<'_>, state: &AppState, opacity: u16) {
    render(frame, state);
    render_style::fade_buffer(frame.buffer_mut(), state.color_mode, opacity);
}

fn render_wide(frame: &mut Frame<'_>, state: &AppState, areas: ScreenAreas) {
    fill_area(frame, areas.routes, panel_style(state.color_mode));
    render_route_rail(frame, areas.routes, state);
    render_vertical_separator(
        frame,
        areas
            .route_separator
            .expect("wide screen owns a route separator"),
        state.color_mode,
    );
    render_workspace(frame, areas.workspace, state);
    render_inspector(
        frame,
        areas.inspector,
        inspector_render::lines(state, false),
        true,
    );
    render_command_bar(frame, areas, state);
}

fn render_narrow(
    frame: &mut Frame<'_>,
    state: &AppState,
    areas: ScreenAreas,
    inspector_lines: Vec<Line<'static>>,
) {
    fill_area(frame, areas.routes, panel_style(state.color_mode));
    render_route_tabs(frame, areas.routes, state);
    render_workspace(frame, areas.workspace, state);
    render_inspector(frame, areas.inspector, inspector_lines, false);
    render_command_bar(frame, areas, state);
}

fn render_route_rail(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let mut lines = vec![
        Line::styled("Pangram", primary_style(state.color_mode)),
        Line::raw(""),
    ];
    for (index, route) in Route::ALL.into_iter().enumerate() {
        lines.push(Line::from(route_spans(route, state).to_vec()));
        if index + 1 != Route::ALL.len() {
            lines.push(Line::raw(""));
        }
    }
    frame.render_widget(Paragraph::new(lines), inset(area, 1, 1));
}

fn render_route_tabs(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    frame.render_widget(
        Paragraph::new(Line::styled("pangram", primary_style(state.color_mode))),
        Rect::new(area.x.saturating_add(2), area.y, 7, area.height),
    );
    let mut spans = Vec::new();
    for (index, route) in Route::ALL.into_iter().enumerate() {
        if index != 0 {
            spans.push(Span::raw(" "));
        }
        let selected = state.route == route;
        let focused = state.focus == Focus::Routes && selected;
        spans.push(Span::styled(
            if state.color_mode == ColorMode::None && selected {
                format!("{} {}", if focused { ">" } else { "*" }, route.name())
            } else {
                format!(" {} ", route.name())
            },
            route_style(state.color_mode, selected),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(
            area.x.saturating_add(16),
            area.y,
            area.width.saturating_sub(16),
            area.height,
        ),
    );
}

fn route_spans(route: Route, state: &AppState) -> [Span<'static>; 2] {
    let selected = state.route == route;
    let focused = state.focus == Focus::Routes && selected;
    let style = route_style(state.color_mode, selected);
    let marker = match (state.color_mode, focused, selected) {
        (ColorMode::None, true, true) => ">",
        (ColorMode::None, false, true) => "*",
        (_, true, _) => ">",
        _ => " ",
    };
    [
        Span::styled(marker, primary_style(state.color_mode)),
        Span::styled(format!(" {} ", route.name()), style),
    ]
}

fn render_vertical_separator(frame: &mut Frame<'_>, area: Rect, color_mode: ColorMode) {
    let buffer = frame.buffer_mut();
    for y in area.y..area.y.saturating_add(area.height) {
        buffer[(area.x, y)]
            .set_symbol("|")
            .set_style(separator_style(color_mode));
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

    let content_area = workspace_content_area(area);
    let dock = analyze_dock_area(content_area);
    let rows = analyze_rows();
    let choices = Rect {
        height: rows.composer.min(content_area.height),
        ..dock
    };
    let composer = analyze_composer_area(content_area);
    let mut choice_lines = vec![Line::raw(""); usize::from(rows.composer)];
    choice_lines[usize::from(rows.check)] = selector_line(
        state.color_mode,
        "Check",
        &[
            (
                state.focus == Focus::CheckAi,
                state.text_mode == TextAnalysisMode::Detection,
                "AI detection",
                true,
            ),
            (
                state.focus == Focus::CheckPlagiarism,
                state.text_mode == TextAnalysisMode::Plagiarism,
                "Plagiarism",
                true,
            ),
            (
                state.focus == Focus::CheckBoth,
                state.text_mode == TextAnalysisMode::Combined,
                "Both",
                true,
            ),
        ],
    );
    choice_lines[usize::from(rows.input)] = selector_line(
        state.color_mode,
        "Input",
        &[
            (state.focus == Focus::InputText, true, "Text", true),
            (state.focus == Focus::InputFiles, false, "Files", false),
        ],
    );
    frame.render_widget(Paragraph::new(choice_lines), choices);
    render_analyze_empty_state(frame, analyze_empty_area(content_area), state.color_mode);

    let composer_label = if state.focus == Focus::Composer {
        "> Text composer"
    } else {
        "  Text composer"
    };
    let composer_text = sanitize_multiline(state.composer.value());
    let cursor_prefix = state.composer.before_cursor();
    let cursor_row = cursor_prefix.matches('\n').count();
    let cursor_column = Line::raw(crate::output::sanitize_terminal(
        cursor_prefix.rsplit('\n').next().unwrap_or_default(),
    ))
    .width();
    let composer_border = if state.focus == Focus::Composer {
        primary_style(state.color_mode)
    } else {
        muted_style(state.color_mode)
    };
    let composer_body = Rect {
        y: composer.y.saturating_add(1),
        height: composer.height.saturating_sub(1),
        ..composer
    };
    frame.render_widget(
        Paragraph::new(Line::styled(composer_label, composer_border)),
        Rect {
            height: composer.height.min(1),
            ..composer
        },
    );
    let composer_block = Block::default()
        .borders(Borders::TOP)
        .border_style(composer_border)
        .style(panel_style(state.color_mode))
        .padding(Padding::new(2, 1, 1, 0));
    let composer_inner = composer_block.inner(composer_body);
    // The edit position owns the derived viewport. Keeping it out of AppState
    // prevents scroll state from drifting after paste, resize, or cursor edits.
    let scroll_x =
        cursor_column.saturating_sub(usize::from(composer_inner.width).saturating_sub(1));
    let scroll_y = cursor_row.saturating_sub(usize::from(composer_inner.height).saturating_sub(1));
    let content = if composer_text.is_empty() {
        Text::from(Line::styled(
            "Type or paste text here",
            muted_style(state.color_mode),
        ))
    } else {
        Text::from(composer_text)
    };
    frame.render_widget(
        Paragraph::new(content)
            .scroll((
                u16::try_from(scroll_y).unwrap_or(u16::MAX),
                u16::try_from(scroll_x).unwrap_or(u16::MAX),
            ))
            .block(composer_block),
        composer_body,
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
    if state.layout() == super::model::ResponsiveLayout::Narrow {
        render_analyze_toolbar(frame, analyze_toolbar_area(content_area), state);
    }
}

fn render_analyze_empty_state(frame: &mut Frame<'_>, area: Rect, color_mode: ColorMode) {
    if area.height < 4 {
        return;
    }
    let content = centered(area, 44, 3);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("Ready to analyze", primary_style(color_mode)),
            Line::raw(""),
            Line::styled(
                "Type or paste text below to begin.",
                muted_style(color_mode),
            ),
        ])
        .alignment(Alignment::Center),
        content,
    );
}

fn render_analyze_toolbar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.height == 0 {
        return;
    }
    let public_link = inspector_render::public_link_spans(state);
    let manual_save = inspector_render::toggle_spans(
        state.color_mode,
        state.focus == Focus::ManualSave,
        state.manual_save,
        "Manual save",
    );
    let controls = analyze_toolbar_areas(
        area,
        if state.public_link_available() {
            toggle_width("Public link", state.public_link)
        } else {
            unavailable_toggle_width("Public link", true)
        },
        toggle_width("Manual save", state.manual_save),
        action_width("Submit"),
    );
    frame.render_widget(
        Paragraph::new(Line::from(public_link)),
        controls.public_link,
    );
    frame.render_widget(
        Paragraph::new(Line::from(manual_save)),
        controls.manual_save,
    );
    let (words, units) = state.billing_estimate();
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!(
                "  {words} words | Estimate {}",
                inspector_render::estimate_label(words, units)
            ),
            muted_style(state.color_mode),
        )),
        controls.stats,
    );
    frame.render_widget(
        Paragraph::new(inspector_render::action_line(
            state.color_mode,
            state.focus == Focus::Submit,
            "Submit",
        )),
        controls.submit,
    );
}

fn try_render_analysis_state(frame: &mut Frame<'_>, area: Rect, state: &AppState) -> bool {
    if !state.analysis.submitting && !analysis_can_reset(state) {
        return false;
    }
    let content_area = workspace_content_area(area);
    let mut lines = Vec::new();
    if state.layout() == super::model::ResponsiveLayout::Wide {
        lines.push(heading(state.color_mode, "Analyze"));
        lines.push(Line::raw(""));
    }
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
        let preamble_rows = wrapped_height(&lines, content_area.width);
        let result_rows = usize::from(content_area.height)
            .saturating_sub(preamble_rows)
            .saturating_sub(1);
        lines.extend(visible_analysis_result_lines(
            analysis,
            &state.result_viewport,
            state.focus == Focus::Result,
            usize::from(content_area.width),
            result_rows,
        ));
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        content_area,
    );
    true
}

fn selector_line(
    color_mode: ColorMode,
    group: &'static str,
    options: &[(bool, bool, &'static str, bool)],
) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("{group:<7}"), muted_style(color_mode))];
    for (index, &(focused, selected, label, available)) in options.iter().enumerate() {
        if index != 0 {
            spans.push(Span::raw(" "));
        }
        let style = control_style(color_mode, focused, selected, available);
        let marker = match (color_mode, focused, selected) {
            (ColorMode::None, true, true) => ">*",
            (ColorMode::None, true, false) => "> ",
            (ColorMode::None, false, true) => "* ",
            (_, true, _) => "> ",
            _ => "  ",
        };
        if color_mode == ColorMode::None {
            spans.push(Span::styled(
                format!(
                    "{marker}{label}{} ",
                    if available { "" } else { " (unavailable)" }
                ),
                style,
            ));
        } else {
            spans.push(Span::styled(
                if focused { ">" } else { " " },
                primary_style(color_mode),
            ));
            spans.push(Span::styled(
                format!(" {label}{} ", if available { "" } else { " (unavailable)" }),
                style,
            ));
        }
    }
    Line::from(spans)
}

fn render_active(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let content_area = workspace_content_area(area);
    let mut content = Vec::new();
    if state.layout() == super::model::ResponsiveLayout::Wide {
        content.push(heading(state.color_mode, "Active"));
        content.push(Line::raw(""));
    }
    if state.active.is_empty() {
        frame.render_widget(Paragraph::new(content), content_area);
        let empty = active_empty_state_area(content_area);
        frame.render_widget(
            Paragraph::new(vec![
                heading(state.color_mode, "Nothing running"),
                Line::raw(""),
                Line::styled(
                    "Start a new analysis when you're ready.",
                    muted_style(state.color_mode),
                ),
            ])
            .alignment(Alignment::Center),
            Rect {
                height: empty.height.saturating_sub(1),
                ..empty
            },
        );
        frame.render_widget(
            Paragraph::new(inspector_render::action_line(
                state.color_mode,
                state.focus == Focus::ActiveList,
                "Analyze",
            )),
            active_empty_action_area(content_area),
        );
        return;
    } else {
        content.push(Line::styled(
            format!("{} unfinished", state.active.len()),
            muted_style(state.color_mode),
        ));
        content.push(Line::raw(""));
    }
    for row in state.active.visible_rows() {
        let selected = state.active.selected_id() == Some(row.id);
        let focused = selected && state.focus == Focus::ActiveList;
        content.push(Line::from(vec![
            Span::styled(
                if focused { ">" } else { " " },
                primary_style(state.color_mode),
            ),
            Span::styled(
                format!(
                    " {} - {} - {} ",
                    row.id,
                    analysis_status_label(row.status),
                    row.source.label(),
                ),
                if focused {
                    action_style(state.color_mode)
                } else {
                    element_style(state.color_mode)
                },
            ),
        ]));
    }
    frame.render_widget(
        Paragraph::new(content).wrap(Wrap { trim: false }),
        content_area,
    );
}

fn render_settings(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let rows = settings_rows();
    let mut lines = vec![Line::raw(""); usize::from(rows.diagnostics + 1)];
    lines[usize::from(rows.account)] = heading(state.color_mode, "Account");
    lines[usize::from(rows.preferences)] = heading(state.color_mode, "Preferences");
    lines[usize::from(rows.diagnostics_heading)] = heading(state.color_mode, "Diagnostics");
    for (row, focus) in [
        (rows.authentication, Focus::SettingsAuthentication),
        (rows.history, Focus::SettingsHistory),
        (rows.intro, Focus::SettingsIntro),
        (rows.keymap, Focus::SettingsKeymap),
        (rows.motion, Focus::SettingsMotion),
        (rows.updates, Focus::SettingsUpdates),
    ] {
        let (label, value) = setting_control(state, focus)
            .expect("every rendered Settings focus has one labeled control");
        lines[usize::from(row)] = setting_line(state, state.focus == focus, label, value);
    }
    lines[usize::from(rows.diagnostics)] = Line::from(vec![
        Span::raw("  "),
        Span::styled("Run `pangram doctor`", muted_style(state.color_mode)),
    ]);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        workspace_content_area(area),
    );
}

fn setting_line(state: &AppState, focused: bool, label: &str, value: &str) -> Line<'static> {
    debug_assert_eq!(
        settings_control_width(value),
        u16::try_from(settings_label_width() + value.len() + 4).unwrap_or(u16::MAX)
    );
    Line::from(vec![
        Span::styled(focus_marker(focused), primary_style(state.color_mode)),
        Span::styled(
            format!("{label:<width$}", width = settings_label_width()),
            body_style(state.color_mode),
        ),
        Span::styled(
            format!(" {value} "),
            if focused {
                action_style(state.color_mode)
            } else {
                muted_style(state.color_mode)
            },
        ),
    ])
}

pub(super) fn labeled_control_spans(
    color_mode: ColorMode,
    focused: bool,
    label: &str,
    value: &str,
) -> Vec<Span<'static>> {
    debug_assert_eq!(
        labeled_control_width(label, value),
        u16::try_from(label.len() + value.len() + 5).unwrap_or(u16::MAX)
    );
    vec![
        Span::styled(if focused { ">" } else { " " }, primary_style(color_mode)),
        Span::styled(
            format!(" {label}  {value} "),
            if focused {
                action_style(color_mode)
            } else {
                element_style(color_mode)
            },
        ),
    ]
}

pub(super) fn setting_control(
    state: &AppState,
    focus: Focus,
) -> Option<(&'static str, &'static str)> {
    match focus {
        Focus::SettingsAuthentication => Some((
            "Authentication",
            if state.settings.credential_present {
                "configured"
            } else {
                "not configured"
            },
        )),
        Focus::SettingsHistory => Some((
            "History",
            if state.settings.history_enabled {
                "enabled"
            } else {
                "disabled"
            },
        )),
        Focus::SettingsIntro => Some(("Intro frequency", intro_label(state.settings.intro))),
        Focus::SettingsKeymap => Some(("Keymap", state.keymap.name())),
        Focus::SettingsMotion => Some(("Motion", motion_label(state.settings.motion))),
        Focus::SettingsUpdates => Some((
            "Updates",
            match state.settings.update_preference {
                Some(true) => "enabled",
                Some(false) => "disabled",
                None => "not set",
            },
        )),
        _ => None,
    }
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

fn render_inspector(frame: &mut Frame<'_>, area: Rect, lines: Vec<Line<'static>>, wide: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inset(area, if wide { 1 } else { 2 }, u16::from(wide)),
    );
}

fn render_command_bar(frame: &mut Frame<'_>, areas: ScreenAreas, state: &AppState) {
    let mut spans = Vec::new();
    push_shortcut(&mut spans, state.color_mode, "tab", "next", false);
    push_shortcut(&mut spans, state.color_mode, "shift+tab", "back", false);
    push_shortcut(&mut spans, state.color_mode, "?", "help", false);
    let (key, label) = focused_command(state);
    push_shortcut(&mut spans, state.color_mode, key, label, true);
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        command_controls_area(areas.command, areas.workspace),
    );
}

fn render_resize_overlay(frame: &mut Frame<'_>, area: Rect, color_mode: ColorMode) {
    let overlay_area = centered(area, 48, 7);
    frame.render_widget(Clear, overlay_area);
    frame.render_widget(
        Paragraph::new(vec![
            heading(color_mode, "Terminal too small"),
            Line::raw(""),
            Line::raw(format!("Resize to at least {MIN_WIDTH}x{MIN_HEIGHT}.")),
            Line::raw(format!("Current size: {}x{}", area.width, area.height)),
        ])
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(Line::styled("Resize", primary_style(color_mode)))
                .borders(Borders::ALL)
                .border_style(primary_style(color_mode))
                .style(panel_style(color_mode)),
        ),
        overlay_area,
    );
}

pub(super) fn heading(color_mode: ColorMode, text: &'static str) -> Line<'static> {
    Line::styled(text, primary_style(color_mode))
}

fn fill_area(frame: &mut Frame<'_>, area: Rect, style: Style) {
    if area.width != 0 && area.height != 0 {
        frame.render_widget(Block::default().style(style), area);
    }
}

pub(super) fn analysis_can_reset(state: &AppState) -> bool {
    state.analysis.current.is_some()
        || state.analysis.progress.is_some()
        || state.analysis.failure.is_some()
}

pub(super) fn analysis_action_label(state: &AppState) -> &'static str {
    if analysis_can_reset(state) {
        "New analysis"
    } else {
        "Submit"
    }
}

pub(super) const fn focus_marker(focused: bool) -> &'static str {
    if focused { "> " } else { "  " }
}

pub(super) fn focused_command(state: &AppState) -> (&'static str, &'static str) {
    match state.focus {
        Focus::Routes => ("arrows", "route"),
        Focus::CheckAi | Focus::CheckPlagiarism | Focus::CheckBoth | Focus::InputText => {
            ("enter", "select")
        }
        Focus::InputFiles => ("enter", "unavailable"),
        Focus::Composer => ("enter", "newline"),
        Focus::PublicLink | Focus::ManualSave => ("enter", "toggle"),
        Focus::Submit => (
            "enter",
            if analysis_can_reset(state) {
                "new analysis"
            } else {
                "submit"
            },
        ),
        Focus::Result => ("arrows/page", "result"),
        Focus::ActiveList if state.active.is_empty() => ("enter", "analyze"),
        Focus::ActiveList => ("arrows", "select"),
        Focus::HistorySearch => ("enter", "search"),
        Focus::HistoryStatusFilter | Focus::HistoryCheckFilter => ("enter", "change"),
        Focus::HistoryList => ("enter", "open"),
        Focus::HistoryRerun => ("enter", "rerun"),
        Focus::HistoryExport => ("enter", "export"),
        Focus::HistoryDelete => ("enter", "delete"),
        Focus::SettingsAuthentication
        | Focus::SettingsHistory
        | Focus::SettingsIntro
        | Focus::SettingsKeymap
        | Focus::SettingsMotion
        | Focus::SettingsUpdates => ("enter", "change"),
        Focus::Quit => ("enter", "quit"),
    }
}

pub(super) fn wrapped_height(lines: &[Line<'_>], width: u16) -> usize {
    let width = usize::from(width).max(1);
    lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(width))
        .sum()
}

fn push_shortcut(
    spans: &mut Vec<Span<'static>>,
    color_mode: ColorMode,
    key: &'static str,
    label: &'static str,
    contextual: bool,
) {
    debug_assert_eq!(
        shortcut_width(key, label),
        u16::try_from(key.len() + label.len() + 3).unwrap_or(u16::MAX)
    );
    if !spans.is_empty() {
        spans.push(Span::styled("  |  ", muted_style(color_mode)));
    }
    spans.push(Span::styled(
        format!(" {key} "),
        if contextual {
            action_style(color_mode)
        } else {
            element_style(color_mode).fg(match color_mode {
                ColorMode::None => Color::Reset,
                ColorMode::Ansi => Color::Indexed(202),
                ColorMode::TrueColor => Color::Rgb(255, 97, 6),
            })
        },
    ));
    spans.push(Span::styled(format!(" {label}"), muted_style(color_mode)));
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
#[path = "render-wrapped-result-tests.rs"]
mod wrapped_result_tests;

#[cfg(test)]
#[path = "render-layout-tests.rs"]
mod layout_tests;

#[cfg(test)]
#[path = "render-tests.rs"]
mod tests;
