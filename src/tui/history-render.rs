//! Rendering for the local Pangram CLI History route and its overlays.
//!
//! The state module owns selection and privacy. This module is a one-way
//! projection and never exposes retained input text, file paths, or extracted
//! file content.

use ratatui::Frame;
use ratatui::layout::Rect;
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
    // History rows need one leading column for hierarchy, while the stable
    // separator already provides their right edge in the wide layout. Keeping
    // the final column lets a 120-column terminal show the full summary fields
    // required by the contract instead of truncating a 16-character name.
    let content_area = inset_left(area, 1);
    let request = state.history.load_request();
    let query = sanitize_single_line(state.history.draft_query());
    let mut lines = vec![
        Line::raw("History - Local Pangram CLI history"),
        Line::raw(format!(
            "{}Search literal: {}",
            focus_marker(state.focus == Focus::HistorySearch),
            if query.is_empty() { "[empty]" } else { &query }
        )),
        Line::raw(format!(
            "{}Status filter: {}",
            focus_marker(state.focus == Focus::HistoryStatusFilter),
            request.status.map_or("all", analysis_status_label)
        )),
        Line::raw(format!(
            "{}Check filter: {}",
            focus_marker(state.focus == Focus::HistoryCheckFilter),
            request.check.map_or("all", check_kind_label)
        )),
        Line::raw(format!("Showing {}", state.history.showing_count())),
    ];

    if let Some(pending) = state.history.pending() {
        lines.push(Line::raw(pending_label(pending)));
    }

    if state.history.showing_count() == 0 {
        lines.push(Line::raw("No saved analyses match these criteria."));
        lines.push(Line::raw("History stays on this device."));
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
            );
        }
        lines.push(Line::raw("Selected detail - retained input redacted"));
        push_redacted_input(&mut lines, detail.input());
        lines.extend(visible_analysis_result_lines(
            detail,
            &state.result_viewport,
            state.focus == Focus::Result,
            usize::from(content_area.width),
            usize::from(content_area.height.saturating_sub(10)),
        ));
    } else {
        for summary in state.history.visible_items() {
            push_summary_line(
                &mut lines,
                summary,
                state.history.selected_id() == Some(summary.id),
                state.focus == Focus::HistoryList,
                content_area.width,
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
            Line::raw(format!("Showing: {}", state.history.showing_count())),
        ]
    };
    if let Some(summary) = state.history.selected_summary() {
        lines.push(Line::raw(format!("Selected: {}", compact_id(summary.id))));
        lines.push(Line::raw(format!(
            "{} | {}",
            analysis_status_label(summary.status),
            save_state_label(summary.save_state)
        )));
    } else {
        lines.push(Line::raw("Selected: none"));
    }
    if let Some(pending) = state.history.pending() {
        lines.push(Line::raw(pending_label(pending)));
    }
    lines.push(Line::raw("Rerun is billable."));
    lines.push(Line::raw(context_actions(state.focus)));
    lines
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
                Line::raw("[Esc] Cancel"),
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
                    Line::raw("[Enter] Choose   [Esc] Cancel"),
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
                Line::raw("[Esc] Cancel"),
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
    lines.push(Line::raw(format!(
        "{prefix}{}",
        fit_name(
            &name,
            usize::from(width).saturating_sub(Span::raw(prefix.as_str()).width())
        )
    )));
}

fn push_redacted_input(lines: &mut Vec<Line<'static>>, input: Option<&AnalysisInput>) {
    match input {
        Some(AnalysisInput::Text(input)) => {
            let name = input
                .name()
                .map(sanitize_single_line)
                .unwrap_or_else(|| "literal text".to_owned());
            lines.push(Line::raw(format!(
                "Input: {name} - {} words, {} bytes",
                input.word_count, input.byte_count
            )));
        }
        Some(AnalysisInput::File(input)) => lines.push(Line::raw(format!(
            "Input file: {} - {} - {} bytes",
            sanitize_single_line(input.filename.as_str()),
            sanitize_single_line(input.media_type.as_str()),
            input.size_bytes,
        ))),
        None => lines.push(Line::raw("Input: unavailable")),
    }
    lines.push(Line::raw("Retained input content: redacted"));
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
        format!("[Left] Cancel   > [Enter] {destructive_label} <")
    } else {
        format!("> [Enter] Cancel <   [Right] {destructive_label}")
    })
}

fn choice_line(focused: bool, label: &str, value: &str) -> Line<'static> {
    Line::raw(format!(
        "{}{label}: {value}{}",
        focus_marker(focused),
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

fn focus_marker(focused: bool) -> &'static str {
    if focused { "> " } else { "  " }
}

fn context_actions(focus: Focus) -> String {
    [
        focused_action(focus == Focus::HistoryRerun, "Rerun"),
        focused_action(focus == Focus::HistoryExport, "Export"),
        focused_action(focus == Focus::HistoryDelete, "Delete"),
    ]
    .join(" ")
}

fn focused_action(focused: bool, label: &str) -> String {
    if focused {
        format!(">{label}<")
    } else {
        format!("[{label}]")
    }
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

fn inset_left(area: Rect, columns: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(columns),
        y: area.y,
        width: area.width.saturating_sub(columns),
        height: area.height,
    }
}
