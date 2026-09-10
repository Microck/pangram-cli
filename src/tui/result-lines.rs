//! Canonical terminal lines for one typed analysis result.
//!
//! Analyze and History both use this module so status, ordered check results,
//! failures, and save state cannot drift between the two routes.
//!
//! Rows follow the rest of the TUI: orange headings name each check, white
//! body rows carry the evidence a person reads, and muted rows carry counts,
//! offsets, identities, and timestamps. Every state stays in the text itself
//! so a no-color terminal loses nothing.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::model::{AppState, ColorMode};
use super::render::{
    EvidenceTone, body_style, heading, muted_style, primary_style, tone_color, tone_style,
};
use crate::domain::{
    AiClassification, Analysis, AnalysisStatus, Check, CheckState, CheckStatus, Confidence,
    Provider, SaveState, Segment,
};
use crate::output::CanonicalError;

/// Facts on one row are separated by two cells, matching inspector rows.
const GAP: &str = "  ";
/// Match text and segment measurements hang under their index.
const INDENT: &str = "   ";
/// The distribution bar stops growing here so very wide terminals do not
/// turn a few segments into a full-width stripe.
const MAX_BAR_WIDTH: usize = 60;
/// Full block, the same glyph the intro uses for its densest colored cell.
const BAR_CELL: &str = "\u{2588}";

/// How the result is painted. Both are user preferences that travel together
/// from `AppState` into the projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResultPresentation {
    pub(crate) color_mode: ColorMode,
    /// Paint segment text in its tone, like the dashboard's text highlight.
    /// Off keeps color on labels only.
    pub(crate) highlight: bool,
}

impl ResultPresentation {
    pub(crate) fn from_state(state: &AppState) -> Self {
        Self {
            color_mode: state.color_mode,
            highlight: state.settings.highlight,
        }
    }
}

