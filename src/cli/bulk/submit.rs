//! The `bulk submit` flow: plan preflight, optional dry-run, real
//! submission, and the accepted/observed/interrupted outcome assembly. Only
//! an HTTP-202 acceptance keeps the pipeline; an ambiguous send is reported
//! through the canonical `submission_outcome_unknown` and never replayed.

use clap::ArgMatches;

use crate::analysis::{BulkAnalysisRequest, StopObserving, WaitOptions};
use crate::cli::StreamTty;
use crate::cli::detect::{self, DetectOutcome, GlobalFlags, ProgressMode};
use crate::domain::{AnalysisStatus, BulkCollection, Sha256Hash, SubmissionOutcome, UtcTimestamp};
use crate::output::{
    CanonicalError, CommandData, CommandEnvelope, EnvelopeMeta, ErrorCode, ExitCode, OutputFormat,
    ResolvedCommand,
};

use super::plan::{plan_from_jsonl, read_jsonl_source};
use super::policy::{resolve_policy, resolve_wait_progress};
use super::{new_runtime, prepare, succeed};

#[allow(clippy::too_many_lines)]
pub(super) fn bulk_submit(
    sub: &ArgMatches,
    root_matches: &ArgMatches,
    global: GlobalFlags,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
) -> DetectOutcome {
    let resolved = ResolvedCommand::BulkSubmit;
    let output = resolve_policy(resolved, sub, &global, streams);

    // One guarded pre-billable format decision for both the submitted and the
    // dry-run flow (contracts.md 9.2): `bulk submit` is envelope-only JSON, so
    // a non-JSON `--format` is an unsupported combination before any source
    // read, plan validation, credential resolution, or network access.
    if output.format != OutputFormat::Json {
        return detect::failure_outcome(resolved, output, started, bulk_json_format_error());
    }

    let max_billable_units = match sub.get_one::<String>("max-billable-units") {
        Some(raw) => match raw.parse::<u64>().ok().filter(|value| *value >= 1) {
            Some(value) => value,
            None => {
                return detect::failure_outcome(
                    resolved,
                    output,
                    started,
                    detect::usage_error(
                        ErrorCode::UnsupportedInput,
                        "--max-billable-units must be an integer of at least 1",
                    ),
                );
            }
        },
        None => {
            return detect::failure_outcome(
                resolved,
                output,
                started,
                detect::usage_error(ErrorCode::InputRequired, "--max-billable-units is required"),
            );
        }
    };

    let dry_run = sub.get_flag("dry-run");
    let wait = sub.get_flag("wait");
    if dry_run && wait {
        return detect::failure_outcome(
            resolved,
            output,
            started,
            detect::usage_error(
                ErrorCode::UnsupportedCombination,
                "--dry-run is unsupported alongside --wait",
            ),
        );
    }

    let source = match read_jsonl_source(sub) {
        Ok(source) => source,
        Err(error) => return detect::failure_outcome(resolved, output, started, error),
    };
    let plan = match plan_from_jsonl(&source, max_billable_units) {
        Ok(plan) => plan,
        Err(error) => return detect::failure_outcome(resolved, output, started, error),
    };
    let request = BulkAnalysisRequest::new(plan);
    let plan_sha256 = request.request_sha256();

    if dry_run {
        return dry_run_outcome(&request, plan_sha256, output, started);
    }

    // Actual submission: only accepted work (a 202) keeps the pipelines. A
    // local observation failure after acceptance changes the local status
    // and exits 1, never a top-level failure envelope.
    let (analyzer, service) = match prepare(resolved, root_matches, output, started) {
        Ok(prepared) => prepared,
        Err(outcome) => return outcome,
    };

    let runtime = match new_runtime(resolved, output, started) {
        Ok(runtime) => runtime,
        Err(outcome) => return outcome,
    };
    let stop = StopObserving::new();
    detect::install_sigint_driver();
    let progress = resolve_wait_progress(sub, output, streams);
    let wait_mode = wait;
    let source_name = sub
        .get_one::<String>("JSONL_PATH")
        .filter(|path| path.as_str() != "-")
        .and_then(|path| {
            std::path::Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        });
    // The automatic-history gate decides before the runtime: the observed
    // children fetch runs only when persistence is armed (contracts.md 14.2;
    // bulk carries no `--save`). The one invocation warning latch is shared
    // by whichever certified unit reaches persistence.
    let history_armed = detect::save::automatic_history_armed(&service);
    let mut bulk_warning = detect::save::BulkSaveWarning::new();
    let result = runtime.block_on(async {
        let bridge = tokio::spawn(detect::bridge_sigint(stop.token().clone()));
        let cancel = stop.token().child_token();
        let outcome = match analyzer.submit_bulk(request, &cancel).await {
            Ok(running) => {
                if wait_mode {
                    let accepted_at = UtcTimestamp::now();
                    let observed_running = running.clone();
                    let observed = running
                        .observe(
                            WaitOptions::UNBOUNDED,
                            |event| match progress {
                                ProgressMode::Jsonl => super::emit_bulk_jsonl_progress(event),
                                ProgressMode::Human => super::emit_bulk_human_progress(event),
                                ProgressMode::Auto | ProgressMode::Quiet => {}
                            },
                            stop.clone(),
                        )
                        .await;
                    // A terminal collection may reconcile only with the
                    // complete results window from that same observation.
                    // Keep a failed or skipped read distinct from an empty
                    // successful window so no mixed-time snapshot can save.
                    let children = if history_armed && matches!(observed, Ok(Ok(_))) {
                        Some(
                            analyzer
                                .bulk_observed_children(
                                    &observed_running,
                                    source_name.as_deref(),
                                    &cancel,
                                )
                                .await
                                .map_err(|_| ()),
                        )
                    } else {
                        None
                    };
                    Analyzed::Observed {
                        outcome: observed,
                        children,
                        accepted: Box::new(observed_running),
                        accepted_at,
                    }
                } else {
                    Analyzed::Accepted(running)
                }
            }
            Err(failure) => {
                if cancel.is_cancelled() {
                    let identity = bulk_error_identity(&failure);
                    Analyzed::Interrupted(failure.into_error(), identity)
                } else {
                    Analyzed::Failed(failure.into_error())
                }
            }
        };
        bridge.abort();
        outcome
    });
    detect::reset_sigint_flag();
    match result {
        Analyzed::Accepted(running) => {
            submit_accepted_outcome(&running, source_name.as_deref(), &service, output, started)
        }
        Analyzed::Observed {
            outcome: Ok(Ok(collection)),
            children,
            ..
        } => {
            let exit = super::status_wait::collection_exit(&collection);
            let collection = super::status_wait::persist_observed_collection(
                collection,
                children,
                &service,
                &mut bulk_warning,
                streams,
            );
            succeed(
                resolved,
                CommandData::BulkWait(collection),
                exit,
                output,
                started,
            )
        }
        Analyzed::Observed {
            outcome: Ok(Err(failure)),
            accepted,
            accepted_at,
            ..
        } => {
            // The observation error remains the primary outcome. Acceptance
            // projection was already validated at HTTP 202, so an impossible
            // local reprojection error cannot replace that honest failure.
            if history_armed {
                let _ = persist_accepted_snapshot(
                    &accepted,
                    source_name.as_deref(),
                    accepted_at,
                    &service,
                    &mut bulk_warning,
                );
            }
            let error = failure.into_error();
            observed_failure_outcome(
                "the bulk job was accepted but its local observation failed",
                error,
                streams,
                started,
            )
        }
        Analyzed::Observed {
            outcome: Err(interrupted),
            ..
        } => {
            let note = bulk_identity_note(&interrupted.identity);
            detect::interrupted_outcome(
                ResolvedCommand::BulkWait,
                output,
                started,
                stopped_observation_error(),
                note,
            )
        }
        Analyzed::Interrupted(error, note) => {
            detect::interrupted_outcome(resolved, output, started, error, note)
        }
        Analyzed::Failed(error) => detect::failure_outcome(resolved, output, started, error),
    }
}

