//! Tests de robustesse et de valeurs exactes de `history` (#222, #223).

use akm_core::history::{
    self, estimate_remaining, format_remaining, format_utc, parse, parse_bytes, valid_entry,
    valid_sample, Clock, History, HistoryEntry, HistoryEvent, SystemClock, MAX_TS, RETENTION_S,
    SCHEMA, VOLTAGE_RANGE,
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
        let p = std::env::temp_dir().join(format!("akm-hx-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
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

// ── format_utc ──────────────────────────────────────────────────────────────

#[test]
fn format_utc_known_dates() {
    assert_eq!(format_utc(0), "1970-01-01 00:00");
    assert_eq!(format_utc(86_399), "1970-01-01 23:59");
    assert_eq!(format_utc(951_782_400), "2000-02-29 00:00"); // bissextile
    assert_eq!(format_utc(1_709_251_199), "2024-02-29 23:59");
    assert_eq!(format_utc(1_709_251_200), "2024-03-01 00:00");
    assert_eq!(format_utc(1_735_689_599), "2024-12-31 23:59");
    assert_eq!(format_utc(1_735_689_600), "2025-01-01 00:00");
    assert_eq!(format_utc(1_790_000_000), "2026-09-21 14:13");
    assert_eq!(format_utc(1_700_000_000), "2023-11-14 22:13");
    assert_eq!(format_utc(4_102_444_800), "2100-01-01 00:00");
    assert_eq!(format_utc(3_600 * 5 + 60 * 7 + 59), "1970-01-01 05:07");
}

// ── valeurs ─────────────────────────────────────────────────────────────────

#[test]
fn valid_sample_bounds() {
    assert!(valid_sample(0.0, None) && valid_sample(100.0, None));
    assert!(!valid_sample(-0.1, None) && !valid_sample(100.1, None));
    assert!(!valid_sample(f64::NAN, None) && !valid_sample(f64::INFINITY, None));
    assert_eq!(VOLTAGE_RANGE, 0.5..=4.5);
    assert!(valid_sample(50.0, Some(0.5)) && valid_sample(50.0, Some(4.5)) && valid_sample(50.0, Some(3.0)));
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
    for (mv, ok) in [(499u32, false), (500, true), (4500, true), (4501, false), (u32::MAX, false)] {
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
    assert_eq!(v.iter().map(|e| e.ts).collect::<Vec<_>>(), vec![1000, 4_102_444_800]);
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
    assert_eq!((s.ts, s.pct, s.voltage, s.schema), (42, 55.5, Some(3.0), Some(SCHEMA)));
    let s = HistoryEntry::sample(42, 55.5, None);
    assert_eq!((s.ts, s.schema), (42, None));
    let m = HistoryEntry::measured(7, 80.0, Some(2976), Some(2940));
    assert_eq!((m.ts, m.pct, m.mv_0x46, m.mv_0x49, m.schema), (7, 80.0, Some(2976), Some(2940), Some(SCHEMA)));
    assert_eq!(m.voltage, Some(2.976));
    assert_eq!(HistoryEntry::measured(7, 80.0, None, Some(1)).voltage, None);
}

// ── autonomie ───────────────────────────────────────────────────────────────

fn volt(ts: u64, v: f64) -> HistoryEntry {
    HistoryEntry::sample(ts, 50.0, Some(v))
}

#[test]
fn estimate_remaining_exact_values() {
    // 3.000 V -> 2.900 V en 10 h : 10 mV/h ; reste (2.9 - 2.0) V = 900 mV -> 90 h.
    let e = vec![volt(0, 3.0), volt(36_000, 2.9)];
    let (rate, hours) = estimate_remaining(&e).unwrap();
    assert!((rate - 10.0).abs() < 1e-6, "{rate}");
    assert!((hours - 90.0).abs() < 1e-6, "{hours}");
    // Ordre : l'estimation part des entrées les plus récentes du tableau.
    assert!(format_remaining(rate, hours).ends_with(" days (10.0 mV/h)"));
}

#[test]
fn estimate_remaining_edge_cases() {
    assert_eq!(estimate_remaining(&[]), None);
    assert_eq!(estimate_remaining(&[volt(0, 3.0)]), None);
    // Moins de 0,01 h (36 s) : refusé ; 37 s : accepté.
    assert_eq!(estimate_remaining(&[volt(0, 3.0), volt(35, 2.9)]), None);
    assert!(estimate_remaining(&[volt(0, 3.0), volt(37, 2.9)]).is_some());
    // La tension monte / ne baisse pas : pas d'estimation.
    assert_eq!(estimate_remaining(&[volt(0, 2.9), volt(36_000, 3.0)]), None);
    assert_eq!(estimate_remaining(&[volt(0, 3.0), volt(36_000, 3.0)]), None);
    // Taux sous 0,1 mV/h : refusé (0,05 mV/h) ; au-dessus : accepté.
    assert_eq!(estimate_remaining(&[volt(0, 3.0), volt(36_000_000, 2.9995)]), None);
    // Tension sous 2,0 V : reste nul, taux conservé.
    let (r, h) = estimate_remaining(&[volt(0, 2.1), volt(36_000, 1.9)]).unwrap();
    assert!((r - 20.0).abs() < 1e-6 && h == 0.0);
    let (r, h) = estimate_remaining(&[volt(0, 2.5), volt(36_000, 1.0)]).unwrap();
    assert!(r > 0.0 && h == 0.0);
    // Les tensions absurdes ne produisent jamais de taux infini.
    assert_eq!(estimate_remaining(&[volt(0, 1e306), volt(36_000, 1.0)]), None);
    assert_eq!(estimate_remaining(&[volt(0, f64::NAN), volt(36_000, 1.0)]), None);
    // Lignes legacy (non fiables) ignorées.
    let mut a = volt(0, 3.0);
    a.voltage_valid = Some(false);
    assert_eq!(estimate_remaining(&[a, volt(36_000, 2.9)]), None);
}

#[test]
fn estimate_remaining_uses_only_the_last_fifty() {
    // 60 points : les 10 plus anciens (tension haute) sont hors de la fenêtre.
    let mut e: Vec<_> = (0..10u64).map(|i| volt(i * 3600, 4.4)).collect();
    e.extend((10..60u64).map(|i| volt(i * 3600, 3.0 - (i - 10) as f64 * 0.001)));
    let (rate, _) = estimate_remaining(&e).unwrap();
    assert!((rate - 1.0).abs() < 1e-6, "{rate}");
}

#[test]
fn format_remaining_switches_to_days() {
    assert_eq!(format_remaining(3.0, 5.0), "5.0h (3.0 mV/h)");
    assert_eq!(format_remaining(3.0, 23.94), "23.9h (3.0 mV/h)");
    assert_eq!(format_remaining(3.0, 24.0), "1.0 days (3.0 mV/h)");
    assert_eq!(format_remaining(1.25, 50.4), "2.1 days (1.2 mV/h)");
}

// ── chemins ─────────────────────────────────────────────────────────────────

#[test]
fn default_paths_follow_xdg() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let (s, d, h) = (std::env::var_os("XDG_STATE_HOME"), std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"));
    std::env::set_var("XDG_STATE_HOME", "/x/state");
    std::env::set_var("XDG_DATA_HOME", "/x/data");
    assert_eq!(history::default_path(), PathBuf::from("/x/state/apple-kb-monitor/history.jsonl"));
    assert_eq!(history::legacy_path(), PathBuf::from("/x/data/apple-kb-monitor/history.jsonl"));
    std::env::set_var("XDG_STATE_HOME", "");
    std::env::set_var("XDG_DATA_HOME", "");
    std::env::set_var("HOME", "/h");
    assert_eq!(history::default_path(), PathBuf::from("/h/.local/state/apple-kb-monitor/history.jsonl"));
    assert_eq!(history::legacy_path(), PathBuf::from("/h/.local/share/apple-kb-monitor/history.jsonl"));
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

// ── fichier ─────────────────────────────────────────────────────────────────

#[test]
fn append_read_since_and_missing_file() {
    let d = Dir::new();
    let h = History::new(d.file(), Fixed(500));
    assert!(h.read().is_empty() && h.read_since(0).is_empty());
    assert_eq!(h.rotate(10).unwrap(), 0);
    assert_eq!(h.mark_legacy_voltages().unwrap(), 0);
    assert_eq!(h.estimate_remaining(), None);
    assert!(h.append(80.0, Some(3.0)).unwrap());
    assert!(!h.append(180.0, None).unwrap());
    assert!(!h.append(80.0, Some(0.0)).unwrap());
    for ts in [100u64, 200, 300] {
        assert!(h.append_entry(&HistoryEntry::sample(ts, 50.0, None)).unwrap());
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
    assert!(old.exists(), "l'ancien fichier est conservé");
    assert_eq!(h.read().len(), 1);
    std::fs::write(&old, format!("{}{}", line(1, 10), line(2, 20))).unwrap();
    assert!(!h.migrate_from(&old).unwrap(), "ne recopie pas sur l'existant");
    assert_eq!(h.read().len(), 1);
    assert!(!History::new(d.0.join("other.jsonl"), Fixed(5)).migrate_from(&d.0.join("none")).unwrap());
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
    assert_eq!(h.rotate(RETENTION_S).unwrap(), 2, "les deux lignes invalides sont retirées");
    assert_eq!(h.read().len(), 10);
    let content = std::fs::read(d.file()).unwrap();
    assert!(!content.windows(2).any(|w| w == b"\xff\xfe"));
    assert_eq!(content.iter().filter(|b| **b == b'\n').count(), 10);
    let corrupt = std::fs::read(d.with_ext("jsonl.corrupt")).unwrap();
    assert_eq!(corrupt, b"\xff\xfe garbage\n{\"ts\":1,\"pct\":999}\n");
    assert_eq!(std::fs::read(d.with_ext("jsonl.prev")).unwrap(), data, ".prev = fichier d'avant");
    // Idempotent : plus rien à retirer, .corrupt inchangé.
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
        h.append_entry(&HistoryEntry::sample(ts, 50.0, None)).unwrap();
    }
    // Seuil = 10_000_000 (ancre : dernier ts borné par l'horloge, ici 9_999_999) - 1_000_000.
    assert_eq!(h.rotate(1_000_000).unwrap(), 1);
    let ts: Vec<u64> = h.read().iter().map(|e| e.ts).collect();
    assert_eq!(ts, vec![10, 9_000_000, 9_999_999]);
    // Rien d'ancien : retourne 0 et ne touche pas au fichier (pas de .prev supplémentaire requis).
    assert_eq!(h.rotate(1_000_000).unwrap(), 0);
    // Seuil exactement sur une entrée : conservée (>=).
    let h2 = History::new(d.0.join("b.jsonl"), Fixed(1_000));
    for ts in [100u64, 400, 900] {
        h2.append_entry(&HistoryEntry::sample(ts, 50.0, None)).unwrap();
    }
    assert_eq!(h2.rotate(500).unwrap(), 1); // cutoff = 900-500 = 400
    assert_eq!(h2.read().iter().map(|e| e.ts).collect::<Vec<_>>(), vec![400, 900]);
}

#[test]
fn clock_far_ahead_does_not_wipe_history() {
    let d = Dir::new();
    let h = History::new(d.file(), Fixed(4_000_000_000));
    for ts in [1_700_000_000u64, 1_700_000_100] {
        h.append_entry(&HistoryEntry::sample(ts, 50.0, None)).unwrap();
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
    assert_eq!(lines[1], b"\xff\xfe not json", "ligne corrompue copiée octet pour octet");
    assert_eq!(lines[2], b"{\"ts\":2,\"pct\":50,\"voltage\":3.0,\"schema\":2}");
    assert_eq!(lines[3], b"{\"ts\":3,\"pct\":50}");
    assert!(String::from_utf8_lossy(lines[0]).contains("\"voltage_valid\":false"));
    assert!(String::from_utf8_lossy(lines[4]).contains("\"voltage_valid\":false"));
    assert_eq!(std::fs::read(d.with_ext("jsonl.pre-schema2")).unwrap(), data);
    // Idempotent : rien à marquer, fichier et sauvegarde inchangés.
    assert_eq!(h.mark_legacy_voltages().unwrap(), 0);
    assert_eq!(std::fs::read(d.file()).unwrap(), out);
    assert_eq!(std::fs::read(d.with_ext("jsonl.pre-schema2")).unwrap(), data);
    assert!(!d.with_ext("jsonl.tmp").exists());
    assert_eq!(h.read().len(), 4);
}

#[test]
fn mark_legacy_backup_is_made_once() {
    let d = Dir::new();
    std::fs::write(d.file(), "{\"ts\":1,\"pct\":50,\"voltage\":0.79}\n").unwrap();
    std::fs::write(d.with_ext("jsonl.pre-schema2"), "ancienne sauvegarde\n").unwrap();
    let h = History::new(d.file(), Fixed(10));
    assert_eq!(h.mark_legacy_voltages().unwrap(), 1);
    assert_eq!(std::fs::read_to_string(d.with_ext("jsonl.pre-schema2")).unwrap(), "ancienne sauvegarde\n");
}

#[test]
fn unreadable_path_is_an_error_not_a_panic() {
    let d = Dir::new();
    // Le chemin est un dossier : ni NotFound ni lisible.
    let h = History::new(&d.0, Fixed(10));
    assert!(h.rotate(10).is_err());
    assert!(h.mark_legacy_voltages().is_err());
    assert!(h.read().is_empty());
}
