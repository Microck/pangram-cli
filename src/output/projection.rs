//! Canonical output projections for every adapter.
//!
//! This module is the single owner of the five Phase 2 projections. Each one
//! consumes the canonical typed [`CommandEnvelope`] exactly as the output and
//! domain modules constructed it; no projection re-runs domain logic, derives
//! a status, or adds or removes a privacy field. JSON, JSONL, and TOON are
//! machine projections of the canonical JSON value and never sanitize,
//! truncate, or reorder content. Markdown and pretty render the same typed
//! data for humans and sanitize control characters plus Markdown structure.
//!
//! Every renderer streams bytes into a caller-supplied [`Write`] and returns
//! `io::Result`, so a short write, broken pipe, or flush failure propagates as
//! the honest general failure instead of claiming success.

use std::io::{self, Write};

use crate::domain::{
    Analysis, AnalysisInput, Check, CheckState, LocalOperationId, SaveState, SubmissionOutcome,
};

use super::{AnalysisOutput, CanonicalError, CommandData, CommandEnvelope, DoctorStatus};

const RESET: &str = "\u{1b}[0m";
const BOLD: &str = "\u{1b}[1m";
const DIM: &str = "\u{1b}[2m";
const GREEN: &str = "\u{1b}[32m";
const YELLOW: &str = "\u{1b}[33m";
const RED: &str = "\u{1b}[31m";
const CYAN: &str = "\u{1b}[36m";

/// The error produced when a projection itself cannot be produced.
///
/// TOON encoding is total for the canonical envelope's owned value space, so
/// this is reserved for the unreachable contradiction where the codec rejects
/// an already-valid union value. It surfaces as `Other` inside
/// [`ProjectionCause::Toon`].
#[derive(Debug)]
pub enum ProjectionCause {
    Toon(toon_format::ToonError),
}

impl std::fmt::Display for ProjectionCause {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Toon(error) => write!(
                formatter,
                "the canonical envelope cannot be encoded as TOON: {error}"
            ),
        }
    }
}

impl std::error::Error for ProjectionCause {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Toon(error) => Some(error),
        }
    }
}

/// The canonical projected output format. The closed set is the locked v1
/// contract: JSON, JSONL, TOON, Markdown, and pretty terminal output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputFormat {
    Json,
    Jsonl,
    Toon,
    Markdown,
    Pretty,
}

/// Whether the pretty projection emits caller-owned ANSI color.
///
/// The output core never inspects the terminal or environment: the caller
/// resolves TTY state, `--no-color`, and `NO_COLOR` and passes the result here.
/// When `Plain`, no byte of a control sequence enters the projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorPolicy {
    /// No ANSI of any kind. Safe for pipes, logs, and every machine consumer.
    #[default]
    Plain,
    /// The caller explicitly enabled color. Trusted structural markers only;
    /// untrusted payload text is always sanitized, never colored by content.
    Color,
}

/// Replaces every control character with U+FFFD so untrusted payload text
/// cannot inject terminal control sequences or forge line structure.
///
/// `char::is_control` covers the ASCII C0 range (newline, carriage return,
/// tab, and the ANSI `ESC` byte), `DEL`, and the C1 range (U+0080 through
/// U+009F, including the CSI and OSC introducers some terminals honor).
/// Printable text, including the ordinary space and all non-control Unicode,
/// passes through unchanged. Machine projections do not call this; they emit
/// the canonical value exactly.
pub(crate) fn sanitize_terminal(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { '\u{FFFD}' } else { ch })
        .collect()
}