/// Projects one canonical analysis into terminal-safe, styled lines.
///
/// `width` is the number of cells one row may use; only the distribution bar
/// depends on it, because every other row wraps downstream.
pub(crate) fn analysis_result_lines(
    analysis: &Analysis<CanonicalError>,
    presentation: ResultPresentation,
    width: usize,
) -> Vec<Line<'static>> {
    let color_mode = presentation.color_mode;
    let body = body_style(color_mode);
    let strong = body.add_modifier(Modifier::BOLD);
    let muted = muted_style(color_mode);

    let mut lines = vec![Line::from(vec![
        Span::styled(
            analysis_status_title(analysis.status()),
            primary_style(color_mode),
        ),
        Span::styled(format!("{GAP}{}", analysis.id), muted),
    ])];

    for check in analysis.checks() {
        lines.push(Line::raw(""));
        match check {
            Check::AiDetection(CheckState::Succeeded { result, .. }) => {
                lines.push(heading(color_mode, "AI detection"));
                lines.push(Line::from(vec![
                    Span::styled(
                        classification_label(result.classification),
                        tone_style(color_mode, classification_tone(result.classification)),
                    ),
                    Span::styled(
                        format!(" - {}", sanitize_single_line(&result.headline)),
                        strong,
                    ),
                ]));
                lines.push(Line::styled(sanitize_single_line(&result.prediction), body));
                // The fractions row doubles as the legend for the bar below
                // it. Pangram's per-kind segment counts ride along muted as
                // AI/AI-assisted/human; the segment list itself is the evidence.
                let mut legend = toned_facts_line(
                    color_mode,
                    [
                        (
                            EvidenceTone::Ai,
                            format!("{:.1}%", result.fraction_ai.get() * 100.0),
                        ),
                        (
                            EvidenceTone::AiAssisted,
                            format!("{:.1}%", result.fraction_ai_assisted.get() * 100.0),
                        ),
                        (
                            EvidenceTone::Human,
                            format!("{:.1}%", result.fraction_human.get() * 100.0),
                        ),
                    ],
                    body,
                );
                legend.spans.push(Span::styled(
                    format!(
                        "{GAP}{} segment{} ({}/{}/{})",
                        result.segments.len(),
                        if result.segments.len() == 1 { "" } else { "s" },
                        result.num_ai_segments,
                        result.num_ai_assisted_segments,
                        result.num_human_segments,
                    ),
                    muted,
                ));
                lines.push(legend);
                if let Some(bar) = distribution_bar(&result.segments, color_mode, width) {
                    lines.push(bar);
                }
                // The document reads as plain paragraphs, one per segment,
                // like the dashboard's left pane. The per-segment facts follow
                // as a compact list so the reading area stays uncluttered.
                for segment in &result.segments {
                    lines.push(Line::raw(""));
                    lines.push(segment_text_line(segment, presentation));
                }
                if !result.segments.is_empty() {
                    lines.push(Line::raw(""));
                }
                for (index, segment) in result.segments.iter().enumerate() {
                    lines.extend(segment_facts_lines(index + 1, segment, color_mode));
                }
                if let Some(link) = &result.dashboard_link {
                    lines.push(Line::from(vec![
                        Span::styled("Public dashboard", muted),
                        Span::styled(format!("{GAP}{}", sanitize_single_line(link)), body),
                    ]));
                }
            }
            Check::AiDetection(CheckState::Failed { error, .. }) => {
                lines.push(heading(color_mode, "AI detection"));
                lines.push(failure_line(error, color_mode));
            }
            Check::AiDetection(state) => {
                lines.push(heading(color_mode, "AI detection"));
                lines.push(Line::styled(check_status_title(state.status()), body));
            }
            Check::Plagiarism(CheckState::Succeeded { result, .. }) => {
                lines.push(heading(color_mode, "Plagiarism"));
                let (verdict, tone) = if result.plagiarism_detected {
                    ("Detected", EvidenceTone::Ai)
                } else {
                    ("Not detected", EvidenceTone::Human)
                };
                lines.push(Line::from(vec![
                    Span::styled(verdict, tone_style(color_mode, tone)),
                    Span::styled(
                        format!(
                            " - {:.1}% across {}/{} sentences",
                            result.percent_plagiarized.get(),
                            result.plagiarized_sentence_count,
                            result.total_sentences,
                        ),
                        strong,
                    ),
                ]));
                for (index, matched) in result.matches.iter().enumerate() {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{:<3}", index + 1), muted),
                        Span::styled(
                            format!("{:.1}% similar", matched.similarity_score.get() * 100.0),
                            strong,
                        ),
                        Span::styled(
                            format!("{GAP}{}", sanitize_single_line(&matched.source_url)),
                            body,
                        ),
                    ]));
                    lines.push(Line::styled(
                        format!("{INDENT}{}", sanitize_single_line(&matched.matched_text)),
                        body,
                    ));
                }
            }
            Check::Plagiarism(CheckState::Failed { error, .. }) => {
                lines.push(heading(color_mode, "Plagiarism"));
                lines.push(failure_line(error, color_mode));
            }
            Check::Plagiarism(state) => {
                lines.push(heading(color_mode, "Plagiarism"));
                lines.push(Line::styled(check_status_title(state.status()), body));
            }
        }
    }

    lines.push(Line::raw(""));
    let provenance = analysis.provenance();
    let mut provider = format!("Provider {}", provider_label(provenance.provider));
    if let Some(version) = &provenance.upstream_version {
        provider.push_str(&format!("{GAP}version {}", sanitize_single_line(version)));
    }
    lines.push(Line::styled(provider, muted));
    if let Some(task_ids) = &provenance.upstream_task_ids
        && !task_ids.as_slice().is_empty()
    {
        let ids = task_ids
            .as_slice()
            .iter()
            .map(|id| sanitize_single_line(id.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(Line::styled(format!("Upstream tasks {ids}"), muted));
    }
    if let Some(bulk_id) = &provenance.upstream_bulk_id {
        lines.push(Line::styled(
            format!("Upstream bulk {}", sanitize_single_line(bulk_id.as_str())),
            muted,
        ));
    }
    if let Some(submitted_at) = provenance.submitted_at {
        lines.push(Line::styled(format!("Submitted {submitted_at}"), muted));
    }
    if let Some(completed_at) = provenance.completed_at {
        lines.push(Line::styled(format!("Completed {completed_at}"), muted));
    }
    for check in analysis.checks() {
        if let Some((label, task_id)) = check_task_identity(check) {
            lines.push(Line::styled(
                format!("{label} task {}", sanitize_single_line(task_id)),
                muted,
            ));
        }
    }

    lines.push(Line::from(vec![
        Span::styled("Save state", muted),
        Span::styled(
            format!("{GAP}{}", save_state_label(analysis.save_state)),
            body,
        ),
    ]));
    lines
}

/// One paragraph of the document. With highlight on it takes its segment's
/// tone so the reader sees where the AI is, the way the dashboard washes
/// highlighted text; off leaves it plain body text.
fn segment_text_line(segment: &Segment, presentation: ResultPresentation) -> Line<'static> {
    let body = body_style(presentation.color_mode);
    let style = match segment_tone(&sanitize_single_line(segment.label.as_str())) {
        Some(tone) if presentation.highlight => body.fg(tone_color(presentation.color_mode, tone)),
        _ => body,
    };
    Line::styled(sanitize_single_line(&segment.text), style)
}