/// The canonical unsupported-combination error for a non-JSON `bulk submit`
/// format. Shared by the submitted and dry-run flows so both name the same
/// envelope-only contract before any work.
fn bulk_json_format_error() -> CanonicalError {
    detect::usage_error(
        ErrorCode::UnsupportedCombination,
        "bulk submit renders only the default JSON envelope shape",
    )
}

/// The intermediate flow states for a bulk submission run. Accepted keeps
/// the running handle for the enqueue projection; Observed carries the
/// observe outcome, the optional observed-children read, and the independently
/// certified acceptance snapshot and timestamp retained for an observation
/// failure.
/// Failed/Interrupted carry a pre-acceptance canonical error.
enum Analyzed {
    Accepted(crate::analysis::RunningBulk),
    Observed {
        outcome: Result<
            Result<BulkCollection, crate::analysis::BulkAnalysisError>,
            crate::analysis::InterruptedBulk,
        >,
        children: Option<Result<Vec<detect::save::BulkChild>, ()>>,
        accepted: Box<crate::analysis::RunningBulk>,
        accepted_at: UtcTimestamp,
    },
    Interrupted(CanonicalError, String),
    Failed(CanonicalError),
}

/// The stderr identity note for an interrupted submission, derived from the
/// canonical error's reconciliation details where present.
fn bulk_error_identity(failure: &crate::analysis::BulkAnalysisError) -> String {
    match failure.canonical().details() {
        Some(crate::output::CanonicalErrorDetails::SubmissionOutcomeUnknown(details)) => {
            let mut note = "interrupted; local bulk id ".to_owned();
            note.push_str(&match details.operation_id() {
                crate::domain::LocalOperationId::AnalysisId(id) => id.to_string(),
                crate::domain::LocalOperationId::BulkId(id) => id.to_string(),
            });
            note.push_str(&format!("; request sha256 {}", details.request_sha256));
            note.push_str(&format!("; last status {}", details.last_status.as_str()));
            note
        }
        _ => "interrupted during bulk submission; no remote action was completed".to_owned(),
    }
}

