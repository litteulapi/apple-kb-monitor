//! Published state of the keyboard and the *watch* that carries it
//! (docs/REVUE-ARCHITECTURE-GLOBALE.md §4.2): one writer (the acquisition
//! thread) replaces an immutable [`Snapshot`] and bumps a version number;
//! readers (D-Bus, tray, UI) take a clone and never hold the lock during I/O.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::report::KbReport;

/// Version of the JSON schema of [`Snapshot`] (D-Bus `Json` property, `--json`).
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    /// [`SCHEMA_VERSION`].
    pub schema: u32,
    /// Publication counter (monotonic within one publisher).
    pub version: u64,
    /// BlueZ says the keyboard is connected *and* it was acquired.
    pub connected: bool,
    pub keyboard: Option<KbReport>,
    /// Why there is no keyboard data (user-facing).
    pub kb_error: Option<String>,
    pub caps_lock: bool,
    pub num_lock: bool,
    pub remaining_display: Option<String>,
    /// Unix time of the RSSI measurement shown (None = no fresh value).
    /// A time, not an age, so the snapshot does not change every second.
    pub rssi_at: Option<u64>,
    /// Unix time of the last successful acquisition (0 = never).
    pub last_update: u64,
    /// Last internal error (acquisition, BlueZ provider...), if any.
    pub last_error: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            version: 0,
            connected: false,
            keyboard: None,
            kb_error: None,
            caps_lock: false,
            num_lock: false,
            remaining_display: None,
            rssi_at: None,
            last_update: 0,
            last_error: None,
        }
    }
}

impl Snapshot {
    pub fn battery_pct(&self) -> Option<f64> {
        self.keyboard.as_ref().and_then(KbReport::battery_pct)
    }
    pub fn voltage(&self) -> Option<f64> {
        self.keyboard.as_ref().and_then(|k| k.battery.voltage)
    }
    pub fn rssi(&self) -> Option<i32> {
        self.keyboard.as_ref().and_then(|k| k.radio.rssi_dbm)
    }
    pub fn model(&self) -> Option<&str> {
        self.keyboard
            .as_ref()
            .and_then(|k| k.device.model.as_deref())
    }
    pub fn mac(&self) -> Option<&str> {
        self.keyboard.as_ref().and_then(|k| k.device.mac.as_deref())
    }

    /// Age of the RSSI value at unix time `now`.
    pub fn rssi_age_s(&self, now: u64) -> Option<u64> {
        self.rssi_at.map(|t| now.saturating_sub(t))
    }

    /// Tray tooltip; "n/a" instead of an invented 0 when the source is absent.
    pub fn tooltip_text(&self) -> String {
        format!(
            "Apple Keyboard \u{2014} Battery: {}",
            self.battery_pct()
                .map(|p| format!("{:.0}%", p))
                .unwrap_or_else(|| "n/a".into())
        )
    }

    /// Same content, ignoring the publication counter.
    pub fn same_content(&self, other: &Snapshot) -> bool {
        Snapshot {
            version: 0,
            ..self.clone()
        } == Snapshot {
            version: 0,
            ..other.clone()
        }
    }
}

/// Latest snapshot + version, with change notification.
#[derive(Debug, Default)]
pub struct Watch {
    inner: Mutex<Snapshot>,
    cv: Condvar,
}

impl Watch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the snapshot if its content changed; returns the current version.
    pub fn publish(&self, mut s: Snapshot) -> u64 {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.same_content(&s) {
            return g.version;
        }
        s.version = g.version.wrapping_add(1);
        s.schema = SCHEMA_VERSION;
        *g = s;
        self.cv.notify_all();
        g.version
    }

    /// Clone of the current snapshot.
    pub fn get(&self) -> Snapshot {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn version(&self) -> u64 {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).version
    }

    /// Block until the version differs from `seen` or `timeout` elapses.
    /// Returns the snapshot if it changed.
    pub fn wait_newer(&self, seen: u64, timeout: Duration) -> Option<Snapshot> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let (g, _) = self
            .cv
            .wait_timeout_while(g, timeout, |s| s.version == seen)
            .unwrap_or_else(|e| e.into_inner());
        (g.version != seen).then(|| g.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn with_battery(p: f64) -> Snapshot {
        let mut k = KbReport::default();
        k.battery.percentage = Some(p);
        Snapshot {
            connected: true,
            keyboard: Some(k),
            ..Default::default()
        }
    }

    #[test]
    fn tooltip_shows_na_without_sources() {
        let t = Snapshot::default().tooltip_text();
        assert!(t.contains("n/a"));
        assert!(!t.contains("0%"));
        assert!(with_battery(90.0).tooltip_text().contains("90%"));
    }

    #[test]
    fn rssi_age_is_derived_from_its_timestamp() {
        let s = Snapshot {
            rssi_at: Some(1_000),
            ..Default::default()
        };
        assert_eq!(s.rssi_age_s(1_030), Some(30));
        assert_eq!(s.rssi_age_s(900), Some(0));
        assert_eq!(Snapshot::default().rssi_age_s(5), None);
    }

    #[test]
    fn publish_bumps_version_only_on_change() {
        let w = Watch::new();
        assert_eq!(w.version(), 0);
        assert_eq!(w.publish(with_battery(90.0)), 1);
        assert_eq!(w.publish(with_battery(90.0)), 1);
        assert_eq!(w.publish(with_battery(89.0)), 2);
        assert_eq!(w.get().battery_pct(), Some(89.0));
        assert_eq!(w.get().schema, SCHEMA_VERSION);
    }

    #[test]
    fn wait_newer_wakes_on_publish_and_times_out() {
        let w = Arc::new(Watch::new());
        assert!(w.wait_newer(0, Duration::from_millis(20)).is_none());
        let w2 = w.clone();
        let h = std::thread::spawn(move || w2.wait_newer(0, Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(50));
        w.publish(with_battery(50.0));
        let got = h.join().unwrap().expect("woken by publish");
        assert_eq!(got.version, 1);
    }

    #[test]
    fn json_schema_is_stable() {
        let s = with_battery(90.0);
        let j = serde_json::to_value(&s).unwrap();
        for k in [
            "schema",
            "version",
            "connected",
            "keyboard",
            "last_update",
            "last_error",
        ] {
            assert!(j.get(k).is_some(), "{k}");
        }
        let back: Snapshot = serde_json::from_value(j).unwrap();
        assert_eq!(back, s);
        // Old / partial payloads still decode.
        let p: Snapshot = serde_json::from_str(r#"{"connected":true}"#).unwrap();
        assert!(p.connected && p.keyboard.is_none());
    }
}
