//! History and analysis effect execution for the interactive adapter.
//!
//! The reducer channel carries IDs, summaries, redacted detail, and typed
//! outcomes only. Retained plaintext needed by a rerun stays on its worker
//! stack and inside `AnalysisRequest` until submission and optional history
//! persistence finish.

// These cold worker boundaries return the canonical error type directly so
// the reducer receives the same typed error as every other adapter. Boxing it
// only to satisfy an ABI-size heuristic would add allocation and unwrap noise.
#![allow(clippy::result_large_err)]

use std::io;
use std::sync::mpsc::Sender;

use crate::analysis::{Accepted, AnalysisRequest, StopObserving, WaitOptions};
use crate::config::ConfigService;
use crate::domain::{Analysis, AnalysisId, SaveState, TextOrigin};
use crate::history::{HistoryError, HistoryErrorCode, HistoryExportError, HistoryStore};
use crate::output::{CanonicalError, ErrorCode, ExitCode};

use super::history::{ExportRequest, HistoryLoadRequest, RedactedAnalysis};
use super::model::{AnalysisFailure, AppEvent};

/// Starts one fresh text analysis without making the terminal loop async.
pub(super) fn spawn_fresh_analysis(
    service: ConfigService,
    text: String,
    public_link: bool,
    manual_save: bool,
    automatic_save: bool,
    stop: StopObserving,
    events: Sender<AppEvent>,
) -> AnalysisId {
    let save_state = requested_save_state(manual_save, automatic_save);
    let retained_text = save_state.map(|_| text.clone());
    let request = fresh_request(text, public_link);
    let analysis_id = request.id();
    let fallback_events = events.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("pangram-tui-analysis".to_owned())
        .spawn(move || {
            run_analysis_worker(service, retained_text, request, save_state, stop, events);
        })
    {
        send_failure(&fallback_events, analysis_id, analysis_runtime_error(error));
    }
    analysis_id
}

/// Loads one certified summary page using the exact applied TUI criteria.
pub(super) fn spawn_history_load(
    service: ConfigService,
    request: HistoryLoadRequest,
    events: Sender<AppEvent>,
) {
    let fallback_request = request.clone();
    let fallback_events = events.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("pangram-tui-history-list".to_owned())
        .spawn(move || {
            let result = catch_history_worker(|| history_items(&service, &request));
            let _ = events.send(AppEvent::HistoryLoaded { request, result });
        })
    {
        let _ = fallback_events.send(AppEvent::HistoryLoaded {
            request: fallback_request,
            result: Err(history_worker_start_error(error)),
        });
    }
}

/// Loads and immediately redacts one certified detail before it crosses the
/// reducer channel seam.
pub(super) fn spawn_history_detail(
    service: ConfigService,
    analysis_id: AnalysisId,
    events: Sender<AppEvent>,
) {
    let fallback_events = events.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("pangram-tui-history-detail".to_owned())
        .spawn(move || {
            let result = catch_history_worker(|| {
                let store = existing_record_store(&service)?;
                store
                    .canonical_analysis(&analysis_id, false)
                    .map(RedactedAnalysis::new)
                    .map_err(|error| error.into_canonical())
            });
            let _ = events.send(AppEvent::HistoryDetailLoaded {
                analysis_id,
                result,
            });
        })
    {
        let _ = fallback_events.send(AppEvent::HistoryDetailLoaded {
            analysis_id,
            result: Err(history_worker_start_error(error)),
        });
    }
}

/// Performs the confirmed mutation in SQLite. The reducer keeps the row on
/// screen until this real result arrives and then requests a certified page.
pub(super) fn spawn_history_delete(
    service: ConfigService,
    analysis_id: AnalysisId,
    events: Sender<AppEvent>,
) {
    let fallback_events = events.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("pangram-tui-history-delete".to_owned())
        .spawn(move || {
            let (result, warning) = catch_history_worker(|| {
                let mut store = existing_record_store(&service)?;
                Ok(delete_history(&mut store, analysis_id))
            })
            .unwrap_or_else(|error| (Err(error), None));
            let _ = events.send(AppEvent::HistoryDeleted {
                analysis_id,
                result,
            });
            if let Some(warning) = warning {
                let _ = events.send(AppEvent::Notice(warning));
            }
        })
    {
        let _ = fallback_events.send(AppEvent::HistoryDeleted {
            analysis_id,
            result: Err(history_worker_start_error(error)),
        });
    }
}

