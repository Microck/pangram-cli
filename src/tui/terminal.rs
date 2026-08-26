//! Full-screen terminal ownership for the TUI runtime.
//!
//! Terminal mutation has one owner so every exit path uses the same cleanup.
//! The panic hook performs cleanup before delegating to the hook that was
//! installed when the session began. Signal handlers are deliberately separate
//! from cleanup: they only set atomic flags for the runtime to poll.

use std::io::{self, Write};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};

use crossterm::cursor;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

pub(crate) type TuiTerminal = Terminal<CrosstermBackend<io::Stdout>>;
type PanicHook = Box<dyn Fn(&panic::PanicHookInfo<'_>) + Send + Sync + 'static>;

/// The process signal observed by the TUI event loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessSignal {
    Interrupt,
    Terminate,
}

/// Pollable process signal state shared with signal-hook's flag handlers.
///
/// The handlers installed by signal-hook only store `true` in these atomics.
/// Terminal I/O remains on the ordinary runtime and panic paths.
#[derive(Clone)]
pub(crate) struct SignalState {
    interrupt: Arc<AtomicBool>,
    terminate: Arc<AtomicBool>,
}

impl SignalState {
    /// Returns and clears a pending shutdown request.
    ///
    /// SIGINT wins when both classes arrived before the same poll. Both flags
    /// are consumed because either request ends the same TUI session.
    pub(crate) fn take(&self) -> Option<ProcessSignal> {
        if !self.is_requested() {
            return None;
        }

        let interrupted = self.interrupt.swap(false, Ordering::SeqCst);
        let terminated = self.terminate.swap(false, Ordering::SeqCst);

        if interrupted {
            Some(ProcessSignal::Interrupt)
        } else if terminated {
            Some(ProcessSignal::Terminate)
        } else {
            None
        }
    }

    pub(crate) fn is_requested(&self) -> bool {
        self.interrupt.load(Ordering::SeqCst) || self.terminate.load(Ordering::SeqCst)
    }

    fn clear(&self) {
        self.interrupt.store(false, Ordering::SeqCst);
        self.terminate.store(false, Ordering::SeqCst);
    }
}

struct StoredIoError {
    kind: io::ErrorKind,
    message: String,
}

impl StoredIoError {
    fn from_error(error: io::Error) -> Self {
        Self {
            kind: error.kind(),
            message: error.to_string(),
        }
    }

    fn to_error(&self) -> io::Error {
        io::Error::new(self.kind, self.message.clone())
    }
}

fn process_signal_state() -> io::Result<SignalState> {
    static REGISTRATION: OnceLock<Result<SignalState, StoredIoError>> = OnceLock::new();

    let registration = REGISTRATION.get_or_init(register_process_signals);
    match registration {
        Ok(state) => Ok(state.clone()),
        Err(error) => Err(error.to_error()),
    }
}

fn register_process_signals() -> Result<SignalState, StoredIoError> {
    let state = SignalState {
        interrupt: Arc::new(AtomicBool::new(false)),
        terminate: Arc::new(AtomicBool::new(false)),
    };

    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&state.interrupt))
        .map_err(StoredIoError::from_error)?;

    // TERM_SIGNALS is signal-hook's portable catchable shutdown set: SIGTERM
    // and SIGINT on Windows, plus SIGQUIT on Unix. SIGINT has its own flag so
    // the runtime can preserve the direct-interrupt exit intent.
    for &signal in signal_hook::consts::TERM_SIGNALS {
        if signal == signal_hook::consts::SIGINT {
            continue;
        }
        signal_hook::flag::register(signal, Arc::clone(&state.terminate))
            .map_err(StoredIoError::from_error)?;
    }

    Ok(state)
}

/// Shared state makes explicit cleanup, Drop, and the panic hook converge on
/// one attempt-all restoration operation.
struct RestoreState {
    armed: AtomicBool,
}

impl RestoreState {
    fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
        }
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn is_armed(&self) -> bool {
        self.armed.load(Ordering::SeqCst)
    }
}

/// Owns the live full-screen terminal until restoration completes.
pub(crate) struct TerminalSession {
    terminal: TuiTerminal,
    restore_state: Arc<RestoreState>,
    signals: SignalState,
    panic_hook: PanicHookGuard,
}

