//! Certified, streaming history export shared by interactive adapters.

use std::io::Write;

use super::{HistoryError, HistoryErrorCode, HistoryStore};

type CanonicalAnalysis = crate::domain::Analysis<crate::output::CanonicalError>;

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
    let Some(store) = store else {
        return write_values(
            writer,
            std::iter::empty::<Result<serde_json::Value, HistoryExportError>>(),
            format,
            redact_content,
        );
    };

    // Keep certification and output on one immutable WAL snapshot. The read
    // transaction is intentionally dropped rather than committed: it never
    // writes, and this avoids a fallible database operation after stdout may
    // contain export bytes.
    let transaction = store
        .connection_ref()
        .unchecked_transaction()
        .map_err(|_| begin_snapshot_error())?;
    super::read_validation::certify_foreign_keys(&transaction)?;
    super::search::certify_search_index(&transaction).map_err(|error| {
        if error.code() == HistoryErrorCode::HistoryUnavailable {
            export_read_error()
        } else {
            error
        }
    })?;
    let identities = export_identities(&transaction)?;

    // Fully consume the first pass before stdout. This validates every
    // canonical aggregate and JSON encoding while retaining only the current
    // record. The lightweight identity/timestamp metadata above is the only
    // whole-export allocation needed for exact chronological ordering.
    visit_export_records(&transaction, &identities, |record| {
        let _ = record.to_value()?;
        Ok(())
    })?;

    write_header(writer, format)?;
    visit_export_records(&transaction, &identities, |record| {
        let mut value = record.to_value()?;
        if redact_content {
            redact(&mut value);
        }
        write_value(writer, &value, format)
    })?;
    writer.flush().map_err(|_| HistoryExportError::Output)
}

fn export_identities(
    connection: &rusqlite::Connection,
) -> Result<Vec<(crate::domain::AnalysisId, crate::domain::UtcTimestamp)>, HistoryExportError> {
    let mut statement = connection
        .prepare("SELECT id, created_at FROM analyses")
        .map_err(|_| export_read_error())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| export_read_error())?;
    let mut identities = Vec::new();
    for row in rows {
        let (id, created_at) = row.map_err(|_| export_corruption_error())?;
        let id = id
            .parse::<crate::domain::AnalysisId>()
            .map_err(|_| export_corruption_error())?;
        let created_at = created_at
            .parse::<crate::domain::UtcTimestamp>()
            .map_err(|_| export_corruption_error())?;
        identities.push((id, created_at));
    }
    identities.sort_by(|(left_id, left_created), (right_id, right_created)| {
        right_created
            .cmp(left_created)
            .then_with(|| left_id.cmp(right_id))
    });
    Ok(identities)
}

fn visit_export_records(
    connection: &rusqlite::Connection,
    identities: &[(crate::domain::AnalysisId, crate::domain::UtcTimestamp)],
    mut visit: impl FnMut(&ExportRecord) -> Result<(), HistoryExportError>,
) -> Result<(), HistoryExportError> {
    for (id, _) in identities {
        let record = ExportRecord::load(connection, id)?;
        visit(&record)?;
    }
    Ok(())
}

struct ExportRecord {
    analysis: CanonicalAnalysis,
}

impl ExportRecord {
    fn load(
        connection: &rusqlite::Connection,
        id: &crate::domain::AnalysisId,
    ) -> Result<Self, HistoryError> {
        let analysis = super::read_validation::certified_analysis_on(connection, id, true)?;
        #[cfg(test)]
        tests::record_opened();
        Ok(Self { analysis })
    }

    fn to_value(&self) -> Result<serde_json::Value, HistoryError> {
        serde_json::to_value(&self.analysis).map_err(|_| export_encoding_error())
    }
}

#[cfg(test)]
impl Drop for ExportRecord {
    fn drop(&mut self) {
        tests::record_dropped();
    }
}

fn begin_snapshot_error() -> HistoryExportError {
    HistoryExportError::History(HistoryError::from_sqlite(
        HistoryErrorCode::HistoryUnavailable,
        "begin export snapshot",
    ))
}

fn export_read_error() -> HistoryError {
    HistoryError::from_sqlite(
        HistoryErrorCode::HistoryUnavailable,
        "read export identities",
    )
}

