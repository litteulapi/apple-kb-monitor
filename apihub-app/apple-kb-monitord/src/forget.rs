//! "Forget this keyboard" from the tray: never in one click.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A request is confirmed within this delay, or forgotten.
pub const CONFIRM_WINDOW: Duration = Duration::from_mins(1);

/// A forget that waits for its confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub mac: String,
    pub name: String,
    asked: Instant,
}

/// One pending request at most; pure (instants given by the caller).
#[derive(Debug, Default)]
pub struct Gate {
    pending: Option<Pending>,
}

impl Gate {
    #[must_use]
    pub const fn new() -> Self {
        Self { pending: None }
    }

    /// The user asked to forget `mac` at `now` (replaces a former request).
    pub fn request(&mut self, mac: &str, name: &str, now: Instant) {
        self.pending = Some(Pending {
            mac: mac.to_ascii_uppercase(),
            name: name.to_string(),
            asked: now,
        });
    }

    /// The confirmation button was pressed at `now`.
    pub fn confirm(&mut self, now: Instant) -> Option<Pending> {
        let p = self.pending.take()?;
        (now.saturating_duration_since(p.asked) <= CONFIRM_WINDOW).then_some(p)
    }

    /// The request was dropped (notification closed).
    pub fn cancel(&mut self) {
        self.pending = None;
    }

    #[cfg(test)]
    #[must_use]
    pub fn is_pending(&self, now: Instant) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|p| now.saturating_duration_since(p.asked) <= CONFIRM_WINDOW)
    }
}

fn gate() -> &'static Mutex<Gate> {
    static G: OnceLock<Mutex<Gate>> = OnceLock::new();
    G.get_or_init(|| Mutex::new(Gate::new()))
}

fn lock() -> std::sync::MutexGuard<'static, Gate> {
    gate()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Ask to forget the keyboard `mac` (shown as `name`): raises the confirmation notification.
pub fn request(mac: &str, name: &str) -> bool {
    if crate::devices::device_path(mac).is_none() {
        return false;
    }
    lock().request(mac, name, Instant::now());
    tracing::info!("forget of {mac} asked: waiting for the confirmation (60 s)");
    let n = crate::notify::forget_confirm_notification(name);
    let spawned = std::thread::Builder::new()
        .name("kb-forget-ask".into())
        .spawn(move || {
            if crate::notify::deliver_blocking(&n).is_none() {
                tracing::warn!(
                    "forget: no notification server to confirm with, opening the guided repair"
                );
                lock().cancel();
                crate::repair::launch_repair(None);
            }
        });
    spawned.is_ok()
}

/// The "Forget" button was pressed: hand the pending keyboard to the link keeper.
pub fn confirmed() -> Option<Pending> {
    let p = lock().confirm(Instant::now());
    if let Some(p) = p.as_ref() {
        tracing::warn!("forget of {} confirmed by the user", p.mac);
        if !crate::repair::user_forget(&p.mac) {
            tracing::warn!("forget: the link keeper is not running, nothing removed");
        }
    } else {
        tracing::info!("forget: confirmation without a pending request (expired?)");
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: &str = "AA:BB:CC:DD:EE:F1";

    #[test]
    fn nothing_is_forgotten_without_a_request_then_a_confirmation() {
        let t0 = Instant::now();
        let mut g = Gate::new();
        assert_eq!(g.confirm(t0), None, "a confirmation alone forgets nothing");
        g.request(MAC, "Alice's keyboard", t0);
        assert!(g.is_pending(t0 + Duration::from_secs(59)));
        let p = g.confirm(t0 + Duration::from_secs(10)).expect("confirmed");
        assert_eq!((p.mac.as_str(), p.name.as_str()), (MAC, "Alice's keyboard"));
        assert_eq!(g.confirm(t0 + Duration::from_secs(11)), None, "once");
    }

    #[test]
    fn a_request_lapses_after_a_minute_or_when_cancelled() {
        let t0 = Instant::now();
        let mut g = Gate::new();
        g.request(MAC, "kb", t0);
        assert!(!g.is_pending(t0 + CONFIRM_WINDOW + Duration::from_secs(1)));
        assert_eq!(
            g.confirm(t0 + CONFIRM_WINDOW + Duration::from_secs(1)),
            None
        );
        assert_eq!(
            g.confirm(t0 + Duration::from_secs(5)),
            None,
            "and it is gone"
        );
        g.request(MAC, "kb", t0);
        g.cancel();
        assert_eq!(g.confirm(t0 + Duration::from_secs(1)), None);
        g.request(MAC, "kb", t0);
        g.request("aa:bb:cc:dd:ee:02", "other", t0 + Duration::from_secs(1));
        assert_eq!(
            g.confirm(t0 + Duration::from_secs(2)).unwrap().mac,
            "AA:BB:CC:DD:EE:02"
        );
    }

    #[test]
    fn a_request_for_something_that_is_not_an_address_is_refused() {
        assert!(!request("not a mac", "x"));
        assert!(!request("", "x"));
    }

    #[test]
    fn only_the_keeper_talks_to_bluez() {
        let src = include_str!("forget.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for forbidden in ["call_method", "hidraw", "set_report", "Connection::system"] {
            assert!(!code.contains(forbidden), "{forbidden}");
        }
    }
}
