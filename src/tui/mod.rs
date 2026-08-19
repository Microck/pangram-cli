//! Interactive terminal adapter.
//!
//! The adapter owns terminal events and effect execution only. The pure
//! reducer owns application state, `Analyzer` owns every Pangram request, and
//! `HistoryStore` owns every SQLite read or write.

#[path = "history.rs"]
mod history;
#[path = "history-reducer.rs"]
mod history_reducer;
#[path = "history-render.rs"]
mod history_render;
#[path = "history-runtime.rs"]
mod history_runtime;
mod input;
mod intro;
#[path = "intro-playback.rs"]
mod intro_playback;
#[path = "intro-render.rs"]
mod intro_render;
#[cfg(test)]
#[path = "intro-render-tests.rs"]
mod intro_render_tests;
mod model;
mod mouse;
mod render;
#[path = "result-lines.rs"]
mod result_lines;
#[path = "result-viewport.rs"]
mod result_viewport;
#[path = "settings-runtime.rs"]
mod settings_runtime;
mod terminal;
#[path = "text-field.rs"]
mod text_field;

mod active;

use std::io::{self, IsTerminal as _};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use crossterm::event;
#[cfg(test)]
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use crate::analysis::StopObserving;
use crate::config::{ConfigError, ConfigOverrides, ConfigService, OnboardingState};

#[cfg(test)]
use input::key_input;
use input::terminal_event;
#[cfg(test)]
use model::KeyInput;
use model::{
    AppEvent, AppState, ColorMode, Effect, IntroFrequency, MotionLevel, SettingsDraft,
    StartupState, TerminalSize,
};
use settings_runtime::{SettingWrite, SettingsWorker};
use terminal::{ProcessSignal, TerminalSession};

const EVENT_POLL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActiveAnalysisIdentity {
    Analysis(crate::domain::AnalysisId),
    PreparingRerun(crate::domain::AnalysisId),
}

struct ActiveAnalysis {
    stop: StopObserving,
    identity: ActiveAnalysisIdentity,
}

struct EffectExecutor<'a> {
    service: &'a ConfigService,
    worker_tx: &'a Sender<AppEvent>,
    settings_worker: &'a SettingsWorker,
    analyzer_source: &'a crate::analysis::AnalyzerSource,
    active_analysis: Option<ActiveAnalysis>,
}

enum LoopExit {
    Process(u8),
    Export(history::ExportRequest),
}

impl ActiveAnalysis {
    fn fresh(stop: StopObserving, analysis_id: crate::domain::AnalysisId) -> Self {
        Self {
            stop,
            identity: ActiveAnalysisIdentity::Analysis(analysis_id),
        }
    }

    fn preparing_rerun(stop: StopObserving, original_id: crate::domain::AnalysisId) -> Self {
        Self {
            stop,
            identity: ActiveAnalysisIdentity::PreparingRerun(original_id),
        }
    }

    fn stop(&self) {
        self.stop.stop();
    }
}

impl Drop for ActiveAnalysis {
    fn drop(&mut self) {
        self.stop();
    }
}

impl<'a> EffectExecutor<'a> {
    fn new(
        service: &'a ConfigService,
        worker_tx: &'a Sender<AppEvent>,
        settings_worker: &'a SettingsWorker,
        analyzer_source: &'a crate::analysis::AnalyzerSource,
    ) -> Self {
        Self {
            service,
            worker_tx,
            settings_worker,
            analyzer_source,
            active_analysis: None,
        }
    }

