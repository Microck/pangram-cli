//! Mouse hit testing at the terminal adapter boundary.
//!
//! Rendering and hit testing share the same layout helpers. This module turns
//! terminal coordinates into typed reducer intents, so mouse support cannot
//! grow a second implementation of analysis, history, or settings behavior.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use super::history_render;
use super::model::{
    AppState, Focus, KeyInput, PointerDirection, PointerIntent, ResponsiveLayout, Route,
};
use super::render::overlay_render::{self, OverlayTarget};
use super::render::{
    action_width, active_empty_action_area, analysis_action_label, analysis_can_reset,
    analyze_composer_area, analyze_dock_area, analyze_inspector_rows, analyze_rows,
    analyze_toolbar_area, analyze_toolbar_areas, command_controls_area, inset, screen_areas,
    selector_width, setting_control, settings_control_width, settings_rows, shortcut_width,
    toggle_width, unavailable_toggle_width, workspace_content_area,
};

pub(super) fn pointer_intent(mouse: MouseEvent, state: &AppState) -> Option<PointerIntent> {
    if state.layout() == ResponsiveLayout::ResizeRequired {
        return None;
    }
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => click_intent(mouse.column, mouse.row, state),
        MouseEventKind::ScrollUp => scroll_intent(mouse.column, mouse.row, state, true),
        MouseEventKind::ScrollDown => scroll_intent(mouse.column, mouse.row, state, false),
        _ => None,
    }
}

fn click_intent(column: u16, row: u16, state: &AppState) -> Option<PointerIntent> {
    let frame = terminal_area(state);
    if state.overlay.is_some() {
        return overlay_intent(column, row, frame, state);
    }

    let areas = state_screen_areas(frame, state);
    if contains(areas.command, column, row) {
        return command_intent(column, row, areas, state);
    }
    if contains(areas.routes, column, row) {
        return route_intent(column, row, areas.routes, areas.wide);
    }
    if contains(areas.workspace, column, row) {
        return workspace_intent(column, row, areas.workspace, state);
    }
    if contains(areas.inspector, column, row) {
        return inspector_intent(column, row, areas.inspector, areas.wide, state);
    }
    None
}

fn scroll_intent(column: u16, row: u16, state: &AppState, previous: bool) -> Option<PointerIntent> {
    if state.overlay.is_some() {
        return None;
    }
    let areas = state_screen_areas(terminal_area(state), state);
    if !contains(areas.workspace, column, row) {
        return None;
    }
    let focus = match state.route {
        Route::Analyze if state.analysis.current.is_some() => Focus::Result,
        Route::Active => Focus::ActiveList,
        Route::History if state.history.selected_detail().is_some() => Focus::Result,
        Route::History => Focus::HistoryList,
        Route::Analyze | Route::Settings => return None,
    };
    Some(PointerIntent::Scroll {
        focus,
        direction: if previous {
            PointerDirection::Previous
        } else {
            PointerDirection::Next
        },
    })
}

fn route_intent(column: u16, row: u16, area: Rect, wide: bool) -> Option<PointerIntent> {
    if wide {
        let content = inset(area, 1, 1);
        let index = usize::from(row.checked_sub(content.y)?.checked_sub(2)?) / 2;
        if row == content.y + 2 + u16::try_from(index).ok()?.saturating_mul(2) {
            return Route::ALL.get(index).copied().map(PointerIntent::Route);
        }
        return None;
    }

    let mut start = area.x.saturating_add(16);
    for route in Route::ALL {
        let width = u16::try_from(route.name().len()).unwrap_or(u16::MAX) + 2;
        if row == area.y && column >= start && column < start.saturating_add(width) {
            return Some(PointerIntent::Route(route));
        }
        start = start.saturating_add(width).saturating_add(1);
    }
    None
}

fn workspace_intent(column: u16, row: u16, area: Rect, state: &AppState) -> Option<PointerIntent> {
    let content = workspace_content_area(area);
    match state.route {
        Route::Analyze => analyze_intent(column, row, content, state),
        Route::Active => active_intent(column, row, content, state),
        Route::History => history_intent(column, row, content, state),
        Route::Settings => settings_intent(column, row, content, state),
    }
}

