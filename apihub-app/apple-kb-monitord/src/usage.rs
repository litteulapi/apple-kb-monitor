//! Usage statistics of the daemon (#109): active minutes per day, counted
//! from the mere fact that the keyboard sent something
//! ([`akm_core::activity`]). No key, no key code, no report ever reaches this
//! module: its only input is a call without argument.
//!
//! Off by default (`[usage] active_time = true` turns it on). The record
//! ([`akm_core::usage::UsageStats`], one number per day) is saved at most
//! every [`SAVE_EVERY`] and when the daemon stops.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use akm_core::history::Clock;
use akm_core::usage::{local_day, UsageStats, UsageSummary};

/// The record is written at most this often while the keyboard is used.
pub const SAVE_EVERY: Duration = Duration::from_secs(300);

#[derive(Debug)]
struct Inner {
    stats: UsageStats,
    dirty: bool,
    last_save: Option<Instant>,
}

/// Counts the active minutes of one daemon.
#[derive(Debug)]
pub struct Tracker {
    inner: Mutex<Inner>,
    /// Where the record is kept (`None`: memory only).
    path: Option<PathBuf>,
}

impl Tracker {
    /// Read the record at `path` (if any) and count from there.
    pub fn new(path: Option<PathBuf>) -> Arc<Self> {
        let stats = path.as_deref().map(UsageStats::load).unwrap_or_default();
        Arc::new(Self {
            inner: Mutex::new(Inner {
                stats,
                dirty: false,
                last_save: None,
            }),
            path,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The keyboard sent something at unix time `now_unix`.
    pub fn note_at(&self, now_unix: u64, now: Instant) {
        let mut g = self.lock();
        if !g.stats.note(now_unix, &local_day(now_unix)) {
            return;
        }
        g.dirty = true;
        let due = g
            .last_save
            .is_none_or(|t| now.saturating_duration_since(t) >= SAVE_EVERY);
        if due {
            g.last_save = Some(now);
            self.save(&mut g);
        }
    }

    /// The keyboard sent something now.
    pub fn note(&self) {
        self.note_at(akm_core::history::SystemClock.now(), Instant::now());
    }

    fn save(&self, g: &mut Inner) {
        let Some(p) = self.path.as_deref() else {
            return;
        };
        match g.stats.save(p) {
            Ok(()) => g.dirty = false,
            Err(e) => tracing::warn!("cannot save {}: {e}", p.display()),
        }
    }

    /// Write what is not on disk yet (daemon stop).
    pub fn flush(&self) {
        let mut g = self.lock();
        if g.dirty {
            self.save(&mut g);
        }
    }

    /// The last 7 days, for `GetState`.
    pub fn summary_at(&self, now_unix: u64) -> UsageSummary {
        self.lock().stats.summary(now_unix)
    }

    pub fn summary(&self) -> UsageSummary {
        self.summary_at(akm_core::history::SystemClock.now())
    }
}

/// Start counting: `tracker` is told every time the passive listener sees an
/// input report arrive (at most a few times per second, and with no data).
pub fn subscribe(tracker: &Arc<Tracker>) {
    let t = tracker.clone();
    akm_core::activity::subscribe(move || t.note());
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_790_944_440;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("akm-usaged-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("usage.json")
    }

    #[test]
    fn active_minutes_come_from_the_presence_of_events_only() {
        let t = Tracker::new(None);
        let i0 = Instant::now();
        // 3 minutes of typing (an event every second), 10 minutes of pause,
        // 2 more minutes.
        for s in 0..180u64 {
            t.note_at(T0 + s, i0 + Duration::from_secs(s));
        }
        for s in 780..900u64 {
            t.note_at(T0 + s, i0 + Duration::from_secs(s));
        }
        let sum = t.summary_at(T0 + 900);
        assert_eq!(sum.today_active_minutes, 5, "the pause is not counted");
        assert_eq!(sum.days.len(), 7);
        assert_eq!(sum.days[6].active_hours, 0.1);
    }

    #[test]
    fn the_record_is_saved_sparingly_and_flushed_at_stop() {
        let path = tmp("save");
        let t = Tracker::new(Some(path.clone()));
        let i0 = Instant::now();
        t.note_at(T0, i0);
        assert_eq!(
            UsageStats::load(&path).days.values().sum::<u16>(),
            1,
            "first minute: saved"
        );
        for m in 1..4u64 {
            t.note_at(T0 + m * 60, i0 + Duration::from_secs(m * 60));
        }
        assert_eq!(
            UsageStats::load(&path).days.values().sum::<u16>(),
            1,
            "not rewritten every minute"
        );
        t.note_at(T0 + 300, i0 + SAVE_EVERY);
        assert_eq!(UsageStats::load(&path).days.values().sum::<u16>(), 5);
        t.note_at(T0 + 360, i0 + SAVE_EVERY + Duration::from_secs(60));
        t.flush();
        assert_eq!(UsageStats::load(&path).days.values().sum::<u16>(), 6);
        // A restart goes on from the record.
        let again = Tracker::new(Some(path.clone()));
        assert_eq!(again.summary_at(T0 + 400).today_active_minutes, 6);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Privacy, end to end: key reports written to the node the listener
    /// reads make minutes count, and nothing of them is in the record.
    #[test]
    fn key_reports_leave_no_trace_but_a_counter() {
        use std::os::fd::AsRawFd;
        let path = tmp("privacy");
        let tracker = Tracker::new(Some(path.clone()));
        subscribe(&tracker);
        let (rx, tx) = std::io::pipe().unwrap();
        // "h", "e", "l", "l", "o" as boot keyboard reports (id 0x01), then EOF.
        let keys = [0x0bu8, 0x08, 0x0f, 0x0f, 0x12];
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            let mut tx = tx;
            for k in keys {
                tx.write_all(&[0x01, 0x00, 0x00, k, 0, 0, 0, 0, 0]).unwrap();
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let mut events = Vec::new();
        let exit = akm_core::passive::listen_fd(rx.as_raw_fd(), 200, &mut || None, &mut |e| {
            events.push(e)
        });
        writer.join().unwrap();
        assert_eq!(exit, akm_core::passive::Exit::Hangup);
        assert!(events.is_empty(), "key reports are not decoded: {events:?}");
        tracker.flush();
        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let days = v["days"].as_object().unwrap();
        assert_eq!(v.as_object().unwrap().len(), 1, "{text}");
        assert_eq!(days.len(), 1);
        let (day, minutes) = days.iter().next().unwrap();
        assert_eq!(day.len(), 10);
        assert!((1..=2).contains(&minutes.as_u64().unwrap()), "{text}");
        // And the module cannot see a report at all.
        let src = include_str!("usage.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for forbidden in ["&[u8]", "PassiveEvent", "hidraw", "keycode", "KbReport"] {
            assert!(!code.contains(forbidden), "{forbidden}");
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