/// Escapes the Markdown constructs a value could forge in the positions this
/// renderer places it. Links require `[`, so neutralizing brackets (with code
/// fences, emphasis, headings, table pipes, HTML, and backslash) blocks every
/// structural injection while leaving common punctuation like parentheses
/// readable: value slots are never at a list/heading/emphasis boundary, so `-`,
/// `+`, `(`, `)`, and `_` pass through as prose. Sanitization runs first.
fn markdown_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in sanitize_terminal(text).chars() {
        if matches!(ch, '\\' | '`' | '*' | '[' | ']' | '<' | '>' | '|' | '#') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Renders machine projections directly onto an already-valid serialized
/// envelope. None of these branches recompute content.
fn write_json(envelope: &CommandEnvelope, out: &mut dyn Write) -> io::Result<()> {
    serde_json::to_writer(&mut *out, envelope).map_err(io::Error::other)?;
    out.write_all(b"\n")
}

fn write_jsonl(envelopes: &[CommandEnvelope], out: &mut dyn Write) -> io::Result<()> {
    for envelope in envelopes {
        write_json(envelope, out)?;
    }
    Ok(())
}

fn write_toon(envelope: &CommandEnvelope, out: &mut dyn Write) -> io::Result<()> {
    let encoded = toon_format::encode_default(envelope)
        .map_err(|error| io::Error::other(ProjectionCause::Toon(error)))?;
    out.write_all(encoded.as_bytes())?;
    out.write_all(b"\n")
}

// Human Markdown/pretty rendering of the same typed union.

fn analysis_status_label(status: crate::domain::AnalysisStatus) -> &'static str {
    match status {
        crate::domain::AnalysisStatus::Queued => "queued",
        crate::domain::AnalysisStatus::Running => "running",
        crate::domain::AnalysisStatus::Succeeded => "succeeded",
        crate::domain::AnalysisStatus::Failed => "failed",
        crate::domain::AnalysisStatus::Partial => "partial",
    }
}

fn submission_label(outcome: SubmissionOutcome) -> &'static str {
    match outcome {
        SubmissionOutcome::NotSubmitted => "not_submitted",
        SubmissionOutcome::Accepted => "accepted",
        SubmissionOutcome::Terminal => "terminal",
        SubmissionOutcome::AcceptanceUnknown => "acceptance_unknown",
    }
}

fn save_label(state: SaveState) -> &'static str {
    match state {
        SaveState::Ephemeral => "ephemeral",
        SaveState::SavedManual => "saved_manual",
        SaveState::SavedHistory => "saved_history",
    }
}

fn input_summary(input: &AnalysisInput) -> Vec<(&'static str, String)> {
    match input {
        AnalysisInput::Text(text) => {
            let mut summary = format!("text ({})", origin_label(text.origin()));
            if let Some(name) = text.name() {
                summary.push_str(&format!(" {name}"));
            }
            summary.push_str(&format!(" {} words", text.word_count));
            let mut fields = vec![("input", summary)];
            // `--include-input` content is part of the canonical value, so a
            // human projection surfaces it (sanitized by the renderer's own
            // escape path) rather than dropping a present privacy field.
            if let Some(content) = &text.text {
                fields.push(("input_text", content.clone()));
            }
            fields
        }
        AnalysisInput::File(file) => {
            let summary = format!(
                "file {} ({}, {} bytes)",
                file.filename.as_str(),
                file.media_type.as_str(),
                file.size_bytes
            );
            let mut fields = vec![("input", summary)];
            if let Some(path) = &file.path {
                fields.push(("input_path", path.clone()));
            }
            if let Some(extracted) = &file.extracted_text {
                fields.push(("input_extracted_text", extracted.clone()));
            }
            fields
        }
    }
}

fn origin_label(origin: crate::domain::TextOrigin) -> &'static str {
    match origin {
        crate::domain::TextOrigin::Literal => "literal",
        crate::domain::TextOrigin::Stdin => "stdin",
        crate::domain::TextOrigin::File => "file",
        crate::domain::TextOrigin::Unknown => "unknown",
    }
}

trait HumanWriter {
    fn escape<T: AsRef<str>>(&self, text: T) -> String;
    fn heading(&mut self, level: u8, text: &str) -> io::Result<()>;
    fn label_value(&mut self, label: &str, value: &str) -> io::Result<()>;
    fn line(&mut self, text: &str) -> io::Result<()>;
    fn blank(&mut self) -> io::Result<()>;
    /// Colors a trusted enum/status token when color is enabled. Markdown and
    /// plain pretty return it unchanged; payload text never passes through it.
    fn token(&self, text: &str) -> String {
        text.to_owned()
    }
}

struct MarkdownWriter<'a> {
    out: &'a mut dyn Write,
}

struct PrettyWriter<'a> {
    out: &'a mut dyn Write,
    color: ColorPolicy,
}

