//! Rendering for the local Pangram CLI History route and its overlays.
//!
//! The state module owns selection and privacy. This module is a one-way
//! projection and never exposes retained input text, file paths, or extracted
//! file content.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::history::{ExportAction, ExportContent, PendingOperation};
use super::model::{AppState, Focus, HistoryExportField, Overlay};
use super::result_lines::{analysis_status_label, sanitize_single_line, save_state_label};
use super::result_viewport::visible_analysis_result_lines;
use crate::domain::{AnalysisInput, AnalysisInputKind, AnalysisSummary, CheckKind};
use crate::history::HistoryExportFormat;

pub(crate) fn render_history(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let content_area = super::render::workspace_content_area(area);
    let request = state.history.load_request();
    let query = sanitize_single_line(state.history.draft_query());
    let mut lines = Vec::new();
    if state.layout() == super::model::ResponsiveLayout::Wide {
        lines.push(super::render::heading(
            state.color_mode,
            "History - Local Pangram CLI history",
        ));
        lines.push(Line::raw(""));
    }
    lines.push(filter_line(
        state,
        state.focus == Focus::HistorySearch,
        "Search",
        if query.is_empty() { "empty" } else { &query },
    ));
    lines.push(Line::raw(""));
    lines.push(filter_row(
        state,
        request.status.map_or("all", analysis_status_label),
        request.check.map_or("all", check_kind_label),
    ));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        format!("Showing {}", state.history.showing_count()),
        super::render::muted_style(state.color_mode),
    ));

    if let Some(pending) = state.history.pending() {
        lines.push(Line::raw(pending_label(pending)));
    }
    lines.push(Line::raw(""));

    if state.history.showing_count() == 0 {
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            content_area,
        );
        render_empty_history(frame, content_area, state);
        return;
    } else if let Some(detail) = state.history.selected_detail() {
        // Detail replaces the scrolling list so canonical result evidence is
        // visible even at the minimum supported 80x24 viewport. The selected
        // summary remains first to preserve context and selection identity.
        if let Some(summary) = state.history.selected_summary() {
            push_summary_line(
                &mut lines,
                summary,
                true,
                state.focus == Focus::HistoryList,
                content_area.width,
                state.color_mode,
            );
        }
        lines.push(Line::raw("  Selected detail - retained input redacted"));
        push_redacted_input(&mut lines, detail.input());
        let preamble_rows = super::render::wrapped_height(&lines, content_area.width);
        let result_rows = usize::from(content_area.height)
            .saturating_sub(preamble_rows)
            .saturating_sub(1);
        lines.extend(visible_analysis_result_lines(
            detail,
            &state.result_viewport,
            state.focus == Focus::Result,
            usize::from(content_area.width),
            result_rows,
        ));
    } else {
        for summary in state.history.visible_items() {
            push_summary_line(
                &mut lines,
                summary,
                state.history.selected_id() == Some(summary.id),
                state.focus == Focus::HistoryList,
                content_area.width,
                state.color_mode,
            );
        }
    }

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        content_area,
    );
}

pub(crate) fn inspector_lines(state: &AppState, narrow: bool) -> Vec<Line<'static>> {
    let mut lines = if narrow {
        vec![Line::raw(format!(
            "Local history - Showing {}",
            state.history.showing_count()
        ))]
    } else {
        vec![
            Line::raw("Local history"),
            Line::raw(""),
            Line::raw(format!("  Showing: {}", state.history.showing_count())),
        ]
    };
    if let Some(summary) = state.history.selected_summary() {
        lines.push(Line::raw(format!("  Selected: {}", compact_id(summary.id))));
        lines.push(Line::raw(format!(
            "  {} | {}",
            analysis_status_label(summary.status),
            save_state_label(summary.save_state)
        )));
    } else {
        lines.push(Line::raw("  Selected: none"));
    }
    if let Some(pending) = state.history.pending() {
        lines.push(Line::raw(format!("  {}", pending_label(pending))));
    }
    if !narrow {
        lines.push(Line::raw(""));
    }
    lines.push(Line::raw("  Rerun costs credits."));
    lines.push(context_actions(state));
    lines
}

pub(super) fn list_row_offset(state: &AppState) -> u16 {
    let heading_rows = u16::from(state.layout() == super::model::ResponsiveLayout::Wide) * 2;
    let filter_rows = 3;
    let grouping_rows = 3;
    let pending_rows = u16::from(state.history.pending().is_some());
    heading_rows + filter_rows + grouping_rows + pending_rows
}

pub(super) struct FilterTargetAreas {
    pub(super) search: Rect,
    pub(super) status: Rect,
    pub(super) check: Rect,
}

