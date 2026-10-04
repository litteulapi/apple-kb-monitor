//! Battery history — JSONL store with a single writer, injectable clock and bounded retention.

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
            .map_or(0, |d| d.as_secs())
    }
}

/// Event attached to a sample (written by the daemon, kept by rotation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryEvent {
    /// First sample on a new set of batteries.
    BatteryReplaced,
}

/// Current schema of the history lines.
pub const SCHEMA: u8 = 2;

/// A single battery history entry.
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
    /// \[measured\] Report 0x46 (instantaneous cell voltage), mV.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mv_0x46: Option<u32>,
    /// \[measured\] Report 0x49 (slow voltage, ~37 mV under 0x46), mV.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mv_0x49: Option<u32>,
    /// `Some(false)` = the `voltage` field of this line is not a measurement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voltage_valid: Option<bool>,
    /// Keyboard that gave the sample (upper-case MAC); `None` = written before 3.2.0-3 or unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}

impl HistoryEntry {
    /// Ordinary sample (voltage in volts, no millivolt detail).
    #[must_use]
    pub fn sample(ts: u64, pct: f64, voltage: Option<f64>) -> Self {
        Self {
            ts,
            pct,
            voltage,
            schema: voltage.map(|_| SCHEMA),
            ..Self::default()
        }
    }

    /// Sample of the current schema with the real voltages in mV.
    #[must_use]
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

    /// Is `voltage` a real measurement?
    #[must_use]
    pub fn voltage_reliable(&self) -> bool {
        self.voltage_valid != Some(false)
    }

    /// The voltage when it is a real measurement.
    #[must_use]
    pub fn reliable_voltage(&self) -> Option<f64> {
        self.voltage.filter(|_| self.voltage_reliable())
    }

    /// The same sample, attributed to the keyboard `mac`.
    #[must_use]
    pub fn with_mac(mut self, mac: Option<&str>) -> Self {
        self.mac = mac.map(str::to_ascii_uppercase);
        self
    }

    /// Does this sample belong to `mac`? `None` asks for every keyboard; an unattributed sample
    /// belongs to every keyboard.
    #[must_use]
    pub fn is_from(&self, mac: Option<&str>) -> bool {
        match (self.mac.as_deref(), mac) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => true,
        }
    }

    fn mark_legacy(&mut self) -> bool {
        if self.schema.is_none() && self.voltage.is_some() && self.voltage_valid.is_none() {
            self.voltage_valid = Some(false);
            return true;
        }
        false
    }
}

/// `YYYY-MM-DD HH:MM` (UTC) of a unix time, without a date crate.
#[must_use]
pub fn format_utc(ts: u64) -> String {
    let (y, m, d, secs) = crate::conv::civil_utc(ts);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60
    )
}

/// Plausible battery voltage range (V) for one or two cells.
pub const VOLTAGE_RANGE: std::ops::RangeInclusive<f64> = 0.5..=4.5;

/// Is this sample worth storing?
#[must_use]
pub fn valid_sample(pct: f64, voltage: Option<f64>) -> bool {
    pct.is_finite()
        && (0.0..=100.0).contains(&pct)
        && voltage.is_none_or(|v| v.is_finite() && VOLTAGE_RANGE.contains(&v))
}

/// The samples of the keyboard `mac` (all of them for `None`), see [`HistoryEntry::is_from`].
#[must_use]
pub fn of_keyboard(entries: Vec<HistoryEntry>, mac: Option<&str>) -> Vec<HistoryEntry> {
    entries.into_iter().filter(|e| e.is_from(mac)).collect()
}

/// `$XDG_STATE_HOME/apple-kb-monitor/history.jsonl` (fallback `~/.local/state`).
#[must_use]
pub fn default_path() -> PathBuf {
    crate::paths::state_home().join(REL_PATH)
}

/// Pre-3.1 location: `$XDG_DATA_HOME/apple-kb-monitor/history.jsonl`.
#[must_use]
pub fn legacy_path() -> PathBuf {
    crate::paths::data_home().join(REL_PATH)
}

/// Latest timestamp accepted when reading (2100-01-01 UTC).
pub const MAX_TS: u64 = 4_102_444_800;
const MV_RANGE: std::ops::RangeInclusive<u32> = 500..=4500;

/// Is a decoded line plausible?
#[must_use]
pub fn valid_entry(e: &HistoryEntry) -> bool {
    e.ts <= MAX_TS
        && valid_sample(e.pct, e.voltage)
        && e.mv_0x46.is_none_or(|mv| MV_RANGE.contains(&mv))
        && e.mv_0x49.is_none_or(|mv| MV_RANGE.contains(&mv))
}

