//! Robustness and exact-value tests of `history`.

use akm_core::history::{
    self, format_utc, parse, parse_bytes, valid_entry, valid_sample, Clock, History, HistoryEntry,
    HistoryEvent, SystemClock, MAX_TS, RETENTION_S, SCHEMA, VOLTAGE_RANGE,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

static ENV: Mutex<()> = Mutex::new(());
static N: AtomicU32 = AtomicU32::new(0);

struct Fixed(u64);
impl Clock for Fixed {
    fn now(&self) -> u64 {
        self.0
    }
}

struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "akm-hx-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn file(&self) -> PathBuf {
        self.0.join("history.jsonl")
    }
    fn with_ext(&self, ext: &str) -> PathBuf {
        self.file().with_extension(ext)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn line(ts: u64, pct: u32) -> String {
    format!("{{\"ts\":{ts},\"pct\":{pct}}}\n")
}

#[test]
fn format_utc_known_dates() {
    assert_eq!(format_utc(0), "1970-01-01 00:00");
    assert_eq!(format_utc(86_399), "1970-01-01 23:59");
    assert_eq!(format_utc(951_782_400), "2000-02-29 00:00"); // leap year
    assert_eq!(format_utc(1_709_251_199), "2024-02-29 23:59");
    assert_eq!(format_utc(1_709_251_200), "2024-03-01 00:00");
    assert_eq!(format_utc(1_735_689_599), "2024-12-31 23:59");
    assert_eq!(format_utc(1_735_689_600), "2025-01-01 00:00");
    assert_eq!(format_utc(1_790_000_000), "2026-09-21 14:13");
    assert_eq!(format_utc(1_700_000_000), "2023-11-14 22:13");
    assert_eq!(format_utc(4_102_444_800), "2100-01-01 00:00");
    for (t, want) in [
        (4_107_542_340u64, "2100-02-28 23:59"), // 2100 is not a leap year
        (4_107_542_400, "2100-03-01 00:00"),
        (3_981_357_000, "2096-02-29 12:30"),
        (4_102_444_740, "2099-12-31 23:59"),
        (2_147_483_640, "2038-01-19 03:14"),
        (978_307_200, "2001-01-01 00:00"),
    ] {
        assert_eq!(format_utc(t), want);
    }
    assert_eq!(format_utc(3_600 * 5 + 60 * 7 + 59), "1970-01-01 05:07");
}

#[test]
fn valid_sample_bounds() {
    assert!(valid_sample(0.0, None) && valid_sample(100.0, None));
    assert!(!valid_sample(-0.1, None) && !valid_sample(100.1, None));
    assert!(!valid_sample(f64::NAN, None) && !valid_sample(f64::INFINITY, None));
    assert_eq!(VOLTAGE_RANGE, 0.5..=4.5);
    assert!(
        valid_sample(50.0, Some(0.5))
            && valid_sample(50.0, Some(4.5))
            && valid_sample(50.0, Some(3.0))
    );
    assert!(!valid_sample(50.0, Some(0.49)) && !valid_sample(50.0, Some(4.51)));
    assert!(!valid_sample(50.0, Some(0.0)) && !valid_sample(50.0, Some(f64::NAN)));
    assert!(!valid_sample(50.0, Some(f64::INFINITY)) && !valid_sample(50.0, Some(-1.0)));
}

#[test]
fn valid_entry_bounds() {
    let mut e = HistoryEntry::measured(MAX_TS, 50.0, Some(3000), Some(2960));
    assert!(valid_entry(&e));
    e.ts = MAX_TS + 1;
    assert!(!valid_entry(&e));
    e.ts = 1;
    for (mv, ok) in [
        (499u32, false),
        (500, true),
        (4500, true),
        (4501, false),
        (u32::MAX, false),
    ] {
        e.mv_0x46 = Some(mv);
        assert_eq!(valid_entry(&e), ok, "0x46 {mv}");
        e.mv_0x46 = Some(3000);
        e.mv_0x49 = Some(mv);
        assert_eq!(valid_entry(&e), ok, "0x49 {mv}");
        e.mv_0x49 = None;
    }
    e.pct = 101.0;
    assert!(!valid_entry(&e));
}

#[test]
fn parse_rejects_implausible_lines_and_keeps_the_rest() {
    let c = format!(
        "{}{}{}{}{}{}{}",
        line(1000, 50),
        "{\"ts\":4102444801,\"pct\":50}\n",
        "{\"ts\":4102444800,\"pct\":50}\n",
        "{\"ts\":5,\"pct\":150}\n",
        "{\"ts\":6,\"pct\":50,\"voltage\":1e306}\n",
        "{\"ts\":7,\"pct\":50,\"voltage\":3.0,\"mv_0x46\":900000}\n",
        "{\"ts\":8,\"pct\":-1}\n",
    );
    let v = parse(&c);
    assert_eq!(
        v.iter().map(|e| e.ts).collect::<Vec<_>>(),
        vec![1000, 4_102_444_800]
    );
}

#[test]
fn parse_handles_crlf_blank_and_missing_final_newline() {
    let v = parse("{\"ts\":1,\"pct\":10}\r\n\r\n   \n{\"ts\":2,\"pct\":20}");
    assert_eq!(v.iter().map(|e| e.ts).collect::<Vec<_>>(), vec![1, 2]);
    assert!(parse("").is_empty() && parse("\n").is_empty());
}

#[test]
fn parse_bytes_survives_invalid_utf8() {
    let mut b = Vec::new();
    b.extend_from_slice(line(1, 10).as_bytes());
    b.extend_from_slice(b"\xff\xfe\x00 garbage\n");
    b.extend_from_slice(b"{\"ts\":2,\"pct\":\xc3\x28}\n");
    b.extend_from_slice(line(3, 30).as_bytes());
    let v = parse_bytes(&b);
    assert_eq!(v.iter().map(|e| e.ts).collect::<Vec<_>>(), vec![1, 3]);
}

#[test]
fn legacy_lines_are_marked_unreliable() {
    let v = parse("{\"ts\":1,\"pct\":50,\"voltage\":0.79}\n");
    assert_eq!(v[0].voltage_valid, Some(false));
    assert!(!v[0].voltage_reliable() && v[0].reliable_voltage().is_none());
    let v = parse("{\"ts\":1,\"pct\":50,\"voltage\":3.0,\"schema\":2}\n");
    assert_eq!(v[0].voltage_valid, None);
    assert_eq!(v[0].reliable_voltage(), Some(3.0));
}

#[test]
fn entry_constructors() {
    let s = HistoryEntry::sample(42, 55.5, Some(3.0));
    assert_eq!(
        (s.ts, s.pct, s.voltage, s.schema),
        (42, 55.5, Some(3.0), Some(SCHEMA))
    );
    let s = HistoryEntry::sample(42, 55.5, None);
    assert_eq!((s.ts, s.schema), (42, None));
    let m = HistoryEntry::measured(7, 80.0, Some(2976), Some(2940));
    assert_eq!(
        (m.ts, m.pct, m.mv_0x46, m.mv_0x49, m.schema),
        (7, 80.0, Some(2976), Some(2940), Some(SCHEMA))
    );
    assert_eq!(m.voltage, Some(2.976));
    assert_eq!(HistoryEntry::measured(7, 80.0, None, Some(1)).voltage, None);
}

#[test]
fn default_paths_follow_xdg() {
    let _g = ENV
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (s, d, h) = (
        std::env::var_os("XDG_STATE_HOME"),
        std::env::var_os("XDG_DATA_HOME"),
        std::env::var_os("HOME"),
    );
    std::env::set_var("XDG_STATE_HOME", "/x/state");
    std::env::set_var("XDG_DATA_HOME", "/x/data");
    assert_eq!(
        history::default_path(),
        PathBuf::from("/x/state/apple-kb-monitor/history.jsonl")
    );
    assert_eq!(
        history::legacy_path(),
        PathBuf::from("/x/data/apple-kb-monitor/history.jsonl")
    );
    std::env::set_var("XDG_STATE_HOME", "");
    std::env::set_var("XDG_DATA_HOME", "");
    std::env::set_var("HOME", "/h");
    assert_eq!(
        history::default_path(),
        PathBuf::from("/h/.local/state/apple-kb-monitor/history.jsonl")
    );
    assert_eq!(
        history::legacy_path(),
        PathBuf::from("/h/.local/share/apple-kb-monitor/history.jsonl")
    );
    for (k, v) in [("XDG_STATE_HOME", s), ("XDG_DATA_HOME", d), ("HOME", h)] {
        match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        }
    }
}

