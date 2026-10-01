//! Tests ajoutés pour tuer les mutants SURVIVANTS de `cargo mutants` sur
//! read_policy.rs / forecast.rs / history.rs (voir docs/AUDIT-OUTILLE.md §6).
//! Les valeurs attendues sont recalculées à la main, pas copiées du code.

use akm_core::decode::HidSource;
use akm_core::forecast;
use akm_core::history::{self, HistoryEntry};
use akm_core::read_policy::{
    self, gate, last_input_age, note_input, read_safe, try_lock, Gate, SafeRead, SafeSource,
    ACTIVE_WINDOW, MIN_GAP,
};
use akm_core::report::KbReport;
use std::cell::RefCell;
use std::io;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

/// Source qui note l'instant de chaque requête et répond depuis un barème.
struct Rec {
    calls: RefCell<Vec<(u8, Instant)>>,
    delay: Duration,
    answers: Vec<(u8, Vec<u8>)>,
}
impl HidSource for Rec {
    fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
        self.calls.borrow_mut().push((id, Instant::now()));
        std::thread::sleep(self.delay);
        self.answers
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}
fn rec(delay_ms: u64, answers: Vec<(u8, Vec<u8>)>) -> Rec {
    Rec { calls: RefCell::new(vec![]), delay: Duration::from_millis(delay_ms), answers }
}

#[test]
fn safe_source_refuses_outside_ids_without_io_and_spaces_requests() {
    let inner = rec(0, vec![(0x47, vec![0x47, 50]), (0x46, vec![0x46, 0xA0, 0x0B]), (0x4C, vec![0x4C; 9])]);
    let s = SafeSource::new(&inner);
    for bad in [0x4Cu8, 0xEA, 0x5A, 0xFF, 0x00, 0x30] {
        let e = s.feature(bad).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    }
    assert_eq!(s.sent(), 0, "aucune requête ne doit partir");
    assert!(inner.calls.borrow().is_empty(), "le périphérique n'a pas à être touché");
    s.feature(0x47).unwrap();
    s.feature(0x46).unwrap();
    assert_eq!(s.sent(), 2);
    let c = inner.calls.borrow();
    let gap = c[1].1.duration_since(c[0].1);
    assert!(gap >= MIN_GAP - Duration::from_millis(15), "espacement {gap:?} < MIN_GAP");
    assert!(gap < MIN_GAP + Duration::from_millis(400), "espacement excessif {gap:?}");
}

#[test]
fn read_safe_decodes_and_bounds_values() {
    // 0x47 = 80 %, 0x46 = 0x0BA0 = 2976 mV (LE), 0x49 absent -> Partial
    let src = rec(0, vec![(0x47, vec![0x47, 80]), (0x46, vec![0x46, 0xA0, 0x0B])]);
    let mut r = KbReport::default();
    assert_eq!(read_safe(&src, &mut r), SafeRead::Partial);
    assert_eq!(r.battery.percentage, Some(80.0));
    assert_eq!(r.battery.voltage, Some(2.976));
    assert_eq!(r.raw.get("0x47").map(String::as_str), Some("50"));
    assert!(r.incomplete);
    // 101 % refusé, 1499 mV refusé (borne basse 1500), 3701 mV refusé (borne haute 3700)
    for (pct, mv, ok_v) in [(101u8, 1499u16, false), (100, 3701, false), (100, 1500, true), (0, 3700, true)] {
        let src = rec(0, vec![
            (0x47, vec![0x47, pct]),
            (0x46, vec![0x46, (mv & 0xFF) as u8, (mv >> 8) as u8]),
            (0x49, vec![0x49, 1, 1]),
        ]);
        let mut r = KbReport::default();
        assert_eq!(read_safe(&src, &mut r), SafeRead::Complete);
        assert_eq!(r.battery.percentage.is_some(), pct <= 100, "pct {pct}");
        assert_eq!(r.battery.voltage.is_some(), ok_v, "mv {mv}");
        assert!(!r.incomplete);
    }
}

#[test]
fn read_safe_stops_when_the_budget_is_spent() {
    // chaque requête dure 1,1 s : après 0x47 (1,1 s) + 0x46 (>= 2,2 s) le budget de 2 s
    // est dépassé, 0x49 ne doit PAS être demandé.
    let src = rec(1100, vec![(0x47, vec![0x47, 50]), (0x46, vec![0x46, 0xA0, 0x0B]), (0x49, vec![0x49, 0xA0, 0x0B])]);
    let mut r = KbReport::default();
    assert_eq!(read_safe(&src, &mut r), SafeRead::Partial);
    let ids: Vec<u8> = src.calls.borrow().iter().map(|c| c.0).collect();
    assert_eq!(ids, vec![0x47, 0x46], "0x49 demandé hors budget");
}

