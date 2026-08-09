//! Certified, streaming history export shared by interactive adapters.

use std::io::Write;

use super::{HistoryError, HistoryErrorCode, HistoryStore};

/// The closed local-history export formats shared by CLI and TUI adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryExportFormat {
    Jsonl,
    Markdown,
}

/// Distinguishes certified history failures from output-device failures.
#[derive(Debug)]
pub enum HistoryExportError {
    History(HistoryError),
    Output,
}

impl From<HistoryError> for HistoryExportError {
    fn from(error: HistoryError) -> Self {
        Self::History(error)
    }
}

/// Certifies every stored analysis before writing the first byte, then
/// serializes one certified record at a time and flushes the caller's writer.
/// A missing store is an empty successful export and never creates history.
pub fn export_history(
    store: Option<&HistoryStore>,
    writer: &mut impl Write,
    format: HistoryExportFormat,
    redact_content: bool,
) -> Result<(), HistoryExportError> {
    // The complete snapshot must certify before Markdown can write its header
    // or JSONL can write its first row. This keeps a corrupt later row from
    // leaking an earlier certified record.
    let analyses = store.map_or_else(|| Ok(Vec::new()), certified_analyses)?;
    let values = analyses.into_iter().map(|analysis| {
        serde_json::to_value(analysis)
            .map_err(|_| HistoryExportError::History(export_encoding_error()))
    });
    write_values(writer, values, format, redact_content)
}

fn certified_analyses(
    store: &HistoryStore,
) -> Result<Vec<crate::domain::Analysis<crate::output::CanonicalError>>, HistoryError> {
    store.with_read_snapshot(|transaction| {
        let mut analyses = super::read_validation::certify_analysis_batch(transaction, true)?;
        analyses.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(analyses)
    })
}

fn write_values(
    writer: &mut impl Write,
    values: impl Iterator<Item = Result<serde_json::Value, HistoryExportError>>,
    format: HistoryExportFormat,
    redact_content: bool,
) -> Result<(), HistoryExportError> {
    if format == HistoryExportFormat::Markdown {
        writer
            .write_all(b"# Pangram history export\n")
            .map_err(|_| HistoryExportError::Output)?;
    }
    for value in values {
        let mut value = value?;
        // Redaction is deliberately stage 2: certification sees the complete
        // stored record, while only the exported projection loses content.
        if redact_content {
            redact(&mut value);
        }
        write_value(writer, &value, format)?;
    }
    writer.flush().map_err(|_| HistoryExportError::Output)
}

fn write_value(
    writer: &mut impl Write,
    value: &serde_json::Value,
    format: HistoryExportFormat,
) -> Result<(), HistoryExportError> {
    match format {
        HistoryExportFormat::Jsonl => {
            serde_json::to_writer(&mut *writer, value).map_err(export_json_error)?;
            writer
                .write_all(b"\n")
                .map_err(|_| HistoryExportError::Output)
        }
        HistoryExportFormat::Markdown => {
            let id = value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("invalid");
            write!(writer, "\n## `{id}`\n\n```json\n").map_err(|_| HistoryExportError::Output)?;
            serde_json::to_writer_pretty(
                MarkdownJsonWriter {
                    inner: &mut *writer,
                },
                value,
            )
            .map_err(export_json_error)?;
            writer
                .write_all(b"\n```\n")
                .map_err(|_| HistoryExportError::Output)
        }
    }
}

fn export_json_error(error: serde_json::Error) -> HistoryExportError {
    if error.is_io() {
        HistoryExportError::Output
    } else {
        HistoryExportError::History(export_encoding_error())
    }
}

fn export_encoding_error() -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::HistoryCorrupt,
        "export history: a canonical analysis could not be encoded",
    )
}

/// Escapes Markdown fence characters while `serde_json` writes a record.
/// Every input byte is consumed or the underlying I/O error is returned, so
/// the serializer never needs a record-sized intermediate string.
struct MarkdownJsonWriter<'a, W> {
    inner: &'a mut W,
}

impl<W: Write> Write for MarkdownJsonWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let mut start = 0;
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'`' {
                self.inner.write_all(&bytes[start..index])?;
                self.inner.write_all(b"\\u0060")?;
                start = index + 1;
            }
        }
        self.inner.write_all(&bytes[start..])?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn redact(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(input) = object
        .get_mut("input")
        .and_then(serde_json::Value::as_object_mut)
    {
        input.remove("text");
        input.remove("path");
        input.remove("extracted_text");
    }
    if let Some(checks) = object
        .get_mut("checks")
        .and_then(serde_json::Value::as_array_mut)
    {
        for check in checks {
            let Some(check) = check.as_object_mut() else {
                continue;
            };
            let (redact_segments, redact_matches) =
                match check.get("kind").and_then(serde_json::Value::as_str) {
                    Some("ai_detection") => (true, false),
                    Some("plagiarism") => (false, true),
                    _ => (false, false),
                };
            let Some(result) = check
                .get_mut("result")
                .and_then(serde_json::Value::as_object_mut)
            else {
                continue;
            };
            result.remove("dashboard_link");
            if redact_segments {
                if let Some(segments) = result
                    .get_mut("segments")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    for segment in segments {
                        if let Some(segment) = segment.as_object_mut() {
                            segment.insert(
                                "text".to_owned(),
                                serde_json::Value::String(String::new()),
                            );
                        }
                    }
                }
            } else if redact_matches {
                if let Some(matches) = result
                    .get_mut("matches")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    matches.retain_mut(redact_plagiarism_match);
                }
            }
        }
    }
}

