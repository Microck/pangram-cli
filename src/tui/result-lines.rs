//! Canonical terminal lines for one typed analysis result.
//!
//! Analyze and History both use this module so status, ordered check results,
//! failures, and save state cannot drift between the two routes.

use ratatui::text::Line;

use crate::domain::{
    AiClassification, Analysis, AnalysisStatus, Check, CheckState, CheckStatus, SaveState,
};
use crate::output::CanonicalError;

/// Projects one canonical analysis into terminal-safe, text-labelled lines.
pub(crate) fn analysis_result_lines(analysis: &Analysis<CanonicalError>) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::raw(format!(
            "Overall: {}",
            analysis_status_label(analysis.status())
        )),
        Line::raw(format!("Analysis: {}", analysis.id)),
    ];

    for check in analysis.checks() {
        match check {
            Check::AiDetection(CheckState::Succeeded { result, .. }) => {
                lines.push(Line::raw(format!(
                    "Classification: {}",
                    classification_label(result.classification)
                )));
                lines.push(Line::raw(format!(
                    "AI {:.1}% | AI-assisted {:.1}% | Human {:.1}%",
                    result.fraction_ai.get() * 100.0,
                    result.fraction_ai_assisted.get() * 100.0,
                    result.fraction_human.get() * 100.0,
                )));
                lines.push(Line::raw(format!(
                    "Result: {}",
                    sanitize_single_line(&result.headline)
                )));
                lines.push(Line::raw(format!(
                    "Prediction: {}",
                    sanitize_single_line(&result.prediction)
                )));
                lines.push(Line::raw(format!(
                    "Segments: {} (AI {}, AI-assisted {}, Human {})",
                    result.segments.len(),
                    result.num_ai_segments,
                    result.num_ai_assisted_segments,
                    result.num_human_segments,
                )));
                for (index, segment) in result.segments.iter().take(6).enumerate() {
                    lines.push(Line::raw(format!(
                        "{}. {} - {:.1}% AI assistance",
                        index + 1,
                        sanitize_single_line(segment.label.as_str()),
                        segment.ai_assistance_score.get() * 100.0,
                    )));
                }
                if let Some(link) = &result.dashboard_link {
                    lines.push(Line::raw(format!(
                        "Public dashboard: {}",
                        sanitize_single_line(link)
                    )));
                }
            }
            Check::AiDetection(CheckState::Failed { error, .. }) => {
                lines.push(Line::raw(format!(
                    "AI detection failed: {}",
                    sanitize_single_line(error.message())
                )));
            }
            Check::AiDetection(state) => lines.push(Line::raw(format!(
                "AI detection: {}",
                check_status_label(state.status())
            ))),
            Check::Plagiarism(CheckState::Succeeded { result, .. }) => {
                lines.push(Line::raw(format!(
                    "Plagiarism: {} - {:.1}% across {}/{} sentences",
                    if result.plagiarism_detected {
                        "detected"
                    } else {
                        "not detected"
                    },
                    result.percent_plagiarized.get(),
                    result.plagiarized_sentence_count,
                    result.total_sentences,
                )));
                for (index, matched) in result.matches.iter().take(6).enumerate() {
                    lines.push(Line::raw(format!(
                        "Match {}: {:.1}% - {} - {}",
                        index + 1,
                        matched.similarity_score.get() * 100.0,
                        sanitize_single_line(&matched.source_url),
                        sanitize_single_line(&matched.matched_text),
                    )));
                }
            }
            Check::Plagiarism(CheckState::Failed { error, .. }) => {
                lines.push(Line::raw(format!(
                    "Plagiarism failed: {}",
                    sanitize_single_line(error.message())
                )));
            }
            Check::Plagiarism(state) => lines.push(Line::raw(format!(
                "Plagiarism: {}",
                check_status_label(state.status())
            ))),
        }
    }

    lines.push(Line::raw(format!(
        "Save state: {}",
        save_state_label(analysis.save_state)
    )));
    lines
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

pub(crate) const fn check_status_label(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Queued => "queued",
        CheckStatus::Running => "running",
        CheckStatus::Succeeded => "succeeded",
        CheckStatus::Failed => "failed",
    }
}

fn classification_label(classification: AiClassification) -> &'static str {
    match classification {
        AiClassification::Ai => "AI",
        AiClassification::Human => "Human",
        AiClassification::Mixed => "Mixed",
    }
}

pub(crate) fn sanitize_single_line(value: &str) -> String {
    crate::output::sanitize_terminal(value).replace('\u{FFFD}', " ")
}
