//! Notifications put aside to be shown later, kept across restarts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// "Remind me tomorrow": this long after the button was pressed.
pub const REMIND_AFTER_S: u64 = 24 * 3600;
/// Entries kept; beyond, the one due first is dropped.
pub const MAX_DEFERRED: usize = 32;

/// Why a notification waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    /// Held by the quiet hours.
    Quiet,
    /// The user pressed "Remind me tomorrow".
    Remind,
}

/// A notification as it will be shown again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    /// `KNotification` event id (`BatteryReminder`...).
    pub event: String,
    pub summary: String,
    /// Body, already escaped for the notification server.
    pub body: String,
    pub icon: String,
    /// freedesktop urgency hint (0 low, 1 normal, 2 critical).
    pub urgency: u8,
    #[serde(default)]
    pub transient: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deferred {
    /// Unix time at which it is shown.
    pub due: u64,
    pub reason: Reason,
    /// One entry per `(reason, slot)`.
    pub slot: String,
    pub notification: Stored,
}

/// The waiting notifications.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredStore {
    #[serde(default)]
    pub items: Vec<Deferred>,
}

impl DeferredStore {
    /// `$XDG_STATE_HOME/apple-kb-monitor/deferred-notifications.json`.
    #[must_use]
    pub fn default_path() -> PathBuf {
        crate::history::default_path().with_file_name("deferred-notifications.json")
    }

    /// An unreadable or corrupt file is an empty store.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let mut s: Self = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        s.items.truncate(MAX_DEFERRED);
        s
    }

    /// Atomic write (temporary file + rename), mode 0600.
    ///
    /// # Errors
    ///
    /// Any I/O error while creating, writing or renaming the file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::fsutil::write_atomic(path, json.as_bytes(), crate::fsutil::Mode::Fixed(0o600))
    }

    /// Put `notification` aside until `due`.
    pub fn defer(&mut self, reason: Reason, slot: &str, notification: Stored, due: u64) {
        self.items
            .retain(|d| !(d.reason == reason && d.slot == slot));
        self.items.push(Deferred {
            due,
            reason,
            slot: slot.to_string(),
            notification,
        });
        while self.items.len() > MAX_DEFERRED {
            // Too many: the one due first goes (it is the oldest request).
            if let Some(i) = self
                .items
                .iter()
                .enumerate()
                .min_by_key(|(_, d)| d.due)
                .map(|(i, _)| i)
            {
                self.items.remove(i);
            }
        }
    }

    /// "Remind me tomorrow" pressed at `now`: due [`REMIND_AFTER_S`] later.
    pub fn remind_tomorrow(&mut self, slot: &str, notification: Stored, now: u64) -> u64 {
        let due = now.saturating_add(REMIND_AFTER_S);
        self.defer(Reason::Remind, slot, notification, due);
        due
    }

    /// Remove and return what is due at `now`, the one due first first.
    pub fn take_due(&mut self, now: u64) -> Vec<Deferred> {
        let (mut due, rest): (Vec<Deferred>, Vec<Deferred>) =
            self.items.drain(..).partition(|d| d.due <= now);
        self.items = rest;
        due.sort_by_key(|d| d.due);
        due
    }

    /// Forget what waits in `slot` (new batteries end a battery reminder).
    pub fn cancel_slot(&mut self, slot: &str) -> usize {
        let before = self.items.len();
        self.items.retain(|d| d.slot != slot);
        before - self.items.len()
    }

    #[must_use]
    pub fn next_due(&self) -> Option<u64> {
        self.items.iter().map(|d| d.due).min()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_790_000_000;

    fn n(summary: &str) -> Stored {
        Stored {
            event: "BatteryReminder".into(),
            summary: summary.into(),
            body: "Tension 2500\u{202f}mV".into(),
            icon: "battery-caution".into(),
            urgency: 1,
            transient: false,
        }
    }

    #[test]
    fn a_file_with_the_old_language_flag_still_loads() {
        let old = r#"{"event":"BatteryReminder","summary":"batteries","body":"Tension 2500\u202fmV","icon":"battery-caution","urgency":1,"transient":false,"french":true}"#;
        let got: Stored = serde_json::from_str(old).unwrap();
        assert_eq!(got, n("batteries"));
    }

    #[test]
    fn remind_me_tomorrow_is_due_exactly_24_hours_later() {
        let mut s = DeferredStore::default();
        assert_eq!(
            s.remind_tomorrow("battery", n("batteries"), T0),
            T0 + 86_400
        );
        assert_eq!(s.next_due(), Some(T0 + 86_400));
        let got = s.take_due(T0);
        assert!(got.is_empty(), "{got:?}");
        assert!(
            s.take_due(T0 + 86_399).is_empty(),
            "one second early: nothing"
        );
        let due = s.take_due(T0 + 86_400);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].notification, n("batteries"));
        assert_eq!(due[0].reason, Reason::Remind);
        assert!(s.is_empty(), "shown once");
        assert!(
            s.take_due(T0 + 10 * 86_400).is_empty(),
            "{:?}",
            s.take_due(T0 + 10 * 86_400)
        );
    }

    #[test]
    fn a_second_press_replaces_the_first_and_slots_are_independent() {
        let mut s = DeferredStore::default();
        s.remind_tomorrow("battery", n("a"), T0);
        s.remind_tomorrow("battery", n("b"), T0 + 3600);
        s.remind_tomorrow("firmware", n("fw"), T0);
        s.defer(Reason::Quiet, "battery", n("held"), T0 + 600);
        assert_eq!(s.len(), 3);
        let due = s.take_due(T0 + 86_400);
        assert_eq!(
            due.iter()
                .map(|d| d.notification.summary.as_str())
                .collect::<Vec<_>>(),
            ["held", "fw"],
            "the first press was replaced; order = due time"
        );
        assert_eq!(s.take_due(T0 + 3600 + 86_400)[0].notification.summary, "b");
    }

    #[test]
    fn new_batteries_cancel_what_waits_in_the_battery_slot() {
        let mut s = DeferredStore::default();
        s.remind_tomorrow("battery", n("a"), T0);
        s.defer(Reason::Quiet, "battery", n("b"), T0 + 10);
        s.remind_tomorrow("firmware", n("fw"), T0);
        assert_eq!(s.cancel_slot("battery"), 2);
        assert_eq!(s.len(), 1);
        assert_eq!(s.cancel_slot("battery"), 0);
    }

    #[test]
    fn the_store_survives_a_restart_and_is_bounded() {
        let dir = std::env::temp_dir().join(format!("akm-deferred-{}", std::process::id()));
        let path = dir.join("deferred-notifications.json");
        let mut s = DeferredStore::default();
        s.remind_tomorrow("battery", n("batteries"), T0);
        s.save(&path).unwrap();
        let mut back = DeferredStore::load(&path);
        assert_eq!(back, s);
        assert_eq!(back.take_due(T0 + 86_400).len(), 1, "due after the restart");
        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&str> = v["items"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["due", "notification", "reason", "slot"]);
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(DeferredStore::load(&path), DeferredStore::default());
        let mut s = DeferredStore::default();
        for i in 0..(MAX_DEFERRED as u64 + 5) {
            s.defer(Reason::Quiet, &format!("slot{i}"), n("x"), T0 + i);
        }
        assert_eq!(s.len(), MAX_DEFERRED);
        assert_eq!(s.next_due(), Some(T0 + 5));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