fn analyze_intent(column: u16, row: u16, content: Rect, state: &AppState) -> Option<PointerIntent> {
    if state.analysis.current.is_some() {
        return Some(PointerIntent::Focus(Focus::Result));
    }
    if state.analysis.submitting || analysis_can_reset(state) {
        return None;
    }

    let dock = analyze_dock_area(content);
    let rows = analyze_rows();
    if row == dock.y.saturating_add(rows.check) {
        return match horizontal_control_hit(
            column,
            dock.x.saturating_add(7),
            1,
            &[
                selector_width("AI detection", true),
                selector_width("Plagiarism", true),
                selector_width("Both", true),
            ],
        ) {
            Some(0) => Some(PointerIntent::Activate(Focus::CheckAi)),
            Some(1) => Some(PointerIntent::Activate(Focus::CheckPlagiarism)),
            Some(2) => Some(PointerIntent::Activate(Focus::CheckBoth)),
            _ => None,
        };
    }
    if row == dock.y.saturating_add(rows.input) {
        return match horizontal_control_hit(
            column,
            dock.x.saturating_add(7),
            1,
            &[selector_width("Text", true), selector_width("Files", false)],
        ) {
            Some(0) => Some(PointerIntent::Activate(Focus::InputText)),
            Some(1) => Some(PointerIntent::Activate(Focus::InputFiles)),
            _ => None,
        };
    }
    if contains(analyze_composer_area(content), column, row) {
        return Some(PointerIntent::Focus(Focus::Composer));
    }
    if state.layout() == ResponsiveLayout::Narrow {
        let toolbar = analyze_toolbar_area(content);
        let controls = analyze_toolbar_areas(
            toolbar,
            if state.public_link_available() {
                toggle_width("Public link", state.public_link)
            } else {
                unavailable_toggle_width("Public link", true)
            },
            toggle_width("Manual save", state.manual_save),
            action_width("Submit"),
        );
        if contains(controls.public_link, column, row) {
            if state.public_link_available() {
                return Some(PointerIntent::Activate(Focus::PublicLink));
            }
            return None;
        }
        if contains(controls.manual_save, column, row) {
            return Some(PointerIntent::Activate(Focus::ManualSave));
        }
        if contains(controls.submit, column, row) {
            return Some(PointerIntent::Activate(Focus::Submit));
        }
    }
    None
}

fn active_intent(column: u16, row: u16, content: Rect, state: &AppState) -> Option<PointerIntent> {
    if state.active.is_empty() {
        return contains(active_empty_action_area(content), column, row)
            .then_some(PointerIntent::Activate(Focus::ActiveList));
    }
    let row_start = content.y
        + if state.layout() == ResponsiveLayout::Wide {
            4
        } else {
            2
        };
    let index = usize::from(row.checked_sub(row_start)?);
    (index < state.active.visible_rows().len()).then_some(PointerIntent::ActiveRow(index))
}

fn history_intent(column: u16, row: u16, content: Rect, state: &AppState) -> Option<PointerIntent> {
    let filters = history_render::filter_target_areas(content, state);
    if contains(filters.search, column, row) {
        return Some(PointerIntent::Focus(Focus::HistorySearch));
    }
    if contains(filters.status, column, row) {
        return Some(PointerIntent::Activate(Focus::HistoryStatusFilter));
    }
    if contains(filters.check, column, row) {
        return Some(PointerIntent::Activate(Focus::HistoryCheckFilter));
    }
    let row_start = content.y + history_render::list_row_offset(state);
    let index = usize::from(row.checked_sub(row_start)?);
    (index < state.history.visible_items().len()).then_some(PointerIntent::HistoryRow(index))
}

fn settings_intent(
    column: u16,
    row: u16,
    content: Rect,
    state: &AppState,
) -> Option<PointerIntent> {
    let relative = row.checked_sub(content.y)?;
    let rows = settings_rows();
    let focus = match relative {
        offset if offset == rows.authentication => Focus::SettingsAuthentication,
        offset if offset == rows.history => Focus::SettingsHistory,
        offset if offset == rows.intro => Focus::SettingsIntro,
        offset if offset == rows.keymap => Focus::SettingsKeymap,
        offset if offset == rows.motion => Focus::SettingsMotion,
        offset if offset == rows.updates => Focus::SettingsUpdates,
        _ => return None,
    };
    let (_, value) = setting_control(state, focus)?;
    let target = Rect::new(
        content.x,
        content.y.saturating_add(relative),
        settings_control_width(value).min(content.width),
        1,
    );
    contains(target, column, row).then_some(PointerIntent::Activate(focus))
}

fn inspector_intent(
    column: u16,
    row: u16,
    area: Rect,
    wide: bool,
    state: &AppState,
) -> Option<PointerIntent> {
    let content = inset(area, if wide { 1 } else { 2 }, u16::from(wide));
    let analyze_rows = analyze_inspector_rows(wide);
    match state.route {
        Route::Analyze if wide => match row.checked_sub(content.y)? {
            offset
                if offset == analyze_rows.public_link
                    && state.public_link_available()
                    && column
                        < content
                            .x
                            .saturating_add(toggle_width("Public link", state.public_link)) =>
            {
                Some(PointerIntent::Activate(Focus::PublicLink))
            }
            offset
                if offset == analyze_rows.manual_save
                    && column
                        < content
                            .x
                            .saturating_add(toggle_width("Manual save", state.manual_save)) =>
            {
                Some(PointerIntent::Activate(Focus::ManualSave))
            }
            offset
                if offset == analyze_rows.submit
                    && column
                        < content
                            .x
                            .saturating_add(action_width(analysis_action_label(state))) =>
            {
                Some(PointerIntent::Activate(Focus::Submit))
            }
            _ => None,
        },
        Route::Analyze => match row.checked_sub(content.y)? {
            offset
                if offset == analyze_rows.public_link
                    && state.public_link_available()
                    && column
                        < content
                            .x
                            .saturating_add(toggle_width("Public link", state.public_link)) =>
            {
                Some(PointerIntent::Activate(Focus::PublicLink))
            }
            offset
                if offset == analyze_rows.manual_save
                    && column >= narrow_manual_save_x(content, state)
                    && column
                        < narrow_manual_save_x(content, state)
                            .saturating_add(toggle_width("Manual save", state.manual_save)) =>
            {
                Some(PointerIntent::Activate(Focus::ManualSave))
            }
            offset
                if offset == analyze_rows.submit
                    && column
                        < content
                            .x
                            .saturating_add(action_width(analysis_action_label(state))) =>
            {
                Some(PointerIntent::Activate(Focus::Submit))
            }
            _ => None,
        },
        Route::History => history_action_intent(column, row, content, state),
        Route::Active | Route::Settings => None,
    }
}

