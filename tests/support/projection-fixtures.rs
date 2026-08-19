//! Shared canonical envelope fixtures for the output-projection tests.
//!
//! Every value is constructed through the typed domain and output owners so a
//! projection can never be fed a value that would not survive the schema. The
//! fixed IDs, hashes, and timestamps make the rendered goldens deterministic.

use std::str::FromStr;

use microck_pangram_cli::domain::{
    AiClassification, AiDetectionResult, Analysis, AnalysisId, AnalysisInput, Check, CheckState,
    Confidence, Fraction, NonEmptyString, OrderedChecks, Provenance, Provider, SaveState, Segment,
    Sha256Hash, SubmissionOutcome, TextInput, TextOrigin, UpstreamIdentity, UpstreamTaskId,
    UtcTimestamp,
};
use microck_pangram_cli::output::{
    AnalysisOutput, CanonicalError, CommandData, CommandEnvelope, EnvelopeMeta, ErrorCode,
    Recovery, ResolvedCommand,
};

const FIXED_ID: &str = "anl_01983c20-0180-7a80-a001-000000000001";
const FIXED_CREATED: &str = "2026-07-23T12:00:00Z";
const FIXED_UPDATED: &str = "2026-07-23T12:00:01Z";

pub fn input_sha() -> Sha256Hash {
    Sha256Hash::digest(b"The text to analyze")
}

pub fn created() -> UtcTimestamp {
    UtcTimestamp::from_str(FIXED_CREATED).unwrap()
}

pub fn updated() -> UtcTimestamp {
    UtcTimestamp::from_str(FIXED_UPDATED).unwrap()
}

pub fn started_meta() -> EnvelopeMeta {
    EnvelopeMeta::default().with_started_at(created())
}

pub fn failed_meta() -> EnvelopeMeta {
    EnvelopeMeta::default().with_failed_at(created())
}

fn segment(text: Option<&str>) -> Segment {
    Segment {
        text: text.unwrap_or("The text to analyze").to_owned(),
        label: NonEmptyString::new("Human Written").unwrap(),
        ai_assistance_score: Fraction::new(0.0).unwrap(),
        confidence: Confidence::High,
        start_index: 0,
        end_index: 19,
        word_count: 4,
        token_length: 4,
        humanizer_score: Some(Fraction::new(0.0).unwrap()),
        is_humanized: Some(false),
    }
}

fn ai_result(segments: Vec<Segment>) -> AiDetectionResult {
    AiDetectionResult {
        classification: AiClassification::Human,
        headline: "Human-written".to_owned(),
        prediction: "The document appears to be human-written.".to_owned(),
        fraction_ai: Fraction::new(0.0).unwrap(),
        fraction_ai_assisted: Fraction::new(0.0).unwrap(),
        fraction_human: Fraction::new(1.0).unwrap(),
        num_ai_segments: 0,
        num_ai_assisted_segments: 0,
        num_human_segments: 1,
        segments,
        dashboard_link: None,
    }
}

fn text_input(origin: TextOrigin, name: Option<String>, text: Option<String>) -> TextInput {
    TextInput::new(origin, name, input_sha(), 19, 4, text).unwrap()
}

fn provenance() -> Provenance {
    Provenance {
        provider: Provider::Pangram,
        upstream_version: Some("4.0".to_owned()),
        upstream_task_ids: None,
        upstream_bulk_id: None,
        submitted_at: None,
        completed_at: None,
    }
}

fn succeeded_ai_analysis(input: TextInput, result: AiDetectionResult) -> Analysis<CanonicalError> {
    let upstream = UpstreamIdentity {
        task_id: Some(UpstreamTaskId::new("task-123").unwrap()),
        last_stage: Some(NonEmptyString::new("STAGE_SUCCESS").unwrap()),
    };
    let checks = OrderedChecks::new([Check::AiDetection(CheckState::Succeeded {
        upstream: Some(upstream),
        result,
    })])
    .unwrap();
    Analysis::new(
        AnalysisId::from_str(FIXED_ID).unwrap(),
        SubmissionOutcome::Terminal,
        AnalysisInput::Text(input),
        checks,
        SaveState::Ephemeral,
        provenance(),
        None,
        None,
        created(),
        updated(),
        Some(updated()),
    )
    .unwrap()
}