/// Two muted rows per segment: the canonical label with score and
/// confidence, then offsets, counts, and humanizer evidence hanging under
/// it. The index matches paragraph order above, and the label keeps its tone
/// so it maps to the paint.
fn segment_facts_lines(
    number: usize,
    segment: &Segment,
    color_mode: ColorMode,
) -> [Line<'static>; 2] {
    let muted = muted_style(color_mode);
    let label = sanitize_single_line(segment.label.as_str());
    let label_style = segment_tone(&label).map_or(muted.add_modifier(Modifier::BOLD), |tone| {
        tone_style(color_mode, tone)
    });
    let mut measurements = format!(
        "{INDENT}{}..{}{GAP}{} words{GAP}{} tokens",
        segment.start_index, segment.end_index, segment.word_count, segment.token_length,
    );
    // Pangram's humanizer decision is called out only when it fired; the
    // score alone carries the negative case.
    if let (Some(score), Some(is_humanized)) = (segment.humanizer_score, segment.is_humanized) {
        measurements.push_str(&format!("{GAP}humanizer {:.1}%", score.get() * 100.0));
        if is_humanized {
            measurements.push_str(&format!("{GAP}humanized"));
        }
    }
    [
        Line::from(vec![
            Span::styled(format!("{number:<3}"), muted),
            Span::styled(label, label_style),
            Span::styled(
                format!(
                    "{GAP}{:.1}% AI assistance{GAP}{} confidence",
                    segment.ai_assistance_score.get() * 100.0,
                    confidence_label(segment.confidence),
                ),
                muted,
            ),
        ]),
        Line::styled(measurements, muted),
    ]
}

fn failure_line(error: &CanonicalError, color_mode: ColorMode) -> Line<'static> {
    Line::from(vec![
        Span::styled("Failed", tone_style(color_mode, EvidenceTone::Ai)),
        Span::styled(
            format!(" - {}", sanitize_single_line(error.message())),
            body_style(color_mode),
        ),
    ])
}

/// One row of `label value` facts, one per tone, with the label in its tone.
fn toned_facts_line(
    color_mode: ColorMode,
    facts: [(EvidenceTone, String); 3],
    value_style: Style,
) -> Line<'static> {
    let mut spans = Vec::with_capacity(6);
    for (index, (tone, value)) in facts.into_iter().enumerate() {
        let separator = if index == 0 { "" } else { GAP };
        spans.push(Span::styled(
            format!("{separator}{}", tone_label(tone)),
            tone_style(color_mode, tone),
        ));
        spans.push(Span::styled(format!(" {value}"), value_style));
    }
    Line::from(spans)
}