#[test]
fn system_clock_is_current() {
    let n = SystemClock.now();
    assert!(n > 1_700_000_000 && n < MAX_TS, "{n}");
    assert_eq!(History::new("/nonexistent/x", Fixed(77)).now(), 77);
}

#[test]
fn append_read_since_and_missing_file() {
    let d = Dir::new();
    let h = History::new(d.file(), Fixed(500));
    assert!(h.read().is_empty() && h.read_since(0).is_empty());
    assert_eq!(h.rotate(10).unwrap(), 0);
    assert_eq!(h.mark_legacy_voltages().unwrap(), 0);
    assert!(h.append(80.0, Some(3.0)).unwrap());
    assert!(!h.append(180.0, None).unwrap());
    assert!(!h.append(80.0, Some(0.0)).unwrap());
    for ts in [100u64, 200, 300] {
        assert!(h
            .append_entry(&HistoryEntry::sample(ts, 50.0, None))
            .unwrap());
    }
    assert_eq!(h.read().len(), 4);
    assert_eq!(h.read()[0].ts, 500);
    let since: Vec<u64> = h.read_since(200).iter().map(|e| e.ts).collect();
    assert_eq!(since, vec![500, 200, 300]);
    assert_eq!(h.read_since(501).len(), 0);
    assert_eq!(h.path(), d.file());
}

