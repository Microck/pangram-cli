//! Interpretation guidance shared by human-readable projections.
//!
//! This is not a new detector, confidence calibration, or numerical interval.

use super::AiDetectionResult;

const SHORT_TEXT_CAUTION: &str =
    "Short text (under 150 words): detection is less certain. Treat this estimate cautiously.";

/// Returns the published short-scan caution for a complete returned segment
/// sequence. Missing evidence and overflowing counts remain unknown.
#[must_use]
pub fn short_text_caution(result: &AiDetectionResult) -> Option<&'static str> {
    let segments = &result.segments;
    let reported = result
        .num_ai_segments
        .checked_add(result.num_ai_assisted_segments)?
        .checked_add(result.num_human_segments)?;
    if segments.is_empty() || u64::try_from(segments.len()).ok()? != reported {
        return None;
    }
    let words = segments.iter().try_fold(0_u64, |total, segment| {
        total.checked_add(segment.word_count)
    });
    match words {
        Some(0..=149) => Some(SHORT_TEXT_CAUTION),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Confidence, Fraction, NonEmptyString, Segment};

    fn segment(words: u64) -> Segment {
        Segment {
            text: "synthetic".to_owned(),
            label: NonEmptyString::new("Human Written").unwrap(),
            ai_assistance_score: Fraction::new(0.0).unwrap(),
            confidence: Confidence::High,
            start_index: 0,
            end_index: 9,
            word_count: words,
            token_length: words,
            humanizer_score: None,
            is_humanized: None,
        }
    }

    #[test]
    fn inconsistent_segment_counters_do_not_claim_a_short_document() {
        let result = crate::domain::AiDetectionResult {
            classification: crate::domain::AiClassification::Human,
            headline: "Human".to_owned(),
            prediction: "Human Written".to_owned(),
            fraction_ai: Fraction::new(0.0).unwrap(),
            fraction_ai_assisted: Fraction::new(0.0).unwrap(),
            fraction_human: Fraction::new(1.0).unwrap(),
            num_ai_segments: 0,
            num_ai_assisted_segments: 0,
            num_human_segments: 2,
            segments: vec![segment(100)],
            dashboard_link: None,
        };
        assert!(short_text_caution(&result).is_none());
        let mut overflow = result.clone();
        overflow.num_ai_segments = u64::MAX;
        assert!(short_text_caution(&overflow).is_none());
        let mut missing = result;
        missing.num_human_segments = 0;
        assert!(short_text_caution(&missing).is_none());
    }

    fn caution(segments: &[Segment]) -> Option<&'static str> {
        short_text_caution(&AiDetectionResult {
            classification: crate::domain::AiClassification::Human,
            headline: "Human".to_owned(),
            prediction: "Human Written".to_owned(),
            fraction_ai: Fraction::new(0.0).unwrap(),
            fraction_ai_assisted: Fraction::new(0.0).unwrap(),
            fraction_human: Fraction::new(1.0).unwrap(),
            num_ai_segments: 0,
            num_ai_assisted_segments: 0,
            num_human_segments: u64::try_from(segments.len()).unwrap(),
            segments: segments.to_vec(),
            dashboard_link: None,
        })
    }

    #[test]
    fn threshold_uses_the_document_total_not_individual_segment_lengths() {
        assert!(caution(&[segment(149)]).is_some());
        assert!(caution(&[segment(50), segment(99)]).is_some());
        assert!(caution(&[segment(150)]).is_none());
        assert!(caution(&[segment(75), segment(75)]).is_none());
    }

    #[test]
    fn missing_or_overflowing_counts_do_not_claim_a_short_document() {
        assert!(caution(&[]).is_none());
        assert!(caution(&[segment(u64::MAX), segment(1)]).is_none());
    }
}
