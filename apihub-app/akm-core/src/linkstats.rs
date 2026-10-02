//! Quality of the Bluetooth link over time (#105): how often the keyboard
//! disconnects, and how the relative signal evolved over 7 days.
//!
//! * **Disconnections**: every one is recorded with its time and its reason
//!   (`timeout`, `remote`, `local`, `unknown`, `authentication`, `suspend`,
//!   `user`, `off`...), kept [`KEEP_S`] (7 days), at most [`MAX_EVENTS`] per
//!   keyboard. Counts are given per hour (the last 24) and per day (the last
//!   7), in windows that end at the time of the question.
//! * **Unstable link**: more than [`UNSTABLE_PER_HOUR`] disconnections within
//!   one hour. Disconnections somebody asked for or that are part of normal
//!   life do not count ([`counts`]): system suspend, "Disconnect" from the
//!   tray, the keyboard switched off with its button. The alert is due once
//!   per episode ([`LinkStats::unstable_due`]); the episode ends when the last
//!   hour is back to [`UNSTABLE_PER_HOUR`] or fewer.
//! * **Signal**: the relative BR/EDR value in dB (0 = ideal range, #174),
//!   aggregated per hour (count, mean, min, max) for 7 days: 168 buckets per
//!   keyboard at most.
//!
//! Pure state with an explicit clock (unix seconds), saved as a small JSON.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What is kept: 7 days.
pub const KEEP_S: u64 = 7 * 86_400;
/// Disconnections kept per keyboard (the oldest go first).
pub const MAX_EVENTS: usize = 2000;
/// The link is unstable with MORE than this many disconnections in an hour.
pub const UNSTABLE_PER_HOUR: usize = 3;
/// Keyboards followed (the least recently seen goes first).
pub const MAX_DEVICES: usize = 8;
const HOUR: u64 = 3600;

/// Does a disconnection for `reason` count towards the unstable-link alert?
/// Not when it was asked for (`user`, `expected`), when the system went to
/// sleep (`suspend`) or when the keyboard was switched off (`off`).
pub fn counts(reason: &str) -> bool {
    !matches!(reason, "suspend" | "user" | "expected" | "off")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disconnect {
    pub ts: u64,
    pub reason: String,
}

/// One hour of signal measurements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RssiHour {
    /// Start of the hour, unix seconds.
    pub start: u64,
    pub samples: u32,
    sum: i64,
    pub min: i32,
    pub max: i32,
}

impl RssiHour {
    pub fn mean(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            (self.sum as f64 / f64::from(self.samples) * 10.0).round() / 10.0
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStats {
    #[serde(default)]
    pub disconnects: Vec<Disconnect>,
    #[serde(default)]
    pub rssi: Vec<RssiHour>,
    /// Start of the unstable episode already notified (None = stable).
    #[serde(default)]
    pub unstable_since: Option<u64>,
}

/// Signal summary of a period.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RssiSummary {
    pub samples: u32,
    pub mean: f64,
    pub min: i32,
    pub max: i32,
}

/// One day of signal (a 24-hour window, oldest first in the summary).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RssiDay {
    /// Start of the window, unix seconds.
    pub start: u64,
    /// None: no measurement in this window.
    pub signal: Option<RssiSummary>,
}

/// What `GetState` carries (`link_quality`) for the keyboard followed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkQuality {
    /// Disconnections of every kind in the last hour / day / 7 days.
    pub disconnects_last_hour: u32,
    pub disconnects_last_day: u32,
    pub disconnects_7d: u32,
    /// Those of the last hour that count towards the alert.
    pub unexpected_last_hour: u32,
    /// The last 24 hours, one count per hour, oldest first.
    pub disconnects_by_hour: Vec<u32>,
    /// The last 7 days, one count per 24 h, oldest first.
    pub disconnects_by_day: Vec<u32>,
    /// More than [`UNSTABLE_PER_HOUR`] unexpected disconnections in the last hour.
    pub unstable: bool,
    /// Unix time the unstable episode was first reported.
    pub unstable_since: Option<u64>,
    /// Relative signal (dB, 0 = ideal) over 7 days; None without measurement.
    pub signal_7d: Option<RssiSummary>,
    /// The last 7 days of signal, oldest first.
    pub signal_by_day: Vec<RssiDay>,
}

/// Link statistics of every keyboard (MAC upper case). Persisted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkStats {
    #[serde(default)]
    pub devices: BTreeMap<String, DeviceStats>,
}

fn summarize<'a>(hours: impl Iterator<Item = &'a RssiHour>) -> Option<RssiSummary> {
    let (mut n, mut sum, mut min, mut max) = (0u32, 0i64, i32::MAX, i32::MIN);
    for h in hours.filter(|h| h.samples > 0) {
        n += h.samples;
        sum += h.sum;
        min = min.min(h.min);
        max = max.max(h.max);
    }
    (n > 0).then(|| RssiSummary {
        samples: n,
        mean: (sum as f64 / f64::from(n) * 10.0).round() / 10.0,
        min,
        max,
    })
}