fn redact_plagiarism_match(value: &mut serde_json::Value) -> bool {
    let Some(matched) = value.as_object_mut() else {
        return false;
    };
    let Some(source) = matched
        .get("source_url")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    let Ok(parsed) = url::Url::parse(source) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(hostname) = parsed.host_str() else {
        return false;
    };
    let hostname = hostname.to_owned();
    matched.insert(
        "matched_text".to_owned(),
        serde_json::Value::String(String::new()),
    );
    matched.insert("source_url".to_owned(), serde_json::Value::String(hostname));
    true
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::{self, Write};
    use std::rc::Rc;

    use serde_json::json;

    use super::{HistoryExportError, HistoryExportFormat, export_history, write_values};

    struct ShortWriter {
        remaining: usize,
        flush_fails: bool,
    }

    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.remaining == 0 {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
            }
            let written = self.remaining.min(bytes.len());
            self.remaining -= written;
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.flush_fails {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn export_short_write_and_broken_pipe_are_output_errors() {
        let error = export_history(
            None,
            &mut ShortWriter {
                remaining: 3,
                flush_fails: false,
            },
            HistoryExportFormat::Markdown,
            false,
        )
        .expect_err("short writer eventually closes");
        assert!(matches!(error, HistoryExportError::Output));
    }

    #[test]
    fn export_flush_failure_is_an_output_error() {
        let error = export_history(
            None,
            &mut ShortWriter {
                remaining: usize::MAX,
                flush_fails: true,
            },
            HistoryExportFormat::Jsonl,
            false,
        )
        .expect_err("flush fails");
        assert!(matches!(error, HistoryExportError::Output));
    }

    struct CountingWriter {
        writes: Rc<Cell<usize>>,
        bytes: Vec<u8>,
    }

    impl Write for CountingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes.set(self.writes.get() + 1);
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct StreamingProbe {
        writes: Rc<Cell<usize>>,
        values: std::vec::IntoIter<serde_json::Value>,
        last_write_count: Option<usize>,
    }

    impl Iterator for StreamingProbe {
        type Item = Result<serde_json::Value, HistoryExportError>;

        fn next(&mut self) -> Option<Self::Item> {
            if let Some(previous) = self.last_write_count {
                assert!(
                    self.writes.get() > previous,
                    "each record must reach the writer before the next is requested"
                );
            }
            let value = self.values.next()?;
            self.last_write_count = Some(self.writes.get());
            Some(Ok(value))
        }
    }

    #[test]
    fn export_serializes_each_record_incrementally_to_the_writer() {
        let writes = Rc::new(Cell::new(0));
        let mut writer = CountingWriter {
            writes: Rc::clone(&writes),
            bytes: Vec::new(),
        };

        let outcome = write_values(
            &mut writer,
            StreamingProbe {
                writes,
                values: vec![json!({"id": "one"}), json!({"id": "two"})].into_iter(),
                last_write_count: None,
            },
            HistoryExportFormat::Jsonl,
            false,
        );

        assert!(outcome.is_ok(), "the streaming export succeeds");
        assert_eq!(writer.bytes, b"{\"id\":\"one\"}\n{\"id\":\"two\"}\n");

        let writes = Rc::new(Cell::new(0));
        let mut writer = CountingWriter {
            writes: Rc::clone(&writes),
            bytes: Vec::new(),
        };
        let outcome = write_values(
            &mut writer,
            StreamingProbe {
                writes,
                values: vec![json!({"id": "one", "text": "`"}), json!({"id": "two"})].into_iter(),
                last_write_count: None,
            },
            HistoryExportFormat::Markdown,
            false,
        );

        assert!(outcome.is_ok(), "the streaming Markdown export succeeds");
        assert_eq!(
            String::from_utf8(writer.bytes).unwrap(),
            "# Pangram history export\n\n## `one`\n\n```json\n{\n  \"id\": \"one\",\n  \"text\": \"\\u0060\"\n}\n```\n\n## `two`\n\n```json\n{\n  \"id\": \"two\"\n}\n```\n"
        );
    }

    #[test]
    fn absent_store_is_an_empty_flushed_export() {
        let mut jsonl = Vec::new();
        export_history(None, &mut jsonl, HistoryExportFormat::Jsonl, false)
            .expect("empty JSONL export");
        assert!(jsonl.is_empty());

        let mut markdown = Vec::new();
        export_history(None, &mut markdown, HistoryExportFormat::Markdown, true)
            .expect("empty Markdown export");
        assert_eq!(markdown, b"# Pangram history export\n");
    }
}