#[test]
fn append_creates_missing_parent_directories() {
    let d = Dir::new();
    let h = History::new(d.0.join("a/b/h.jsonl"), Fixed(5));
    assert!(h.append(10.0, None).unwrap());
    assert_eq!(h.read().len(), 1);
}

#[test]
fn migration_copies_once() {
    let d = Dir::new();
    let old = d.0.join("old.jsonl");
    std::fs::write(&old, line(1, 10)).unwrap();
    let h = History::new(d.0.join("sub/new.jsonl"), Fixed(5));
    assert!(h.migrate_from(&old).unwrap());
    assert!(old.exists(), "the old file is kept");
    assert_eq!(h.read().len(), 1);
    std::fs::write(&old, format!("{}{}", line(1, 10), line(2, 20))).unwrap();
    assert!(
        !h.migrate_from(&old).unwrap(),
        "does not copy over the existing file"
    );
    assert_eq!(h.read().len(), 1);
    assert!(!History::new(d.0.join("other.jsonl"), Fixed(5))
        .migrate_from(&d.0.join("none"))
        .unwrap());
    assert!(!History::new(&old, Fixed(5)).migrate_from(&old).unwrap());
}

#[test]
fn rotate_survives_invalid_utf8_and_sets_corrupt_lines_aside() {
    let d = Dir::new();
    let mut data = Vec::new();
    for i in 0..10u64 {
        data.extend_from_slice(line(1_000_000 + i * 60, 90).as_bytes());
    }
    data.extend_from_slice(b"\xff\xfe garbage\n");
    data.extend_from_slice(b"{\"ts\":1,\"pct\":999}\n");
    std::fs::write(d.file(), &data).unwrap();
    let h = History::new(d.file(), Fixed(2_000_000));
    assert_eq!(h.read().len(), 10);
    assert_eq!(
        h.rotate(RETENTION_S).unwrap(),
        2,
        "both invalid lines are removed"
    );
    assert_eq!(h.read().len(), 10);
    let content = std::fs::read(d.file()).unwrap();
    assert!(!content.windows(2).any(|w| w == b"\xff\xfe"));
    assert_eq!(content.split(|&b| b == b'\n').count() - 1, 10);
    let corrupt = std::fs::read(d.with_ext("jsonl.corrupt")).unwrap();
    assert_eq!(corrupt, b"\xff\xfe garbage\n{\"ts\":1,\"pct\":999}\n");
    assert_eq!(
        std::fs::read(d.with_ext("jsonl.prev")).unwrap(),
        data,
        ".prev = previous file"
    );
    assert_eq!(h.rotate(RETENTION_S).unwrap(), 0);
    assert_eq!(std::fs::read(d.with_ext("jsonl.corrupt")).unwrap(), corrupt);
    assert!(!d.with_ext("jsonl.tmp").exists());
}

#[test]
fn rotate_drops_old_keeps_events_and_counts() {
    let d = Dir::new();
    let h = History::new(d.file(), Fixed(10_000_000));
    let mut ev = HistoryEntry::sample(10, 100.0, None);
    ev.event = Some(HistoryEvent::BatteryReplaced);
    h.append_entry(&ev).unwrap();
    for ts in [20u64, 9_000_000, 9_999_999] {
        h.append_entry(&HistoryEntry::sample(ts, 50.0, None))
            .unwrap();
    }
    assert_eq!(h.rotate(1_000_000).unwrap(), 1);
    let ts: Vec<u64> = h.read().iter().map(|e| e.ts).collect();
    assert_eq!(ts, vec![10, 9_000_000, 9_999_999]);
    assert_eq!(h.rotate(1_000_000).unwrap(), 0);
    let h2 = History::new(d.0.join("b.jsonl"), Fixed(1_000));
    for ts in [100u64, 400, 900] {
        h2.append_entry(&HistoryEntry::sample(ts, 50.0, None))
            .unwrap();
    }
    assert_eq!(h2.rotate(500).unwrap(), 1); // cutoff = 900-500 = 400
    assert_eq!(
        h2.read().iter().map(|e| e.ts).collect::<Vec<_>>(),
        vec![400, 900]
    );
}