impl DeviceStats {
    fn prune(&mut self, now: u64) {
        let cutoff = now.saturating_sub(KEEP_S);
        self.disconnects.retain(|d| d.ts >= cutoff && d.ts <= now);
        if self.disconnects.len() > MAX_EVENTS {
            let extra = self.disconnects.len() - MAX_EVENTS;
            self.disconnects.drain(..extra);
        }
        self.rssi
            .retain(|h| h.start + HOUR > cutoff && h.start <= now);
    }

    fn in_window(&self, from: u64, to: u64) -> impl Iterator<Item = &Disconnect> {
        // `from` excluded, `to` included: a window "ending now".
        self.disconnects
            .iter()
            .filter(move |d| d.ts > from && d.ts <= to)
    }

    fn unexpected_last_hour(&self, now: u64) -> usize {
        self.in_window(now.saturating_sub(HOUR), now)
            .filter(|d| counts(&d.reason))
            .count()
    }

    pub fn quality(&self, now: u64) -> LinkQuality {
        let count = |from: u64, to: u64| self.in_window(from, to).count() as u32;
        let back = |n: u64, unit: u64| now.saturating_sub(n * unit);
        let unexpected = self.unexpected_last_hour(now);
        LinkQuality {
            disconnects_last_hour: count(back(1, HOUR), now),
            disconnects_last_day: count(back(24, HOUR), now),
            disconnects_7d: count(back(7, 86_400), now),
            unexpected_last_hour: unexpected as u32,
            disconnects_by_hour: (0..24u64)
                .rev()
                .map(|i| count(back(i + 1, HOUR), back(i, HOUR)))
                .collect(),
            disconnects_by_day: (0..7u64)
                .rev()
                .map(|i| count(back(i + 1, 86_400), back(i, 86_400)))
                .collect(),
            unstable: unexpected > UNSTABLE_PER_HOUR,
            unstable_since: self.unstable_since,
            signal_7d: summarize(
                self.rssi
                    .iter()
                    .filter(|h| h.start + HOUR > back(7, 86_400)),
            ),
            signal_by_day: (0..7u64)
                .rev()
                .map(|i| {
                    let (from, to) = (back(i + 1, 86_400), back(i, 86_400));
                    RssiDay {
                        start: from,
                        signal: summarize(
                            self.rssi.iter().filter(|h| h.start > from && h.start <= to),
                        ),
                    }
                })
                .collect(),
        }
    }
}

impl LinkStats {
    /// `$XDG_STATE_HOME/apple-kb-monitor/link-quality.json`.
    pub fn default_path() -> PathBuf {
        crate::history::default_path().with_file_name("link-quality.json")
    }

