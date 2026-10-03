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

/// Event attached to a sample (written by the daemon, kept by rotation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryEvent {
    /// First sample on a new set of batteries (#85).
    BatteryReplaced,
}

/// Current schema of the history lines. Lines without `schema` are legacy:
/// their `voltage` was the constant `0xF5` x 3.3 / 1023, not a measurement
/// (#136, #138, #180).
pub const SCHEMA: u8 = 2;

/// A single battery history entry. `voltage` is absent when the HID
/// diagnostic report could not be read (kernel percentage only). `event`
/// is absent on ordinary samples (older readers ignore the field).
///
/// Since [`SCHEMA`] 2 the real cell voltage is stored in mV (`mv_0x46`,
/// `mv_0x49`) and `voltage` mirrors `mv_0x46 / 1000` for old readers. A legacy
/// line keeps its `voltage` but carries `voltage_valid = false`: it is never
/// used for the autonomy, the replacement detection or a chart (#180).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub ts: u64,
    pub pct: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voltage: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<HistoryEvent>,
    /// Layout version of the line (absent = legacy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<u8>,
    /// [mesuré] Report 0x46 (instantaneous cell voltage), mV.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mv_0x46: Option<u32>,
    /// [mesuré] Report 0x49 (slow voltage, ~37 mV under 0x46), mV.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mv_0x49: Option<u32>,
    /// `Some(false)` = the `voltage` field of this line is not a measurement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voltage_valid: Option<bool>,
}

impl HistoryEntry {
    /// Ordinary sample (voltage in volts, no millivolt detail). A caller that
    /// passes a voltage vouches that it is a measurement: the line is written
    /// with the current schema.
    pub fn sample(ts: u64, pct: f64, voltage: Option<f64>) -> Self {
        Self {
            ts,
            pct,
            voltage,
            schema: voltage.map(|_| SCHEMA),
            ..Self::default()
        }
    }

    /// Sample of the current schema with the real voltages in mV (#180).
    pub fn measured(ts: u64, pct: f64, mv_0x46: Option<u32>, mv_0x49: Option<u32>) -> Self {
        Self {
            ts,
            pct,
            voltage: mv_0x46.map(|mv| f64::from(mv) / 1000.0),
            schema: Some(SCHEMA),
            mv_0x46,
            mv_0x49,
            ..Self::default()
        }
    }

    /// Is `voltage` a real measurement? False for legacy lines (#180).
    pub fn voltage_reliable(&self) -> bool {
        self.voltage_valid != Some(false)
    }

    /// The voltage when it is a real measurement.
    pub fn reliable_voltage(&self) -> Option<f64> {
        self.voltage.filter(|_| self.voltage_reliable())
    }

    /// Marks the legacy `voltage` as unreliable; true if the line changed.
    fn mark_legacy(&mut self) -> bool {
        if self.schema.is_none() && self.voltage.is_some() && self.voltage_valid.is_none() {
            self.voltage_valid = Some(false);
            return true;
        }
        false
    }
}

/// `YYYY-MM-DD HH:MM` (UTC) of a unix time, without a date crate.
pub fn format_utc(ts: u64) -> String {
    let days = (ts / 86_400) as i64;
    let secs = ts % 86_400;
    // Civil-from-days (H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60
    )
}

/// Plausible battery voltage range (V) for one or two cells: a raw ADC read
/// can decode to hundreds of volts, and 0 V is an invented value.
pub const VOLTAGE_RANGE: std::ops::RangeInclusive<f64> = 0.5..=4.5;

