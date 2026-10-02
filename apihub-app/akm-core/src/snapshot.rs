//! Published state of the keyboard and the *watch* that carries it
//! (docs/REVUE-ARCHITECTURE-GLOBALE.md §4.2): one writer (the acquisition
//! thread) replaces an immutable [`Snapshot`] and bumps a version number;
//! readers (D-Bus, tray, UI) take a clone and never hold the lock during I/O.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::forecast::{format_days, Forecast};
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
    /// Autonomy forecast from the history (#83); None = unavailable.
    pub forecast: Option<Forecast>,
    /// Unix time the current batteries were installed (#85), if detected.
    pub batteries_installed_at: Option<u64>,
    /// "Batteries changed too often": the last two sets lasted less than 30
    /// days each (#108); None = nothing to say.
    pub battery_advice: Option<crate::advice::ShortLife>,
    /// Disconnection counts, 7 days of relative signal and the "unstable
    /// link" state of the keyboard followed (#105); None = nothing recorded.
    pub link_quality: Option<crate::linkstats::LinkQuality>,
    /// Active minutes per day, from the mere presence of input reports
    /// (#109); None = statistics disabled (the default).
    pub usage: Option<crate::usage::UsageSummary>,
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
            forecast: None,
            batteries_installed_at: None,
            battery_advice: None,
            link_quality: None,
            usage: None,
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
    /// Relative BR/EDR RSSI in dB (0 = ideal range), not dBm (#174).
    /// Firmware block of the keyboard report (#227).
    pub fn firmware(&self) -> Option<&crate::report::KbFirmware> {
        self.keyboard.as_ref().map(|k| &k.firmware)
    }

    pub fn rssi(&self) -> Option<i32> {
        self.keyboard.as_ref().and_then(|k| k.radio.rel_db())
    }
    pub fn model(&self) -> Option<&str> {
        self.keyboard
            .as_ref()
            .and_then(|k| k.device.model.as_deref())
    }
    pub fn mac(&self) -> Option<&str> {
        self.keyboard.as_ref().and_then(|k| k.device.mac.as_deref())
    }
    /// The user's alias (BlueZ `Alias`), if known and non-empty.
    pub fn alias(&self) -> Option<&str> {
        self.keyboard
            .as_ref()
            .and_then(|k| k.device.alias.as_deref())
            .filter(|a| !a.is_empty())
    }
    /// Name the kernel registered the input device under (`HID_NAME`), if
    /// known and non-empty. KWin and System Settings > Keyboard show this one.
    pub fn kernel_name(&self) -> Option<&str> {
        self.keyboard
            .as_ref()
            .and_then(|k| k.device.name.as_deref())
            .filter(|n| !n.is_empty())
    }
    /// Name to show for the keyboard: the user's alias, else the name the
    /// keyboard registered under (empty strings count as absent).
    pub fn display_name(&self) -> Option<&str> {
        let d = &self.keyboard.as_ref()?.device;
        [d.alias.as_deref(), d.name.as_deref()]
            .into_iter()
            .flatten()
            .find(|n| !n.is_empty())
    }

    /// Age of the last successful acquisition at unix time `now`, `None` if
    /// there was none. The kernel percentage steps down only at reconnections
    /// (docs/VERIF-BATTERIE.md §1.2bis): this age is what tells how stale the
    /// shown indication can be (#179).
    pub fn update_age_s(&self, now: u64) -> Option<u64> {
        (self.last_update > 0).then(|| now.saturating_sub(self.last_update))
    }

    /// Age of the RSSI value at unix time `now`.
    pub fn rssi_age_s(&self, now: u64) -> Option<u64> {
        self.rssi_at.map(|t| now.saturating_sub(t))
    }

    /// Seconds of autonomy left at unix time `now` (None = unavailable).
    pub fn remaining_s(&self, now: u64) -> Option<u64> {
        self.forecast.as_ref().map(|f| f.remaining_s(now))
    }

    /// Tray tooltip; "n/a" instead of an invented 0 when the source is absent.
    pub fn tooltip_text(&self) -> String {
        let now = crate::history::Clock::now(&crate::history::SystemClock);
        self.tooltip_text_at(now)
    }

    /// [`Self::tooltip_text`] at a given unix time (testable).
    pub fn tooltip_text_at(&self, now: u64) -> String {
        let mut t = format!(
            "{} \u{2014} Battery: {}",
            self.display_name().unwrap_or("Apple Keyboard"),
            self.battery_pct()
                .map(|p| format!("{:.0}%", p))
                .unwrap_or_else(|| "n/a".into())
        );
        if let (Some(_), Some(r)) = (self.battery_pct(), self.remaining_s(now)) {
            t.push_str(" \u{2014} ");
            t.push_str(&format_days(r));
        }
        t
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
    fn display_name_prefers_alias_then_hid_name() {
        assert_eq!(Snapshot::default().display_name(), None);
        let mut s = with_battery(90.0);
        assert_eq!(s.display_name(), None);
        assert!(s.tooltip_text().starts_with("Apple Keyboard"));
        let d = &mut s.keyboard.as_mut().unwrap().device;
        d.name = Some("Clavier HID".into());
        assert_eq!(s.display_name(), Some("Clavier HID"));
        let d = &mut s.keyboard.as_mut().unwrap().device;
        d.alias = Some("Bureau".into());
        assert_eq!(s.display_name(), Some("Bureau"));
        assert!(s.tooltip_text().starts_with("Bureau \u{2014} Battery: 90%"));
        let d = &mut s.keyboard.as_mut().unwrap().device;
        d.alias = Some(String::new());
        assert_eq!(s.display_name(), Some("Clavier HID"));
    }

    #[test]
    fn tooltip_shows_the_forecast_when_known() {
        let mut s = with_battery(41.0);
        s.forecast = Some(Forecast {
            rate_pct_per_day: 1.0,
            empty_at: 1_000 + 41 * 86_400,
            fitted_pct: 41.0,
            span_s: 0,
            buckets: 0,
        });
        assert_eq!(s.remaining_s(1_000), Some(41 * 86_400));
        assert_eq!(
            s.tooltip_text_at(1_000),
            "Apple Keyboard \u{2014} Battery: 41% \u{2014} \u{2248} 41 days left"
        );
        assert_eq!(Snapshot::default().remaining_s(0), None);
    }

    #[test]
    fn update_age_is_derived_from_last_update() {
        let s = Snapshot {
            last_update: 1_000,
            ..Snapshot::default()
        };
        assert_eq!(s.update_age_s(1_090), Some(90));
        assert_eq!(s.update_age_s(900), Some(0));
        assert_eq!(Snapshot::default().update_age_s(5), None);
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
