//! Shared pane and control geometry for rendering and pointer hit testing.

use ratatui::layout::{Constraint, Layout, Rect};

use crate::tui::model::{Route, WIDE_WIDTH};

const ROUTE_RAIL_WIDTH: u16 = 14;
const INSPECTOR_WIDTH: u16 = 24;
const COMPACT_COMMAND_BAR_HEIGHT: u16 = 1;
const SPACIOUS_COMMAND_BAR_HEIGHT: u16 = 3;
const SPACIOUS_NON_ANALYZE_MIN_HEIGHT: u16 = 26;
const NARROW_TABS_HEIGHT: u16 = 1;
const WORKSPACE_HORIZONTAL_INSET: u16 = 2;
const WORKSPACE_TOP_INSET: u16 = 1;
const ANALYZE_DOCK_HEIGHT: u16 = 15;
const ANALYZE_COMPOSER_HEIGHT: u16 = 7;
const ANALYZE_TOOLBAR_GAP: u16 = 1;
const SETTINGS_LABEL_WIDTH: usize = 18;

#[derive(Clone, Copy)]
pub(in crate::tui) struct AnalyzeToolbarAreas {
    pub(in crate::tui) public_link: Rect,
    pub(in crate::tui) manual_save: Rect,
    pub(in crate::tui) stats: Rect,
    pub(in crate::tui) submit: Rect,
}

#[derive(Clone, Copy)]
pub(in crate::tui) struct ScreenAreas {
    pub(in crate::tui) routes: Rect,
    pub(in crate::tui) route_separator: Option<Rect>,
    pub(in crate::tui) workspace: Rect,
    pub(in crate::tui) inspector: Rect,
    pub(in crate::tui) command: Rect,
    pub(in crate::tui) wide: bool,
}

#[derive(Clone, Copy)]
pub(in crate::tui) struct AnalyzeRows {
    pub(in crate::tui) check: u16,
    pub(in crate::tui) input: u16,
    pub(in crate::tui) composer: u16,
    pub(in crate::tui) toolbar: u16,
}

pub(in crate::tui) const fn analyze_rows() -> AnalyzeRows {
    AnalyzeRows {
        check: 0,
        input: 2,
        composer: 5,
        toolbar: 13,
    }
}

/// Anchors the complete pre-submission workflow near the command bar. This
/// makes spare height an intentional result canvas instead of separating the
/// composer from its controls.
pub(in crate::tui) fn analyze_dock_area(area: Rect) -> Rect {
    let height = area.height.min(ANALYZE_DOCK_HEIGHT);
    Rect {
        x: area.x,
        y: area.y.saturating_add(area.height.saturating_sub(height)),
        width: area.width,
        height,
    }
}

pub(in crate::tui) fn analyze_empty_area(area: Rect) -> Rect {
    let dock = analyze_dock_area(area);
    Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: dock.y.saturating_sub(area.y),
    }
}

/// Rendering and mouse hit testing share this exact editor rectangle.
pub(in crate::tui) fn analyze_composer_area(area: Rect) -> Rect {
    let dock = analyze_dock_area(area);
    let offset = analyze_rows().composer.min(dock.height);
    Rect {
        x: dock.x,
        y: dock.y.saturating_add(offset),
        width: dock.width,
        height: dock
            .height
            .saturating_sub(offset)
            .min(ANALYZE_COMPOSER_HEIGHT),
    }
}

pub(in crate::tui) fn analyze_toolbar_area(area: Rect) -> Rect {
    let dock = analyze_dock_area(area);
    let offset = analyze_rows().toolbar.min(dock.height);
    Rect {
        x: dock.x,
        y: dock.y.saturating_add(offset),
        width: dock.width,
        height: u16::from(offset < dock.height),
    }
}