impl TerminalSession {
    /// Enters raw mode and the alternate screen, then hides the cursor.
    pub(crate) fn enter() -> io::Result<Self> {
        let restore_state = Arc::new(RestoreState::new());
        let mut panic_hook = PanicHookGuard::install(Arc::clone(&restore_state))?;

        // Claim process-global terminal ownership before consuming a pending
        // signal. A rejected concurrent entry must not alter the active
        // session's signal state.
        let signals = process_signal_state()?;
        signals.clear();

        // Arm before the first terminal mutation. Cleanup commands are safe on
        // a partially entered terminal, so a panic or error between writes can
        // still run the complete inverse sequence.
        restore_state.arm();
        if let Err(error) = enable_raw_mode() {
            return Err(clean_up_failed_entry(
                error,
                &restore_state,
                &mut panic_hook,
            ));
        }

        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            cursor::Hide
        ) {
            return Err(clean_up_failed_entry(
                error,
                &restore_state,
                &mut panic_hook,
            ));
        }

        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                return Err(clean_up_failed_entry(
                    error,
                    &restore_state,
                    &mut panic_hook,
                ));
            }
        };

        Ok(Self {
            terminal,
            restore_state,
            signals,
            panic_hook,
        })
    }

    pub(crate) fn draw<F>(&mut self, render: F) -> io::Result<()>
    where
        F: FnOnce(&mut ratatui::Frame<'_>),
    {
        self.terminal.draw(render).map(|_| ())
    }

    pub(crate) fn signals(&self) -> &SignalState {
        &self.signals
    }

    /// Restores input mode, cursor visibility, and the main screen.
    ///
    /// Every restoration command is attempted even if an earlier command
    /// fails. A failed attempt rearms cleanup so Drop can try once more.
    pub(crate) fn restore(&mut self) -> io::Result<()> {
        // Ratatui tracks whether its last frame hid the cursor and otherwise
        // emits a final Show command from Terminal::drop. Clear that tracked
        // state before leaving the alternate screen so later stdout, such as
        // a history export, cannot be prefixed by terminal control bytes.
        let cursor_result = if self.restore_state.is_armed() {
            self.terminal.show_cursor()
        } else {
            Ok(())
        };
        let terminal_result =
            restore_terminal(&self.restore_state, &mut *self.terminal.backend_mut());
        let restore_result =
            finish_restore_attempt(&self.restore_state, cursor_result, terminal_result);
        if restore_result.is_ok() {
            self.panic_hook.restore();
        }
        restore_result
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn clean_up_failed_entry(
    error: io::Error,
    restore_state: &RestoreState,
    panic_hook: &mut PanicHookGuard,
) -> io::Error {
    let mut stdout = io::stdout();
    let restore_error = restore_terminal(restore_state, &mut stdout).err();
    panic_hook.restore();

    match restore_error {
        Some(restore_error) => io::Error::new(
            error.kind(),
            format!("{error}; terminal restoration also failed: {restore_error}"),
        ),
        None => error,
    }
}

fn restore_terminal<W>(state: &RestoreState, writer: &mut W) -> io::Result<()>
where
    W: Write,
{
    if !state.armed.swap(false, Ordering::SeqCst) {
        return Ok(());
    }

    let mut first_error = None;
    record_first_error(
        &mut first_error,
        execute!(&mut *writer, DisableBracketedPaste),
    );
    record_first_error(
        &mut first_error,
        execute!(&mut *writer, DisableMouseCapture),
    );
    record_first_error(&mut first_error, disable_raw_mode());
    record_first_error(&mut first_error, execute!(&mut *writer, cursor::Show));
    record_first_error(
        &mut first_error,
        execute!(&mut *writer, LeaveAlternateScreen),
    );

    if let Some(error) = first_error {
        state.armed.store(true, Ordering::SeqCst);
        Err(error)
    } else {
        Ok(())
    }
}

fn record_first_error(first_error: &mut Option<io::Error>, result: io::Result<()>) {
    if first_error.is_none() {
        *first_error = result.err();
    }
}

fn finish_restore_attempt(
    state: &RestoreState,
    cursor_result: io::Result<()>,
    terminal_result: io::Result<()>,
) -> io::Result<()> {
    let result = match (cursor_result, terminal_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(cursor_error), Ok(())) => Err(cursor_error),
        (Ok(()), Err(terminal_error)) => Err(terminal_error),
        (Err(cursor_error), Err(terminal_error)) => Err(io::Error::new(
            cursor_error.kind(),
            format!("{cursor_error}; terminal restoration also failed: {terminal_error}"),
        )),
    };

    if result.is_err() {
        // A successful low-level terminal sequence can otherwise disarm the
        // state after ratatui's cursor restoration failed. Keep the full
        // inverse sequence available to Drop and the installed panic hook.
        state.arm();
    }
    result
}

static PANIC_HOOK_SESSION: Mutex<()> = Mutex::new(());

struct PanicHookGuard {
    prior: Arc<Mutex<Option<PanicHook>>>,
    installed: bool,
    // A panic hook is process-global. Holding this lock for the full session
    // prevents sequential or accidental concurrent sessions from wrapping one
    // another's hook.
    _session_lock: MutexGuard<'static, ()>,
}

impl PanicHookGuard {
    fn install(restore_state: Arc<RestoreState>) -> io::Result<Self> {
        let session_lock = match PANIC_HOOK_SESSION.try_lock() {
            Ok(session_lock) => session_lock,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "another terminal session is active",
                ));
            }
        };
        let prior = Arc::new(Mutex::new(Some(panic::take_hook())));
        let prior_for_hook = Arc::clone(&prior);
        let terminal_owner = std::thread::current().id();

        panic::set_hook(Box::new(move |panic_info| {
            // Only an unwind on the thread that owns the terminal may tear
            // down raw mode and the alternate screen. Worker panics are
            // caught by their owner and projected into reducer events.
            if std::thread::current().id() == terminal_owner {
                let mut stdout = io::stdout();
                let _ = restore_terminal(&restore_state, &mut stdout);

                let prior = prior_for_hook
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(prior) = prior.as_ref() {
                    prior(panic_info);
                }
            }
        }));

        Ok(Self {
            prior,
            installed: true,
            _session_lock: session_lock,
        })
    }

    fn restore(&mut self) {
        if !self.installed {
            return;
        }

        if std::thread::panicking() {
            // std::panic rejects hook mutation from the panicking thread. The
            // hook has already restored the terminal before unwind begins, so
            // a short helper can safely restore the exact previous hook before
            // this guard releases the process-wide session lock.
            let prior = Arc::clone(&self.prior);
            let restored = std::thread::Builder::new()
                .name("pangram-panic-hook-restore".to_owned())
                .spawn(move || restore_previous_panic_hook(&prior))
                .and_then(|worker| {
                    worker
                        .join()
                        .map_err(|_| io::Error::other("panic-hook restorer panicked"))
                })
                .is_ok();
            if restored {
                self.installed = false;
            }
        } else if restore_previous_panic_hook(&self.prior) {
            self.installed = false;
        }
    }
}