/// Is this sample worth storing? (no invented 0 V / NaN / out-of-range %).
pub fn valid_sample(pct: f64, voltage: Option<f64>) -> bool {
    pct.is_finite()
        && (0.0..=100.0).contains(&pct)
        && voltage.is_none_or(|v| v.is_finite() && VOLTAGE_RANGE.contains(&v))
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

/// Latest timestamp accepted when reading (2100-01-01 UTC): beyond it the
/// line is corrupt or hand-edited, and downstream arithmetic is not safe.
pub const MAX_TS: u64 = 4_102_444_800;
/// Plausible range of a millivolt reading (same bounds as [`VOLTAGE_RANGE`]).
const MV_RANGE: std::ops::RangeInclusive<u32> = 500..=4500;

/// Is a decoded line plausible? (`valid_sample`, bounded `ts`, bounded mV).
pub fn valid_entry(e: &HistoryEntry) -> bool {
    e.ts <= MAX_TS
        && valid_sample(e.pct, e.voltage)
        && e.mv_0x46.is_none_or(|mv| MV_RANGE.contains(&mv))
        && e.mv_0x49.is_none_or(|mv| MV_RANGE.contains(&mv))
}

/// One line (bytes, no terminator) -> a validated entry, legacy lines marked.
fn parse_line(line: &[u8]) -> Option<HistoryEntry> {
    let mut e = serde_json::from_slice::<HistoryEntry>(line).ok()?;
    if !valid_entry(&e) {
        return None;
    }
    e.mark_legacy();
    Some(e)
}

/// Lines of a byte buffer like `str::lines` (`\n` or `\r\n`), whatever the
/// bytes: a line that is not UTF-8 is just a line that will not parse.
fn byte_lines(content: &[u8]) -> Vec<&[u8]> {
    if content.is_empty() {
        return Vec::new();
    }
    let body = content.strip_suffix(b"\n").unwrap_or(content);
    body.split(|b| *b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .collect()
}

/// Parse JSONL bytes; malformed, non-UTF-8 or implausible lines are skipped.
pub fn parse_bytes(content: &[u8]) -> Vec<HistoryEntry> {
    byte_lines(content).into_iter().filter_map(parse_line).collect()
}

/// Parse JSONL content; malformed or implausible lines are skipped (see
/// [`valid_entry`]). Legacy lines come back with `voltage_valid = Some(false)`
/// (#180).
pub fn parse(content: &str) -> Vec<HistoryEntry> {
    parse_bytes(content.as_bytes())
}

/// Discharge rate (mV/h) and remaining hours down to 2.0 V, from the last 50
/// entries that carry a **reliable** voltage (legacy lines are ignored, #180). `None` without enough data or if not discharging.
pub fn estimate_remaining(entries: &[HistoryEntry]) -> Option<(f64, f64)> {
    let recent: Vec<(u64, f64)> = entries
        .iter()
        .rev()
        .filter_map(|e| {
            e.reliable_voltage()
                .filter(|v| v.is_finite() && VOLTAGE_RANGE.contains(v))
                .map(|v| (e.ts, v))
        })
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
    if !rate_mvh.is_finite() || rate_mvh < 0.1 {
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
        Self {
            path: path.into(),
            clock,
        }
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

    /// Marks every legacy `voltage` as unreliable in the file (#180):
    /// `voltage_valid = false` is written on the lines without `schema`.
    /// Idempotent; a copy `history.jsonl.pre-schema2` is made once before the
    /// rewrite. Malformed lines are kept as they are. Returns the number of
    /// lines marked.
    pub fn mark_legacy_voltages(&self) -> io::Result<usize> {
        let content = match std::fs::read(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let mut changed = 0usize;
        let mut out: Vec<u8> = Vec::with_capacity(content.len() + 64);
        for line in byte_lines(&content) {
            // Lines that do not parse (or are not UTF-8) are copied byte for byte.
            let marked = serde_json::from_slice::<HistoryEntry>(line)
                .ok()
                .and_then(|mut e| e.mark_legacy().then_some(e));
            match marked {
                Some(e) => {
                    changed += 1;
                    out.extend_from_slice(serde_json::to_string(&e).map_err(io::Error::other)?.as_bytes());
                }
                None => out.extend_from_slice(line),
            }
            out.push(b'\n');
        }
        if changed == 0 {
            return Ok(0);
        }
        let backup = self.path.with_extension("jsonl.pre-schema2");
        if !backup.exists() {
            std::fs::copy(&self.path, &backup)?;
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&out)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(changed)
    }

    /// Append a sample stamped with the clock. `Ok(false)` if rejected as invalid.
    pub fn append(&self, pct: f64, voltage: Option<f64>) -> io::Result<bool> {
        self.append_entry(&HistoryEntry::sample(self.clock.now(), pct, voltage))
    }

    /// Current time of the store's clock.
    pub fn now(&self) -> u64 {
        self.clock.now()
    }

    /// Append a fully built entry (timestamp included). `Ok(false)` if invalid.
    pub fn append_entry(&self, entry: &HistoryEntry) -> io::Result<bool> {
        if !valid_sample(entry.pct, entry.voltage) {
            return Ok(false);
        }
        // One sample per second (C7): a Refresh or a restart of the daemon in
        // the same second as the last sample wrote it twice. An event (new
        // batteries) is never dropped.
        if entry.event.is_none() && self.last_ts() == Some(entry.ts) {
            return Ok(false);
        }
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let line = serde_json::to_string(entry).map_err(io::Error::other)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        // One write call: an interruption cannot leave a line without its `\n`.
        f.write_all(format!("{line}\n").as_bytes())?;
        Ok(true)
    }

    /// Timestamp of the last entry, read from the end of the file only.
    fn last_ts(&self) -> Option<u64> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(&self.path).ok()?;
        let len = f.metadata().ok()?.len();
        let from = len.saturating_sub(4096);
        f.seek(SeekFrom::Start(from)).ok()?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;
        parse_bytes(&buf).last().map(|e| e.ts)
    }

    /// Every entry (empty if the file is missing).
    pub fn read(&self) -> Vec<HistoryEntry> {
        // Bytes, not `read_to_string`: one invalid byte must not hide the rest.
        std::fs::read(&self.path)
            .map(|c| parse_bytes(&c))
            .unwrap_or_default()
    }

    /// Entries with `ts >= since`.
    pub fn read_since(&self, since: u64) -> Vec<HistoryEntry> {
        self.read().into_iter().filter(|e| e.ts >= since).collect()
    }

    /// Drop entries older than `retention_s` (atomic rewrite). Returns the
    /// number of removed lines (malformed lines are removed too). Entries
    /// carrying an event (battery replacement) are kept whatever their age:
    /// battery sets last longer than the retention.
    pub fn rotate(&self, retention_s: u64) -> io::Result<usize> {
        let content = match std::fs::read(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let lines = byte_lines(&content);
        let total = lines.len();
        let mut entries = Vec::new();
        let mut rejected: Vec<&[u8]> = Vec::new();
        for l in lines {
            match parse_line(l) {
                Some(e) => entries.push(e),
                None if l.iter().all(u8::is_ascii_whitespace) => {}
                None => rejected.push(l),
            }
        }
        // The threshold is anchored on the data as well as on the wall clock:
        // a clock running ahead (no NTP yet, dead RTC battery) cannot make
        // the whole file look old (#166).
        let anchor = entries
            .iter()
            .map(|e| e.ts)
            .max()
            .map_or(self.clock.now(), |m| m.min(self.clock.now()));
        let cutoff = anchor.saturating_sub(retention_s);
        let keep: Vec<HistoryEntry> = entries
            .into_iter()
            .filter(|e| e.ts >= cutoff || e.event.is_some())
            .collect();
        if keep.len() == total {
            return Ok(0);
        }
        // Corrupt lines are set aside before the rewrite drops them.
        if !rejected.is_empty() {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.path.with_extension("jsonl.corrupt"))?;
            let mut buf = Vec::new();
            for l in &rejected {
                buf.extend_from_slice(l);
                buf.push(b'\n');
            }
            f.write_all(&buf)?;
            f.sync_all()?;
        }
        // One generation of safety net before any rewrite.
        std::fs::copy(&self.path, self.path.with_extension("jsonl.prev"))?;
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
            let p = std::env::temp_dir().join(format!(
                "akm-hist-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
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
    fn clock_ahead_does_not_wipe_the_history() {
        // #166: 100 real samples, clock 5 years ahead.
        let t = Tmp::new();
        let real = 1_800_000_000u64;
        let h = History::new(t.0.join("h.jsonl"), FakeClock::at(real));
        for i in 0..100 {
            h.append_entry(&HistoryEntry::sample(real - i * 3600, 50.0, None))
                .unwrap();
        }
        let ahead = History::new(t.0.join("h.jsonl"), FakeClock::at(real + 5 * 365 * 86_400));
        assert_eq!(ahead.rotate(RETENTION_S).unwrap(), 0);
        assert_eq!(ahead.read().len(), 100);
        // Genuinely old lines still go, and a .prev copy is kept.
        let h2 = History::new(t.0.join("h.jsonl"), FakeClock::at(real));
        h2.append_entry(&HistoryEntry::sample(real - 200 * 86_400, 50.0, None))
            .unwrap();
        assert_eq!(h2.rotate(RETENTION_S).unwrap(), 1);
        assert_eq!(h2.read().len(), 100);
        assert!(t.0.join("h.jsonl.prev").exists());
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
        assert_eq!(e[0], HistoryEntry::sample(1_000, 90.0, Some(2.81)));
        assert_eq!(e[1], HistoryEntry::sample(1_300, 89.0, None));
        assert_eq!(h.read_since(1_100).len(), 1);
    }

    #[test]
    fn invalid_samples_are_rejected() {
        let t = Tmp::new();
        let h = History::new(t.0.join("h.jsonl"), FakeClock::at(1));
        for (p, v) in [
            (f64::NAN, Some(2.8)),
            (101.0, None),
            (-1.0, None),
            (50.0, Some(0.0)),
            (50.0, Some(f64::INFINITY)),
        ] {
            assert!(!h.append(p, v).unwrap(), "{p} {v:?}");
        }
        assert!(h.read().is_empty());
        assert!(!h.path().exists());
    }

    #[test]
    fn legacy_format_and_garbage_lines() {
        let e =
            parse("{\"ts\":1,\"pct\":90.0,\"voltage\":2.8}\nnot json\n{\"ts\":2,\"pct\":89.0}\n");
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].voltage, Some(2.8));
        assert_eq!(e[1].voltage, None);
        // voltage-less entries serialise without the field
        let s = serde_json::to_string(&HistoryEntry::sample(5, 1.0, None)).unwrap();
        assert_eq!(s, r#"{"ts":5,"pct":1.0}"#);
        // event marker round-trips; unknown fields from newer writers are ignored
        let m = HistoryEntry {
            event: Some(HistoryEvent::BatteryReplaced),
            ..HistoryEntry::sample(6, 99.0, Some(3.0))
        };
        let j = serde_json::to_string(&m).unwrap();
        assert_eq!(
            j,
            r#"{"ts":6,"pct":99.0,"voltage":3.0,"event":"battery_replaced","schema":2}"#
        );
        assert_eq!(parse(&j)[0], m);
        assert_eq!(parse(r#"{"ts":7,"pct":1.0,"future":true}"#).len(), 1);
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
        // replacement markers survive rotation
        h.append_entry(&HistoryEntry {
            event: Some(HistoryEvent::BatteryReplaced),
            ..HistoryEntry::sample(RETENTION_S + 300, 100.0, None)
        })
        .unwrap();
        c.set(3 * RETENTION_S);
        h.append(70.0, None).unwrap();
        assert_eq!(h.rotate(RETENTION_S).unwrap(), 1);
        let e = h.read();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].event, Some(HistoryEvent::BatteryReplaced));
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
        assert!(!History::new(t.0.join("x.jsonl"), FakeClock::at(0))
            .migrate_from(&t.0.join("missing"))
            .unwrap());
    }

    #[test]
    fn estimate_from_voltage_slope() {
        let e = |ts, v| HistoryEntry::sample(ts, 50.0, v);
        // 2.9 V -> 2.8 V in 10 h = 10 mV/h; 800 mV left to 2.0 V = 80 h.
        let (rate, hours) =
            estimate_remaining(&[e(0, Some(2.9)), e(18_000, None), e(36_000, Some(2.8))]).unwrap();
        assert!(
            (rate - 10.0).abs() < 1e-6 && (hours - 80.0).abs() < 1e-6,
            "{rate} {hours}"
        );
        assert_eq!(format_remaining(rate, hours), "3.3 days (10.0 mV/h)");
        assert_eq!(format_remaining(3.0, 5.0), "5.0h (3.0 mV/h)");
        // not enough data / not discharging / too short
        assert!(estimate_remaining(&[e(0, Some(2.9))]).is_none());
        assert!(estimate_remaining(&[e(0, Some(2.8)), e(36_000, Some(2.9))]).is_none());
        assert!(estimate_remaining(&[e(0, Some(2.9)), e(10, Some(2.8))]).is_none());
        // empty battery
        assert_eq!(
            estimate_remaining(&[e(0, Some(2.1)), e(3600, Some(1.9))])
                .unwrap()
                .1,
            0.0
        );
    }

    #[test]
    fn legacy_voltage_is_read_without_voltage_and_ignored_for_autonomy() {
        // #180: the historised `voltage` was the constant 0xF5 * 3.3 / 1023.
        let e = parse(r#"{"ts":100,"pct":90.0,"voltage":2.9032}"#);
        assert_eq!(e[0].pct, 90.0);
        assert!(!e[0].voltage_reliable());
        assert_eq!(e[0].reliable_voltage(), None);
        // Falling "voltages" of legacy lines give no autonomy.
        let legacy: Vec<HistoryEntry> = (0..10)
            .map(|i| {
                parse(&format!(
                    r#"{{"ts":{},"pct":90.0,"voltage":{}}}"#,
                    i * 3600,
                    3.0 - f64::from(i) * 0.01
                ))
                .remove(0)
            })
            .collect();
        assert!(estimate_remaining(&legacy).is_none());
        // The same slope on schema 2 lines does give one.
        let real: Vec<HistoryEntry> = (0..10)
            .map(|i| {
                HistoryEntry::measured(u64::from(i) * 3600, 90.0, Some(3000 - i * 10), Some(2960 - i * 10))
            })
            .collect();
        let (rate, _) = estimate_remaining(&real).unwrap();
        assert!((rate - 10.0).abs() < 1e-6, "{rate}");
        // Mixed: legacy lines are skipped, the real ones count.
        let mut mixed = legacy;
        mixed.extend(real);
        assert!(estimate_remaining(&mixed).is_some());
    }

    #[test]
    fn same_second_sample_is_written_once() {
        // C7: a Refresh / a restart in the same second wrote the line twice.
        let dir = std::env::temp_dir().join(format!("akm-hist-c7-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let h = History::new(dir.join("h.jsonl"), SystemClock);
        let e = HistoryEntry::measured(1_791_007_043, 55.0, Some(2969), Some(2908));
        assert!(h.append_entry(&e).unwrap());
        // A new History on the same file: the daemon was restarted.
        let h2 = History::new(dir.join("h.jsonl"), SystemClock);
        assert!(!h2.append_entry(&e).unwrap(), "same second: not written");
        let mut ev = HistoryEntry::measured(1_791_007_043, 99.0, None, None);
        ev.event = Some(HistoryEvent::BatteryReplaced);
        assert!(h2.append_entry(&ev).unwrap(), "an event is never dropped");
        let next = HistoryEntry::measured(1_791_007_044, 55.0, Some(2969), Some(2908));
        assert!(h2.append_entry(&next).unwrap());
        assert_eq!(h2.read().len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn measured_entry_stores_real_millivolts() {
        let e = HistoryEntry::measured(7, 99.0, Some(2986), Some(2945));
        let j = serde_json::to_string(&e).unwrap();
        assert_eq!(
            j,
            r#"{"ts":7,"pct":99.0,"voltage":2.986,"schema":2,"mv_0x46":2986,"mv_0x49":2945}"#
        );
        let back = parse(&j);
        assert_eq!(back[0], e);
        assert!(back[0].voltage_reliable());
        // No voltage read: the line stays valid and carries no voltage.
        let k = HistoryEntry::measured(8, 98.0, None, None);
        assert_eq!(serde_json::to_string(&k).unwrap(), r#"{"ts":8,"pct":98.0,"schema":2}"#);
    }

    #[test]
    fn migration_marks_legacy_lines_once_and_keeps_the_rest() {
        let t = Tmp::new();
        let path = t.0.join("h.jsonl");
        std::fs::write(
            &path,
            "{\"ts\":1,\"pct\":98.0,\"voltage\":2.9806}\nnot json\n{\"ts\":2,\"pct\":97.0}\n{\"ts\":3,\"pct\":96.0,\"voltage\":2.986,\"schema\":2,\"mv_0x46\":2986}\n",
        )
        .unwrap();
        let h = History::new(&path, FakeClock::at(10));
        assert_eq!(h.mark_legacy_voltages().unwrap(), 1);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.lines().next().unwrap().contains("\"voltage_valid\":false"));
        assert!(text.contains("not json"), "garbage line is kept");
        assert!(t.0.join("h.jsonl.pre-schema2").exists());
        // Idempotent: nothing more to mark, the real line is untouched.
        assert_eq!(h.mark_legacy_voltages().unwrap(), 0);
        let e = h.read();
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].voltage, Some(2.9806), "legacy value kept, not deleted");
        assert!(!e[0].voltage_reliable() && e[2].voltage_reliable());
        assert_eq!(e[1].voltage_valid, None, "no voltage: nothing to mark");
        // Missing file: no error.
        let none = History::new(t.0.join("absent.jsonl"), FakeClock::at(1));
        assert_eq!(none.mark_legacy_voltages().unwrap(), 0);
    }

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01 00:00");
        assert_eq!(format_utc(1_790_818_907), "2026-10-01 01:41");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00");
    }

    #[test]
    fn default_paths_follow_xdg() {
        // Only checks the suffix: env vars are process-global, not mutated here.
        assert!(default_path().ends_with(REL_PATH));
        assert!(legacy_path().ends_with(REL_PATH));
        assert_ne!(default_path(), legacy_path());
    }
}