/// The stderr identity note for an interrupted observation: local bulk ID,
/// upstream ID when accepted, and the last observed status.
pub(super) fn bulk_identity_note(identity: &crate::analysis::BulkOperationIdentity) -> String {
    let mut note = format!("interrupted; local bulk id {}", identity.bulk_id);
    if let Some(upstream) = &identity.upstream_bulk_id {
        note.push_str(&format!(
            "; upstream bulk id {}",
            detect::sanitize_for_stderr(upstream.as_str())
        ));
    }
    note
}

/// The canonical local-stop error for a wait-phase cancellation: the job was
/// accepted; local observation stopped without any remote cancellation.
pub(super) fn stopped_observation_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::NetworkUnavailable,
        "observation was interrupted locally; no remote cancellation was sent",
    )
    .expect("static template")
}

/// A local observation failure after acceptance reports through an accepted
/// status-changed envelope (exit 1), never a top-level failure: the billable
/// acceptance is real, so the caller sees the running collection with the
/// note that local observation degraded (contracts.md 4.8).
fn observed_failure_outcome(
    note: &str,
    error: CanonicalError,
    streams: &dyn StreamTty,
    started: UtcTimestamp,
) -> DetectOutcome {
    detect::note_stderr(streams, note);
    detect::note_stderr(
        streams,
        &format!(
            "reconcile manually with pangram bulk status; error: {}",
            detect::sanitize_for_stderr(error.message())
        ),
    );
    failure_status_envelope(error, started)
}

/// The accepted status-changed envelope: a canonical failure envelope at the
/// collection command with the observation error, exit 1. Success-style
/// envelope assembly cannot fabricate a collection, so the canonical failure
/// envelope is the honest surface for the local observation failure.
fn failure_status_envelope(error: CanonicalError, started: UtcTimestamp) -> DetectOutcome {
    let exit_code = 1_u8;
    let envelope = CommandEnvelope::failure(
        ResolvedCommand::BulkWait,
        error,
        EnvelopeMeta::default()
            .with_started_at(started)
            .with_failed_at(UtcTimestamp::now()),
    );
    DetectOutcome {
        exit_code,
        envelopes: vec![envelope],
        rendered: false,
        primary_ok: true,
    }
}

