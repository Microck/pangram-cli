//! Modal overlay projection and the matching pointer geometry.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap};

use super::{
    action_style, centered, control_style, history_render, panel_style, primary_style,
    wrapped_height,
};
use crate::tui::model::{AppState, HistoryExportField, Overlay};

const WIDTH: u16 = 56;
const HEIGHT: u16 = 12;
const HORIZONTAL_PADDING: u16 = 2;
const VERTICAL_PADDING: u16 = 1;

#[derive(Clone, Copy)]
pub(in crate::tui) struct OverlayAreas {
    pub(in crate::tui) outer: Rect,
    pub(in crate::tui) content: Rect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::tui) enum OverlayTarget {
    Primary,
    Secondary,
    Toggle,
    Cancel,
    Confirm,
    ExportField(HistoryExportField),
    Dismiss,
}

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState, overlay: &Overlay) {
    let areas = areas(area);
    let (title, lines) = content(state, overlay);
    let border = overlay_block(state, title);

    frame.render_widget(Clear, areas.outer);
    frame.render_widget(border, areas.outer);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        areas.content,
    );
}

pub(in crate::tui) fn areas(area: Rect) -> OverlayAreas {
    let outer = centered(area, WIDTH, HEIGHT);
    let content = overlay_block_geometry().inner(outer);
    OverlayAreas { outer, content }
}

pub(in crate::tui) fn target(
    column: u16,
    row: u16,
    frame: Rect,
    state: &AppState,
) -> Option<OverlayTarget> {
    let overlay = state.overlay.as_ref()?;
    let areas = areas(frame);
    let (_, lines) = content(state, overlay);
    let content_row = |index: usize| {
        areas.content.y.saturating_add(
            u16::try_from(wrapped_height(&lines[..index], areas.content.width)).unwrap_or(u16::MAX),
        )
    };

    match overlay {
        Overlay::Credential(_) if row == content_row(4) => segment_target(
            column,
            areas.content.x,
            &lines[4],
            &[OverlayTarget::Primary, OverlayTarget::Secondary],
        ),
        Overlay::UpdatePreference { .. } if row == content_row(2) => Some(OverlayTarget::Toggle),
        Overlay::UpdatePreference { .. } if row == content_row(4) => segment_target(
            column,
            areas.content.x,
            &lines[4],
            if state.settings.credential_present {
                &[OverlayTarget::Primary]
            } else {
                &[OverlayTarget::Primary, OverlayTarget::Secondary]
            },
        ),
        Overlay::HistoryConsent if row == content_row(3) => segment_target(
            column,
            areas.content.x,
            &lines[3],
            &[OverlayTarget::Primary, OverlayTarget::Secondary],
        ),
        Overlay::ConfirmHistoryDelete { .. } if row == content_row(4) => segment_target(
            column,
            areas.content.x,
            &lines[4],
            &[OverlayTarget::Cancel, OverlayTarget::Confirm],
        ),
        Overlay::ConfirmHistoryDelete { .. } if row == content_row(5) => {
            segment_target(column, areas.content.x, &lines[5], &[OverlayTarget::Cancel])
        }
        Overlay::HistoryExport { .. } if row == content_row(0) => {
            Some(OverlayTarget::ExportField(HistoryExportField::Format))
        }
        Overlay::HistoryExport { .. } if row == content_row(1) => {
            Some(OverlayTarget::ExportField(HistoryExportField::Content))
        }
        Overlay::HistoryExport { .. } if row == content_row(2) => {
            Some(OverlayTarget::ExportField(HistoryExportField::Action))
        }
        Overlay::HistoryExport { .. } if row == content_row(5) => segment_target(
            column,
            areas.content.x,
            &lines[5],
            &[OverlayTarget::Primary, OverlayTarget::Secondary],
        ),
        Overlay::ConfirmFullHistoryExport { .. } if row == content_row(4) => segment_target(
            column,
            areas.content.x,
            &lines[4],
            &[OverlayTarget::Cancel, OverlayTarget::Confirm],
        ),
        Overlay::ConfirmFullHistoryExport { .. } if row == content_row(5) => {
            segment_target(column, areas.content.x, &lines[5], &[OverlayTarget::Cancel])
        }
        Overlay::Help => Some(OverlayTarget::Dismiss),
        Overlay::Credential(_)
        | Overlay::UpdatePreference { .. }
        | Overlay::HistoryConsent
        | Overlay::ConfirmHistoryDelete { .. }
        | Overlay::HistoryExport { .. }
        | Overlay::ConfirmFullHistoryExport { .. } => None,
    }
}

fn content(state: &AppState, overlay: &Overlay) -> (&'static str, Vec<Line<'static>>) {
    match overlay {
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
                key_hints(state, &[("enter", "save"), ("esc", "skip")]),
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
                key_hints(
                    state,
                    if state.settings.credential_present {
                        &[("enter", "continue")]
                    } else {
                        &[("enter", "continue"), ("esc", "back")]
                    },
                ),
            ],
        ),
        Overlay::HistoryConsent => (
            "Enable local history",
            vec![
                Line::raw("History stores full input and results in plaintext."),
                Line::raw("Records remain until you delete them."),
                Line::raw(""),
                key_hints(state, &[("y/enter", "enable"), ("n/esc", "cancel")]),
            ],
        ),
        Overlay::Help => (
            "Help",
            vec![
                Line::raw("Arrows or Tab move focus. Enter acts."),
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
    }
}

fn overlay_block(state: &AppState, title: &'static str) -> Block<'static> {
    overlay_block_geometry()
        .title(Line::styled(title, primary_style(state.color_mode)))
        .border_style(primary_style(state.color_mode))
        .style(panel_style(state.color_mode))
}

fn overlay_block_geometry() -> Block<'static> {
    Block::default().borders(Borders::ALL).padding(Padding::new(
        HORIZONTAL_PADDING,
        HORIZONTAL_PADDING,
        VERTICAL_PADDING,
        VERTICAL_PADDING,
    ))
}

fn key_hints(state: &AppState, hints: &[(&'static str, &'static str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, &(key, label)) in hints.iter().enumerate() {
        if !spans.is_empty() {
            spans.push(Span::raw("   "));
        }
        let style = if index == 0 {
            action_style(state.color_mode)
        } else {
            control_style(state.color_mode, false, false, true)
        };
        spans.push(Span::styled(format!(" {key}"), style));
        spans.push(Span::styled(format!(" {label} "), style));
    }
    Line::from(spans)
}

fn segment_target(
    column: u16,
    start_x: u16,
    line: &Line<'_>,
    targets: &[OverlayTarget],
) -> Option<OverlayTarget> {
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let mut offset = 0_u16;
    for (segment, target) in text.split("   ").zip(targets.iter().copied()) {
        let width = u16::try_from(Span::raw(segment).width()).unwrap_or(u16::MAX);
        let left = start_x.saturating_add(offset);
        if column >= left && column < left.saturating_add(width) {
            return Some(target);
        }
        offset = offset.saturating_add(width).saturating_add(3);
    }
    None
}