/// Splits the narrow analysis toolbar into the exact rectangles rendered as
/// controls. Pointer handling consumes the same geometry, so nearby blank
/// cells never behave like buttons.
pub(in crate::tui) fn analyze_toolbar_areas(
    area: Rect,
    public_link_width: u16,
    manual_save_width: u16,
    submit_width: u16,
) -> AnalyzeToolbarAreas {
    let public_link_width = public_link_width.min(area.width);
    let public_link = Rect::new(area.x, area.y, public_link_width, area.height);
    let manual_x = public_link
        .right()
        .saturating_add(ANALYZE_TOOLBAR_GAP)
        .min(area.right());
    let manual_save_width = manual_save_width.min(area.right().saturating_sub(manual_x));
    let manual_save = Rect::new(manual_x, area.y, manual_save_width, area.height);
    let submit_width = submit_width.min(area.width);
    let submit = Rect::new(
        area.right().saturating_sub(submit_width),
        area.y,
        submit_width,
        area.height,
    );
    let stats_x = manual_save.right();
    let stats = Rect::new(
        stats_x,
        area.y,
        submit.x.saturating_sub(stats_x),
        area.height,
    );
    AnalyzeToolbarAreas {
        public_link,
        manual_save,
        stats,
        submit,
    }
}

pub(in crate::tui) fn toggle_width(label: &str, enabled: bool) -> u16 {
    let state_width = if enabled { 2 } else { 3 };
    u16::try_from(label.len())
        .unwrap_or(u16::MAX)
        .saturating_add(state_width)
        .saturating_add(4)
}

pub(in crate::tui) fn unavailable_toggle_width(label: &str, trailing_space: bool) -> u16 {
    u16::try_from(label.len())
        .unwrap_or(u16::MAX)
        .saturating_add(6)
        .saturating_add(u16::from(trailing_space))
}

pub(in crate::tui) fn action_width(label: &str) -> u16 {
    u16::try_from(label.len())
        .unwrap_or(u16::MAX)
        .saturating_add(3)
}

/// Width of one marker gutter plus a padded `label  value` target.
pub(in crate::tui) fn labeled_control_width(label: &str, value: &str) -> u16 {
    u16::try_from(label.len().saturating_add(value.len()).saturating_add(5)).unwrap_or(u16::MAX)
}

/// Width of the marker gutter, fixed label column, and padded value.
pub(in crate::tui) fn settings_control_width(value: &str) -> u16 {
    u16::try_from(
        SETTINGS_LABEL_WIDTH
            .saturating_add(value.len())
            .saturating_add(4),
    )
    .unwrap_or(u16::MAX)
}

pub(in crate::tui) const fn settings_label_width() -> usize {
    SETTINGS_LABEL_WIDTH
}

pub(in crate::tui) fn active_empty_state_area(area: Rect) -> Rect {
    centered(area, 48, 5)
}

pub(in crate::tui) fn active_empty_action_area(area: Rect) -> Rect {
    let empty = active_empty_state_area(area);
    let width = action_width("Analyze").min(empty.width);
    Rect::new(
        empty
            .x
            .saturating_add(empty.width.saturating_sub(width) / 2),
        empty.y.saturating_add(empty.height.saturating_sub(1)),
        width,
        u16::from(empty.height > 0),
    )
}

pub(in crate::tui) fn selector_width(label: &str, available: bool) -> u16 {
    u16::try_from(label.len())
        .unwrap_or(u16::MAX)
        .saturating_add(3)
        .saturating_add(if available { 0 } else { 14 })
}

pub(in crate::tui) fn shortcut_width(key: &str, label: &str) -> u16 {
    u16::try_from(key.len().saturating_add(label.len()).saturating_add(3)).unwrap_or(u16::MAX)
}

#[derive(Clone, Copy)]
pub(in crate::tui) struct AnalyzeInspectorRows {
    pub(in crate::tui) public_link: u16,
    pub(in crate::tui) manual_save: u16,
    pub(in crate::tui) words: u16,
    pub(in crate::tui) estimate: u16,
    pub(in crate::tui) submit: u16,
    pub(in crate::tui) height: u16,
}