/// Resolves retained input privately, acknowledges successful preflight, and
/// then enters the same Analyzer execution path as a fresh submission.
pub(super) fn spawn_history_rerun(
    service: ConfigService,
    original_id: AnalysisId,
    automatic_save: bool,
    stop: StopObserving,
    events: Sender<AppEvent>,
) {
    let fallback_events = events.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("pangram-tui-history-rerun".to_owned())
        .spawn(move || {
            let prepared = catch_history_worker(|| prepare_rerun(&service, original_id));
            let request = match prepared {
                Ok(request) => request,
                Err(error) => {
                    let _ = events.send(AppEvent::HistoryRerunPrepared {
                        analysis_id: original_id,
                        result: Err(error),
                    });
                    return;
                }
            };
            if events
                .send(AppEvent::HistoryRerunPrepared {
                    analysis_id: original_id,
                    result: Ok(request.id()),
                })
                .is_err()
            {
                return;
            }

            // Retained text never crosses the worker channel. It exists here only
            // because a successful automatic save must retain the submitted
            // bytes after Analyzer consumes the private request.
            let save_state = automatic_save.then_some(SaveState::SavedHistory);
            let retained_text = save_state.map(|_| request.text().to_owned());
            run_analysis_worker(service, retained_text, request, save_state, stop, events);
        })
    {
        let _ = fallback_events.send(AppEvent::HistoryRerunPrepared {
            analysis_id: original_id,
            result: Err(history_worker_start_error(error)),
        });
    }
}

/// Runs the certified raw export after terminal ownership has ended.
pub(super) fn export_after_restore(service: &ConfigService, request: ExportRequest) -> u8 {
    let store = match HistoryStore::open_existing(service.paths().data_dir()) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("{}", error.message());
            return ExitCode::LocalState.as_u8();
        }
    };
    let mut stdout = io::stdout().lock();
    match crate::history::export_history(
        store.as_ref(),
        &mut stdout,
        request.format,
        request.redact_content,
    ) {
        Ok(()) => ExitCode::Success.as_u8(),
        Err(HistoryExportError::History(error)) => {
            eprintln!("{}", error.message());
            ExitCode::LocalState.as_u8()
        }
        Err(HistoryExportError::Output) => ExitCode::GeneralFailure.as_u8(),
    }
}

fn run_analysis_worker(
    service: ConfigService,
    retained_text: Option<String>,
    request: AnalysisRequest,
    save_state: Option<SaveState>,
    stop: StopObserving,
    events: Sender<AppEvent>,
) {
    let analysis_id = request.id();
    let panic_events = events.clone();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                send_failure(&events, analysis_id, analysis_runtime_error(error));
                return;
            }
        };
        runtime.block_on(run_analysis(
            service,
            retained_text,
            request,
            save_state,
            stop,
            events,
        ));
    }));
    if outcome.is_err() {
        send_failure(&panic_events, analysis_id, analysis_worker_panic_error());
    }
}

async fn run_analysis(
    service: ConfigService,
    retained_text: Option<String>,
    request: AnalysisRequest,
    save_state: Option<SaveState>,
    stop: StopObserving,
    events: Sender<AppEvent>,
) {
    let analysis_id = request.id();
    let resolution = match service.credentials().resolve(service.overrides()) {
        Ok(resolution) => resolution,
        Err(error) => {
            send_failure(&events, analysis_id, crate::analysis::config_error(error));
            return;
        }
    };
    let Some(api_key) = resolution.key_for_client() else {
        send_failure(&events, analysis_id, crate::output::missing_api_key_error());
        return;
    };
    let analyzer = match crate::analysis::build_analyzer(&service, api_key) {
        Ok(analyzer) => analyzer,
        Err(error) => {
            send_failure(&events, analysis_id, error);
            return;
        }
    };

    let accepted = match analyzer.start_full(request, stop.token()).await {
        Ok(accepted) => accepted,
        Err(failure) => {
            send_failure(
                &events,
                failure.task_error.analysis_id(),
                failure.task_error.into_error(),
            );
            return;
        }
    };
    let completed = match accepted {
        Accepted::Terminal(analysis) => Some(*analysis),
        Accepted::Task(accepted) => {
            let running = analyzer.running(accepted);
            if events
                .send(AppEvent::AnalysisAccepted(running.snapshot()))
                .is_err()
            {
                return;
            }
            match running
                .observe(
                    WaitOptions::UNBOUNDED,
                    |progress| {
                        let _ = events.send(AppEvent::AnalysisProgress(progress.clone()));
                    },
                    stop,
                )
                .await
            {
                Ok(Ok(analysis)) => Some(analysis),
                Ok(Err(error)) => {
                    send_failure(&events, error.analysis_id(), error.into_error());
                    None
                }
                Err(_) => None,
            }
        }
    };
    let Some(mut analysis) = completed else {
        return;
    };

    if let Some(save_state) = save_state {
        let text = retained_text
            .as_deref()
            .expect("a requested save retains its submitted text");
        match save_analysis(&service, &analysis, text, save_state) {
            Ok(()) => {
                analysis = analysis.with_save_state(save_state);
                let _ = events.send(AppEvent::HistoryChanged);
            }
            Err(error) => {
                // A local retention failure cannot erase the remote result or
                // falsely claim a durable save.
                let _ = events.send(AppEvent::AnalysisFinished(analysis));
                let _ = events.send(AppEvent::Notice(format!(
                    "{} save failed: {}",
                    if save_state == SaveState::SavedManual {
                        "Manual"
                    } else {
                        "History"
                    },
                    error.message()
                )));
                return;
            }
        }
    }
    let _ = events.send(AppEvent::AnalysisFinished(analysis));
}