    fn apply_event(&mut self, state: &mut AppState, event: AppEvent) -> Option<LoopExit> {
        update_active_analysis(&event, &mut self.active_analysis);
        for effect in model::reduce_in_place(state, event) {
            match effect {
                Effect::SubmitText {
                    text,
                    mode,
                    public_link,
                    save,
                    automatic_save,
                } => {
                    let stop = StopObserving::new();
                    let analysis_id = history_runtime::spawn_fresh_analysis(
                        self.service.clone(),
                        history_runtime::FreshAnalysisOptions {
                            text,
                            mode,
                            public_link,
                            manual_save: save,
                            automatic_save,
                        },
                        stop.clone(),
                        self.worker_tx.clone(),
                        self.analyzer_source.clone(),
                    );
                    self.active_analysis = Some(ActiveAnalysis::fresh(stop, analysis_id));
                }
                Effect::StoreCredential { credential } => {
                    self.settings_worker
                        .store(SettingWrite::Credential(credential));
                }
                Effect::StoreUpdatePreference(choice) => {
                    self.settings_worker
                        .store(SettingWrite::UpdatePreference(choice));
                }
                Effect::StoreHistory(enabled) => {
                    self.settings_worker.store(SettingWrite::History(enabled));
                }
                Effect::StoreIntro(intro) => {
                    self.settings_worker.store(SettingWrite::Intro(intro));
                }
                Effect::StoreKeymap(keymap) => {
                    self.settings_worker.store(SettingWrite::Keymap(keymap));
                }
                Effect::StoreMotion(motion) => {
                    self.settings_worker.store(SettingWrite::Motion(motion));
                }
                Effect::LoadHistory(request) => history_runtime::spawn_history_load(
                    self.service.clone(),
                    request,
                    self.worker_tx.clone(),
                ),
                Effect::LoadHistoryDetail(analysis_id) => history_runtime::spawn_history_detail(
                    self.service.clone(),
                    analysis_id,
                    self.worker_tx.clone(),
                ),
                Effect::DeleteHistory(analysis_id) => history_runtime::spawn_history_delete(
                    self.service.clone(),
                    analysis_id,
                    self.worker_tx.clone(),
                ),
                Effect::PrepareHistoryRerun {
                    analysis_id,
                    automatic_save,
                } => {
                    let stop = StopObserving::new();
                    history_runtime::spawn_history_rerun(
                        self.service.clone(),
                        analysis_id,
                        automatic_save,
                        stop.clone(),
                        self.worker_tx.clone(),
                        self.analyzer_source.clone(),
                    );
                    self.active_analysis = Some(ActiveAnalysis::preparing_rerun(stop, analysis_id));
                }
                Effect::ExportHistory(request) => return Some(LoopExit::Export(request)),
                Effect::Exit(exit_code) => return Some(LoopExit::Process(exit_code)),
            }
        }
        None
    }

    fn stop_active_analysis(&mut self) {
        if let Some(active) = self.active_analysis.take() {
            active.stop();
        }
    }
}

/// Runs the full-screen adapter and returns a process exit intent.
pub(crate) fn run(analyzer_source: crate::analysis::AnalyzerSource) -> u8 {
    match run_inner(analyzer_source) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("pangram: terminal interface failed: {error}");
            1
        }
    }
}