pub(in crate::tui) const fn analyze_inspector_rows(wide: bool) -> AnalyzeInspectorRows {
    if wide {
        AnalyzeInspectorRows {
            public_link: 2,
            manual_save: 4,
            words: 7,
            estimate: 9,
            submit: 12,
            height: 13,
        }
    } else {
        AnalyzeInspectorRows {
            public_link: 0,
            manual_save: 0,
            words: 3,
            estimate: 3,
            submit: 6,
            height: 8,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::tui) struct SettingsRows {
    pub(in crate::tui) account: u16,
    pub(in crate::tui) authentication: u16,
    pub(in crate::tui) preferences: u16,
    pub(in crate::tui) history: u16,
    pub(in crate::tui) intro: u16,
    pub(in crate::tui) keymap: u16,
    pub(in crate::tui) motion: u16,
    pub(in crate::tui) updates: u16,
    pub(in crate::tui) diagnostics_heading: u16,
    pub(in crate::tui) diagnostics: u16,
}

pub(in crate::tui) const fn settings_rows() -> SettingsRows {
    SettingsRows {
        account: 0,
        authentication: 2,
        preferences: 5,
        history: 7,
        intro: 9,
        keymap: 11,
        motion: 13,
        updates: 15,
        diagnostics_heading: 18,
        diagnostics: 20,
    }
}

pub(in crate::tui) fn screen_areas(
    area: Rect,
    narrow_inspector_height: u16,
    route: Route,
) -> ScreenAreas {
    let spacious = route != Route::Analyze && area.height >= SPACIOUS_NON_ANALYZE_MIN_HEIGHT;
    let command_height = if spacious {
        SPACIOUS_COMMAND_BAR_HEIGHT
    } else {
        COMPACT_COMMAND_BAR_HEIGHT
    };
    let [body, command] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(command_height)]).areas(area);
    if area.width < WIDE_WIDTH {
        let [routes, workspace, inspector] = Layout::vertical([
            Constraint::Length(NARROW_TABS_HEIGHT),
            Constraint::Min(1),
            Constraint::Length(narrow_inspector_height),
        ])
        .areas(body);
        return ScreenAreas {
            routes,
            route_separator: None,
            workspace,
            inspector,
            command,
            wide: false,
        };
    }

    let [rail, rail_separator, workspace, _, inspector] = Layout::horizontal([
        Constraint::Length(ROUTE_RAIL_WIDTH),
        Constraint::Length(1),
        Constraint::Min(50),
        Constraint::Length(2),
        Constraint::Length(INSPECTOR_WIDTH),
    ])
    .areas(body);
    // The rail is the terminal's full-height navigation surface. Only the
    // workspace and inspector yield their bottom rows to the command band.
    let rail = Rect {
        height: area.bottom().saturating_sub(rail.y),
        ..rail
    };
    let rail_separator = Rect {
        height: area.bottom().saturating_sub(rail_separator.y),
        ..rail_separator
    };
    let command = Rect {
        x: workspace.x,
        width: area.right().saturating_sub(workspace.x),
        ..command
    };
    ScreenAreas {
        routes: rail,
        route_separator: Some(rail_separator),
        workspace,
        inspector,
        command,
        wide: true,
    }
}

/// Aligns wide command controls with the workspace instead of the route rail.
/// The spacious bar uses its middle row; the compact bar uses its only row.
pub(in crate::tui) fn command_controls_area(area: Rect, workspace: Rect) -> Rect {
    let x = workspace.x.clamp(area.x, area.right());
    inset(
        Rect {
            x,
            y: area.y.saturating_add(area.height.saturating_sub(1) / 2),
            width: area.right().saturating_sub(x),
            height: u16::from(area.height > 0),
        },
        WORKSPACE_HORIZONTAL_INSET,
        0,
    )
}

pub(in crate::tui) fn inset(area: Rect, horizontal: u16, vertical: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(horizontal),
        y: area.y.saturating_add(vertical),
        width: area.width.saturating_sub(horizontal.saturating_mul(2)),
        height: area.height.saturating_sub(vertical.saturating_mul(2)),
    }
}

pub(in crate::tui) fn workspace_content_area(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(WORKSPACE_HORIZONTAL_INSET),
        y: area.y.saturating_add(WORKSPACE_TOP_INSET),
        width: area
            .width
            .saturating_sub(WORKSPACE_HORIZONTAL_INSET.saturating_mul(2)),
        height: area.height.saturating_sub(WORKSPACE_TOP_INSET),
    }
}

pub(in crate::tui) fn centered(area: Rect, preferred_width: u16, preferred_height: u16) -> Rect {
    let width = preferred_width.min(area.width.saturating_sub(2).max(1));
    let height = preferred_height.min(area.height.saturating_sub(2).max(1));
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}
