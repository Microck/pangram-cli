//! Canonical terminal lines for one typed analysis result.
//!
//! Analyze and History both use this module so status, ordered check results,
//! failures, and save state cannot drift between the two routes.

use ratatui::text::Line;

use crate::domain::{
    AiClassification, Analysis, AnalysisStatus, Check, CheckState, CheckStatus, Confidence,
    Provider, SaveState,
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
                for (index, segment) in result.segments.iter().enumerate() {
                    let mut summary = format!(
                        "{}. {} - {:.1}% AI assistance | Text: {} | Confidence: {} | Offsets: {}..{} | Words: {} | Tokens: {}",
                        index + 1,
                        sanitize_single_line(segment.label.as_str()),
                        segment.ai_assistance_score.get() * 100.0,
                        sanitize_single_line(&segment.text),
                        confidence_label(segment.confidence),
                        segment.start_index,
                        segment.end_index,
                        segment.word_count,
                        segment.token_length,
                    );
                    if let (Some(score), Some(is_humanized)) =
                        (segment.humanizer_score, segment.is_humanized)
                    {
                        summary.push_str(&format!(
                            " | Humanizer score: {:.1}% | Humanized: {}",
                            score.get() * 100.0,
                            if is_humanized { "yes" } else { "no" },
                        ));
                    }
                    lines.push(Line::raw(summary));
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
                for (index, matched) in result.matches.iter().enumerate() {
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

    let provenance = analysis.provenance();
    lines.push(Line::raw(format!(
        "Provider: {}",
        provider_label(provenance.provider)
    )));
    if let Some(version) = &provenance.upstream_version {
        lines.push(Line::raw(format!(
            "Upstream version: {}",
            sanitize_single_line(version)
        )));
    }
    if let Some(task_ids) = &provenance.upstream_task_ids
        && !task_ids.as_slice().is_empty()
    {
        let ids = task_ids
            .as_slice()
            .iter()
            .map(|id| sanitize_single_line(id.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(Line::raw(format!("Upstream task IDs: {ids}")));
    }
    if let Some(bulk_id) = &provenance.upstream_bulk_id {
        lines.push(Line::raw(format!(
            "Upstream bulk ID: {}",
            sanitize_single_line(bulk_id.as_str())
        )));
    }
    if let Some(submitted_at) = provenance.submitted_at {
        lines.push(Line::raw(format!("Submitted at: {submitted_at}")));
    }
    if let Some(completed_at) = provenance.completed_at {
        lines.push(Line::raw(format!("Completed at: {completed_at}")));
    }
    for check in analysis.checks() {
        if let Some((label, task_id)) = check_task_identity(check) {
            lines.push(Line::raw(format!(
                "{label} task ID: {}",
                sanitize_single_line(task_id)
            )));
        }
    }

    lines.push(Line::raw(format!(
        "Save state: {}",
        save_state_label(analysis.save_state)
    )));
    lines
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

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn complete_segment_evidence_is_ordered_terminal_safe_and_one_line_per_segment() {
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

        let lines = analysis_result_lines(&analysis);
        let text: Vec<_> = lines.iter().map(line_text).collect();

        assert_eq!(lines.len(), 12, "each segment remains one viewport line");
        assert_eq!(
            text,
            [
                "Overall: succeeded",
                "Analysis: anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8aff",
                "Classification: Mixed",
                "AI 50.0% | AI-assisted 25.0% | Human 25.0%",
                "Result: Mixed [31m authorship",
                "Prediction: The document contains mixed authorship.",
                "Segments: 2 (AI 1, AI-assisted 0, Human 1)",
                "1. AI [31m label - 72.5% AI assistance | Text: provider [2J text tail | Confidence: low | Offsets: 4..29 | Words: 5 | Tokens: 7 | Humanizer score: 31.0% | Humanized: yes",
                "2. Human Written - 0.0% AI assistance | Text: second segment | Confidence: medium | Offsets: 29..43 | Words: 2 | Tokens: 3 | Humanizer score: 0.0% | Humanized: no",
                "Public dashboard: https://dashboard.test/result [0m forged",
                "Provider: Pangram",
                "Save state: saved history",
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

        let lines = analysis_result_lines(&analysis);
        let text: Vec<_> = lines.iter().map(line_text).collect();
        let tail = &text[text.len() - 9..];

        assert_eq!(
            tail,
            [
                "Provider: Pangram",
                "Upstream version: 4.0 [2J forged",
                "Upstream task IDs: task-ai [31m, task-plagiarism forged",
                "Upstream bulk ID: bulk-123 [0m",
                "Submitted at: 2026-07-23T12:00:00Z",
                "Completed at: 2026-07-23T12:00:01Z",
                "AI detection task ID: task-ai [31m",
                "Plagiarism task ID: task-plagiarism forged",
                "Save state: saved history",
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

        let lines = analysis_result_lines(&analysis);
        let text: Vec<_> = lines.iter().map(line_text).collect();

        assert_eq!(
            &text[text.len() - 2..],
            ["Provider: Pangram", "Save state: saved history"]
        );
        assert!(!text.iter().any(|line| line.starts_with("Upstream ")));
    }
}
