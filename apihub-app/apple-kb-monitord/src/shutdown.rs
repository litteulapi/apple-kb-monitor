//! `WillShutdown` at shutdown / restart, as macOS does.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use akm_core::parity::Outcome;
use akm_core::Watch;

/// Longest wait for the write, so that the shutdown is never held longer.
pub const BUDGET: Duration = Duration::from_millis(3500);

static ENABLED: AtomicBool = AtomicBool::new(true);
static WATCH: OnceLock<Arc<Watch>> = OnceLock::new();

/// Called once by `main`: `[apple] will_shutdown` and the daemon's view of the link.
pub fn install(enabled: bool, watch: Arc<Watch>) {
    ENABLED.store(enabled, Ordering::Relaxed);
    let _ = WATCH.set(watch);
    crate::sleep::set_shutdown_hook(|| {
        notify("logind PrepareForShutdown");
    });
    tracing::info!(
        "WillShutdown at shutdown: {} ([apple] will_shutdown)",
        if enabled { "on" } else { "off" }
    );
}

/// Is `[apple] will_shutdown` on?
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn connected() -> bool {
    WATCH.get().is_some_and(|w| w.get().connected)
}

/// Send `WillShutdown` now if every condition holds (see [`akm_core::parity`]).
#[allow(clippy::must_use_candidate)] // sends the frame: the shutdown hook has no use for the outcome
pub fn notify(source: &str) -> Outcome {
    let (enabled, connected) = (enabled(), connected());
    notify_with(source, BUDGET, move || {
        akm_core::hidraw::send_will_shutdown(enabled, connected)
    })
}

/// [`notify`] with the sender injected (tests use a spy: never the hardware).
pub fn notify_with(
    source: &str,
    budget: Duration,
    send: impl FnOnce() -> Outcome + Send + 'static,
) -> Outcome {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("kb-will-shutdown".into())
        .spawn(move || {
            let _ = tx.send(send());
        });
    let outcome = match spawned {
        Err(e) => Outcome::Failed(format!("cannot spawn the writer: {e}")),
        Ok(_) => rx.recv_timeout(budget).unwrap_or_else(|_| {
            Outcome::Failed(format!("no answer within {budget:?}, not waiting longer"))
        }),
    };
    if outcome.is_sent() {
        tracing::info!("{source}: {}", outcome.describe());
    } else {
        tracing::warn!("{source}: {}", outcome.describe());
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_what_the_sender_said() {
        let o = notify_with("test", Duration::from_secs(2), || Outcome::Sent);
        assert_eq!(o, Outcome::Sent);
        let o = notify_with("test", Duration::from_secs(2), || Outcome::AlreadySent);
        assert_eq!(o, Outcome::AlreadySent);
    }

    #[test]
    fn a_mute_keyboard_never_holds_the_shutdown_beyond_the_budget() {
        let t = std::time::Instant::now();
        let o = notify_with("test", Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(2));
            Outcome::Sent
        });
        assert!(matches!(o, Outcome::Failed(_)));
        assert!(t.elapsed() < Duration::from_millis(1500));
    }
}