#[test]
fn clock_far_ahead_does_not_wipe_history() {
    let d = Dir::new();
    let h = History::new(d.file(), Fixed(4_000_000_000));
    for ts in [1_700_000_000u64, 1_700_000_100] {
        h.append_entry(&HistoryEntry::sample(ts, 50.0, None))
            .unwrap();
    }
    assert_eq!(h.rotate(RETENTION_S).unwrap(), 0);
    assert_eq!(h.read().len(), 2);
}

#[test]
fn mark_legacy_voltages_is_byte_safe_and_idempotent() {
    let d = Dir::new();
    let mut data = Vec::new();
    data.extend_from_slice(b"{\"ts\":1,\"pct\":50,\"voltage\":0.79}\n");
    data.extend_from_slice(b"\xff\xfe not json\n");
    data.extend_from_slice(b"{\"ts\":2,\"pct\":50,\"voltage\":3.0,\"schema\":2}\n");
    data.extend_from_slice(b"{\"ts\":3,\"pct\":50}\n");
    data.extend_from_slice(b"{\"ts\":4,\"pct\":50,\"voltage\":0.79}\n");
    std::fs::write(d.file(), &data).unwrap();
    let h = History::new(d.file(), Fixed(10));
    assert_eq!(h.mark_legacy_voltages().unwrap(), 2);
    let out = std::fs::read(d.file()).unwrap();
    let lines: Vec<&[u8]> = out.split(|b| *b == b'\n').collect();
    assert_eq!(
        lines[1], b"\xff\xfe not json",
        "corrupt line copied byte for byte"
    );
    assert_eq!(
        lines[2],
        b"{\"ts\":2,\"pct\":50,\"voltage\":3.0,\"schema\":2}"
    );
    assert_eq!(lines[3], b"{\"ts\":3,\"pct\":50}");
    assert!(String::from_utf8_lossy(lines[0]).contains("\"voltage_valid\":false"));
    assert!(String::from_utf8_lossy(lines[4]).contains("\"voltage_valid\":false"));
    assert_eq!(
        std::fs::read(d.with_ext("jsonl.pre-schema2")).unwrap(),
        data
    );
    assert_eq!(h.mark_legacy_voltages().unwrap(), 0);
    assert_eq!(std::fs::read(d.file()).unwrap(), out);
    assert_eq!(
        std::fs::read(d.with_ext("jsonl.pre-schema2")).unwrap(),
        data
    );
    assert!(!d.with_ext("jsonl.tmp").exists());
    assert_eq!(h.read().len(), 4);
}

#[test]
fn mark_legacy_backup_is_made_once() {
    let d = Dir::new();
    std::fs::write(d.file(), "{\"ts\":1,\"pct\":50,\"voltage\":0.79}\n").unwrap();
    std::fs::write(d.with_ext("jsonl.pre-schema2"), "old backup\n").unwrap();
    let h = History::new(d.file(), Fixed(10));
    assert_eq!(h.mark_legacy_voltages().unwrap(), 1);
    assert_eq!(
        std::fs::read_to_string(d.with_ext("jsonl.pre-schema2")).unwrap(),
        "old backup\n"
    );
}

#[test]
fn unreadable_path_is_an_error_not_a_panic() {
    let d = Dir::new();
    // A directory inside d, so the sidecar lock file is removed with d.
    let dir = d.0.join("hist");
    std::fs::create_dir(&dir).unwrap();
    let h = History::new(&dir, Fixed(10));
    assert!(h.rotate(10).is_err());
    assert!(h.mark_legacy_voltages().is_err());
    let got = h.read();
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn blank_lines_are_not_kept_as_corrupt() {
    let d = Dir::new();
    let mut data = Vec::new();
    data.extend_from_slice(line(1_000_000, 90).as_bytes());
    data.extend_from_slice(b"   \n\n\t\n");
    data.extend_from_slice(b"junk\n");
    std::fs::write(d.file(), &data).unwrap();
    let h = History::new(d.file(), Fixed(2_000_000));
    assert_eq!(h.rotate(RETENTION_S).unwrap(), 4);
    assert_eq!(
        std::fs::read(d.with_ext("jsonl.corrupt")).unwrap(),
        b"junk\n"
    );
    assert_eq!(h.read().len(), 1);
}