pub(super) fn filter_target_areas(content: Rect, state: &AppState) -> FilterTargetAreas {
    let heading_rows = u16::from(state.layout() == super::model::ResponsiveLayout::Wide) * 2;
    let query = sanitize_single_line(state.history.draft_query());
    let request = state.history.load_request();
    let search_value = if query.is_empty() {
        "empty"
    } else {
        query.as_str()
    };
    let status_value = request.status.map_or("all", analysis_status_label);
    let check_value = request.check.map_or("all", check_kind_label);
    let search = Rect::new(
        content.x,
        content.y.saturating_add(heading_rows),
        super::render::labeled_control_width("Search", search_value).min(content.width),
        1,
    );
    let status = Rect::new(
        content.x,
        content.y.saturating_add(heading_rows).saturating_add(2),
        super::render::labeled_control_width("Status", status_value).min(content.width),
        1,
    );
    let check_x = status.right().saturating_add(1).min(content.right());
    let check = Rect::new(
        check_x,
        status.y,
        super::render::labeled_control_width("Check", check_value)
            .min(content.right().saturating_sub(check_x)),
        1,
    );
    FilterTargetAreas {
        search,
        status,
        check,
    }
}

pub(crate) fn overlay_lines(
    state: &AppState,
    overlay: &Overlay,
) -> Option<(&'static str, Vec<Line<'static>>)> {
    match overlay {
        Overlay::ConfirmHistoryDelete {
            analysis_id,
            confirm,
        } => Some((
            "Delete local history record",
            vec![
                Line::raw(format!("Analysis: {analysis_id}")),
                Line::raw("This removes the local CLI record."),
                Line::raw("Backups may retain copies."),
                Line::raw(""),
                confirm_actions(*confirm, "Delete"),
                Line::raw("esc cancel"),
            ],
        )),
        Overlay::HistoryExport { field } => {
            let choices = state.history.export_choices();
            Some((
                "Export local history",
                vec![
                    choice_line(
                        *field == HistoryExportField::Format,
                        "Format",
                        export_format_label(choices.format()),
                    ),
                    choice_line(
                        *field == HistoryExportField::Content,
                        "Content",
                        export_content_label(choices.content()),
                    ),
                    choice_line(
                        *field == HistoryExportField::Action,
                        "Action",
                        export_action_label(choices.action()),
                    ),
                    Line::raw(""),
                    Line::raw("Redacted omits retained content and evidence text."),
                    Line::raw("enter choose   esc cancel"),
                ],
            ))
        }
        Overlay::ConfirmFullHistoryExport { request, confirm } => Some((
            "Export full retained content",
            vec![
                Line::raw(format!("Format: {}", export_format_label(request.format))),
                Line::raw("Full export can include submitted text and file paths."),
                Line::raw("It can also include result evidence and matched text."),
                Line::raw(""),
                confirm_actions(*confirm, "Export full content"),
                Line::raw("esc cancel"),
            ],
        )),
        _ => None,
    }
}

fn push_summary_line(
    lines: &mut Vec<Line<'static>>,
    summary: &AnalysisSummary,
    selected: bool,
    list_focused: bool,
    width: u16,
    color_mode: super::model::ColorMode,
) {
    let marker = if selected && list_focused { "> " } else { "  " };
    let name = summary
        .display_name
        .as_deref()
        .map(sanitize_single_line)
        .unwrap_or_else(|| input_fallback(summary.input_kind).to_owned());
    let checks = summary
        .checks
        .iter()
        .copied()
        .map(check_kind_short_label)
        .collect::<Vec<_>>()
        .join("+");
    let prefix = format!(
        "{marker}{} {} {checks} {} {} ",
        compact_id(summary.id),
        analysis_status_label(summary.status),
        save_state_short_label(summary.save_state),
        compact_timestamp(summary.created_at)
    );
    let line = format!(
        "{prefix}{}",
        fit_name(
            &name,
            usize::from(width).saturating_sub(Span::raw(prefix.as_str()).width())
        )
    );
    lines.push(Line::styled(
        line,
        if selected && list_focused {
            super::render::primary_style(color_mode)
        } else {
            Style::default()
        },
    ));
}

fn push_redacted_input(lines: &mut Vec<Line<'static>>, input: Option<&AnalysisInput>) {
    match input {
        Some(AnalysisInput::Text(input)) => {
            let name = input
                .name()
                .map(sanitize_single_line)
                .unwrap_or_else(|| "literal text".to_owned());
            lines.push(Line::raw(format!(
                "  Input: {name} - {} words, {} bytes",
                input.word_count, input.byte_count
            )));
        }
        Some(AnalysisInput::File(input)) => lines.push(Line::raw(format!(
            "  Input file: {} - {} - {} bytes",
            sanitize_single_line(input.filename.as_str()),
            sanitize_single_line(input.media_type.as_str()),
            input.size_bytes,
        ))),
        None => lines.push(Line::raw("  Input: unavailable")),
    }
    lines.push(Line::raw("  Retained input content: redacted"));
}

fn pending_label(pending: &PendingOperation) -> &'static str {
    match pending {
        PendingOperation::Reload(_) => "Pending: refreshing local history",
        PendingOperation::Detail(_) => "Pending: loading selected detail",
        PendingOperation::Delete(_) => "Pending: deleting local record",
        PendingOperation::Rerun { .. } => "Pending: preparing billable rerun",
        PendingOperation::Export(_) => "Pending: exporting local history",
    }
}