fn run_inner(analyzer_source: crate::analysis::AnalyzerSource) -> Result<u8, TuiError> {
    let overrides = ConfigOverrides::merge(
        ConfigOverrides::default(),
        ConfigOverrides::from_environment(),
    );
    let service = ConfigService::new(&overrides)?;
    let startup = startup_state(&service)?;
    let (columns, rows) = crossterm::terminal::size()?;
    let terminal_size = TerminalSize { columns, rows };
    let intro_state = intro::load_state(service.paths().data_dir());
    let intro_plan = intro::plan_intro(
        startup.settings.intro,
        startup.settings.motion,
        intro_state.seen(),
        intro::LaunchCapabilities::new(
            io::stdin().is_terminal(),
            io::stdout().is_terminal(),
            io::stderr().is_terminal(),
            std::env::var_os("CI").is_some(),
            std::env::var("TERM").ok().as_deref(),
            terminal_size,
        ),
    );
    let intro_frequency = startup.settings.intro;
    let mut state = AppState::new(terminal_size, startup);
    if let Some(diagnostic) = intro_state.diagnostic() {
        state.notice = Some(diagnostic.message().to_owned());
    }
    let (worker_tx, worker_rx) = mpsc::channel();
    let settings_worker = SettingsWorker::spawn(service.clone(), worker_tx.clone());
    // Declare the terminal owner after the settings worker so Rust's reverse
    // drop order restores the terminal before waiting on any queued durable
    // write during an early-error return.
    let mut session = TerminalSession::enter()?;
    #[cfg(feature = "dev-tools")]
    inject_terminal_failure_for_test()?;
    let (intro_resolution, deferred_input) =
        match intro_playback::play(&mut session, intro_plan, &state)? {
            intro_playback::PlaybackExit::Continue {
                resolution,
                deferred,
            } => (resolution, deferred),
            intro_playback::PlaybackExit::Process(exit_code) => {
                session.restore()?;
                drop(settings_worker);
                drop(session);
                return Ok(exit_code);
            }
        };
    let intro_marker = intro_resolution
        .filter(|resolution| intro::should_mark_seen(intro_frequency, intro_plan, *resolution))
        .and_then(|_| {
            let data_dir = service.paths().data_dir().to_owned();
            let events = worker_tx.clone();
            std::thread::Builder::new()
                .name("pangram-tui-intro-state".to_owned())
                .spawn(move || {
                    if let Err(diagnostic) = intro::mark_seen(&data_dir) {
                        let _ = events.send(AppEvent::Notice(diagnostic.message().to_owned()));
                    }
                })
                .map_err(|_| {
                    state.notice = Some(intro::IntroDiagnostic::Write.message().to_owned());
                })
                .ok()
        });
    let mut effects = EffectExecutor::new(&service, &worker_tx, &settings_worker, &analyzer_source);

    // The first useful frame is independent of disk-backed history. Loading
    // follows it so a large or contended database cannot delay TUI startup.
    session.draw(|frame| render::render(frame, &state))?;
    history_runtime::spawn_history_load(
        service.clone(),
        state.history.load_request(),
        worker_tx.clone(),
    );

    let redraw_after_deferred_input = !deferred_input.is_empty();
    let mut deferred_exit = None;
    for input in deferred_input {
        if let Some(event) = terminal_event(input, &state)
            && let Some(loop_exit) = effects.apply_event(&mut state, event)
        {
            deferred_exit = Some(loop_exit);
            break;
        }
    }
    if deferred_exit.is_none() && redraw_after_deferred_input {
        session.draw(|frame| render::render(frame, &state))?;
    }

    let loop_exit = if let Some(loop_exit) = deferred_exit {
        loop_exit
    } else {
        'main: loop {
            let mut redraw = false;
            while let Ok(event) = worker_rx.try_recv() {
                if let Some(loop_exit) = effects.apply_event(&mut state, event) {
                    break 'main loop_exit;
                }
                redraw = true;
            }

            if let Some(signal) = session.signals().take() {
                break LoopExit::Process(match signal {
                    ProcessSignal::Interrupt => 130,
                    ProcessSignal::Terminate => 1,
                });
            }

            if redraw {
                session.draw(|frame| render::render(frame, &state))?;
            }
            if event::poll(EVENT_POLL)?
                && let Some(event) = terminal_event(event::read()?, &state)
            {
                if let Some(loop_exit) = effects.apply_event(&mut state, event) {
                    break loop_exit;
                }
                session.draw(|frame| render::render(frame, &state))?;
            }
        }
    };

    effects.stop_active_analysis();
    session.restore()?;
    if let Some(worker) = intro_marker {
        let _ = worker.join();
    }
    drop(effects);
    drop(settings_worker);
    // Raw export is a primary stdout surface. Drop the terminal owner before
    // the first export byte so neither its panic hook nor alternate-screen
    // backend remains active during streaming output.
    drop(session);
    Ok(match loop_exit {
        LoopExit::Process(exit_code) => exit_code,
        LoopExit::Export(request) => history_runtime::export_after_restore(&service, request),
    })
}