fn parse_line(line: &[u8]) -> Option<HistoryEntry> {
    let mut e = serde_json::from_slice::<HistoryEntry>(line).ok()?;
    if !valid_entry(&e) {
        return None;
    }
    e.mark_legacy();
    Some(e)
}

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
    let mut seen = std::collections::HashSet::new();
    byte_lines(content)
        .into_iter()
        .filter_map(parse_line)
        // Samples written twice in one second by older versions: the first one is kept (R6).
        .filter(|e| e.event.is_some() || seen.insert(e.ts))
        .collect()
}

/// Parse JSONL content; malformed or implausible lines are skipped (see [`valid_entry`]).
#[must_use]
pub fn parse(content: &str) -> Vec<HistoryEntry> {
    parse_bytes(content.as_bytes())
}

/// Exclusive lock of the history file at `path`, held by every writer (daemon and akmctl) until dropped.
///
/// A sidecar file: the rewrites replace the data file's inode, which would drop a lock taken on it.
///
/// # Errors
///
/// Any I/O error while opening or locking the lock file.
pub fn lock(path: &Path) -> io::Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    let f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path.with_file_name(name))?;
    loop {
        // SAFETY: flock(2) on a descriptor owned by `f`, which outlives the call.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(f);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// History store bound to a file and a clock.
pub struct History<C: Clock = SystemClock> {
    path: PathBuf,
    clock: C,
}

impl History<SystemClock> {
    /// Store at [`default_path`], wall clock.
    #[must_use]
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

    /// Copy `legacy` to our path once (if ours does not exist).
    ///
    /// # Errors
    ///
    /// Any I/O error while copying the file.
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

    /// Marks every legacy `voltage` as unreliable in the file.
    ///
    /// # Errors
    ///
    /// Any I/O error while reading or rewriting the file.
    pub fn mark_legacy_voltages(&self) -> io::Result<usize> {
        let _lock = lock(&self.path)?;
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
                    out.extend_from_slice(
                        serde_json::to_string(&e)
                            .map_err(io::Error::other)?
                            .as_bytes(),
                    );
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
        crate::fsutil::write_atomic(&self.path, &out, crate::fsutil::Mode::Keep(0o666))?;
        Ok(changed)
    }

    /// Attributes the unattributed lines to `mac`, once: only while no line carries a MAC yet.
    ///
    /// Before 3.2.0-3 the history had no MAC; its lines are given to the first keyboard followed after
    /// the upgrade, the one the history was being written for.
    ///
    /// # Errors
    ///
    /// Any I/O error while reading or rewriting the file.
    pub fn attribute_unowned(&self, mac: &str) -> io::Result<usize> {
        let _lock = lock(&self.path)?;
        let content = match std::fs::read(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let lines = byte_lines(&content);
        if lines
            .iter()
            .filter_map(|l| parse_line(l))
            .any(|e| e.mac.is_some())
        {
            return Ok(0);
        }
        let mut changed = 0usize;
        let mut out: Vec<u8> = Vec::with_capacity(content.len() + lines.len() * 28);
        for line in lines {
            // Lines that do not parse are copied byte for byte.
            match serde_json::from_slice::<HistoryEntry>(line) {
                Ok(e) => {
                    changed += 1;
                    let e = e.with_mac(Some(mac));
                    out.extend_from_slice(
                        serde_json::to_string(&e)
                            .map_err(io::Error::other)?
                            .as_bytes(),
                    );
                }
                Err(_) => out.extend_from_slice(line),
            }
            out.push(b'\n');
        }
        if changed == 0 {
            return Ok(0);
        }
        let backup = self.path.with_extension("jsonl.pre-mac");
        if !backup.exists() {
            std::fs::copy(&self.path, &backup)?;
        }
        crate::fsutil::write_atomic(&self.path, &out, crate::fsutil::Mode::Keep(0o666))?;
        Ok(changed)
    }

    /// Append a sample stamped with the clock.
    ///
    /// # Errors
    ///
    /// Any I/O error while appending to the file.
    pub fn append(&self, pct: f64, voltage: Option<f64>) -> io::Result<bool> {
        self.append_entry(&HistoryEntry::sample(self.clock.now(), pct, voltage))
    }

    /// Current time of the store's clock.
    pub fn now(&self) -> u64 {
        self.clock.now()
    }

    /// Append a fully built entry (timestamp included).
    ///
    /// # Errors
    ///
    /// Any I/O error while appending to the file.
    pub fn append_entry(&self, entry: &HistoryEntry) -> io::Result<bool> {
        if !valid_sample(entry.pct, entry.voltage) {
            return Ok(false);
        }
        // One sample per second: a Refresh or restart in the same second would write it twice.
        let _lock = lock(&self.path)?;
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

    /// Drop entries older than `retention_s` (atomic rewrite).
    ///
    /// # Errors
    ///
    /// Any I/O error while reading or rewriting the file.
    pub fn rotate(&self, retention_s: u64) -> io::Result<usize> {
        let _lock = lock(&self.path)?;
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
        // The threshold is anchored on the data as well as on the wall clock.
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
        let mut out = Vec::new();
        for e in &keep {
            writeln!(
                out,
                "{}",
                serde_json::to_string(e).map_err(io::Error::other)?
            )?;
        }
        crate::fsutil::write_atomic(&self.path, &out, crate::fsutil::Mode::Keep(0o666))?;
        Ok(total - keep.len())
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
            self.0.store(t, Ordering::SeqCst);
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
        let got = h.read();
        assert!(got.is_empty(), "{got:?}");
        assert!(!h.path().exists());
    }

    #[test]
    fn legacy_format_and_garbage_lines() {
        let e =
            parse("{\"ts\":1,\"pct\":90.0,\"voltage\":2.8}\nnot json\n{\"ts\":2,\"pct\":89.0}\n");
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].voltage, Some(2.8));
        assert_eq!(e[1].voltage, None);
        let s = serde_json::to_string(&HistoryEntry::sample(5, 1.0, None)).unwrap();
        assert_eq!(s, r#"{"ts":5,"pct":1.0}"#);
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
    fn duplicate_samples_of_one_second_are_read_once() {
        let e =
            parse("{\"ts\":9,\"pct\":90.0}\n{\"ts\":9,\"pct\":90.0}\n{\"ts\":10,\"pct\":89.0}\n");
        assert_eq!(e.iter().map(|x| x.ts).collect::<Vec<_>>(), [9, 10]);
    }

    #[allow(clippy::float_cmp)] // reason: exact values are the property under test
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

    #[allow(clippy::float_cmp)] // reason: exact values are the property under test
    #[test]
    fn legacy_voltage_is_read_without_voltage() {
        let e = parse(r#"{"ts":100,"pct":90.0,"voltage":2.9032}"#);
        assert_eq!(e[0].pct, 90.0);
        assert!(!e[0].voltage_reliable());
        assert_eq!(e[0].reliable_voltage(), None);
    }

    #[test]
    fn same_second_sample_is_written_once() {
        let dir = std::env::temp_dir().join(format!("akm-hist-c7-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let h = History::new(dir.join("h.jsonl"), SystemClock);
        let e = HistoryEntry::measured(1_791_007_043, 55.0, Some(2969), Some(2908));
        assert!(h.append_entry(&e).unwrap());
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
        let k = HistoryEntry::measured(8, 98.0, None, None);
        assert_eq!(
            serde_json::to_string(&k).unwrap(),
            r#"{"ts":8,"pct":98.0,"schema":2}"#
        );
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
        assert!(text
            .lines()
            .next()
            .unwrap()
            .contains("\"voltage_valid\":false"));
        assert!(text.contains("not json"), "garbage line is kept");
        assert!(t.0.join("h.jsonl.pre-schema2").exists());
        assert_eq!(h.mark_legacy_voltages().unwrap(), 0);
        let e = h.read();
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].voltage, Some(2.9806), "legacy value kept, not deleted");
        assert!(!e[0].voltage_reliable() && e[2].voltage_reliable());
        assert_eq!(e[1].voltage_valid, None, "no voltage: nothing to mark");
        let none = History::new(t.0.join("absent.jsonl"), FakeClock::at(1));
        assert_eq!(none.mark_legacy_voltages().unwrap(), 0);
    }

    #[test]
    fn unattributed_lines_go_to_the_first_keyboard_once() {
        let t = Tmp::new();
        let h = History::new(t.0.join("h.jsonl"), SystemClock);
        std::fs::write(
            h.path(),
            "{\"ts\":1,\"pct\":90.0}\nnot json\n{\"ts\":2,\"pct\":89.0}\n",
        )
        .unwrap();
        assert_eq!(h.attribute_unowned("aa:bb:cc:dd:ee:01").unwrap(), 2);
        let e = h.read();
        assert!(e
            .iter()
            .all(|x| x.mac.as_deref() == Some("AA:BB:CC:DD:EE:01")));
        assert!(std::fs::read_to_string(h.path())
            .unwrap()
            .contains("not json"));
        // A second keyboard never takes them over.
        assert_eq!(h.attribute_unowned("AA:BB:CC:DD:EE:02").unwrap(), 0);
        assert_eq!(of_keyboard(h.read(), Some("AA:BB:CC:DD:EE:02")).len(), 0);
        assert_eq!(of_keyboard(h.read(), Some("aa:bb:cc:dd:ee:01")).len(), 2);
        assert_eq!(of_keyboard(h.read(), None).len(), 2);
    }

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01 00:00");
        assert_eq!(format_utc(1_790_818_907), "2026-10-01 01:41");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00");
    }

    #[test]
    fn default_paths_follow_xdg() {
        assert!(default_path().ends_with(REL_PATH));
        assert!(legacy_path().ends_with(REL_PATH));
        assert_ne!(default_path(), legacy_path());
    }
}