/// Word-proportional stripe of the segments in document order, like the
/// dashboard's "AI usage across document" chart. Colored terminals draw
/// full-block cells in the tone, following the intro's glyph choice;
/// no-color terminals use one ASCII symbol per tone so the shape survives.
/// Cells are never bare spaces because whitespace-only rows wrap badly in
/// ratatui. Returns `None` when there is nothing to scale.
fn distribution_bar(
    segments: &[Segment],
    color_mode: ColorMode,
    width: usize,
) -> Option<Line<'static>> {
    let total_words: u64 = segments.iter().map(|segment| segment.word_count).sum();
    let bar_width = width.min(MAX_BAR_WIDTH);
    if total_words == 0 || bar_width == 0 {
        return None;
    }
    // Cumulative rounding keeps the cells summing to exactly `bar_width`.
    let mut spans = Vec::with_capacity(segments.len());
    let mut words_so_far = 0_u64;
    let mut cells_so_far = 0_usize;
    for segment in segments {
        words_so_far += segment.word_count;
        let cells_end = usize::try_from(
            (u128::from(words_so_far) * bar_width as u128 + u128::from(total_words) / 2)
                / u128::from(total_words),
        )
        .unwrap_or(bar_width);
        let cells = cells_end.saturating_sub(cells_so_far);
        cells_so_far = cells_end;
        if cells == 0 {
            continue;
        }
        let tone = segment_tone(&sanitize_single_line(segment.label.as_str()));
        let (symbol, style) = match (color_mode, tone) {
            (ColorMode::None, Some(tone)) => (tone.bar_symbol(), Style::default()),
            (ColorMode::None, None) => ("-", Style::default()),
            (_, Some(tone)) => (BAR_CELL, Style::default().fg(tone_color(color_mode, tone))),
            (_, None) => (BAR_CELL, Style::default().fg(Color::DarkGray)),
        };
        spans.push(Span::styled(symbol.repeat(cells), style));
    }
    Some(Line::from(spans))
}

/// Maps Pangram's free-text segment label onto a tone. Labels are provider
/// authored ("AI-Assisted", "Human Written", "AI Generated"), so this is a
/// case-insensitive keyword match; unknown labels keep the plain bold style.
fn segment_tone(label: &str) -> Option<EvidenceTone> {
    let lowered = label.to_ascii_lowercase();
    if lowered.contains("human") {
        Some(EvidenceTone::Human)
    } else if lowered.contains("assist") {
        Some(EvidenceTone::AiAssisted)
    } else if lowered.contains("ai") {
        Some(EvidenceTone::Ai)
    } else {
        None
    }
}

/// Mixed documents share the AI-assisted amber, matching the dashboard's
/// "Mixed" badge.
const fn classification_tone(classification: AiClassification) -> EvidenceTone {
    match classification {
        AiClassification::Ai => EvidenceTone::Ai,
        AiClassification::Human => EvidenceTone::Human,
        AiClassification::Mixed => EvidenceTone::AiAssisted,
    }
}

const fn tone_label(tone: EvidenceTone) -> &'static str {
    match tone {
        EvidenceTone::Ai => "AI",
        EvidenceTone::AiAssisted => "AI-assisted",
        EvidenceTone::Human => "Human",
    }
}

fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::Pangram => "Pangram",
    }
}

fn state_task_id<R, E>(state: &CheckState<R, E>) -> Option<&str> {
    state
        .upstream()
        .and_then(|identity| identity.task_id.as_ref())
        .map(|task_id| task_id.as_str())
}

fn check_task_identity(check: &Check<CanonicalError>) -> Option<(&'static str, &str)> {
    match check {
        Check::AiDetection(state) => state_task_id(state).map(|id| ("AI detection", id)),
        Check::Plagiarism(state) => state_task_id(state).map(|id| ("Plagiarism", id)),
    }
}

pub(crate) const fn save_state_label(state: SaveState) -> &'static str {
    match state {
        SaveState::Ephemeral => "ephemeral",
        SaveState::SavedManual => "saved manual",
        SaveState::SavedHistory => "saved history",
    }
}

pub(crate) const fn analysis_status_label(status: AnalysisStatus) -> &'static str {
    match status {
        AnalysisStatus::Queued => "queued",
        AnalysisStatus::Running => "running",
        AnalysisStatus::Succeeded => "succeeded",
        AnalysisStatus::Failed => "failed",
        AnalysisStatus::Partial => "partial",
    }
}

/// Title-case status for the result's first row; filters keep the lowercase
/// `analysis_status_label` because they sit inside a control.
const fn analysis_status_title(status: AnalysisStatus) -> &'static str {
    match status {
        AnalysisStatus::Queued => "Queued",
        AnalysisStatus::Running => "Running",
        AnalysisStatus::Succeeded => "Succeeded",
        AnalysisStatus::Failed => "Failed",
        AnalysisStatus::Partial => "Partial",
    }
}