fn do_color(policy: ColorPolicy, code: &str, text: &str) -> String {
    if policy == ColorPolicy::Color {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

impl PrettyWriter<'_> {
    fn style(&self, code: &str, text: &str) -> String {
        do_color(self.color, code, text)
    }
}

impl HumanWriter for MarkdownWriter<'_> {
    fn escape<T: AsRef<str>>(&self, text: T) -> String {
        markdown_escape(text.as_ref())
    }

    fn heading(&mut self, level: u8, text: &str) -> io::Result<()> {
        let marker = "#".repeat(level as usize);
        writeln!(self.out, "{marker} {text}")
    }

    fn label_value(&mut self, label: &str, value: &str) -> io::Result<()> {
        writeln!(self.out, "- **{label}:** {value}")
    }

    fn line(&mut self, text: &str) -> io::Result<()> {
        writeln!(self.out, "{text}")
    }

    fn blank(&mut self) -> io::Result<()> {
        writeln!(self.out)
    }
}

impl HumanWriter for PrettyWriter<'_> {
    fn escape<T: AsRef<str>>(&self, text: T) -> String {
        sanitize_terminal(text.as_ref())
    }

    fn heading(&mut self, _level: u8, text: &str) -> io::Result<()> {
        writeln!(self.out, "{}", self.style(BOLD, text))
    }

    fn label_value(&mut self, label: &str, value: &str) -> io::Result<()> {
        writeln!(self.out, "  {}: {value}", self.style(DIM, label))
    }

    fn line(&mut self, text: &str) -> io::Result<()> {
        writeln!(self.out, "{text}")
    }

    fn blank(&mut self) -> io::Result<()> {
        writeln!(self.out)
    }

    fn token(&self, text: &str) -> String {
        let code = match text {
            "succeeded" | "human" | "terminal" | "pass" => GREEN,
            "failed" | "fail" => RED,
            "partial" | "mixed" | "warn" | "running" | "accepted" | "acceptance_unknown" => YELLOW,
            "queued" | "not_submitted" | "ephemeral" => DIM,
            _ => CYAN,
        };
        self.style(code, text)
    }
}

fn meta_lines<W: HumanWriter>(writer: &mut W, envelope: &CommandEnvelope) -> io::Result<()> {
    let meta = envelope.meta();
    if let Some(started) = meta.started_at() {
        writer.label_value("started_at", &started.to_string())?;
    }
    if let Some(completed) = meta.completed_at() {
        writer.label_value("completed_at", &completed.to_string())?;
    }
    if let Some(failed) = meta.failed_at() {
        writer.label_value("failed_at", &failed.to_string())?;
    }
    if let Some(duration) = meta.duration_ms() {
        writer.label_value("duration_ms", &duration.to_string())?;
    }
    Ok(())
}

fn write_analysis_markdown<W: HumanWriter>(
    writer: &mut W,
    analysis: &Analysis<CanonicalError>,
) -> io::Result<()> {
    // Trusted enum values go through `token` so pretty can color them; free
    // text goes through `escape` and is never colored by its content.
    let status = analysis_status_label(analysis.status()).to_owned();
    writer.heading(2, "Analysis")?;
    writer.label_value("id", &analysis.id.to_string())?;
    writer.label_value("status", &writer.token(&status))?;
    writer.label_value(
        "submission_outcome",
        submission_label(analysis.submission_outcome()),
    )?;
    if let Some(input) = analysis.input() {
        for (label, value) in input_summary(input) {
            writer.label_value(&writer.escape(label), &writer.escape(value))?;
        }
    }
    writer.label_value("save_state", save_label(analysis.save_state))?;
    writer.label_value("created_at", &analysis.created_at.to_string())?;
    writer.label_value("updated_at", &analysis.updated_at.to_string())?;
    if let Some(completed) = analysis.completed_at {
        writer.label_value("completed_at", &completed.to_string())?;
    }
    writer.blank()?;
    // Segment/evidence text echoes submitted content: emit it in the human
    // projection only when the user opted in to echoing input
    // (`--include-input`, which makes the canonical input record carry the
    // text). Headlines/predictions and all numeric evidence still always
    // render.
    let echo_segment_text = match analysis.input() {
        Some(AnalysisInput::Text(text)) => text.text.is_some(),
        Some(AnalysisInput::File(file)) => file.extracted_text.is_some(),
        None => false,
    };
    writer.heading(3, "Checks")?;
    for check in analysis.checks() {
        match check {
            Check::AiDetection(state) => {
                write_check(writer, "ai_detection", state, echo_segment_text)?
            }
            Check::Plagiarism(state) => {
                write_check(writer, "plagiarism", state, echo_segment_text)?
            }
        }
    }
    Ok(())
}

/// Renders one check. The check variant fixes its result type, so this
/// function pattern-matches each kind with its own state type and calls the
/// concrete result renderer without any downcast.
trait RenderResult {
    fn render<W: HumanWriter>(&self, writer: &mut W, echo_text: bool) -> io::Result<()>;
}

