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
        let mac = self.connected.take()?;
        self.lost = Some(mac.clone());
        Some(LinkEvent::Disconnected { mac })
    }

    pub fn is_connected(&self) -> bool {
        self.connected.is_some()
    }
}

/// Notification text `(summary, body)`.
pub fn text(ev: &LinkEvent) -> (String, String) {
    match ev {
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
    use super::*;

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
