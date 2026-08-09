//! Input resolution for detection: literal text, piped or explicit stdin, and
//! UTF-8 text files. Each source category becomes a priceable `ResolvedInput`
//! with its canonical `TextOrigin` and whitespace-token word count before any
//! billable work. Empty or undetectable content is rejected here as a usage
//! error, never submitted upstream.

use std::io::Read as _;

use crate::analysis::AnalysisRequest;
use crate::domain::{TextOrigin, text_billable_units};
use crate::output::{CanonicalError, ErrorCode};

use super::render::usage_error;

/// A resolved input source before any billable work. `origin` feeds the
/// canonical `TextOrigin`; `name` is present only for files.
#[derive(Debug)]
pub(super) struct ResolvedInput {
    pub(super) text: String,
    pub(super) origin: TextOrigin,
    pub(super) name: Option<String>,
    pub(super) word_count: u64,
}

/// Resolves the single source category into validated inputs. Literal text,
/// explicit stdin (`-` or piped content), and `--file` are already mutually
/// exclusive (Clap rejected conflicts for the explicit `detect` spelling, and
/// the bare path selects exactly one). Bare `[TEXT]` and bare `-` reach here
/// through root dispatch.
pub(super) fn resolve_inputs(
    source: super::Source,
    streams: &dyn crate::cli::StreamTty,
    stdin_text: Option<String>,
) -> Result<Vec<ResolvedInput>, CanonicalError> {
    match source {
        super::Source::Files(files) => {
            let mut inputs = Vec::with_capacity(files.len());
            for path in &files {
                inputs.push(read_text_file(path)?);
            }
            Ok(inputs)
        }
        super::Source::Literal(text) => Ok(vec![literal(text)?]),
        // Explicit `-` or an implicit piped stdin: detects only when it
        // carries content; a TTY stdin or empty pipe is the canonical
        // input_required.
        super::Source::Stdin => resolve_stdin_source(streams, stdin_text),
    }
}

/// Builds one literal-text input, rejecting empty/whitespace-only content.
fn literal(text: String) -> Result<ResolvedInput, CanonicalError> {
    let word_count = AnalysisRequest::eligible_text_word_count(&text).ok_or_else(|| {
        usage_error(
            ErrorCode::InputRequired,
            "detection requires non-empty text",
        )
    })?;
    Ok(ResolvedInput {
        text,
        origin: TextOrigin::Literal,
        name: None,
        word_count,
    })
}

/// Reads stdin once (the harness supplies the text so tests never depend on a
/// real pipe) and rejects it when empty or whitespace-only.
fn resolve_stdin_source(
    streams: &dyn crate::cli::StreamTty,
    stdin_text: Option<String>,
) -> Result<Vec<ResolvedInput>, CanonicalError> {
    if streams.stdin() {
        return Err(usage_error(
            ErrorCode::InputRequired,
            "no input: provide text, a file, or piped stdin",
        ));
    }
    let text = match stdin_text {
        Some(text) => text,
        None => read_all_stdin()?,
    };
    let word_count = AnalysisRequest::eligible_text_word_count(&text)
        .ok_or_else(|| usage_error(ErrorCode::InputRequired, "stdin carried no detectable text"))?;
    Ok(vec![ResolvedInput {
        text,
        origin: TextOrigin::Stdin,
        name: None,
        word_count,
    }])
}

/// Reads one UTF-8 text file, rejecting binary or undecodable content before
/// any billable request. Empty text is a usage error.
fn read_text_file(path: &str) -> Result<ResolvedInput, CanonicalError> {
    let bytes = std::fs::read(path).map_err(|error| {
        usage_error(
            ErrorCode::InputRequired,
            &format!("cannot read {path}: {}", crate::cli::redact_io(&error)),
        )
    })?;
    let text = String::from_utf8(bytes).map_err(|_| {
        usage_error(
            ErrorCode::UnsupportedInput,
            &format!("{path} is not a UTF-8 text file"),
        )
    })?;
    let word_count = AnalysisRequest::eligible_text_word_count(&text).ok_or_else(|| {
        usage_error(
            ErrorCode::UnsupportedInput,
            &format!("{path} contains no detectable text"),
        )
    })?;
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned);
    Ok(ResolvedInput {
        text,
        origin: TextOrigin::File,
        name,
        word_count,
    })
}

/// Applies `--max-billable-units` before any submission: the sum of started
/// 100-word estimates across every input must fit the single ceiling.
pub(super) fn enforce_billable_ceiling(
    ceiling: Option<u64>,
    inputs: &[ResolvedInput],
) -> Result<(), CanonicalError> {
    let Some(ceiling) = ceiling else {
        return Ok(());
    };
    let estimated: u64 = inputs
        .iter()
        .map(|input| text_billable_units(input.word_count))
        .fold(0_u64, u64::saturating_add);
    if estimated > ceiling {
        return Err(usage_error(
            ErrorCode::UnsupportedInput,
            &format!(
                "estimated {estimated} billable unit(s) exceeds --max-billable-units {ceiling}"
            ),
        ));
    }
    Ok(())
}

/// Reads the full stdin stream as UTF-8, used when no text was injected.
fn read_all_stdin() -> Result<String, CanonicalError> {
    let mut buffer = String::new();
    std::io::stdin()
        .lock()
        .read_to_string(&mut buffer)
        .map_err(|error| {
            usage_error(
                ErrorCode::UnsupportedInput,
                &format!(
                    "stdin must be readable UTF-8: {}",
                    crate::cli::redact_io(&error)
                ),
            )
        })?;
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_count_uses_whitespace_tokens() {
        assert_eq!(AnalysisRequest::eligible_text_word_count("a b c"), Some(3));
        assert_eq!(
            AnalysisRequest::eligible_text_word_count("  one  two\nthree\t"),
            Some(3)
        );
        assert_eq!(AnalysisRequest::eligible_text_word_count("a"), Some(1));
    }

    #[test]
    fn eligibility_rejects_ascii_and_unicode_only_whitespace() {
        for text in ["", " \n\t", "\u{00a0}\u{2003}\u{2029}"] {
            assert_eq!(
                AnalysisRequest::eligible_text_word_count(text),
                None,
                "{text:?}"
            );
        }
        assert_eq!(
            AnalysisRequest::eligible_text_word_count("\u{2003}one\u{00a0}two"),
            Some(2)
        );
    }

    #[test]
    fn billing_ceiling_sums_started_estimates() {
        // 50 words -> 1 unit; 150 words -> 2 units; total 3.
        let inputs = vec![
            ResolvedInput {
                text: String::new(),
                origin: TextOrigin::Literal,
                name: None,
                word_count: 50,
            },
            ResolvedInput {
                text: String::new(),
                origin: TextOrigin::Literal,
                name: None,
                word_count: 150,
            },
        ];
        assert!(enforce_billable_ceiling(Some(3), &inputs).is_ok());
        assert!(enforce_billable_ceiling(Some(2), &inputs).is_err());
        assert!(enforce_billable_ceiling(None, &inputs).is_ok());
    }
}
