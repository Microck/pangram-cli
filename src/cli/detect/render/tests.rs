//! Deterministic render-failure proofs for the render lane's
//! sink-injectable seams. A scripted faulting sink replaces a
//! `/dev/full`-only proof: a first scripted write fails while a later write
//! genuinely succeeds, so a silent bypass of the real write boundary can
//! never make these tests pass. Nothing here exposes a production API; the
//! suite exercises only crate-visible items through `super::*`.

use super::*;

/// One byte-sequence-level scripted sink: every write or flush advances
/// the scripted sequence, succeeding only when the script says so. The
/// bytes that arrived are retained for assertion. This is the
/// deterministic replacement for a `/dev/full`-only proof: the first
/// scripted write fails and the second genuinely succeeds, so a silent
/// bypass of the real write boundary can never make the test pass.
struct ScriptedSink {
    writes: Vec<std::io::Result<()>>,
    flushes: Vec<std::io::Result<()>>,
    received: Vec<u8>,
    write_calls: usize,
    flush_calls: usize,
}

impl ScriptedSink {
    fn scripted(writes: Vec<std::io::Result<()>>, flushes: Vec<std::io::Result<()>>) -> Self {
        Self {
            writes,
            flushes,
            received: Vec::new(),
            write_calls: 0,
            flush_calls: 0,
        }
    }
}

