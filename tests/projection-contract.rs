//! Contract tests for the five canonical output projections.
//!
//! Every assertion consumes typed envelopes built by the shared fixtures, so a
//! projection can never be fed a value that would not survive the schema. The
//! goldens for Markdown and pretty are recorded after the implementation is
//! frozen; JSON/JSONL/TOON assert semantic equality against the canonical
//! serialization and the `toon-format` codec rather than a literal string so a
//! serde field-order refactor cannot create noise.

use microck_pangram_cli::output::{
    AnalysisOutput, CanonicalError, ColorPolicy, CommandData, CommandEnvelope, EnvelopeMeta,
    ErrorCode, OutputFormat, Recovery, ResolvedCommand, render,
};

#[path = "support/projection-fixtures.rs"]
mod fixtures;

use fixtures::{
    adversarial, adversarial_envelope, created, failure_envelope, input_content_envelope,
    second_analysis, started_meta, success_envelope, updated,
};

fn render_string(
    format: OutputFormat,
    color: ColorPolicy,
    envelopes: &[CommandEnvelope],
) -> String {
    let mut bytes = Vec::new();
    render(format, color, envelopes, &mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}

fn missing_key_envelope() -> CommandEnvelope {
    let recovery = Recovery::new("Configure a key")
        .unwrap()
        .with_command("pangram auth set --api-key-stdin")
        .unwrap();
    let error = CanonicalError::new(
        ErrorCode::MissingApiKey,
        "No Pangram API key is configured.",
    )
    .unwrap()
    .with_recovery(recovery)
    .unwrap();
    let meta = EnvelopeMeta::default()
        .with_started_at(created())
        .with_duration_ms(12345)
        .with_failed_at(updated());
    CommandEnvelope::failure(ResolvedCommand::Detect, error, meta)
}

fn assert_no_ansi(text: &str) {
    assert!(
        !text.contains('\u{1b}') && !text.contains('\u{9b}'),
        "output contains a terminal escape/C1 introducer: {text:?}"
    );
}

fn assert_one_trailing_newline(text: &str) {
    assert!(text.ends_with('\n'), "missing trailing newline: {text:?}");
    assert!(!text.ends_with("\n\n"), "extra trailing newline: {text:?}");
}

#[test]
fn json_success_envelope_is_canonical_parseable_and_single_line() {
    let envelope = success_envelope();
    let rendered = render_string(
        OutputFormat::Json,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );

    let expected = serde_json::to_string(&envelope).unwrap();
    assert_eq!(rendered, format!("{expected}\n"), "JSON golden drift");

    assert_no_ansi(&rendered);
    assert_one_trailing_newline(&rendered);
    assert!(
        !rendered[..rendered.len() - 1].contains('\n'),
        "JSON must be one line"
    );

    let decoded: serde_json::Value = serde_json::from_str(rendered.trim_end()).unwrap();
    assert_eq!(decoded["schema_version"], "1");
    assert_eq!(decoded["command"], "detect");
    assert!(decoded.get("error").is_none());
    let roundtrip: CommandEnvelope = serde_json::from_str(rendered.trim_end()).unwrap();
    assert_eq!(
        roundtrip, envelope,
        "JSON must roundtrip to the same envelope"
    );
}

#[test]
fn json_failure_envelope_matches_exact_canonical_serialization() {
    let envelope = failure_envelope();
    let rendered = render_string(
        OutputFormat::Json,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    let expected = serde_json::to_string(&envelope).unwrap();
    assert_eq!(
        rendered,
        format!("{expected}\n"),
        "JSON failure golden drift"
    );

    let decoded: serde_json::Value = serde_json::from_str(rendered.trim_end()).unwrap();
    assert_eq!(decoded["error"]["code"], "missing_api_key");
    assert!(decoded.get("data").is_none());
}

#[test]
fn jsonl_writes_one_canonical_envelope_per_line_in_input_order_with_no_wrapper() {
    let first = success_envelope();
    let second_envelope = CommandEnvelope::success(
        CommandData::Detect(AnalysisOutput::one(second_analysis())),
        started_meta(),
    );
    let rendered = render_string(
        OutputFormat::Jsonl,
        ColorPolicy::Plain,
        &[first.clone(), second_envelope.clone()],
    );

    assert_no_ansi(&rendered);
    assert!(
        !rendered.starts_with('['),
        "JSONL must not be a wrapper array"
    );

    let mut lines = rendered.lines();
    let first_line = lines.next().unwrap();
    let second_line = lines.next().unwrap();
    assert!(lines.next().is_none(), "more than two JSONL lines");
    assert!(
        rendered.ends_with('\n'),
        "MUST end each line, including the last"
    );

    let first_json: serde_json::Value = serde_json::from_str(first_line).unwrap();
    let second_json: serde_json::Value = serde_json::from_str(second_line).unwrap();
    assert_eq!(
        first_json["data"]["checks"][0]["upstream"]["task_id"],
        "task-123"
    );
    assert_eq!(
        second_json["data"]["checks"][0]["upstream"]["task_id"],
        "task-456"
    );

    let first_exact: CommandEnvelope = serde_json::from_str(first_line).unwrap();
    let second_exact: CommandEnvelope = serde_json::from_str(second_line).unwrap();
    assert_eq!(first_exact, first);
    assert_eq!(second_exact, second_envelope);
}

#[test]
fn repeated_jsonl_calls_append_one_envelope_per_line_for_repeated_file_use() {
    let first = success_envelope();
    let second_envelope = CommandEnvelope::success(
        CommandData::Detect(AnalysisOutput::one(second_analysis())),
        started_meta(),
    );

    // A repeated-file caller stitches batches: each call must produce complete
    // lines that concatenate into valid JSONL without separators or wrappers.
    let mut buffer = Vec::new();
    render(
        OutputFormat::Jsonl,
        ColorPolicy::Plain,
        std::slice::from_ref(&first),
        &mut buffer,
    )
    .unwrap();
    render(
        OutputFormat::Jsonl,
        ColorPolicy::Plain,
        std::slice::from_ref(&second_envelope),
        &mut buffer,
    )
    .unwrap();
    let joined = String::from_utf8(buffer).unwrap();

    let lines: Vec<&str> = joined.lines().collect();
    assert_eq!(lines.len(), 2, "joined repeated JSONL must be two lines");
    let first_decoded: CommandEnvelope = serde_json::from_str(lines[0]).unwrap();
    let second_decoded: CommandEnvelope = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(first_decoded, first);
    assert_eq!(second_decoded, second_envelope);
}

#[test]
fn jsonl_semantics_identical_to_json_for_the_same_single_envelope() {
    let envelope = success_envelope();
    let json = render_string(
        OutputFormat::Json,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    let jsonl = render_string(
        OutputFormat::Jsonl,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    assert_eq!(
        json, jsonl,
        "JSON and single-line JSONL must be byte-identical"
    );
}

#[test]
fn toon_projects_the_same_canonical_json_value_without_independent_semantics() {
    let envelope = success_envelope();
    let rendered = render_string(
        OutputFormat::Toon,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    let expected = toon_format::encode_default(&envelope).unwrap();
    assert_eq!(rendered, format!("{expected}\n"), "TOON golden drift");

    assert_no_ansi(&rendered);
    assert_one_trailing_newline(&rendered);

    // TOON has no independent semantics: encoding is exact (string-equality
    // above) and the decoded value deserializes back to the same canonical
    // typed envelope. Whole-number floats normalize under TOON decode, so
    // typed equality (not raw scalar equality) is the semantic bar.
    let toon_decoded: serde_json::Value = toon_format::decode_default(&rendered).unwrap();
    let typed_roundtrip: CommandEnvelope = serde_json::from_value(toon_decoded).unwrap();
    assert_eq!(typed_roundtrip, envelope);
}

#[test]
fn toon_failure_projection_roundtrips_to_the_canonical_error_envelope() {
    let envelope = failure_envelope();
    let rendered = render_string(
        OutputFormat::Toon,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    let expected = toon_format::encode_default(&envelope).unwrap();
    assert_eq!(
        rendered,
        format!("{expected}\n"),
        "TOON failure golden drift"
    );

    let toon_decoded: serde_json::Value = toon_format::decode_default(&rendered).unwrap();
    let typed_roundtrip: CommandEnvelope = serde_json::from_value(toon_decoded.clone()).unwrap();
    assert_eq!(typed_roundtrip, envelope);
    assert_eq!(toon_decoded["error"]["code"], "missing_api_key");
}

#[test]
fn markdown_success_golden_is_deterministic_and_free_of_ansi() {
    let envelope = success_envelope();
    let rendered = render_string(
        OutputFormat::Markdown,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    assert_no_ansi(&rendered);
    assert!(rendered.ends_with('\n'));
    let expected = include_str!("support/golden/markdown_success.md");
    assert_eq!(rendered, expected, "Markdown success golden drift");
}

#[test]
fn markdown_failure_golden_is_deterministic_and_free_of_ansi() {
    let envelope = failure_envelope();
    let rendered = render_string(
        OutputFormat::Markdown,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    assert_no_ansi(&rendered);
    let expected = include_str!("support/golden/markdown_failure.md");
    assert_eq!(rendered, expected, "Markdown failure golden drift");
}

#[test]
fn pretty_plain_golden_is_deterministic_and_free_of_ansi() {
    let envelope = success_envelope();
    let rendered = render_string(
        OutputFormat::Pretty,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    assert_no_ansi(&rendered);
    assert!(rendered.ends_with('\n'));
    let expected = include_str!("support/golden/pretty_success.txt");
    assert_eq!(rendered, expected, "pretty success golden drift");
}

#[test]
fn pretty_failure_golden_is_deterministic_and_free_of_ansi() {
    let envelope = failure_envelope();
    let rendered = render_string(
        OutputFormat::Pretty,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    assert_no_ansi(&rendered);
    let expected = include_str!("support/golden/pretty_failure.txt");
    assert_eq!(rendered, expected, "pretty failure golden drift");
}

#[test]
fn pretty_color_wraps_only_trusted_status_and_label_markers_never_payload() {
    let envelope = success_envelope();
    let rendered = render_string(
        OutputFormat::Pretty,
        ColorPolicy::Color,
        std::slice::from_ref(&envelope),
    );

    let expected = include_str!("support/golden/pretty_success_color.txt");
    assert_eq!(rendered, expected, "pretty color golden drift");

    assert!(rendered.contains('\u{1b}'));
    assert!(
        rendered.contains("\u{1b}[32m"),
        "expected green for succeeded"
    );
    assert!(rendered.contains("\u{1b}[2m"), "expected dim for labels");
    assert!(
        !rendered.contains("Human-written\u{1b}") && !rendered.contains("\u{1b}[31mHuman"),
        "payload text must not be colored"
    );
}

#[test]
fn adversarial_payload_cannot_inject_structure_or_terminal_controls_into_any_projection() {
    let envelope = adversarial_envelope();

    let json = render_string(
        OutputFormat::Json,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    let jsonl = render_string(
        OutputFormat::Jsonl,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    let toon = render_string(
        OutputFormat::Toon,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    let markdown = render_string(
        OutputFormat::Markdown,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    let pretty = render_string(
        OutputFormat::Pretty,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );

    // Machine formats are canonical projections: the untrusted string survives
    // byte-exactly because they carry data, not presentation.
    let payload = adversarial::SEGMENT_TEXT;
    let serialized_payload = serde_json::to_string(payload).unwrap();
    assert!(json.contains(&serialized_payload[..30]));
    assert!(jsonl.contains(&serialized_payload[..30]));

    // Human formats must contain no terminal control character at all.
    assert_no_ansi(&markdown);
    assert_no_ansi(&pretty);

    // Human formats must neutralize the C0/DEL/C1 controls in the payload.
    assert!(!markdown.contains('\x00') && !markdown.contains('\x07'));
    assert!(!pretty.contains('\x00') && !pretty.contains('\x07'));

    // TOON carries the canonical value and decodes back to it unchanged.
    let decoded: serde_json::Value = toon_format::decode_default(&toon).unwrap();
    assert_eq!(decoded["schema_version"], "1");

    // Markdown must escape structural characters the payload tried to forge.
    let forged_heading = markdown
        .lines()
        .any(|line| line.starts_with("# forged") || line.starts_with("# a"));
    assert!(!forged_heading, "payload forged a Markdown heading");
    assert!(!markdown.contains("[pwn](https://evil.example") || markdown.contains("\\[pwn\\]"));
}

#[test]
fn color_enabled_pretty_never_lets_payload_chars_pass_through_as_ansi() {
    let envelope = adversarial_envelope();
    let rendered = render_string(
        OutputFormat::Pretty,
        ColorPolicy::Color,
        std::slice::from_ref(&envelope),
    );

    // Strip every legal color marker the projection itself owns; whatever is
    // left must be free of terminals, so no payload byte can masquerade as ANSI.
    let stripped = rendered
        .replace("\u{1b}[0m", "")
        .replace("\u{1b}[1m", "")
        .replace("\u{1b}[2m", "")
        .replace("\u{1b}[32m", "")
        .replace("\u{1b}[33m", "")
        .replace("\u{1b}[31m", "")
        .replace("\u{1b}[36m", "");
    assert_no_ansi(&stripped);
}

#[test]
fn privacy_absent_input_text_stays_absent_across_every_format() {
    let envelope = success_envelope();
    let canonical = serde_json::to_value(&envelope).unwrap();
    assert!(canonical["data"]["input"].get("text").is_none());

    // The absent-input fixture uses a segment text without the sentinel, so
    // the sentinel's absence proves input content never crosses a format.
    for format in [
        OutputFormat::Json,
        OutputFormat::Jsonl,
        OutputFormat::Toon,
        OutputFormat::Markdown,
        OutputFormat::Pretty,
    ] {
        let rendered = render_string(format, ColorPolicy::Plain, std::slice::from_ref(&envelope));
        assert!(
            !rendered.contains("unique-input-sentinel"),
            "{format:?} leaked omitted input text"
        );
    }
}

#[test]
fn privacy_input_text_is_exact_in_machine_formats_and_sanitized_in_human_formats() {
    let envelope = input_content_envelope();
    let input = adversarial::INPUT_SENTINEL;
    let canonical = serde_json::to_value(&envelope).unwrap();
    assert_eq!(
        canonical["data"]["input"]["text"], input,
        "fixture sanity: the canonical envelope carries the sentinel"
    );

    // Machine formats carry the content exactly.
    let json = render_string(
        OutputFormat::Json,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    assert!(json.contains(&serde_json::to_string(input).unwrap()));

    // Human formats surface the present input text, sanitized so the sentinel
    // markers survive but the embedded control bytes (and Markdown structure)
    // cannot forge anything.
    let markdown = render_string(
        OutputFormat::Markdown,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    assert_no_ansi(&markdown);
    assert!(markdown.contains("unique-input-sentinel xQmZ9"));
    assert!(
        markdown.contains("\\# forged"),
        "Markdown must escape the heading marker"
    );
    assert!(
        !markdown.contains("forged\nheading"),
        "control bytes replaced, not reordered"
    );

    let pretty = render_string(
        OutputFormat::Pretty,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    assert_no_ansi(&pretty);
    assert!(pretty.contains("unique-input-sentinel xQmZ9"));
    assert!(pretty.contains('\u{FFFD}'), "pretty controls become U+FFFD");

    let pretty_color = render_string(
        OutputFormat::Pretty,
        ColorPolicy::Color,
        std::slice::from_ref(&envelope),
    );
    // Strip every legal color marker the projection itself owns; no payload
    // byte may pass through as an ANSI sequence.
    let stripped = pretty_color
        .replace("\u{1b}[0m", "")
        .replace("\u{1b}[1m", "")
        .replace("\u{1b}[2m", "")
        .replace("\u{1b}[32m", "")
        .replace("\u{1b}[33m", "")
        .replace("\u{1b}[31m", "")
        .replace("\u{1b}[36m", "");
    assert_no_ansi(&stripped);
}

#[test]
fn write_and_flush_failures_propagate_instead_of_claiming_success() {
    struct FailsOnWrite;
    impl std::io::Write for FailsOnWrite {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "write failed",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct FailsOnFlush;
    impl std::io::Write for FailsOnFlush {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "flush failed",
            ))
        }
    }

    let envelope = success_envelope();
    for format in [
        OutputFormat::Json,
        OutputFormat::Jsonl,
        OutputFormat::Toon,
        OutputFormat::Markdown,
        OutputFormat::Pretty,
    ] {
        let mut fail_write = FailsOnWrite;
        assert!(
            render(
                format,
                ColorPolicy::Plain,
                std::slice::from_ref(&envelope),
                &mut fail_write
            )
            .is_err(),
            "{format:?} did not surface a write failure"
        );
        let mut fail_flush = FailsOnFlush;
        assert!(
            render(
                format,
                ColorPolicy::Plain,
                std::slice::from_ref(&envelope),
                &mut fail_flush
            )
            .is_err(),
            "{format:?} did not surface a flush failure"
        );
    }
}

#[test]
fn human_and_machine_renderings_never_lose_command_error_recovery_schema_or_meta() {
    let envelope = missing_key_envelope();

    let json = render_string(
        OutputFormat::Json,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    let toon = render_string(
        OutputFormat::Toon,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    let markdown = render_string(
        OutputFormat::Markdown,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );
    let pretty = render_string(
        OutputFormat::Pretty,
        ColorPolicy::Plain,
        std::slice::from_ref(&envelope),
    );

    for rendered in [&json, &toon, &markdown, &pretty] {
        assert!(
            rendered.contains("schema_version"),
            "schema_version dropped"
        );
        assert!(rendered.contains("missing_api_key"), "error code dropped");
        assert!(rendered.contains("authentication"), "category dropped");
        assert!(rendered.contains("auth set"), "recovery command dropped");
        assert!(rendered.contains("12345"), "duration_ms dropped");
        assert!(
            rendered.contains("Configure a key"),
            "recovery message dropped"
        );
    }
}

#[test]
fn single_envelope_formats_reject_a_multi_envelope_series() {
    let envelopes = [success_envelope(), success_envelope()];
    for format in [
        OutputFormat::Json,
        OutputFormat::Toon,
        OutputFormat::Markdown,
        OutputFormat::Pretty,
    ] {
        let mut sink = Vec::new();
        let result = render(format, ColorPolicy::Plain, &envelopes, &mut sink);
        assert!(
            result.is_err(),
            "{format:?} accepted a multi-envelope series"
        );
    }
}

#[test]
fn jsonl_rejects_an_empty_series() {
    let mut sink = Vec::new();
    let result = render(OutputFormat::Jsonl, ColorPolicy::Plain, &[], &mut sink);
    assert!(result.is_err(), "JSONL accepted zero envelopes");
}