fn export_corruption_error() -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::HistoryCorrupt,
        "export history: a stored analysis identity or timestamp is invalid",
    )
}

fn write_values(
    writer: &mut impl Write,
    values: impl Iterator<Item = Result<serde_json::Value, HistoryExportError>>,
    format: HistoryExportFormat,
    redact_content: bool,
) -> Result<(), HistoryExportError> {
    write_header(writer, format)?;
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

fn write_header(
    writer: &mut impl Write,
    format: HistoryExportFormat,
) -> Result<(), HistoryExportError> {
    if format == HistoryExportFormat::Markdown {
        writer
            .write_all(b"# Pangram history export\n")
            .map_err(|_| HistoryExportError::Output)?;
    }
    Ok(())
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

    use crate::domain::{
        AnalysisStatus, CheckKind, SaveState, Sha256Hash, SubmissionOutcome, UtcTimestamp,
    };
    use crate::history::{HistoryStore, InputKind, StoredAnalysis, StoredUpstreamTask};

    use super::{
        HistoryErrorCode, HistoryExportError, HistoryExportFormat, export_history, write_values,
    };

    #[derive(Clone, Copy, Default)]
    struct RecordProbeState {
        enabled: bool,
        live: usize,
        peak: usize,
        opened: usize,
    }

    thread_local! {
        static RECORD_PROBE: Cell<RecordProbeState> = const {
            Cell::new(RecordProbeState {
                enabled: false,
                live: 0,
                peak: 0,
                opened: 0,
            })
        };
    }

    pub(super) fn record_opened() {
        RECORD_PROBE.with(|probe| {
            let mut state = probe.get();
            if state.enabled {
                state.live += 1;
                state.peak = state.peak.max(state.live);
                state.opened += 1;
                probe.set(state);
            }
        });
    }

    pub(super) fn record_dropped() {
        RECORD_PROBE.with(|probe| {
            let mut state = probe.get();
            if state.enabled {
                state.live = state
                    .live
                    .checked_sub(1)
                    .expect("tracked export records must be balanced");
                probe.set(state);
            }
        });
    }

    struct RecordProbe;

    impl RecordProbe {
        fn start() -> Self {
            RECORD_PROBE.with(|probe| {
                probe.set(RecordProbeState {
                    enabled: true,
                    ..RecordProbeState::default()
                });
            });
            Self
        }

        fn state(&self) -> RecordProbeState {
            RECORD_PROBE.with(Cell::get)
        }
    }

    impl Drop for RecordProbe {
        fn drop(&mut self) {
            RECORD_PROBE.with(|probe| probe.set(RecordProbeState::default()));
        }
    }

    fn seed_running_analysis(store: &mut HistoryStore, id: &str, created_at: &str) {
        let id = id.parse().expect("analysis identity");
        let created_at = created_at.parse::<UtcTimestamp>().expect("timestamp");
        let record = StoredAnalysis {
            id,
            bulk: None,
            caller_id: None,
            status: AnalysisStatus::Running,
            submission_outcome: SubmissionOutcome::Accepted,
            save_state: SaveState::SavedHistory,
            input_kind: InputKind::Text,
            input_sha256: Sha256Hash::from_bytes([0; 32]),
            display_name: None,
            input_json: "null".to_owned(),
            result_json: None,
            error_json: None,
            upstream_version: None,
            retry_of: None,
            rerun_of: None,
            submitted_at: None,
            created_at,
            updated_at: created_at,
            completed_at: None,
            search_input_text: None,
            search_filename: None,
            search_headline: None,
            search_source_urls: None,
        };
        store
            .save_analysis_atomic(
                &record,
                &[StoredUpstreamTask {
                    analysis_id: id,
                    check_kind: CheckKind::AiDetection,
                    upstream_task_id: format!("task-{id}"),
                    last_stage: Some("STAGE_INFERENCE".to_owned()),
                    observed_at: created_at,
                }],
            )
            .expect("seed canonical running analysis");
    }

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
    fn export_mid_record_io_failure_is_an_output_error() {
        let error = write_values(
            &mut ShortWriter {
                remaining: 3,
                flush_fails: false,
            },
            [Ok(json!({"id": "one", "status": "running"}))].into_iter(),
            HistoryExportFormat::Jsonl,
            false,
        )
        .expect_err("the output device closes during JSON serialization");
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
    fn redacted_export_never_keeps_more_than_one_full_canonical_record_live() {
        let root = tempfile::tempdir().expect("temporary history root");
        let mut store = HistoryStore::open(root.path()).expect("history store");
        seed_running_analysis(
            &mut store,
            "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a71",
            "2026-08-01T10:00:00Z",
        );
        seed_running_analysis(
            &mut store,
            "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a72",
            "2026-08-01T11:00:00Z",
        );
        seed_running_analysis(
            &mut store,
            "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a73",
            "2026-08-01T12:00:00Z",
        );
        let probe = RecordProbe::start();
        let mut output = Vec::new();

        export_history(Some(&store), &mut output, HistoryExportFormat::Jsonl, true)
            .expect("redacted export");

        let state = probe.state();
        assert_eq!(
            state.opened, 6,
            "three records are loaded in each cursor pass"
        );
        assert_eq!(state.peak, 1, "full records must never be prebuffered");
        assert_eq!(state.live, 0, "each full record is dropped after use");
        assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 3);
    }

    #[test]
    fn export_orders_fractional_timestamps_chronologically_then_by_identity() {
        let root = tempfile::tempdir().expect("temporary history root");
        let mut store = HistoryStore::open(root.path()).expect("history store");
        let older = "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a61";
        let newer = "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a62";
        seed_running_analysis(&mut store, older, "2026-08-01T10:00:00Z");
        seed_running_analysis(&mut store, newer, "2026-08-01T10:00:00.1Z");
        let mut output = Vec::new();

        export_history(Some(&store), &mut output, HistoryExportFormat::Jsonl, true)
            .expect("ordered export");

        let ids = String::from_utf8(output)
            .expect("UTF-8 JSONL")
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).expect("JSONL row")["id"]
                    .as_str()
                    .expect("analysis identity")
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, [newer, older]);
    }

    #[test]
    fn corrupt_later_record_writes_no_jsonl_or_markdown_prefix() {
        let root = tempfile::tempdir().expect("temporary history root");
        let mut store = HistoryStore::open(root.path()).expect("history store");
        seed_running_analysis(
            &mut store,
            "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a81",
            "2026-08-01T10:00:00Z",
        );
        seed_running_analysis(
            &mut store,
            "anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a82",
            "2026-08-01T11:00:00Z",
        );
        store
            .with_connection(|connection| {
                connection.execute(
                    "DELETE FROM analysis_search WHERE analysis_id = ?1",
                    ["anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a81"],
                )
            })
            .expect("borrow history connection")
            .expect("corrupt the later export record");

        for format in [HistoryExportFormat::Jsonl, HistoryExportFormat::Markdown] {
            let mut output = Vec::new();
            let error = export_history(Some(&store), &mut output, format, false)
                .expect_err("the complete export is certified first");
            let HistoryExportError::History(error) = error else {
                panic!("corruption must remain a history error")
            };
            assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
            assert!(output.is_empty(), "corruption must write no prefix");
        }
    }

    #[test]
    fn orphan_search_row_is_rejected_before_markdown_header() {
        let root = tempfile::tempdir().expect("temporary history root");
        let store = HistoryStore::open(root.path()).expect("history store");
        store
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO analysis_search (analysis_id) VALUES (?1)",
                    ["anl_0198b16f-2c6f-7d0a-b6e0-9c2a1c0f8a91"],
                )
            })
            .expect("borrow history connection")
            .expect("insert orphan search row");
        let mut output = Vec::new();

        let error = export_history(
            Some(&store),
            &mut output,
            HistoryExportFormat::Markdown,
            false,
        )
        .expect_err("orphan search state must fail closed");

        let HistoryExportError::History(error) = error else {
            panic!("search corruption must remain a history error")
        };
        assert_eq!(error.code(), HistoryErrorCode::HistoryCorrupt);
        assert!(output.is_empty(), "Markdown must not write its header");
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