const fn check_status_title(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Queued => "Queued",
        CheckStatus::Running => "Running",
        CheckStatus::Succeeded => "Succeeded",
        CheckStatus::Failed => "Failed",
    }
}

fn classification_label(classification: AiClassification) -> &'static str {
    match classification {
        AiClassification::Ai => "AI",
        AiClassification::Human => "Human",
        AiClassification::Mixed => "Mixed",
    }
}

fn confidence_label(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

pub(crate) fn sanitize_single_line(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() || character == '\u{FFFD}' {
                ' '
            } else {
                character
            }
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;
    use crate::domain::{
        AiDetectionResult, AnalysisId, AnalysisInput, Confidence, Fraction, NonEmptyString,
        OrderedChecks, Provenance, Provider, Segment, Sha256Hash, SubmissionOutcome, TextInput,
        TextOrigin, UpstreamBulkId, UpstreamIdentity, UpstreamTaskId, UpstreamTaskIds,
        UtcTimestamp,
    };
    use crate::output::ErrorCode;

    fn timestamp(value: &str) -> UtcTimestamp {
        UtcTimestamp::from_str(value).expect("canonical timestamp")
    }

    fn failed_check_error() -> CanonicalError {
        CanonicalError::new(
            ErrorCode::UpstreamAnalysisFailed,
            "Pangram could not complete this check.",
        )
        .expect("canonical error")
    }

    fn analysis_with_identity(
        checks: OrderedChecks<Check<CanonicalError>>,
        provenance: Provenance,
    ) -> Analysis<CanonicalError> {
        let input = TextInput::new(
            TextOrigin::Literal,
            None,
            Sha256Hash::digest(b"identity projection fixture"),
            27,
            3,
            None,
        )
        .expect("canonical text input");
        Analysis::new(
            AnalysisId::from_str("anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8aff")
                .expect("canonical analysis ID"),
            SubmissionOutcome::Terminal,
            AnalysisInput::Text(input),
            checks,
            SaveState::SavedHistory,
            provenance,
            None,
            None,
            timestamp("2026-07-23T12:00:00Z"),
            timestamp("2026-07-23T12:00:01Z"),
            Some(timestamp("2026-07-23T12:00:01Z")),
        )
        .expect("canonical terminal analysis")
    }

    fn lines_text(analysis: &Analysis<CanonicalError>) -> Vec<String> {
        analysis_result_lines(analysis, plain(), 80)
            .iter()
            .map(line_text)
            .collect()
    }

    const fn plain() -> ResultPresentation {
        ResultPresentation {
            color_mode: ColorMode::None,
            highlight: false,
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn complete_segment_evidence_is_ordered_terminal_safe_as_paragraphs_then_facts() {
        let result = AiDetectionResult {
            classification: AiClassification::Mixed,
            headline: "Mixed\u{1b}[31m\nauthorship".to_owned(),
            prediction: "The document\u{009b}contains mixed authorship.".to_owned(),
            fraction_ai: Fraction::new(0.5).expect("valid fraction"),
            fraction_ai_assisted: Fraction::new(0.25).expect("valid fraction"),
            fraction_human: Fraction::new(0.25).expect("valid fraction"),
            num_ai_segments: 1,
            num_ai_assisted_segments: 0,
            num_human_segments: 1,
            segments: vec![
                Segment {
                    text: "provider\u{1b}[2J\ntext\u{FFFD}tail".to_owned(),
                    label: NonEmptyString::new("AI\u{1b}[31m\nlabel").expect("segment label"),
                    ai_assistance_score: Fraction::new(0.725).expect("valid fraction"),
                    confidence: Confidence::Low,
                    start_index: 4,
                    end_index: 29,
                    word_count: 5,
                    token_length: 7,
                    humanizer_score: Some(Fraction::new(0.31).expect("valid fraction")),
                    is_humanized: Some(true),
                },
                Segment {
                    text: "second segment".to_owned(),
                    label: NonEmptyString::new("Human Written").expect("segment label"),
                    ai_assistance_score: Fraction::new(0.0).expect("valid fraction"),
                    confidence: Confidence::Medium,
                    start_index: 29,
                    end_index: 43,
                    word_count: 2,
                    token_length: 3,
                    humanizer_score: Some(Fraction::new(0.0).expect("valid fraction")),
                    is_humanized: Some(false),
                },
            ],
            dashboard_link: Some("https://dashboard.test/result\u{1b}[0m\nforged".to_owned()),
        };
        let checks = OrderedChecks::new([Check::AiDetection(CheckState::Succeeded {
            upstream: None,
            result,
        })])
        .expect("canonical checks");
        let analysis = analysis_with_identity(
            checks,
            Provenance {
                provider: Provider::Pangram,
                upstream_version: None,
                upstream_task_ids: None,
                upstream_bulk_id: None,
                submitted_at: None,
                completed_at: None,
            },
        );

        let text = lines_text(&analysis);

        assert_eq!(
            text.len(),
            20,
            "segments read as paragraphs, then two facts rows each"
        );
        assert_eq!(
            text,
            [
                "Succeeded  anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8aff",
                "",
                "AI detection",
                "Mixed - Mixed [31m authorship",
                "The document contains mixed authorship.",
                "AI 50.0%  AI-assisted 25.0%  Human 25.0%  2 segments (1/0/1)",
                &format!("{}{}", "#".repeat(43), ".".repeat(17)),
                "",
                "provider [2J text tail",
                "",
                "second segment",
                "",
                "1  AI [31m label  72.5% AI assistance  low confidence",
                "   4..29  5 words  7 tokens  humanizer 31.0%  humanized",
                "2  Human Written  0.0% AI assistance  medium confidence",
                "   29..43  2 words  3 tokens  humanizer 0.0%",
                "Public dashboard  https://dashboard.test/result [0m forged",
                "",
                "Provider Pangram",
                "Save state  saved history",
            ]
        );
        assert!(text.iter().all(|line| !line.contains(['\u{1b}', '\n'])));
    }

    #[test]
    fn identity_rich_result_projects_terminal_safe_provenance_in_hierarchy_order() {
        let ai_task = UpstreamTaskId::new("task-ai\u{1b}[31m").expect("task ID");
        let plagiarism_task = UpstreamTaskId::new("task-plagiarism\nforged").expect("task ID");
        let checks = OrderedChecks::new([
            Check::AiDetection(CheckState::Failed {
                upstream: Some(UpstreamIdentity {
                    task_id: Some(ai_task.clone()),
                    last_stage: Some(NonEmptyString::new("AI_DONE").expect("stage")),
                }),
                error: failed_check_error(),
            }),
            Check::Plagiarism(CheckState::Failed {
                upstream: Some(UpstreamIdentity {
                    task_id: Some(plagiarism_task.clone()),
                    last_stage: Some(NonEmptyString::new("PLAG_DONE").expect("stage")),
                }),
                error: failed_check_error(),
            }),
        ])
        .expect("canonical checks");
        let analysis = analysis_with_identity(
            checks,
            Provenance {
                provider: Provider::Pangram,
                upstream_version: Some("4.0\u{1b}[2J\nforged".to_owned()),
                upstream_task_ids: Some(
                    UpstreamTaskIds::new(vec![ai_task, plagiarism_task]).expect("task IDs"),
                ),
                upstream_bulk_id: Some(UpstreamBulkId::new("bulk-123\u{1b}[0m").expect("bulk ID")),
                submitted_at: Some(timestamp("2026-07-23T12:00:00Z")),
                completed_at: Some(timestamp("2026-07-23T12:00:01Z")),
            },
        );

        let text = lines_text(&analysis);
        let tail = &text[text.len() - 9..];

        assert_eq!(
            tail,
            [
                "",
                "Provider Pangram  version 4.0 [2J forged",
                "Upstream tasks task-ai [31m, task-plagiarism forged",
                "Upstream bulk bulk-123 [0m",
                "Submitted 2026-07-23T12:00:00Z",
                "Completed 2026-07-23T12:00:01Z",
                "AI detection task task-ai [31m",
                "Plagiarism task task-plagiarism forged",
                "Save state  saved history",
            ]
        );
        assert!(text.iter().all(|line| !line.contains(['\u{1b}', '\n'])));
    }

    #[test]
    fn result_omits_absent_identity_fields_without_inference() {
        let checks = OrderedChecks::new([Check::AiDetection(CheckState::Failed {
            upstream: None,
            error: failed_check_error(),
        })])
        .expect("canonical checks");
        let analysis = analysis_with_identity(
            checks,
            Provenance {
                provider: Provider::Pangram,
                upstream_version: None,
                upstream_task_ids: None,
                upstream_bulk_id: None,
                submitted_at: None,
                completed_at: None,
            },
        );

        let text = lines_text(&analysis);

        assert_eq!(
            &text[text.len() - 2..],
            ["Provider Pangram", "Save state  saved history"]
        );
        assert!(!text.iter().any(|line| line.starts_with("Upstream ")));
    }

    #[test]
    fn colored_rows_use_heading_body_and_muted_styles_by_role() {
        let checks = OrderedChecks::new([Check::AiDetection(CheckState::Failed {
            upstream: None,
            error: failed_check_error(),
        })])
        .expect("canonical checks");
        let analysis = analysis_with_identity(
            checks,
            Provenance {
                provider: Provider::Pangram,
                upstream_version: None,
                upstream_task_ids: None,
                upstream_bulk_id: None,
                submitted_at: None,
                completed_at: None,
            },
        );

        let lines = analysis_result_lines(
            &analysis,
            ResultPresentation {
                color_mode: ColorMode::TrueColor,
                highlight: false,
            },
            80,
        );

        assert_eq!(lines[0].spans[0].style, primary_style(ColorMode::TrueColor));
        assert_eq!(lines[0].spans[1].style, muted_style(ColorMode::TrueColor));
        assert_eq!(lines[2].style, primary_style(ColorMode::TrueColor));
        assert!(
            lines[3].spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(lines[5].style, muted_style(ColorMode::TrueColor));
    }

    #[test]
    fn highlight_paints_segment_text_in_its_tone_and_off_keeps_body_text() {
        let segment = Segment {
            text: "painted".to_owned(),
            label: NonEmptyString::new("Human Written").expect("label"),
            ai_assistance_score: Fraction::new(0.0).expect("fraction"),
            confidence: Confidence::High,
            start_index: 0,
            end_index: 7,
            word_count: 1,
            token_length: 1,
            humanizer_score: None,
            is_humanized: None,
        };
        let text_row = |highlight: bool| {
            segment_text_line(
                &segment,
                ResultPresentation {
                    color_mode: ColorMode::TrueColor,
                    highlight,
                },
            )
        };

        assert_eq!(
            text_row(true).style.fg,
            Some(tone_color(ColorMode::TrueColor, EvidenceTone::Human))
        );
        assert_eq!(text_row(false).style, body_style(ColorMode::TrueColor));
    }

    #[test]
    fn distribution_bar_scales_segments_by_words_and_keeps_tone_order() {
        let segment = |label: &str, words: u64| Segment {
            text: "x".to_owned(),
            label: NonEmptyString::new(label).expect("label"),
            ai_assistance_score: Fraction::new(0.0).expect("fraction"),
            confidence: Confidence::High,
            start_index: 0,
            end_index: 1,
            word_count: words,
            token_length: words,
            humanizer_score: None,
            is_humanized: None,
        };
        let segments = [
            segment("AI Generated", 1),
            segment("AI-Assisted", 2),
            segment("Human Written", 1),
            segment("Unlabeled", 0),
        ];

        let ascii = distribution_bar(&segments, ColorMode::None, 200).expect("bar");
        assert_eq!(
            line_text(&ascii),
            format!("{}{}{}", "#".repeat(15), "=".repeat(30), ".".repeat(15)),
            "cells sum to the 60-cell cap and zero-word segments take no cell"
        );

        let colored = distribution_bar(&segments, ColorMode::TrueColor, 20).expect("bar");
        assert_eq!(line_text(&colored), BAR_CELL.repeat(20));
        assert_eq!(
            colored
                .spans
                .iter()
                .map(|span| span.style.fg)
                .collect::<Vec<_>>(),
            [
                Some(tone_color(ColorMode::TrueColor, EvidenceTone::Ai)),
                Some(tone_color(ColorMode::TrueColor, EvidenceTone::AiAssisted)),
                Some(tone_color(ColorMode::TrueColor, EvidenceTone::Human)),
            ]
        );
        assert!(distribution_bar(&segments[3..], ColorMode::None, 20).is_none());
    }
}
