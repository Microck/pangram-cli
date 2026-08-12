//! SIGINT plumbing for detection. Credential resolution and analyzer
//! construction stay in the shared analysis execution context.

use tokio_util::sync::CancellationToken;

/// The CTRL+C/SIGINT request flag. The low-level signal handler only ever
/// stores this atomic (an async-signal-safe operation), so the handler can
/// never deadlock on a lock the interrupted thread already holds, and never
/// wakes Tokio wakers or takes the token-internal mutex from signal context
/// (a CodeRabbit stability finding). A normal async task translates a set
/// flag into the active observation's `CancellationToken` cancel.
fn sigint_flag() -> &'static std::sync::Arc<std::sync::atomic::AtomicBool> {
    static FLAG: std::sync::OnceLock<std::sync::Arc<std::sync::atomic::AtomicBool>> =
        std::sync::OnceLock::new();
    FLAG.get_or_init(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)))
}

/// Installs the SIGINT driver exactly once. The handler records the interrupt
/// on the process-global atomic flag; it does no other work, so registration
/// maps every target the signal-hook crate supports while staying
/// signal-safe. A driver-install failure is non-fatal: without it no SIGINT
/// is trapped, so the interruption path is simply never exercised.
pub(crate) fn install_sigint_driver() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = signal_hook::flag::register(
            signal_hook::consts::SIGINT,
            std::sync::Arc::clone(sigint_flag()),
        );
    });
}

/// Watches the SIGINT flag and cancels `token` once an interrupt arrives.
/// Spawned inside the async runtime; polling the observed flag at the shared
/// pacing interval keeps SIGINT response well under the observation poll
/// cadence while doing zero work on the signal handler path.
pub(crate) async fn bridge_sigint(token: CancellationToken) {
    install_sigint_driver();
    loop {
        if sigint_flag().load(std::sync::atomic::Ordering::SeqCst) {
            token.cancel();
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// Clears the SIGINT flag when one observation flow ends so a delivered
/// interrupt from a finished flow is not re-read by a later flow.
pub(crate) fn reset_sigint_flag() {
    sigint_flag().store(false, std::sync::atomic::Ordering::SeqCst);
}