impl RenderResult for crate::domain::AiDetectionResult {
    fn render<W: HumanWriter>(&self, writer: &mut W, echo_text: bool) -> io::Result<()> {
        write_ai_result(writer, self, echo_text)
    }
}

impl RenderResult for crate::domain::PlagiarismResult {
    fn render<W: HumanWriter>(&self, writer: &mut W, echo_text: bool) -> io::Result<()> {
        write_plagiarism_result(writer, self, echo_text)
    }
}

fn write_check<R: RenderResult, W: HumanWriter>(
    writer: &mut W,
    kind: &str,
    state: &CheckState<R, CanonicalError>,
    echo_text: bool,
) -> io::Result<()> {
    let status = check_status_label(state.status()).to_owned();
    writer.label_value("kind", &writer.escape(kind))?;
    writer.label_value("status", &writer.token(&status))?;
    if let Some(identity) = state_upstream(state) {
        if let Some(task) = &identity.task_id {
            writer.label_value("upstream_task_id", &writer.escape(task.as_str()))?;
        }
        if let Some(stage) = &identity.last_stage {
            writer.label_value("upstream_stage", &writer.escape(stage.as_str()))?;
        }
    }
    match state {
        CheckState::Succeeded { result, .. } => result.render(writer, echo_text)?,
        CheckState::Failed { error, .. } => write_error(writer, error)?,
        CheckState::Queued { .. } | CheckState::Running { .. } => {}
    }
    Ok(())
}

fn state_upstream<R>(
    state: &CheckState<R, CanonicalError>,
) -> Option<&crate::domain::UpstreamIdentity> {
    match state {
        CheckState::Queued { upstream }
        | CheckState::Running { upstream }
        | CheckState::Succeeded { upstream, .. }
        | CheckState::Failed { upstream, .. } => upstream.as_ref(),
    }
}

fn write_plagiarism_result<W: HumanWriter>(
    writer: &mut W,
    result: &crate::domain::PlagiarismResult,
    echo_text: bool,
) -> io::Result<()> {
    writer.label_value(
        "plagiarism_detected",
        &result.plagiarism_detected.to_string(),
    )?;
    writer.label_value("total_sentences", &result.total_sentences.to_string())?;
    writer.label_value(
        "plagiarized_sentence_count",
        &result.plagiarized_sentence_count.to_string(),
    )?;
    writer.label_value(
        "percent_plagiarized",
        &result.percent_plagiarized.get().to_string(),
    )?;
    writer.label_value("matches", &result.matches.len().to_string())?;
    for match_ in &result.matches {
        writer.line(&format!("- {}", writer.escape(&match_.source_url)))?;
        // Matched text echoes submitted content: gated on --include-input
        // like segment text.
        if echo_text {
            writer.label_value("matched_text", &writer.escape(&match_.matched_text))?;
        }
        writer.label_value(
            "similarity_score",
            &match_.similarity_score.get().to_string(),
        )?;
    }
    Ok(())
}

fn check_status_label(status: crate::domain::CheckStatus) -> &'static str {
    match status {
        crate::domain::CheckStatus::Queued => "queued",
        crate::domain::CheckStatus::Running => "running",
        crate::domain::CheckStatus::Succeeded => "succeeded",
        crate::domain::CheckStatus::Failed => "failed",
    }
}

fn write_ai_result<W: HumanWriter>(
    writer: &mut W,
    result: &crate::domain::AiDetectionResult,
    echo_text: bool,
) -> io::Result<()> {
    writer.label_value("headline", &writer.escape(&result.headline))?;
    writer.label_value("prediction", &writer.escape(&result.prediction))?;
    writer.label_value(
        "classification",
        &writer.token(classification_label(result.classification)),
    )?;
    writer.label_value("fraction_ai", &result.fraction_ai.get().to_string())?;
    writer.label_value(
        "fraction_ai_assisted",
        &result.fraction_ai_assisted.get().to_string(),
    )?;
    writer.label_value("fraction_human", &result.fraction_human.get().to_string())?;
    writer.label_value("segments", &result.segments.len().to_string())?;
    for segment in &result.segments {
        // Segment text echoes submitted content: present only when the run
        // opted into echoing input (`--include-input`).
        if echo_text {
            writer.line(&format!("- {}", writer.escape(&segment.text)))?;
        }
        writer.label_value("label", &writer.escape(segment.label.as_str()))?;
        writer.label_value(
            "confidence",
            &writer.token(confidence_label(segment.confidence)),
        )?;
        writer.label_value(
            "ai_assistance_score",
            &segment.ai_assistance_score.get().to_string(),
        )?;
        if let Some(score) = segment.humanizer_score {
            writer.label_value("humanizer_score", &score.get().to_string())?;
        }
        if let Some(is_humanized) = segment.is_humanized {
            writer.label_value("is_humanized", &is_humanized.to_string())?;
        }
        writer.label_value(
            "range",
            &format!("{}..{}", segment.start_index, segment.end_index),
        )?;
        writer.label_value("word_count", &segment.word_count.to_string())?;
        writer.label_value("token_length", &segment.token_length.to_string())?;
    }
    if let Some(link) = &result.dashboard_link {
        writer.label_value("dashboard_link", &writer.escape(link))?;
    }
    Ok(())
}