#[test]
fn activity_gate_follows_the_last_input_report() {
    note_input();
    let now = Instant::now();
    let age = last_input_age(now).expect("entrée notée");
    assert!(age < Duration::from_secs(2), "âge {age:?}");
    // 30 s plus tard : encore actif ; 61 s plus tard : inactif
    let a30 = last_input_age(now + Duration::from_secs(30)).unwrap();
    assert!(a30 >= Duration::from_secs(29) && a30 < Duration::from_secs(33), "{a30:?}");
    assert_eq!(gate(Some(a30)), Gate::Allowed);
    let a61 = last_input_age(now + Duration::from_secs(61)).unwrap();
    assert!(a61 >= Duration::from_secs(60), "{a61:?}");
    assert_eq!(gate(Some(a61)), Gate::Idle);
    assert_eq!(gate(Some(ACTIVE_WINDOW)), Gate::Idle, "borne exclue");
    assert_eq!(gate(None), Gate::Idle);
}

/// Un seul lecteur : verrou inter-processus (`flock`) respecté, puis libéré.
#[test]
fn try_lock_is_exclusive_across_file_descriptors_and_released() {
    let dir = std::env::temp_dir().join(format!("akm-audit-lock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", &dir);
    let p = read_policy::lock_path();
    assert_eq!(p, dir.join("apple-kb-monitor").join("hid.lock"));
    // sans concurrent : obtenu
    let l = try_lock(Duration::from_millis(100)).expect("verrou libre");
    drop(l);
    // un autre processus (simulé par un autre fd) tient le verrou : refus
    let other = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
    assert_eq!(unsafe { libc::flock(other.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0);
    let t = Instant::now();
    assert!(try_lock(Duration::from_millis(120)).is_none(), "verrou pris par un autre lecteur ignoré");
    let waited = t.elapsed();
    assert!(waited >= Duration::from_millis(100) && waited < Duration::from_secs(2), "attente {waited:?}");
    // libéré : de nouveau disponible
    assert_eq!(unsafe { libc::flock(other.as_raw_fd(), libc::LOCK_UN) }, 0);
    assert!(try_lock(Duration::from_millis(100)).is_some());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn estimate_remaining_known_values() {
    // 3.000 V -> 2.900 V en 100 h : 1 mV/h ; reste (2.9-2.0)*1000/1 = 900 h
    let e = vec![
        HistoryEntry::sample(1_000_000, 90.0, Some(3.0)),
        HistoryEntry::sample(1_000_000 + 100 * 3600, 80.0, Some(2.9)),
    ];
    let (rate, hours) = history::estimate_remaining(&e).unwrap();
    assert!((rate - 1.0).abs() < 1e-6, "taux {rate}");
    assert!((hours - 900.0).abs() < 1e-3, "heures {hours}");
    // pas de décharge -> None ; moins de 0,01 h -> None
    // ordre inversé (la tension remonte) : pas de décharge
    assert!(history::estimate_remaining(&[e[1].clone(), e[0].clone()]).is_none());
    let flat = vec![HistoryEntry::sample(0, 90.0, Some(3.0)), HistoryEntry::sample(7200, 90.0, Some(3.0))];
    assert!(history::estimate_remaining(&flat).is_none());
    let short = vec![HistoryEntry::sample(0, 90.0, Some(3.0)), HistoryEntry::sample(30, 80.0, Some(2.0))];
    assert!(history::estimate_remaining(&short).is_none());
    // sous 2.0 V : reste 0
    let low = vec![HistoryEntry::sample(0, 10.0, Some(2.5)), HistoryEntry::sample(7200, 5.0, Some(1.9))];
    assert_eq!(history::estimate_remaining(&low).unwrap().1, 0.0);
}

#[test]
fn format_utc_known_dates() {
    assert_eq!(history::format_utc(0), "1970-01-01 00:00");
    assert_eq!(history::format_utc(951_782_400 + 3600 * 13 + 60 * 7), "2000-02-29 13:07");
    assert_eq!(history::format_utc(1_767_225_599), "2025-12-31 23:59");
    assert_eq!(history::format_utc(1_767_225_600), "2026-01-01 00:00");
}

#[test]
fn forecast_linear_discharge_known_values() {
    // 100 % -> 70 % en 30 jours, un point toutes les 6 h : 1 %/jour, vide 70 jours après le dernier point
    let start = 1_700_000_000u64;
    let n = 30 * 4;
    let e: Vec<HistoryEntry> = (0..=n)
        .map(|i| HistoryEntry::sample(start + i as u64 * 6 * 3600, 100.0 - i as f64 * 30.0 / n as f64, None))
        .collect();
    let f = forecast::estimate(&e).expect("prévision");
    assert!((f.rate_pct_per_day - 1.0).abs() < 0.02, "taux {}", f.rate_pct_per_day);
    assert!((f.fitted_pct - 70.0).abs() < 1.0, "ajusté {}", f.fitted_pct);
    let last = start + n as u64 * 6 * 3600;
    let days = (f.empty_at - last) as f64 / 86_400.0;
    assert!((days - 70.0).abs() < 2.0, "jours restants {days}");
    assert!(f.span_s >= 29 * 86_400 && f.span_s <= 30 * 86_400, "{}", f.span_s);
    // pas assez de données (< 48 h) / pas de décharge
    assert_eq!(forecast::estimate(&e[..4]).unwrap_err(), forecast::Unavailable::NotEnoughData);
    let flat: Vec<HistoryEntry> = (0..100).map(|i| HistoryEntry::sample(start + i * 3600, 80.0, None)).collect();
    assert_eq!(forecast::estimate(&flat).unwrap_err(), forecast::Unavailable::NotDischarging);
}
