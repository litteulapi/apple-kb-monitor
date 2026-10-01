//! Connection / disconnection / reconnection notifications (#84, F03).
//!
//! Pure tracker fed by the daemon: `disconnected()` when BlueZ reports the
//! keyboard gone, `acquired(mac, pct)` after each successful read. The first
//! acquisition of a run (session start, daemon restart) is silent; a failed
//! read while still connected does not count as a disconnection.

/// What to tell the user.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    Disconnected { mac: String },
    /// The keyboard was switched off (Input `0x13` bit 1 = 0 just before the
    /// link dropped, #190): not a lost link, no "disconnected" alert.
    PoweredOff { mac: String },
    Reconnected { mac: String, pct: Option<f64> },
}

#[derive(Debug, Clone, Default)]
pub struct LinkTracker {
    /// MAC of the keyboard seen connected, while it is.
    connected: Option<String>,
    /// MAC of the keyboard that went away (pending reconnection).
    lost: Option<String>,
}

impl LinkTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// A read succeeded. Returns `Reconnected` only after a disconnection
    /// seen during this run.
    pub fn acquired(&mut self, mac: &str, pct: Option<f64>) -> Option<LinkEvent> {
        if self
            .connected
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case(mac))
        {
            return None;
        }
        self.connected = Some(mac.to_string());
        self.lost.take().map(|_| LinkEvent::Reconnected {
            mac: mac.to_string(),
            pct,
        })
    }

    /// BlueZ reported the keyboard disconnected.
    pub fn disconnected(&mut self) -> Option<LinkEvent> {
        self.disconnected_as(false)
    }

    /// Same, knowing whether the keyboard announced its own switch-off
    /// (`powered_off`, see [`take_keyboard_off`]): the event is then
    /// [`LinkEvent::PoweredOff`], like macOS which drops its `Disconnected`
    /// notification after a `KeyboardOff`. The reconnection is tracked alike.
    pub fn disconnected_as(&mut self, powered_off: bool) -> Option<LinkEvent> {
        let mac = self.connected.take()?;
        self.lost = Some(mac.clone());
        Some(if powered_off {
            LinkEvent::PoweredOff { mac }
        } else {
            LinkEvent::Disconnected { mac }
        })
    }

    pub fn is_connected(&self) -> bool {
        self.connected.is_some()
    }
}

// ── "the keyboard says it is switching off" (#190) ─────────────────────────

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a switch-off announcement explains the disconnection that follows.
pub const OFF_WINDOW: Duration = Duration::from_secs(15);

/// Pure holder of the announcement (no global state: testable with instants).
#[derive(Debug, Default)]
pub struct OffMark(Option<Instant>);

impl OffMark {
    pub const fn new() -> Self {
        Self(None)
    }
    /// The keyboard sent `0x13` with bit 1 = 0 at `now`.
    pub fn mark(&mut self, now: Instant) {
        self.0 = Some(now);
    }
    /// Consume the announcement: true if it is at most [`OFF_WINDOW`] old.
    pub fn take(&mut self, now: Instant) -> bool {
        self.0
            .take()
            .is_some_and(|t| now.saturating_duration_since(t) <= OFF_WINDOW)
    }
    pub fn clear(&mut self) {
        self.0 = None;
    }
}

static OFF: Mutex<OffMark> = Mutex::new(OffMark::new());

fn off() -> std::sync::MutexGuard<'static, OffMark> {
    OFF.lock().unwrap_or_else(|e| e.into_inner())
}

/// Called by the passive listener when `0x13` arrives with bit 1 = 0.
pub fn mark_keyboard_off() {
    off().mark(Instant::now());
}

/// Called when the link drops: was a switch-off announced just before?
pub fn take_keyboard_off() -> bool {
    off().take(Instant::now())
}

/// Called when the keyboard is back (new node, `0x13` with bit 1 = 1).
pub fn clear_keyboard_off() {
    off().clear();
}

// ── "a forget is in progress": Apple's SuppressDisconnectNotifications ─────

static EXPECTED: Mutex<OffMark> = Mutex::new(OffMark::new());

fn expected() -> std::sync::MutexGuard<'static, OffMark> {
    EXPECTED.lock().unwrap_or_else(|e| e.into_inner())
}