fn classification_label(value: crate::domain::AiClassification) -> &'static str {
    match value {
        crate::domain::AiClassification::Ai => "ai",
        crate::domain::AiClassification::Human => "human",
        crate::domain::AiClassification::Mixed => "mixed",
    }
}

fn confidence_label(value: crate::domain::Confidence) -> &'static str {
    match value {
        crate::domain::Confidence::High => "high",
        crate::domain::Confidence::Medium => "medium",
        crate::domain::Confidence::Low => "low",
    }
}

fn write_error<W: HumanWriter>(writer: &mut W, error: &CanonicalError) -> io::Result<()> {
    writer.label_value("code", error.code().as_str())?;
    writer.label_value("category", error.category().as_str())?;
    writer.label_value("message", &writer.escape(error.message()))?;
    writer.label_value("retryable", &error.retryable().to_string())?;
    if let Some(ms) = error.retry_after_ms() {
        writer.label_value("retry_after_ms", &ms.to_string())?;
    }
    if let Some(recovery) = error.recovery() {
        writer.label_value("recovery", &writer.escape(recovery.message()))?;
        if let Some(command) = recovery.command() {
            writer.label_value("recovery_command", &writer.escape(command))?;
        }
    }
    if let Some(details) = error.details() {
        write_error_details(writer, details)?;
    }
    Ok(())
}