    /// An unreadable or corrupt file is an empty record.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Atomic write (temporary file + rename), mode 0600.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(
            serde_json::to_string(self)
                .map_err(std::io::Error::other)?
                .as_bytes(),
        )?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    }

    fn device(&mut self, mac: &str, now: u64) -> &mut DeviceStats {
        let key = mac.to_ascii_uppercase();
        if !self.devices.contains_key(&key) && self.devices.len() >= MAX_DEVICES {
            // Bounded: the keyboard heard of the longest ago makes room.
            let last_seen = |d: &DeviceStats| {
                d.disconnects
                    .last()
                    .map(|x| x.ts)
                    .max(d.rssi.last().map(|h| h.start))
                    .unwrap_or(0)
            };
            if let Some(old) = self
                .devices
                .iter()
                .min_by_key(|(_, d)| last_seen(d))
                .map(|(k, _)| k.clone())
            {
                self.devices.remove(&old);
            }
        }
        let d = self.devices.entry(key).or_default();
        d.prune(now);
        d
    }

    /// The keyboard `mac` disconnected at `now` for `reason`.
    pub fn record_disconnect(&mut self, mac: &str, now: u64, reason: &str) {
        let d = self.device(mac, now);
        d.disconnects.push(Disconnect {
            ts: now,
            reason: reason.to_string(),
        });
        d.prune(now);
    }

    /// The reason of the disconnection just recorded became known (BlueZ
    /// sends `Connected = false` and `Disconnected(reason)` in either order):
    /// an `unknown` recorded at most `within` seconds ago takes `reason`.
    /// Returns whether a record was refined.
    pub fn refine_last(&mut self, mac: &str, now: u64, reason: &str, within: u64) -> bool {
        let d = self.device(mac, now);
        match d.disconnects.last_mut() {
            Some(last) if last.reason == "unknown" && now.saturating_sub(last.ts) <= within => {
                last.reason = reason.to_string();
                true
            }
            _ => false,
        }
    }

    /// A signal measurement (relative dB) of `mac` at `now`.
    pub fn record_rssi(&mut self, mac: &str, now: u64, rel_db: i32) {
        let start = now - now % HOUR;
        let d = self.device(mac, now);
        match d.rssi.last_mut() {
            Some(h) if h.start == start => {
                h.samples += 1;
                h.sum += i64::from(rel_db);
                h.min = h.min.min(rel_db);
                h.max = h.max.max(rel_db);
            }
            _ => d.rssi.push(RssiHour {
                start,
                samples: 1,
                sum: i64::from(rel_db),
                min: rel_db,
                max: rel_db,
            }),
        }
    }

    /// Is the "unstable link" alert due for `mac` at `now`? `Some(n)` (the
    /// number of unexpected disconnections in the last hour) once per
    /// episode; the episode ends, and the alert is armed again, when the last
    /// hour is back to [`UNSTABLE_PER_HOUR`] or fewer.
    pub fn unstable_due(&mut self, mac: &str, now: u64) -> Option<usize> {
        let d = self.device(mac, now);
        let n = d.unexpected_last_hour(now);
        if n > UNSTABLE_PER_HOUR {
            if d.unstable_since.is_none() {
                d.unstable_since = Some(now);
                return Some(n);
            }
        } else {
            d.unstable_since = None;
        }
        None
    }

    /// Summary for `mac` at `now` (None: nothing known about it).
    pub fn quality(&self, mac: &str, now: u64) -> Option<LinkQuality> {
        self.devices
            .get(&mac.to_ascii_uppercase())
            .map(|d| d.quality(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: &str = "AA:BB:CC:DD:EE:F1";
    const T0: u64 = 1_790_000_000;
    const MIN: u64 = 60;

    #[test]
    fn counts_per_hour_and_per_day() {
        let mut s = LinkStats::default();
        // 2 today (one 30 min ago, one 5 h ago), 1 three days ago.
        s.record_disconnect(MAC, T0 - 3 * 86_400, "timeout");
        s.record_disconnect(MAC, T0 - 5 * HOUR + MIN, "remote");
        s.record_disconnect(MAC, T0 - 30 * MIN, "timeout");
        let q = s.quality(MAC, T0).unwrap();
        assert_eq!(
            (
                q.disconnects_last_hour,
                q.disconnects_last_day,
                q.disconnects_7d
            ),
            (1, 2, 3)
        );
        assert_eq!(q.disconnects_by_hour.len(), 24);
        assert_eq!(q.disconnects_by_hour[23], 1, "the last hour is last");
        assert_eq!(q.disconnects_by_hour[19], 1, "between 4 and 5 h ago");
        assert_eq!(q.disconnects_by_hour.iter().sum::<u32>(), 2);
        assert_eq!(q.disconnects_by_day, [0, 0, 0, 1, 0, 0, 2]);
        assert!(!q.unstable);
        assert_eq!(s.quality("AA:BB:CC:DD:EE:02", T0), None);
        // The MAC is not case sensitive.
        assert!(s.quality(&MAC.to_ascii_lowercase(), T0).is_some());
    }

    #[test]
    fn more_than_three_in_an_hour_is_unstable_once_per_episode() {
        let mut s = LinkStats::default();
        for i in 0..3u64 {
            s.record_disconnect(MAC, T0 + i * 10 * MIN, "timeout");
            assert_eq!(
                s.unstable_due(MAC, T0 + i * 10 * MIN),
                None,
                "{} is not more than 3",
                i + 1
            );
        }
        // The 4th within the hour: due, once.
        s.record_disconnect(MAC, T0 + 30 * MIN, "timeout");
        assert_eq!(s.unstable_due(MAC, T0 + 30 * MIN), Some(4));
        assert_eq!(s.unstable_due(MAC, T0 + 31 * MIN), None, "already told");
        s.record_disconnect(MAC, T0 + 40 * MIN, "unknown");
        assert_eq!(s.unstable_due(MAC, T0 + 40 * MIN), None, "same episode");
        let q = s.quality(MAC, T0 + 40 * MIN).unwrap();
        assert!(q.unstable);
        assert_eq!(q.unstable_since, Some(T0 + 30 * MIN));
        assert_eq!(q.unexpected_last_hour, 5);
        // Two quiet hours: the episode is over.
        assert_eq!(s.unstable_due(MAC, T0 + 3 * HOUR), None);
        assert!(!s.quality(MAC, T0 + 3 * HOUR).unwrap().unstable);
        assert_eq!(s.quality(MAC, T0 + 3 * HOUR).unwrap().unstable_since, None);
        // A new burst is a new episode: told again.
        for i in 0..4u64 {
            s.record_disconnect(MAC, T0 + 4 * HOUR + i * MIN, "timeout");
        }
        assert_eq!(s.unstable_due(MAC, T0 + 4 * HOUR + 3 * MIN), Some(4));
    }

    #[test]
    fn wanted_disconnections_do_not_make_the_link_unstable() {
        let mut s = LinkStats::default();
        for (i, reason) in ["suspend", "user", "off", "expected", "suspend", "user"]
            .iter()
            .enumerate()
        {
            s.record_disconnect(MAC, T0 + i as u64 * MIN, reason);
        }
        assert_eq!(s.unstable_due(MAC, T0 + 10 * MIN), None);
        let q = s.quality(MAC, T0 + 10 * MIN).unwrap();
        assert_eq!((q.disconnects_last_hour, q.unexpected_last_hour), (6, 0));
        assert!(!q.unstable);
        for r in ["timeout", "remote", "local", "unknown", "authentication"] {
            assert!(counts(r), "{r}");
        }
    }

    #[test]
    fn a_reason_known_just_after_refines_the_record() {
        let mut s = LinkStats::default();
        s.record_disconnect(MAC, T0, "unknown");
        assert!(s.refine_last(MAC, T0 + 1, "suspend", 5));
        assert_eq!(s.devices[MAC].disconnects[0].reason, "suspend");
        assert!(!s.refine_last(MAC, T0 + 2, "timeout", 5), "already known");
        s.record_disconnect(MAC, T0 + 100, "unknown");
        assert!(
            !s.refine_last(MAC, T0 + 200, "timeout", 5),
            "too late: another event"
        );
        assert_eq!(s.devices[MAC].disconnects.len(), 2);
    }

    #[test]
    fn the_signal_is_kept_seven_days_by_the_hour() {
        let mut s = LinkStats::default();
        let start = T0 - T0 % HOUR;
        // 8 days of one measurement every 20 minutes, getting worse.
        for i in 0..(8 * 72u64) {
            let t = start + i * 20 * MIN;
            s.record_rssi(MAC, t, -((i / 72) as i32));
        }
        let now = start + 8 * 86_400;
        s.record_rssi(MAC, now, -8);
        let d = &s.devices[MAC];
        assert!(
            d.rssi.len() <= 7 * 24 + 1,
            "{} hourly buckets",
            d.rssi.len()
        );
        assert!(d.rssi.iter().all(|h| h.start + HOUR > now - KEEP_S));
        assert_eq!(d.rssi[0].samples, 3, "3 measurements per hour");
        let q = s.quality(MAC, now).unwrap();
        let sig = q.signal_7d.unwrap();
        assert_eq!((sig.min, sig.max), (-8, -1), "day 0 (value 0) aged out");
        assert_eq!(q.signal_by_day.len(), 7);
        assert_eq!(q.signal_by_day[6].signal.as_ref().unwrap().min, -8);
        assert_eq!(q.signal_by_day[0].signal.as_ref().unwrap().max, -1);
        assert_eq!(d.rssi[0].mean(), -1.0);
        // No measurement: no invented value.
        let mut e = LinkStats::default();
        e.record_disconnect(MAC, T0, "timeout");
        let q = e.quality(MAC, T0).unwrap();
        assert_eq!(q.signal_7d, None);
        assert!(q.signal_by_day.iter().all(|d| d.signal.is_none()));
    }

    #[test]
    fn the_record_is_bounded_and_survives_a_restart() {
        let mut s = LinkStats::default();
        for i in 0..(MAX_EVENTS as u64 + 500) {
            s.record_disconnect(MAC, T0 + i, "timeout");
        }
        assert_eq!(s.devices[MAC].disconnects.len(), MAX_EVENTS);
        // Old events age out.
        s.record_disconnect(MAC, T0 + 8 * 86_400, "timeout");
        assert_eq!(s.devices[MAC].disconnects.len(), 1);
        // At most MAX_DEVICES keyboards.
        for i in 0..20u64 {
            s.record_disconnect(
                &format!("AA:BB:CC:DD:EE:{i:02X}"),
                T0 + 9 * 86_400 + i,
                "timeout",
            );
        }
        assert_eq!(s.devices.len(), MAX_DEVICES);
        assert!(
            s.devices.contains_key("AA:BB:CC:DD:EE:13"),
            "the latest stay"
        );
        let dir = std::env::temp_dir().join(format!("akm-linkstats-{}", std::process::id()));
        let path = dir.join("link-quality.json");
        s.unstable_due("AA:BB:CC:DD:EE:13", T0 + 9 * 86_400 + 30);
        s.save(&path).unwrap();
        assert_eq!(LinkStats::load(&path), s);
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(LinkStats::load(&path), LinkStats::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