/// `akmctl repair` just sent `RecantConnection` (`0x41`): the disconnection
/// that follows within [`OFF_WINDOW`] is expected and not notified (macOS
/// `recantConnection` sets `SuppressDisconnectNotifications`, #217).
pub fn mark_expected_disconnect() {
    expected().mark(Instant::now());
}

/// Called when the link drops: is this the expected disconnection?
pub fn take_expected_disconnect() -> bool {
    expected().take(Instant::now())
}

/// Notification text `(summary, body)`.
pub fn text(ev: &LinkEvent) -> (String, String) {
    match ev {
        LinkEvent::PoweredOff { .. } => (
            "Apple Keyboard \u{2014} switched off".into(),
            "The keyboard was switched off (not a lost connection)".into(),
        ),
        LinkEvent::Disconnected { .. } => (
            "Apple Keyboard \u{2014} disconnected".into(),
            "The keyboard is no longer connected".into(),
        ),
        LinkEvent::Reconnected { pct, .. } => (
            "Apple Keyboard \u{2014} reconnected".into(),
            match pct {
                Some(p) => format!("Reconnected ({p:.0}%)"),
                None => "Reconnected".into(),
            },
        ),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn expected_disconnect_is_consumed_once() {
        assert!(!take_expected_disconnect());
        mark_expected_disconnect();
        assert!(take_expected_disconnect());
        assert!(!take_expected_disconnect(), "one disconnection only");
    }

    use super::*;

    #[test]
    fn announced_switch_off_makes_a_powered_off_event_not_a_disconnection() {
        let mut t = LinkTracker::new();
        t.acquired(MAC, Some(50.0));
        assert_eq!(t.disconnected_as(true), Some(LinkEvent::PoweredOff { mac: MAC.into() }));
        // reconnection is tracked as after any loss
        assert!(matches!(t.acquired(MAC, Some(50.0)), Some(LinkEvent::Reconnected { .. })));
        t.disconnected_as(false);
        let mut t = LinkTracker::new();
        t.acquired(MAC, None);
        assert_eq!(t.disconnected(), Some(LinkEvent::Disconnected { mac: MAC.into() }));
        let (s, b) = text(&LinkEvent::PoweredOff { mac: MAC.into() });
        assert!(s.contains("switched off") && b.contains("not a lost connection"));
    }

    #[test]
    fn off_mark_is_consumed_once_and_expires() {
        let t0 = Instant::now();
        let mut m = OffMark::new();
        assert!(!m.take(t0), "nothing announced");
        m.mark(t0);
        assert!(m.take(t0 + Duration::from_secs(3)));
        assert!(!m.take(t0 + Duration::from_secs(3)), "consumed");
        m.mark(t0);
        assert!(!m.take(t0 + OFF_WINDOW + Duration::from_secs(1)), "too old");
        m.mark(t0);
        m.clear();
        assert!(!m.take(t0));
    }

    const MAC: &str = "04:DB:56:CA:42:EE";

    #[test]
    fn startup_is_silent_then_disconnect_and_reconnect_notify() {
        let mut t = LinkTracker::new();
        assert_eq!(t.acquired(MAC, Some(90.0)), None, "session start");
        assert_eq!(t.acquired(MAC, Some(89.0)), None, "periodic reads");
        assert_eq!(
            t.disconnected(),
            Some(LinkEvent::Disconnected { mac: MAC.into() })
        );
        assert_eq!(t.disconnected(), None, "only once");
        let r = t.acquired(MAC, Some(88.4)).unwrap();
        assert_eq!(text(&r).1, "Reconnected (88%)");
        assert!(t.is_connected());
        assert_eq!(t.acquired(MAC, Some(88.0)), None);
    }

    #[test]
    fn disconnection_before_any_read_is_silent() {
        let mut t = LinkTracker::new();
        assert_eq!(t.disconnected(), None);
        assert_eq!(t.acquired(MAC, None), None);
    }

    #[test]
    fn texts() {
        let (s, b) = text(&LinkEvent::Disconnected { mac: MAC.into() });
        assert!(s.contains("disconnected") && !b.is_empty());
        let (_, b) = text(&LinkEvent::Reconnected {
            mac: MAC.into(),
            pct: None,
        });
        assert_eq!(b, "Reconnected");
    }
}