fn history_action_intent(
    column: u16,
    row: u16,
    content: Rect,
    state: &AppState,
) -> Option<PointerIntent> {
    let action_row = content.y.saturating_add(
        u16::try_from(
            history_render::inspector_lines(
                state,
                !matches!(state.layout(), ResponsiveLayout::Wide),
            )
            .len(),
        )
        .unwrap_or(u16::MAX)
        .saturating_sub(1),
    );
    if row != action_row {
        return None;
    }
    match horizontal_control_hit(
        column,
        content.x,
        1,
        &[
            action_width("Rerun"),
            action_width("Export"),
            action_width("Delete"),
        ],
    )? {
        0 => Some(PointerIntent::Activate(Focus::HistoryRerun)),
        1 => Some(PointerIntent::Activate(Focus::HistoryExport)),
        2 => Some(PointerIntent::Activate(Focus::HistoryDelete)),
        _ => None,
    }
}

fn command_intent(
    column: u16,
    row: u16,
    areas: super::render::ScreenAreas,
    state: &AppState,
) -> Option<PointerIntent> {
    let controls = command_controls_area(areas.command, areas.workspace);
    if !contains(controls, column, row) {
        return None;
    }
    let (focused_key, focused_label) = super::render::focused_command(state);
    let widths = [
        shortcut_width("tab", "next"),
        shortcut_width("shift+tab", "back"),
        shortcut_width("?", "help"),
        shortcut_width(focused_key, focused_label),
    ];
    match horizontal_control_hit(column, controls.x, 5, &widths)? {
        0 => Some(PointerIntent::Key(KeyInput::Tab)),
        1 => Some(PointerIntent::Key(KeyInput::BackTab)),
        2 => Some(PointerIntent::Key(KeyInput::Character('?'))),
        3 => Some(PointerIntent::Key(KeyInput::Enter)),
        _ => None,
    }
}

fn overlay_intent(column: u16, row: u16, frame: Rect, state: &AppState) -> Option<PointerIntent> {
    if !contains(overlay_render::areas(frame).outer, column, row) {
        return Some(PointerIntent::Key(KeyInput::Escape));
    }
    match overlay_render::target(column, row, frame, state)? {
        OverlayTarget::Primary => Some(PointerIntent::Key(KeyInput::Enter)),
        OverlayTarget::Secondary | OverlayTarget::Cancel | OverlayTarget::Dismiss => {
            Some(PointerIntent::Key(KeyInput::Escape))
        }
        OverlayTarget::Toggle => Some(PointerIntent::Key(KeyInput::Right)),
        OverlayTarget::Confirm => Some(PointerIntent::Key(KeyInput::Character('y'))),
        OverlayTarget::ExportField(field) => Some(PointerIntent::HistoryExportField(field)),
    }
}

fn terminal_area(state: &AppState) -> Rect {
    Rect::new(0, 0, state.terminal.columns, state.terminal.rows)
}

fn state_screen_areas(frame: Rect, state: &AppState) -> super::render::ScreenAreas {
    let inspector_height = if frame.width < super::model::WIDE_WIDTH {
        u16::try_from(super::render::inspector_lines(state, true).len()).unwrap_or(u16::MAX)
    } else {
        0
    };
    screen_areas(frame, inspector_height, state.route)
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    area.contains(Position::new(column, row))
}

fn narrow_manual_save_x(content: Rect, state: &AppState) -> u16 {
    let public_link_width = if state.public_link_available() {
        toggle_width("Public link", state.public_link)
    } else {
        unavailable_toggle_width("Public link", false)
    };
    content
        .x
        .saturating_add(public_link_width)
        .saturating_add(3)
}

fn horizontal_control_hit(column: u16, start: u16, gap: u16, widths: &[u16]) -> Option<usize> {
    let mut control_start = start;
    for (index, width) in widths.iter().copied().enumerate() {
        if column >= control_start && column < control_start.saturating_add(width) {
            return Some(index);
        }
        control_start = control_start.saturating_add(width).saturating_add(gap);
    }
    None
}