fn update_active_analysis(event: &AppEvent, active: &mut Option<ActiveAnalysis>) {
    let Some(owned) = active.as_mut() else {
        return;
    };
    let finished = match (owned.identity, event) {
        (
            ActiveAnalysisIdentity::PreparingRerun(expected),
            AppEvent::HistoryRerunPrepared {
                analysis_id,
                result: Ok(rerun_id),
            },
        ) if expected == *analysis_id => {
            owned.identity = ActiveAnalysisIdentity::Analysis(*rerun_id);
            false
        }
        (
            ActiveAnalysisIdentity::PreparingRerun(expected),
            AppEvent::HistoryRerunPrepared {
                analysis_id,
                result: Err(_),
            },
        ) => expected == *analysis_id,
        (ActiveAnalysisIdentity::Analysis(expected), AppEvent::AnalysisFinished(analysis)) => {
            expected == analysis.id
        }
        (ActiveAnalysisIdentity::Analysis(expected), AppEvent::AnalysisFailed(failure)) => {
            expected == failure.analysis_id
        }
        _ => false,
    };
    if finished {
        *active = None;
    }
}

#[cfg(feature = "dev-tools")]
fn inject_terminal_failure_for_test() -> Result<(), TuiError> {
    // Compiled PTY tests need failures after terminal ownership begins. These
    // seams do not exist in normal builds and accept one exact sentinel so an
    // inherited environment cannot trigger them by accident.
    if std::env::var_os("PANGRAM_TUI_TEST_PANIC_AFTER_ENTER").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        panic!("injected terminal-owner panic");
    }
    if std::env::var_os("PANGRAM_TUI_TEST_IO_ERROR_AFTER_ENTER").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        return Err(io::Error::other("injected terminal I/O failure").into());
    }
    Ok(())
}

fn startup_state(service: &ConfigService) -> Result<StartupState, ConfigError> {
    let persisted = service.persisted()?;
    let onboarding = OnboardingState::read_with(
        service.paths(),
        service.credentials(),
        service.overrides(),
        Some(&persisted),
    )?;
    let effective = persisted.with_defaults();
    let tui = effective.tui.expect("TUI defaults are complete");
    let settings = SettingsDraft {
        credential_present: onboarding.credential_configured(),
        history_enabled: effective
            .history
            .and_then(|history| history.enabled)
            .expect("history defaults are complete"),
        intro: match tui.intro.expect("intro default is complete") {
            crate::config::IntroMode::Once => IntroFrequency::Once,
            crate::config::IntroMode::Always => IntroFrequency::Always,
            crate::config::IntroMode::Off => IntroFrequency::Off,
        },
        motion: match tui.motion.expect("motion default is complete") {
            crate::config::Motion::Full => MotionLevel::Full,
            crate::config::Motion::Reduced => MotionLevel::Reduced,
            crate::config::Motion::Off => MotionLevel::Off,
        },
        update_preference: effective
            .updates
            .and_then(|updates| updates.check_on_tui_start),
    };
    let keymap = match tui.keymap.expect("keymap default is complete") {
        crate::config::Keymap::Regular => model::Keymap::Regular,
        crate::config::Keymap::Vim => model::Keymap::Vim,
    };
    Ok(StartupState {
        settings,
        keymap,
        color_mode: resolve_color_mode(
            std::env::var_os("NO_COLOR").as_deref(),
            std::env::var("TERM").ok().as_deref(),
            std::env::var("COLORTERM").ok().as_deref(),
        ),
    })
}

fn resolve_color_mode(
    no_color: Option<&std::ffi::OsStr>,
    term: Option<&str>,
    colorterm: Option<&str>,
) -> ColorMode {
    if no_color.is_some_and(|value| !value.is_empty())
        || term.is_some_and(|value| value.eq_ignore_ascii_case("dumb"))
    {
        return ColorMode::None;
    }
    if colorterm.is_some_and(|value| {
        value.eq_ignore_ascii_case("truecolor") || value.eq_ignore_ascii_case("24bit")
    }) {
        ColorMode::TrueColor
    } else {
        ColorMode::Ansi
    }
}

enum TuiError {
    Config(ConfigError),
    Terminal(io::Error),
}

impl std::fmt::Display for TuiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::Terminal(error) => formatter.write_str(&crate::config::redact_io(error)),
        }
    }
}

impl From<ConfigError> for TuiError {
    fn from(error: ConfigError) -> Self {
        Self::Config(error)
    }
}