/// The fixed valid success envelope used by the projection goldens: one
/// AI-detection analysis with input content omitted (the privacy default).
pub fn success_envelope() -> CommandEnvelope {
    let analysis = succeeded_ai_analysis(
        text_input(TextOrigin::Stdin, None, None),
        ai_result(vec![segment(None)]),
    );
    CommandEnvelope::success(
        CommandData::Detect(AnalysisOutput::one(analysis)),
        started_meta(),
    )
}

/// The fixed canonical failure envelope used by the projection goldens.
pub fn failure_envelope() -> CommandEnvelope {
    let recovery = Recovery::new("Configure a persistent key or set PANGRAM_API_KEY.")
        .unwrap()
        .with_command("pangram auth")
        .unwrap();
    let error = CanonicalError::new(
        ErrorCode::MissingApiKey,
        "No Pangram API key is configured.",
    )
    .unwrap()
    .with_recovery(recovery)
    .unwrap();
    CommandEnvelope::failure(ResolvedCommand::Detect, error, failed_meta())
}

/// A second fixed analysis so JSONL order is observable.
pub fn second_analysis() -> Analysis<CanonicalError> {
    // Re-wrap with a distinct upstream task id so the two envelopes differ.
    let upstream = UpstreamIdentity {
        task_id: Some(UpstreamTaskId::new("task-456").unwrap()),
        last_stage: Some(NonEmptyString::new("STAGE_SUCCESS").unwrap()),
    };
    let checks = OrderedChecks::new([Check::AiDetection(CheckState::Succeeded {
        upstream: Some(upstream),
        result: ai_result(vec![segment(None)]),
    })])
    .unwrap();
    Analysis::new(
        AnalysisId::from_str(FIXED_ID).unwrap(),
        SubmissionOutcome::Terminal,
        AnalysisInput::Text(text_input(TextOrigin::Stdin, None, None)),
        checks,
        SaveState::Ephemeral,
        provenance(),
        None,
        None,
        created(),
        updated(),
        Some(updated()),
    )
    .unwrap()
}

/// An adversarial payload that embeds terminal control sequences and Markdown
/// structure in every untrusted free-text field the projection renders.
pub mod adversarial {
    pub const HEADLINE: &str =
        "Human-written\u{1b}[31m# forged heading\n```rust\n[pwn](https://evil.example)";
    pub const PREDICTION: &str =
        "Looks human\u{1b}]8;;https://evil.example\u{7}link\u{1b}]8;;\u{7}| pipe";
    pub const SEGMENT_TEXT: &str =
        "ok text\u{1b}[0m\x00\x07\x7f\u{9b}# heading\n- [x] `code` | cell";
    pub const FILE_NAME: &str = "notes\u{1b}[1m# a\n[link](x).txt";

    /// A sentinel unique to the explicit-input fixture so an `input.text` leak
    /// cannot be confused with a segment-text match.
    pub const INPUT_SENTINEL: &str =
        "unique-input-sentinel xQmZ9 with # forged\nheading and \u{1b}[31mANSI";
}

/// A success envelope whose untrusted free-text fields carry the adversarial
/// payload, proving machine projections stay canonical and human projections
/// stay escaped.
pub fn adversarial_envelope() -> CommandEnvelope {
    let mut result = ai_result(vec![segment(Some(adversarial::SEGMENT_TEXT))]);
    result.headline = adversarial::HEADLINE.to_owned();
    result.prediction = adversarial::PREDICTION.to_owned();
    let analysis = succeeded_ai_analysis(
        text_input(
            TextOrigin::File,
            Some(adversarial::FILE_NAME.to_owned()),
            None,
        ),
        result,
    );
    CommandEnvelope::success(
        CommandData::Detect(AnalysisOutput::one(analysis)),
        started_meta(),
    )
}

/// A success envelope whose input content is present (`--include-input`), used
/// for the privacy regression: machine formats keep it byte-exact, human
/// formats sanitize the control characters it carries. The input content uses
/// a sentinel distinct from any segment text so an input leak is unambiguous.
pub fn input_content_envelope() -> CommandEnvelope {
    let mut result = ai_result(vec![segment(None)]);
    result.headline = "Human-written".to_owned();
    let analysis = succeeded_ai_analysis(
        text_input(
            TextOrigin::Stdin,
            None,
            Some(adversarial::INPUT_SENTINEL.to_owned()),
        ),
        result,
    );
    CommandEnvelope::success(
        CommandData::Detect(AnalysisOutput::one(analysis)),
        started_meta(),
    )
}