impl RenderWrite for ScriptedSink {
    fn write_all_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.write_calls += 1;
        let step = if self.writes.is_empty() {
            Ok(())
        } else {
            self.writes.remove(0)
        };
        match step {
            Ok(()) => {
                self.received.extend_from_slice(bytes);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
    fn flush_bytes(&mut self) -> std::io::Result<()> {
        self.flush_calls += 1;
        if self.flushes.is_empty() {
            Ok(())
        } else {
            self.flushes.remove(0)
        }
    }
}

fn buffer_sink() -> ScriptedSink {
    ScriptedSink::scripted(Vec::new(), Vec::new())
}

fn failed_write() -> std::io::Result<()> {
    Err(std::io::Error::other("deterministic write failure"))
}

fn ok() -> std::io::Result<()> {
    Ok(())
}

fn sink_pair<'a>(stdout: &'a mut ScriptedSink, stderr: &'a mut ScriptedSink) -> RenderSinks<'a> {
    RenderSinks {
        stdout: &mut *stdout,
        stderr: &mut *stderr,
    }
}

fn save_failure() -> CanonicalError {
    crate::output::CanonicalError::new(
        ErrorCode::InsecureHistoryPermissions,
        "history permissions could not be verified",
    )
    .expect("static template")
}

fn output(error: ErrorSurface) -> ResolvedOutput {
    ResolvedOutput {
        format: crate::output::OutputFormat::Json,
        color: crate::output::ColorPolicy::Plain,
        error,
    }
}

fn instant() -> crate::domain::UtcTimestamp {
    crate::domain::UtcTimestamp::now()
}

/// A primary render that already failed keeps the general render-failure
/// exit 1 even after a category-7 save failure attaches itself
/// afterwards: the history exit can never overwrite the render exit
/// (contracts.md 14.2 note). The sink faulting is deterministic: the
/// primary write fails first, the attachment's would-be second write is
/// never attempted (and would have succeeded), so a bypass of the real
/// write boundary cannot pass.
#[test]
fn attach_failure_never_overwrites_a_failed_primary_render_exit() {
    // The primary write fails deterministically on the first scripted
    // byte. `emit_primary_with` must thread that real failure into the
    // outcome it returns: `primary_ok == false` comes from the render
    // lane itself, never from a hand-set flag in the test.
    let envelope = CommandEnvelope::failure(
        ResolvedCommand::Detect,
        internal_error("primary content"),
        EnvelopeMeta::default(),
    );
    let mut stdout = ScriptedSink::scripted(vec![failed_write()], Vec::new());
    let mut stderr = buffer_sink();
    let resolution = output(ErrorSurface::Json);
    let mut outcome = emit_primary_with(
        ResolvedCommand::Detect,
        std::slice::from_ref(&envelope),
        resolution,
        0,
        instant(),
        &mut sink_pair(&mut stdout, &mut stderr),
    );
    assert_eq!(
        outcome.exit_code, 1,
        "a failed primary render exits 1 through the outcome `emit_primary_with` returns"
    );
    assert!(
        !outcome.primary_ok,
        "the failed primary render must clear `primary_ok` in the returned outcome itself"
    );
    // A post-primary history attachment against the real failed-primary
    // outcome (exactly as returned, unchanged) must keep exit 1 and must
    // not attempt any further stdout write (the scripted sink arm for a
    // second write is unused; one would be recorded if it happened).
    outcome.attach_failure_with(
        ResolvedCommand::Detect,
        resolution,
        instant(),
        save_failure(),
        &mut sink_pair(&mut stdout, &mut stderr),
    );
    assert_eq!(
        outcome.exit_code, 1,
        "the general render-failure exit 1 is preserved; exit 7 must not overwrite it"
    );
    assert_eq!(
        stdout.write_calls, 1,
        "no second stdout write was attempted after the failed primary"
    );
}

/// The text surface: the primary pretty write fails, the lane reports
/// exit 1 with `primary_ok == false` from the returned outcome itself,
/// and a later save-failure attachment can never replace it with exit 7.
#[test]
fn failed_text_primary_render_keeps_exit_1_through_the_returned_outcome() {
    let envelope = CommandEnvelope::failure(
        ResolvedCommand::Detect,
        internal_error("primary content"),
        EnvelopeMeta::default(),
    );
    let mut stdout = ScriptedSink::scripted(vec![failed_write()], Vec::new());
    let mut stderr = buffer_sink();
    let resolution = ResolvedOutput {
        format: crate::output::OutputFormat::Pretty,
        color: crate::output::ColorPolicy::Plain,
        error: ErrorSurface::Text,
    };
    let mut outcome = emit_primary_with(
        ResolvedCommand::Detect,
        std::slice::from_ref(&envelope),
        resolution,
        0,
        instant(),
        &mut sink_pair(&mut stdout, &mut stderr),
    );
    assert_eq!(outcome.exit_code, 1);
    assert!(
        !outcome.primary_ok,
        "the failed text render clears `primary_ok` in the returned outcome itself"
    );
    outcome.attach_failure_with(
        ResolvedCommand::Detect,
        resolution,
        instant(),
        save_failure(),
        &mut sink_pair(&mut stdout, &mut stderr),
    );
    assert_eq!(
        outcome.exit_code, 1,
        "exit 1 survives the save-failure attachment on the text surface"
    );
}

/// The JSON surface: a primary that honestly rendered reports the save
/// failure after it at the canonical exit 7; both halves stay
/// machine-readable in order on stdout.
#[test]
fn attach_failure_appends_the_history_envelope_after_a_rendered_json_primary() {
    let mut stdout = ScriptedSink::scripted(vec![ok()], vec![ok()]);
    let mut stderr = buffer_sink();
    let resolution = output(ErrorSurface::Json);
    let mut outcome = DetectOutcome {
        exit_code: 0,
        envelopes: vec![],
        rendered: true,
        primary_ok: true,
    };
    outcome.attach_failure_with(
        ResolvedCommand::Detect,
        resolution,
        instant(),
        save_failure(),
        &mut sink_pair(&mut stdout, &mut stderr),
    );
    assert_eq!(outcome.exit_code, 7, "the save failure reports exit 7");
    assert!(outcome.primary_ok, "no render failure occurred");
    let lines = String::from_utf8(stdout.received.clone()).expect("utf8 out");
    let failure: serde_json::Value = serde_json::from_str(lines.trim_end()).unwrap();
    assert_eq!(failure["command"], "detect");
    assert_eq!(failure["error"]["code"], "insecure_history_permissions");
    assert_eq!(failure["error"]["category"], "local_history");
}

/// The text surface: a failed primary text render (`primary_ok == false`)
/// keeps exit 1 through the attachment, and no stderr write for the save
/// failure is ever attempted.
#[test]
fn attach_failure_never_overwrites_a_failed_primary_text_render_exit() {
    let mut stdout = buffer_sink();
    let mut stderr = ScriptedSink::scripted(vec![failed_write()], Vec::new());
    let mut outcome = DetectOutcome {
        exit_code: 1,
        envelopes: vec![],
        rendered: true,
        primary_ok: false,
    };
    outcome.attach_failure_with(
        ResolvedCommand::Detect,
        output(ErrorSurface::Text),
        instant(),
        save_failure(),
        &mut sink_pair(&mut stdout, &mut stderr),
    );
    assert_eq!(
        outcome.exit_code, 1,
        "the text-surface render failure stays exit 1, never exit 7"
    );
    assert_eq!(
        stderr.write_calls, 0,
        "no stderr write was attempted for a save failure hidden behind the failed primary"
    );
}

/// The text surface itself failing while writing the save-failure
/// message degrades to exit 1 rather than reporting exit 7: the warning
/// surface could not honestly render either.
#[test]
fn attach_failure_text_write_failure_degrades_to_exit_1() {
    let mut stdout = buffer_sink();
    // First stderr write succeeds, the second (help line) fails: a
    // deterministic mid-message failure.
    let mut stderr = ScriptedSink::scripted(vec![ok(), failed_write()], vec![ok()]);
    let mut outcome = DetectOutcome {
        exit_code: 0,
        envelopes: vec![],
        rendered: true,
        primary_ok: true,
    };
    outcome.attach_failure_with(
        ResolvedCommand::Detect,
        output(ErrorSurface::Text),
        instant(),
        save_failure(),
        &mut sink_pair(&mut stdout, &mut stderr),
    );
    assert_eq!(
        outcome.exit_code, 1,
        "an unrenderable save-failure message degrades to exit 1, never reported as exit 7"
    );
    assert!(!outcome.primary_ok);
}
