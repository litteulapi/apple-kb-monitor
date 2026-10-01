//! Battery history — JSONL store with a single writer, injectable clock and
//! bounded retention (docs/REVUE-ARCHITECTURE-GLOBALE.md §4.5, A10).
//!
//! Location: `$XDG_STATE_HOME/apple-kb-monitor/history.jsonl`
//! (default `~/.local/state/...`). The old `~/.local/share/...` file is copied
//! once on first use and kept.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Entries older than this are dropped by [`History::rotate`].
pub const RETENTION_S: u64 = 90 * 24 * 3600;
const REL_PATH: &str = "apple-kb-monitor/history.jsonl";

/// Time source (unix seconds), injectable for tests.
pub trait Clock: Send + Sync {
    fn now(&self) -> u64;
}

/// Wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// A single battery history entry. `voltage` is absent when the HID
/// diagnostic report could not be read (kernel percentage only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub ts: u64,
    pub pct: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voltage: Option<f64>,
}

/// Is this sample worth storing? (no invented 0 V / NaN / out-of-range %).
pub fn valid_sample(pct: f64, voltage: Option<f64>) -> bool {
    pct.is_finite() && (0.0..=100.0).contains(&pct) && voltage.is_none_or(|v| v.is_finite() && v > 0.0)
}

/// `$XDG_STATE_HOME/apple-kb-monitor/history.jsonl` (fallback `~/.local/state`).
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(REL_PATH)
}

/// Pre-3.1 location: `$XDG_DATA_HOME/apple-kb-monitor/history.jsonl`.
pub fn legacy_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(REL_PATH)
}