fn write_error_details<W: HumanWriter>(
    writer: &mut W,
    details: &super::CanonicalErrorDetails,
) -> io::Result<()> {
    match details {
        super::CanonicalErrorDetails::SubmissionOutcomeUnknown(details) => {
            let id = match details.operation_id() {
                LocalOperationId::AnalysisId(id) => id.to_string(),
                LocalOperationId::BulkId(id) => id.to_string(),
            };
            writer.label_value("operation_id", &writer.escape(&id))?;
            writer.label_value(
                "request_sha256",
                &writer.escape(details.request_sha256.to_string()),
            )?;
            if let Some(task) = &details.upstream_task_id {
                writer.label_value("upstream_task_id", &writer.escape(task.as_str()))?;
            }
            if let Some(bulk) = &details.upstream_bulk_id {
                writer.label_value("upstream_bulk_id", &writer.escape(bulk.as_str()))?;
            }
            writer.label_value("last_status", &writer.escape(details.last_status.as_str()))?;
        }
        super::CanonicalErrorDetails::Fields(fields) => {
            for (key, value) in fields {
                let rendered = match value {
                    serde_json::Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                writer.label_value(&writer.escape(key), &writer.escape(rendered))?;
            }
        }
    }
    Ok(())
}

fn write_doctor<W: HumanWriter>(writer: &mut W, status: &DoctorStatus) -> io::Result<()> {
    writer.heading(2, "Checks")?;
    for check in status.checks() {
        let marker = match check.status() {
            super::DoctorCheckStatus::Pass => "pass",
            super::DoctorCheckStatus::Warn => "warn",
            super::DoctorCheckStatus::Fail => "fail",
        };
        let marker = writer.token(marker);
        let line = match check.message() {
            Some(message) => format!("{marker} {}: {}", check.name(), writer.escape(message)),
            None => format!("{marker} {}", check.name()),
        };
        writer.line(&line)?;
    }
    Ok(())
}

fn write_success_data<W: HumanWriter>(writer: &mut W, data: &CommandData) -> io::Result<()> {
    match data {
        CommandData::Detect(AnalysisOutput::One(analysis))
        | CommandData::Plagiarism(AnalysisOutput::One(analysis))
        | CommandData::Analyze(AnalysisOutput::One(analysis)) => {
            write_analysis_markdown(writer, analysis.as_ref())?;
        }
        CommandData::TaskStatus(analysis)
        | CommandData::TaskWait(analysis)
        | CommandData::HistoryShow(analysis)
        | CommandData::HistoryRerun(analysis) => {
            write_analysis_markdown(writer, analysis)?;
        }
        CommandData::Detect(AnalysisOutput::Many(analyses))
        | CommandData::Plagiarism(AnalysisOutput::Many(analyses))
        | CommandData::Analyze(AnalysisOutput::Many(analyses)) => {
            writer.heading(2, "Analyses")?;
            for analysis in analyses.as_slice() {
                write_analysis_markdown(writer, analysis)?;
            }
        }
        CommandData::Doctor(status) => write_doctor(writer, status)?,
        other => render_generic_json(writer, other)?,
    }
    Ok(())
}

/// The fallback for non-analysis success payloads renders the canonical JSON
/// value so no typed field is dropped and every scalar is sanitized through
/// the same escape path.
fn render_generic_json<W: HumanWriter>(writer: &mut W, data: &CommandData) -> io::Result<()> {
    let value = serde_json::to_value(data).map_err(io::Error::other)?;
    writer.line(&format!(
        "```json\n{}\n```",
        writer.escape(value.to_string())
    ))
}

fn render_envelope<W: HumanWriter>(writer: &mut W, envelope: &CommandEnvelope) -> io::Result<()> {
    writer.heading(1, "Pangram")?;
    writer.label_value("schema_version", "1")?;
    writer.label_value("command", envelope.command().as_str())?;
    meta_lines(writer, envelope)?;
    match (envelope.data(), envelope.error()) {
        (Some(data), None) => {
            writer.blank()?;
            write_success_data(writer, data)?;
        }
        (None, Some(error)) => {
            writer.blank()?;
            writer.heading(2, "Error")?;
            write_error(writer, error)?;
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "an envelope must contain exactly one of data or error",
            ));
        }
    }
    Ok(())
}

fn write_markdown(envelope: &CommandEnvelope, out: &mut dyn Write) -> io::Result<()> {
    let mut writer = MarkdownWriter { out };
    render_envelope(&mut writer, envelope)
}

fn write_pretty(
    envelope: &CommandEnvelope,
    color: ColorPolicy,
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut writer = PrettyWriter { out, color };
    render_envelope(&mut writer, envelope)
}

/// Renders one envelope or an ordered series of envelopes to `out` in the
/// requested format. JSON succeeds and fails on the canonical envelope. JSONL
/// writes one canonical envelope per line in the supplied input order and is
/// the format a caller selects to render repeated files without changing a
/// single envelope's semantics. TOON, Markdown, and pretty accept one envelope
/// per render and reject a multi-envelope series so a caller cannot silently
/// downgrade repeated work into a single document.
///
/// Every format ends its output with a flush; a short write or flush failure
/// propagates as `Err`.
pub fn render(
    format: OutputFormat,
    color: ColorPolicy,
    envelopes: &[CommandEnvelope],
    out: &mut dyn Write,
) -> io::Result<()> {
    match format {
        OutputFormat::Json => {
            let envelope = single(envelopes, "JSON")?;
            write_json(envelope, out)?;
        }
        OutputFormat::Jsonl => {
            if envelopes.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "JSONL projection requires at least one envelope",
                ));
            }
            write_jsonl(envelopes, out)?;
        }
        OutputFormat::Toon => {
            let envelope = single(envelopes, "TOON")?;
            write_toon(envelope, out)?;
        }
        OutputFormat::Markdown => {
            let envelope = single(envelopes, "Markdown")?;
            write_markdown(envelope, out)?;
        }
        OutputFormat::Pretty => {
            let envelope = single(envelopes, "pretty")?;
            write_pretty(envelope, color, out)?;
        }
    }
    out.flush()
}

fn single<'a>(envelopes: &'a [CommandEnvelope], name: &str) -> io::Result<&'a CommandEnvelope> {
    match envelopes {
        [envelope] => Ok(envelope),
        [] => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} projection requires exactly one envelope, got none"),
        )),
        many => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{name} projection requires exactly one envelope, got {}",
                many.len()
            ),
        )),
    }
}