fn requested_save_state(manual_save: bool, automatic_save: bool) -> Option<SaveState> {
    if manual_save {
        Some(SaveState::SavedManual)
    } else if automatic_save {
        Some(SaveState::SavedHistory)
    } else {
        None
    }
}

fn fresh_request(text: String, public_link: bool) -> AnalysisRequest {
    let word_count = crate::analysis::canonical_text_word_count(&text);
    AnalysisRequest::new(
        text,
        TextOrigin::Literal,
        None,
        word_count,
        false,
        public_link,
    )
}

fn prepare_rerun(
    service: &ConfigService,
    analysis_id: AnalysisId,
) -> Result<AnalysisRequest, CanonicalError> {
    let store = existing_record_store(service)?;
    let original = store
        .canonical_analysis(&analysis_id, true)
        .map_err(|error| error.into_canonical())?;
    AnalysisRequest::from_saved_rerun(&original).ok_or_else(unresolvable_rerun)
}

fn history_items(
    service: &ConfigService,
    request: &HistoryLoadRequest,
) -> Result<Vec<crate::domain::AnalysisSummary>, CanonicalError> {
    let Some(store) = HistoryStore::open_existing(service.paths().data_dir())
        .map_err(|error| error.into_canonical())?
    else {
        return Ok(Vec::new());
    };
    let hits = match request.query.as_deref() {
        Some(query) => store.search_filtered(query, request.status, request.check, request.limit),
        None => store.list_filtered(request.status, request.check, request.limit, 0),
    }
    .map_err(|error| error.into_canonical())?;
    crate::history::summary_page(hits)
        .map(|page| page.items)
        .map_err(|error| error.into_canonical())
}

fn existing_record_store(service: &ConfigService) -> Result<HistoryStore, CanonicalError> {
    HistoryStore::open_existing(service.paths().data_dir())
        .map_err(|error| error.into_canonical())?
        .ok_or_else(|| missing_record().into_canonical())
}

fn save_analysis(
    service: &ConfigService,
    analysis: &Analysis<CanonicalError>,
    text: &str,
    save_state: SaveState,
) -> Result<(), HistoryError> {
    let mut store = HistoryStore::open(service.paths().data_dir())?;
    crate::history::save_complete_analysis(&mut store, analysis, save_state, Some(text))
}

fn delete_history(
    store: &mut HistoryStore,
    analysis_id: AnalysisId,
) -> (Result<(), CanonicalError>, Option<String>) {
    match store.delete_analysis(&analysis_id) {
        Ok(()) => (Ok(()), None),
        Err(error) => {
            // `delete_analysis` commits before its WAL truncation. If that
            // cleanup reports a warning, prove the row is absent rather than
            // making the reducer guess whether the mutation committed.
            let committed = error.code() == HistoryErrorCode::HistoryWriteFailed
                && matches!(
                    store.canonical_analysis(&analysis_id, false),
                    Err(probe) if probe.code() == HistoryErrorCode::NotFound
                );
            if committed {
                (
                    Ok(()),
                    Some(format!(
                        "History deletion committed, but cleanup failed: {}",
                        error.message()
                    )),
                )
            } else {
                (Err(error.into_canonical()), None)
            }
        }
    }
}

fn catch_history_worker<T>(
    work: impl FnOnce() -> Result<T, CanonicalError>,
) -> Result<T, CanonicalError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .unwrap_or_else(|_| Err(history_worker_panic_error()))
}

fn send_failure(events: &Sender<AppEvent>, analysis_id: AnalysisId, error: CanonicalError) {
    let _ = events.send(AppEvent::AnalysisFailed(AnalysisFailure {
        analysis_id,
        error,
    }));
}

fn missing_record() -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::NotFound,
        "no analysis with that identity is recorded",
    )
}

fn unresolvable_rerun() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::LocalTaskUnresolvable,
        "The saved analysis does not retain exact text that can be rerun.",
    )
    .expect("static error")
}

fn analysis_runtime_error(error: io::Error) -> CanonicalError {
    CanonicalError::new(
        ErrorCode::UpstreamError,
        format!(
            "could not start analysis runtime: {}",
            crate::config::redact_io(&error)
        ),
    )
    .and_then(|error| error.with_contextual_retryability(false))
    .expect("runtime error is valid")
}

fn analysis_worker_panic_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::UpstreamError,
        "the analysis worker stopped unexpectedly",
    )
    .and_then(|error| error.with_contextual_retryability(false))
    .expect("worker panic error is valid")
}

fn history_worker_start_error(error: io::Error) -> CanonicalError {
    CanonicalError::new(
        ErrorCode::HistoryUnavailable,
        format!(
            "could not start the history worker: {}",
            crate::config::redact_io(&error)
        ),
    )
    .and_then(|error| error.with_contextual_retryability(false))
    .expect("history worker error is valid")
}

fn history_worker_panic_error() -> CanonicalError {
    CanonicalError::new(
        ErrorCode::HistoryUnavailable,
        "the history worker stopped unexpectedly",
    )
    .and_then(|error| error.with_contextual_retryability(false))
    .expect("history worker panic error is valid")
}
