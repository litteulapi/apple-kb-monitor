//! Connection / disconnection / reconnection notifications (F03).

/// What to tell the user.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    Disconnected {
        mac: String,
    },
    /// The keyboard was switched off: not a lost link, no "disconnected" alert.
    PoweredOff {
        mac: String,
    },
    Reconnected {
        mac: String,
        pct: Option<f64>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct LinkTracker {
    connected: Option<String>,
    lost: Option<String>,
}

impl LinkTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A read succeeded. Returns `Reconnected` only after a disconnection seen during this run.
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

    /// `BlueZ` reported the keyboard disconnected.
    pub fn disconnected(&mut self) -> Option<LinkEvent> {
        self.disconnected_as(false)
    }

    /// Same, knowing whether the keyboard announced its own switch-off.
    pub fn disconnected_as(&mut self, powered_off: bool) -> Option<LinkEvent> {
        let mac = self.connected.take()?;
        self.lost = Some(mac.clone());
        Some(if powered_off {
            LinkEvent::PoweredOff { mac }
        } else {
            LinkEvent::Disconnected { mac }
        })
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.is_some()
    }
}

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a switch-off announcement explains the disconnection that follows.
pub const OFF_WINDOW: Duration = Duration::from_secs(15);

/// Pure holder of the announcement (no global state: testable with instants).
#[derive(Debug, Default)]
pub struct OffMark(Option<Instant>);

impl OffMark {
    #[must_use]
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
    OFF.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Called by the passive listener when `0x13` arrives with bit 1 = 0.
pub fn mark_keyboard_off() {
    off().mark(Instant::now());
}

/// Called when the link drops: was a switch-off announced just before?
#[must_use]
pub fn take_keyboard_off() -> bool {
    let now = Instant::now();
    let taken = off().take(now);
    if taken {
        *recent(&OFF_TAKEN) = Some(now);
    }
    taken
}

/// How long after it explained a disconnection an announcement still tells the other observers of
/// that same disconnection why it happened.
pub const RECENT_WINDOW: Duration = Duration::from_secs(5);

static OFF_TAKEN: Mutex<Option<Instant>> = Mutex::new(None);
static EXPECTED_TAKEN: Mutex<Option<Instant>> = Mutex::new(None);

fn recent(m: &Mutex<Option<Instant>>) -> std::sync::MutexGuard<'_, Option<Instant>> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn is_recent(mark: &OffMark, taken: &Mutex<Option<Instant>>, now: Instant) -> bool {
    mark.0
        .is_some_and(|t| now.saturating_duration_since(t) <= OFF_WINDOW)
        || recent(taken).is_some_and(|t| now.saturating_duration_since(t) <= RECENT_WINDOW)
}

/// Was a switch-off announced for the disconnection happening now?
#[must_use]
pub fn keyboard_off_recent() -> bool {
    is_recent(&off(), &OFF_TAKEN, Instant::now())
}

/// Same for the disconnection `akmctl repair` announced.
#[must_use]
pub fn expected_disconnect_recent() -> bool {
    is_recent(&expected(), &EXPECTED_TAKEN, Instant::now())
}

/// Called when the keyboard is back (new node, `0x13` with bit 1 = 1).
pub fn clear_keyboard_off() {
    off().clear();
}

static EXPECTED: Mutex<OffMark> = Mutex::new(OffMark::new());

fn expected() -> std::sync::MutexGuard<'static, OffMark> {
    EXPECTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `akmctl repair` just sent `RecantConnection`.
pub fn mark_expected_disconnect() {
    expected().mark(Instant::now());
}

/// Called when the link drops: is this the expected disconnection?
#[must_use]
pub fn take_expected_disconnect() -> bool {
    let now = Instant::now();
    let taken = expected().take(now);
    if taken {
        *recent(&EXPECTED_TAKEN) = Some(now);
    }
    taken
}

/// Notification text `(summary, body)`.
#[must_use]
pub fn text(ev: &LinkEvent) -> (String, String) {
    use crate::tr;
    match ev {
        LinkEvent::PoweredOff { .. } => (
            tr!("Apple Keyboard \u{2014} switched off"),
            tr!("The keyboard was switched off (not a lost connection)"),
        ),
        LinkEvent::Disconnected { .. } => (
            tr!("Apple Keyboard \u{2014} disconnected"),
            tr!("The keyboard is no longer connected"),
        ),
        LinkEvent::Reconnected { pct, .. } => (
            tr!("Apple Keyboard \u{2014} reconnected"),
            match pct {
                Some(p) => tr!("Reconnected ({pct}%)", pct = format!("{p:.0}")),
                None => tr!("Reconnected"),
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
        assert_eq!(
            t.disconnected_as(true),
            Some(LinkEvent::PoweredOff { mac: MAC.into() })
        );
        assert!(matches!(
            t.acquired(MAC, Some(50.0)),
            Some(LinkEvent::Reconnected { .. })
        ));
        t.disconnected_as(false);
        let mut t = LinkTracker::new();
        t.acquired(MAC, None);
        assert_eq!(
            t.disconnected(),
            Some(LinkEvent::Disconnected { mac: MAC.into() })
        );
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

    const MAC: &str = "AA:BB:CC:DD:EE:F1";

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
