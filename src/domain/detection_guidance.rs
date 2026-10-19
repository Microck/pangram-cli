//! Interpretation guidance shared by human-readable projections.
//!
//! This is not a new detector, confidence calibration, or numerical interval.

use super::Segment;

const SHORT_TEXT_CAUTION: &str =
    "Short text (under 150 words): detection is less certain. Treat this estimate cautiously.";

/// Returns the published short-scan caution for a complete returned segment
/// sequence. Missing evidence and overflowing counts remain unknown.
#[must_use]
pub fn short_text_caution(segments: &[Segment]) -> Option<&'static str> {
    if segments.is_empty() {
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
    use crate::domain::{Confidence, Fraction, NonEmptyString};

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
    fn threshold_uses_the_document_total_not_individual_segment_lengths() {
        assert!(short_text_caution(&[segment(149)]).is_some());
        assert!(short_text_caution(&[segment(50), segment(99)]).is_some());
        assert!(short_text_caution(&[segment(150)]).is_none());
        assert!(short_text_caution(&[segment(75), segment(75)]).is_none());
    }

    #[test]
    fn missing_or_overflowing_counts_do_not_claim_a_short_document() {
        assert!(short_text_caution(&[]).is_none());
        assert!(short_text_caution(&[segment(u64::MAX), segment(1)]).is_none());
    }
}