/// The accepted (enqueued) submission outcome: the running collection
/// projected from the validated HTTP 202 acceptance snapshot. With no
/// `--wait`, the adapter records the truthful accept state (accepted and
/// immediately failed counts, and the derived collection status) without
/// observing remotely; it never fabricates all-queued-zero counters over an
/// acceptance that may already report immediate failures (contracts.md 12).
/// Under the automatic history gate the accepted collection and its plan
/// children persist through the same save seam.
fn submit_accepted_outcome(
    running: &crate::analysis::RunningBulk,
    source_name: Option<&str>,
    service: &crate::config::ConfigService,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
) -> DetectOutcome {
    let mut bulk_warning = detect::save::BulkSaveWarning::new();
    let Some(collection) = persist_accepted_snapshot(
        running,
        source_name,
        UtcTimestamp::now(),
        service,
        &mut bulk_warning,
    ) else {
        return detect::failure_outcome(
            ResolvedCommand::BulkSubmit,
            output,
            started,
            detect::internal_error("the accepted bulk state could not be projected"),
        );
    };
    succeed(
        ResolvedCommand::BulkSubmit,
        CommandData::BulkSubmit(crate::output::BulkSubmitOutput::collection(collection)),
        ExitCode::Success,
        output,
        started,
    )
}

/// Persists the independently certified HTTP 202 acceptance unit. A failed
/// later observation may still save this earlier snapshot because its parent,
/// children, local inputs, and task identities all come from the same
/// acceptance response. `None` denotes the structurally impossible case where
/// that already validated acceptance cannot be projected into its domain type.
fn persist_accepted_snapshot(
    running: &crate::analysis::RunningBulk,
    source_name: Option<&str>,
    accepted_at: UtcTimestamp,
    service: &crate::config::ConfigService,
    warning: &mut detect::save::BulkSaveWarning,
) -> Option<BulkCollection> {
    let identity = running.identity();
    let plan = running.plan();
    let estimated = plan.map(|plan| plan.estimated_billable_units());
    let (status, counters) = running.accepted_snapshot();
    // A 202 may immediately reject every item. That accepted snapshot is
    // already terminal, so its observation time is also its completion time.
    let completed_at = matches!(
        status,
        AnalysisStatus::Succeeded | AnalysisStatus::Failed | AnalysisStatus::Partial
    )
    .then_some(accepted_at);
    let collection = BulkCollection::new(
        identity.bulk_id,
        identity.upstream_bulk_id.clone(),
        status,
        SubmissionOutcome::Accepted,
        counters,
        estimated,
        accepted_at,
        accepted_at,
        completed_at,
    )
    .ok()?;
    let children = running.acceptance_children(source_name, accepted_at);
    Some(detect::save::persist_bulk_collection(&collection, children, service, warning).0)
}

/// The dry-run outcome: the canonical typed reconciliation shape at exit 0
/// with credentials and network skipped (contracts.md 9.2). The record is
/// built by the Rust-owned [`crate::output::BulkDryRun`] type and projected
/// through the single canonical envelope/projection owner, never manual JSON.
/// The JSON-only format guard runs once at the head of the flow, before this
/// point, so a dry run is always reached at the default JSON format.
fn dry_run_outcome(
    request: &BulkAnalysisRequest,
    plan_sha256: Sha256Hash,
    output: detect::ResolvedOutput,
    started: UtcTimestamp,
) -> DetectOutcome {
    let item_count = u64::try_from(request.plan().items().len()).unwrap_or(u64::MAX);
    let dry_run = crate::output::BulkDryRun::new(
        request.id(),
        plan_sha256,
        request.plan().estimated_billable_units(),
        item_count,
    );
    let meta = EnvelopeMeta::default()
        .with_started_at(started)
        .with_duration_ms(detect::elapsed_ms(started));
    let envelope = CommandEnvelope::success(
        CommandData::BulkSubmit(crate::output::BulkSubmitOutput::dry_run(dry_run)),
        meta,
    );
    detect::primary_outcome(
        ResolvedCommand::BulkSubmit,
        &envelope,
        output,
        ExitCode::Success.as_u8(),
        started,
    )
}