/// Parse JSONL content; malformed lines are skipped.
pub fn parse(content: &str) -> Vec<HistoryEntry> {
    content.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Discharge rate (mV/h) and remaining hours down to 2.0 V, from the last 50
/// entries that carry a voltage. `None` without enough data or if not discharging.
pub fn estimate_remaining(entries: &[HistoryEntry]) -> Option<(f64, f64)> {
    let recent: Vec<(u64, f64)> = entries
        .iter()
        .rev()
        .filter_map(|e| e.voltage.filter(|v| v.is_finite() && *v > 0.0).map(|v| (e.ts, v)))
        .take(50)
        .collect();
    if recent.len() < 2 {
        return None;
    }
    let (last_ts, last_v) = recent[0];
    let (first_ts, first_v) = recent[recent.len() - 1];
    let hours = (last_ts as f64 - first_ts as f64) / 3600.0;
    if hours < 0.01 {
        return None;
    }
    let rate_mvh = (first_v - last_v) * 1000.0 / hours;
    if rate_mvh < 0.1 {
        return None;
    }
    let remaining_mv = (last_v - 2.0) * 1000.0;
    if remaining_mv <= 0.0 {
        return Some((rate_mvh, 0.0));
    }
    Some((rate_mvh, remaining_mv / rate_mvh))
}

/// Human text of an estimate: `"5.0h (3.0 mV/h)"` or `"2.1 days (...)"`.
pub fn format_remaining(rate: f64, hours: f64) -> String {
    if hours < 24.0 {
        format!("{:.1}h ({:.1} mV/h)", hours, rate)
    } else {
        format!("{:.1} days ({:.1} mV/h)", hours / 24.0, rate)
    }
}

/// History store bound to a file and a clock.
pub struct History<C: Clock = SystemClock> {
    path: PathBuf,
    clock: C,
}

impl History<SystemClock> {
    /// Store at [`default_path`], wall clock.
    pub fn open_default() -> Self {
        Self::new(default_path(), SystemClock)
    }
}

impl<C: Clock> History<C> {
    pub fn new(path: impl Into<PathBuf>, clock: C) -> Self {
        Self { path: path.into(), clock }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Copy `legacy` to our path once (if ours does not exist). The legacy
    /// file is kept. Returns true if a copy happened.
    pub fn migrate_from(&self, legacy: &Path) -> io::Result<bool> {
        if self.path.exists() || !legacy.is_file() || legacy == self.path {
            return Ok(false);
        }
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::copy(legacy, &self.path)?;
        Ok(true)
    }

    /// Append a sample stamped with the clock. `Ok(false)` if rejected as invalid.
    pub fn append(&self, pct: f64, voltage: Option<f64>) -> io::Result<bool> {
        if !valid_sample(pct, voltage) {
            return Ok(false);
        }
        let entry = HistoryEntry { ts: self.clock.now(), pct, voltage };
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let line = serde_json::to_string(&entry).map_err(io::Error::other)?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(f, "{line}")?;
        Ok(true)
    }

    /// Every entry (empty if the file is missing).
    pub fn read(&self) -> Vec<HistoryEntry> {
        std::fs::read_to_string(&self.path).map(|c| parse(&c)).unwrap_or_default()
    }

    /// Entries with `ts >= since`.
    pub fn read_since(&self, since: u64) -> Vec<HistoryEntry> {
        self.read().into_iter().filter(|e| e.ts >= since).collect()
    }

    /// Drop entries older than `retention_s` (atomic rewrite). Returns the
    /// number of removed lines (malformed lines are removed too).
    pub fn rotate(&self, retention_s: u64) -> io::Result<usize> {
        let content = match std::fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let cutoff = self.clock.now().saturating_sub(retention_s);
        let total = content.lines().count();
        let keep: Vec<HistoryEntry> = parse(&content).into_iter().filter(|e| e.ts >= cutoff).collect();
        if keep.len() == total {
            return Ok(0);
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            for e in &keep {
                writeln!(f, "{}", serde_json::to_string(e).map_err(io::Error::other)?)?;
            }
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(total - keep.len())
    }

    /// Estimate from the stored entries.
    pub fn estimate_remaining(&self) -> Option<(f64, f64)> {
        estimate_remaining(&self.read())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::Arc;

    #[derive(Clone)]
    struct FakeClock(Arc<AtomicU64>);
    impl FakeClock {
        fn at(t: u64) -> Self {
            Self(Arc::new(AtomicU64::new(t)))
        }
        fn set(&self, t: u64) {
            self.0.store(t, Ordering::SeqCst)
        }
    }
    impl Clock for FakeClock {
        fn now(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    static N: AtomicU32 = AtomicU32::new(0);
    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("akm-hist-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn append_uses_injected_clock_and_reads_back() {
        let t = Tmp::new();
        let c = FakeClock::at(1_000);
        let h = History::new(t.0.join("sub/h.jsonl"), c.clone());
        assert!(h.append(90.0, Some(2.81)).unwrap());
        c.set(1_300);
        assert!(h.append(89.0, None).unwrap());
        let e = h.read();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], HistoryEntry { ts: 1_000, pct: 90.0, voltage: Some(2.81) });
        assert_eq!(e[1], HistoryEntry { ts: 1_300, pct: 89.0, voltage: None });
        assert_eq!(h.read_since(1_100).len(), 1);
    }

    #[test]
    fn invalid_samples_are_rejected() {
        let t = Tmp::new();
        let h = History::new(t.0.join("h.jsonl"), FakeClock::at(1));
        for (p, v) in [(f64::NAN, Some(2.8)), (101.0, None), (-1.0, None), (50.0, Some(0.0)), (50.0, Some(f64::INFINITY))] {
            assert!(!h.append(p, v).unwrap(), "{p} {v:?}");
        }
        assert!(h.read().is_empty());
        assert!(!h.path().exists());
    }

    #[test]
    fn legacy_format_and_garbage_lines() {
        let e = parse("{\"ts\":1,\"pct\":90.0,\"voltage\":2.8}\nnot json\n{\"ts\":2,\"pct\":89.0}\n");
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].voltage, Some(2.8));
        assert_eq!(e[1].voltage, None);
        // voltage-less entries serialise without the field
        let s = serde_json::to_string(&HistoryEntry { ts: 5, pct: 1.0, voltage: None }).unwrap();
        assert_eq!(s, r#"{"ts":5,"pct":1.0}"#);
    }

    #[test]
    fn rotation_drops_old_entries_only() {
        let t = Tmp::new();
        let c = FakeClock::at(100);
        let h = History::new(t.0.join("h.jsonl"), c.clone());
        h.append(90.0, None).unwrap();
        c.set(RETENTION_S + 200);
        h.append(80.0, None).unwrap();
        assert_eq!(h.rotate(RETENTION_S).unwrap(), 1);
        let e = h.read();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].pct, 80.0);
        assert_eq!(h.rotate(RETENTION_S).unwrap(), 0);
        // missing file is not an error
        assert_eq!(History::new(t.0.join("none"), c).rotate(1).unwrap(), 0);
    }

    #[test]
    fn migration_copies_once_and_keeps_legacy() {
        let t = Tmp::new();
        let legacy = t.0.join("old.jsonl");
        std::fs::write(&legacy, "{\"ts\":1,\"pct\":50.0,\"voltage\":2.5}\n").unwrap();
        let h = History::new(t.0.join("state/h.jsonl"), FakeClock::at(2));
        assert!(h.migrate_from(&legacy).unwrap());
        assert!(legacy.exists());
        assert_eq!(h.read().len(), 1);
        h.append(49.0, None).unwrap();
        assert!(!h.migrate_from(&legacy).unwrap(), "never overwrites");
        assert_eq!(h.read().len(), 2);
        assert!(!History::new(t.0.join("x.jsonl"), FakeClock::at(0)).migrate_from(&t.0.join("missing")).unwrap());
    }

    #[test]
    fn estimate_from_voltage_slope() {
        let e = |ts, v| HistoryEntry { ts, pct: 50.0, voltage: v };
        // 2.9 V -> 2.8 V in 10 h = 10 mV/h; 800 mV left to 2.0 V = 80 h.
        let (rate, hours) = estimate_remaining(&[e(0, Some(2.9)), e(18_000, None), e(36_000, Some(2.8))]).unwrap();
        assert!((rate - 10.0).abs() < 1e-6 && (hours - 80.0).abs() < 1e-6, "{rate} {hours}");
        assert_eq!(format_remaining(rate, hours), "3.3 days (10.0 mV/h)");
        assert_eq!(format_remaining(3.0, 5.0), "5.0h (3.0 mV/h)");
        // not enough data / not discharging / too short
        assert!(estimate_remaining(&[e(0, Some(2.9))]).is_none());
        assert!(estimate_remaining(&[e(0, Some(2.8)), e(36_000, Some(2.9))]).is_none());
        assert!(estimate_remaining(&[e(0, Some(2.9)), e(10, Some(2.8))]).is_none());
        // empty battery
        assert_eq!(estimate_remaining(&[e(0, Some(2.1)), e(3600, Some(1.9))]).unwrap().1, 0.0);
    }

    #[test]
    fn default_paths_follow_xdg() {
        // Only checks the suffix: env vars are process-global, not mutated here.
        assert!(default_path().ends_with(REL_PATH));
        assert!(legacy_path().ends_with(REL_PATH));
        assert_ne!(default_path(), legacy_path());
    }
}