impl Drop for PanicHookGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

fn restore_previous_panic_hook(prior: &Arc<Mutex<Option<PanicHook>>>) -> bool {
    let installed_hook = panic::take_hook();
    let prior_hook = prior
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();

    match prior_hook {
        Some(prior_hook) => {
            panic::set_hook(prior_hook);
            drop(installed_hook);
            true
        }
        None => {
            // Another restoration already consumed the prior hook. Preserve
            // whichever hook was installed when this redundant call began.
            panic::set_hook(installed_hook);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_attempt_reports_both_failures_and_remains_armed() {
        let state = RestoreState::new();
        let error = finish_restore_attempt(
            &state,
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "cursor restoration failed",
            )),
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "terminal restoration failed",
            )),
        )
        .expect_err("both restoration failures must be reported");

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(
            error.to_string(),
            "cursor restoration failed; terminal restoration also failed: terminal restoration failed"
        );
        assert!(state.is_armed(), "Drop must be able to retry cleanup");
    }

    #[test]
    fn enter_fails_fast_without_consuming_signal_when_panic_hook_is_owned() {
        let signals = process_signal_state().expect("process signals register");
        signals.clear();
        signals.interrupt.store(true, Ordering::SeqCst);
        let session_lock = PANIC_HOOK_SESSION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let error = match TerminalSession::enter() {
            Ok(_) => panic!("a concurrent terminal session must be rejected"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(error.to_string(), "another terminal session is active");
        assert!(
            signals.is_requested(),
            "rejected entry must not consume the active session's signal"
        );

        signals.clear();
        drop(session_lock);
    }
}