fn confirm_actions(confirm: bool, destructive_label: &str) -> Line<'static> {
    Line::raw(if confirm {
        format!("left cancel   > {destructive_label}")
    } else {
        format!("> Cancel   right {destructive_label}")
    })
}

fn choice_line(focused: bool, label: &str, value: &str) -> Line<'static> {
    Line::raw(format!(
        "{}{label}  {value}{}",
        super::render::focus_marker(focused),
        if focused { " <" } else { "" }
    ))
}

fn compact_id(id: crate::domain::AnalysisId) -> String {
    let id = id.to_string();
    let suffix = id.chars().rev().take(8).collect::<String>();
    format!("...{}", suffix.chars().rev().collect::<String>())
}

fn compact_timestamp(timestamp: crate::domain::UtcTimestamp) -> String {
    timestamp.to_string().chars().take(16).collect()
}

fn check_kind_label(kind: CheckKind) -> &'static str {
    match kind {
        CheckKind::AiDetection => "AI detection",
        CheckKind::Plagiarism => "plagiarism",
    }
}

fn check_kind_short_label(kind: CheckKind) -> &'static str {
    match kind {
        CheckKind::AiDetection => "AI",
        CheckKind::Plagiarism => "Plag",
    }
}

fn save_state_short_label(state: crate::domain::SaveState) -> &'static str {
    match state {
        crate::domain::SaveState::Ephemeral => "ephemeral",
        crate::domain::SaveState::SavedManual => "manual",
        crate::domain::SaveState::SavedHistory => "history",
    }
}

fn input_fallback(kind: AnalysisInputKind) -> &'static str {
    match kind {
        AnalysisInputKind::Text => "(unnamed text)",
        AnalysisInputKind::File => "(unnamed file)",
    }
}

fn export_format_label(format: HistoryExportFormat) -> &'static str {
    match format {
        HistoryExportFormat::Jsonl => "JSONL",
        HistoryExportFormat::Markdown => "Markdown",
    }
}

fn export_content_label(content: ExportContent) -> &'static str {
    match content {
        ExportContent::Redacted => "redacted",
        ExportContent::Full => "full retained content",
    }
}

fn export_action_label(action: ExportAction) -> &'static str {
    match action {
        ExportAction::Cancel => "cancel",
        ExportAction::Export => "export",
    }
}

fn context_actions(state: &AppState) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (focus, label)) in [
        (Focus::HistoryRerun, "Rerun"),
        (Focus::HistoryExport, "Export"),
        (Focus::HistoryDelete, "Delete"),
    ]
    .into_iter()
    .enumerate()
    {
        if index != 0 {
            spans.push(Span::raw(" "));
        }
        let focused = state.focus == focus;
        spans.push(Span::styled(
            if focused { ">" } else { " " },
            super::render::primary_style(state.color_mode),
        ));
        spans.push(Span::styled(
            format!(" {label} "),
            if focused {
                super::render::action_style(state.color_mode)
            } else {
                super::render::element_style(state.color_mode)
            },
        ));
    }
    Line::from(spans)
}

fn render_empty_history(frame: &mut Frame<'_>, content: Rect, state: &AppState) {
    let offset = list_row_offset(state).min(content.height);
    let remaining = Rect {
        y: content.y.saturating_add(offset),
        height: content.height.saturating_sub(offset),
        ..content
    };
    if remaining.height < 3 {
        return;
    }
    frame.render_widget(
        Paragraph::new(vec![
            super::render::heading(state.color_mode, "No saved analyses"),
            Line::raw(""),
            Line::styled(
                "History stays on this device.",
                super::render::muted_style(state.color_mode),
            ),
        ])
        .alignment(Alignment::Center),
        super::render::centered(remaining, 48, 3),
    );
}

fn filter_line(state: &AppState, focused: bool, label: &str, value: &str) -> Line<'static> {
    Line::from(super::render::labeled_control_spans(
        state.color_mode,
        focused,
        label,
        value,
    ))
}

fn filter_row(state: &AppState, status: &str, check: &str) -> Line<'static> {
    let mut spans = super::render::labeled_control_spans(
        state.color_mode,
        state.focus == Focus::HistoryStatusFilter,
        "Status",
        status,
    );
    spans.push(Span::raw(" "));
    spans.extend(super::render::labeled_control_spans(
        state.color_mode,
        state.focus == Focus::HistoryCheckFilter,
        "Check",
        check,
    ));
    Line::from(spans)
}

fn fit_name(name: &str, available: usize) -> String {
    if available <= 3 {
        return if Span::raw(name).width() <= available {
            name.to_owned()
        } else {
            ".".repeat(available)
        };
    }
    let content_width = available - 3;
    let mut used_width = 0;
    let mut fitted = String::new();
    let name_span = Span::raw(name);
    for grapheme in name_span.styled_graphemes(Style::default()) {
        let width = Span::raw(grapheme.symbol).width();
        used_width += width;
        if used_width <= content_width {
            fitted.push_str(grapheme.symbol);
        }
        if used_width > available {
            fitted.push_str("...");
            return fitted;
        }
    }
    name.to_owned()
}