impl From<io::Error> for TuiError {
    fn from(error: io::Error) -> Self {
        Self::Terminal(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

    fn pointer_state(columns: u16) -> AppState {
        let mut state = AppState::default();
        state.terminal = TerminalSize { columns, rows: 40 };
        state.overlay = None;
        state
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn failed_event(analysis_id: crate::domain::AnalysisId) -> AppEvent {
        AppEvent::AnalysisFailed(model::AnalysisFailure {
            analysis_id,
            error: crate::output::CanonicalError::new(
                crate::output::ErrorCode::NetworkUnavailable,
                "offline",
            )
            .expect("valid canonical error"),
        })
    }

    #[test]
    fn control_keys_do_not_become_printable_text() {
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(
            key_input(KeyEvent::new(KeyCode::Char('c'), ctrl)),
            Some(KeyInput::CtrlC)
        );
        assert_eq!(key_input(KeyEvent::new(KeyCode::Char('x'), ctrl)), None);
    }

    #[test]
    fn shifted_printable_keys_stay_printable() {
        assert_eq!(
            key_input(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT)),
            Some(KeyInput::Character('H'))
        );
    }

    #[test]
    fn bracketed_paste_stays_one_atomic_event() {
        let payload = "one\ttwo\n?j";
        let state = pointer_state(120);
        let Some(AppEvent::Paste(actual)) =
            terminal_event(Event::Paste(payload.to_owned()), &state)
        else {
            panic!("bracketed paste must remain atomic")
        };
        assert_eq!(actual, payload);
    }

    #[test]
    fn mouse_targets_emit_typed_intent_for_visible_controls() {
        let mut state = pointer_state(120);

        assert!(matches!(
            terminal_event(mouse(MouseEventKind::Down(MouseButton::Left), 2, 7), &state,),
            Some(AppEvent::Pointer(model::PointerIntent::Route(
                model::Route::History
            )))
        ));
        assert!(matches!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 45, 24),
                &state,
            ),
            Some(AppEvent::Pointer(model::PointerIntent::Activate(
                model::Focus::CheckPlagiarism
            )))
        ));
        assert!(matches!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 20, 32),
                &state,
            ),
            Some(AppEvent::Pointer(model::PointerIntent::Focus(
                model::Focus::Composer
            )))
        ));
        assert!(matches!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 17, 39),
                &state,
            ),
            Some(AppEvent::Pointer(model::PointerIntent::Key(
                model::KeyInput::Tab
            )))
        ));
        assert!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 2, 39),
                &state,
            )
            .is_none(),
            "wide footer space beneath the route rail must not be clickable"
        );
        assert!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 14, 39),
                &state,
            )
            .is_none(),
            "footer separators must not behave like controls"
        );
        assert!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 110, 39),
                &state,
            )
            .is_none(),
            "blank footer space must not activate the focused command"
        );

        state.route = model::Route::History;
        assert!(matches!(
            terminal_event(mouse(MouseEventKind::ScrollDown, 40, 10), &state),
            Some(AppEvent::Pointer(model::PointerIntent::Scroll {
                focus: model::Focus::HistoryList,
                direction: model::PointerDirection::Next,
            }))
        ));
        assert!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Right), 2, 7),
                &state,
            )
            .is_none()
        );

        let mut narrow = pointer_state(111);
        assert!(matches!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 2, 39),
                &narrow,
            ),
            Some(AppEvent::Pointer(model::PointerIntent::Key(
                model::KeyInput::Tab
            )))
        ));
        assert!(matches!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 4, 37),
                &narrow,
            ),
            Some(AppEvent::Pointer(model::PointerIntent::Activate(
                model::Focus::PublicLink
            )))
        ));
        assert!(matches!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 105, 37),
                &narrow,
            ),
            Some(AppEvent::Pointer(model::PointerIntent::Activate(
                model::Focus::Submit
            )))
        ));
        narrow.text_mode = crate::analysis::TextAnalysisMode::Plagiarism;
        assert!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 4, 37),
                &narrow,
            )
            .is_none(),
            "an unavailable public-link control must not be clickable"
        );
        assert!(matches!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 22, 37),
                &narrow,
            ),
            Some(AppEvent::Pointer(model::PointerIntent::Activate(
                model::Focus::ManualSave
            )))
        ));

        let mut submitting = pointer_state(120);
        submitting.analysis.submitting = true;
        assert!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 40, 10),
                &submitting,
            )
            .is_none(),
            "progress-only workspace has no result viewport"
        );
        assert!(
            terminal_event(mouse(MouseEventKind::ScrollDown, 40, 10), &submitting).is_none(),
            "progress-only workspace must not scroll a nonexistent result"
        );

        state.overlay = Some(model::Overlay::HistoryExport {
            field: model::HistoryExportField::Action,
        });
        assert!(matches!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 34, 16),
                &state,
            ),
            Some(AppEvent::Pointer(model::PointerIntent::HistoryExportField(
                model::HistoryExportField::Format
            )))
        ));
    }

    #[test]
    fn overlay_mouse_targets_follow_the_rendered_action_labels() {
        let mut state = pointer_state(120);
        state.overlay = Some(model::Overlay::Credential(model::CredentialEntry::default()));

        assert!(matches!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 36, 20),
                &state,
            ),
            Some(AppEvent::Pointer(model::PointerIntent::Key(
                model::KeyInput::Enter
            )))
        ));
        assert!(matches!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 49, 20),
                &state,
            ),
            Some(AppEvent::Pointer(model::PointerIntent::Key(
                model::KeyInput::Escape
            )))
        ));
        assert!(
            terminal_event(
                mouse(MouseEventKind::Down(MouseButton::Left), 46, 20),
                &state,
            )
            .is_none()
        );
    }

    #[test]
    fn terminal_color_capability_honors_explicit_disable_and_truecolor() {
        assert_eq!(
            resolve_color_mode(
                Some(std::ffi::OsStr::new("1")),
                Some("xterm-256color"),
                Some("truecolor")
            ),
            ColorMode::None
        );
        assert_eq!(
            resolve_color_mode(None, Some("dumb"), Some("truecolor")),
            ColorMode::None
        );
        assert_eq!(
            resolve_color_mode(None, Some("xterm-256color"), Some("truecolor")),
            ColorMode::TrueColor
        );
        assert_eq!(
            resolve_color_mode(None, Some("xterm-256color"), None),
            ColorMode::Ansi
        );
    }

    #[test]
    fn stop_token_owner_ignores_unrelated_analysis_completions() {
        let expected = crate::domain::AnalysisId::new();
        let unrelated = crate::domain::AnalysisId::new();
        let stop = StopObserving::new();
        let observer = stop.clone();
        let mut active = Some(ActiveAnalysis::fresh(stop, expected));

        update_active_analysis(&failed_event(unrelated), &mut active);
        assert!(active.is_some());
        assert!(!observer.token().is_cancelled());

        update_active_analysis(&failed_event(expected), &mut active);
        assert!(active.is_none());
        assert!(observer.token().is_cancelled());
    }

    #[test]
    fn rerun_stop_token_binds_only_to_its_prepared_analysis() {
        let source = crate::domain::AnalysisId::new();
        let unrelated_source = crate::domain::AnalysisId::new();
        let rerun = crate::domain::AnalysisId::new();
        let unrelated_analysis = crate::domain::AnalysisId::new();
        let stop = StopObserving::new();
        let observer = stop.clone();
        let mut active = Some(ActiveAnalysis::preparing_rerun(stop, source));

        update_active_analysis(
            &AppEvent::HistoryRerunPrepared {
                analysis_id: unrelated_source,
                result: Ok(unrelated_analysis),
            },
            &mut active,
        );
        update_active_analysis(&failed_event(unrelated_analysis), &mut active);
        assert!(active.is_some());
        assert!(!observer.token().is_cancelled());

        update_active_analysis(
            &AppEvent::HistoryRerunPrepared {
                analysis_id: source,
                result: Ok(rerun),
            },
            &mut active,
        );
        update_active_analysis(&failed_event(unrelated_analysis), &mut active);
        assert!(active.is_some());
        assert!(!observer.token().is_cancelled());

        update_active_analysis(&failed_event(rerun), &mut active);
        assert!(active.is_none());
        assert!(observer.token().is_cancelled());
    }
}
