//! Propriétés de la logique pure : machine d'états de connexion, récupération,
//! alertes, chimie, historique / prévision (jamais NaN, ∞, négatif).

use akm_core::alerts::{AlertConfig, AlertState};
use akm_core::batteries;
use akm_core::chemistry::{assess, estimate_charge, Chemistry};
use akm_core::forecast;
use akm_core::history::{self, HistoryEntry};
use akm_core::machine::{Action, Event, Machine};
use akm_core::recovery::{Action as RAction, ConnectError, DisconnectReason, Recovery};
use proptest::prelude::*;
use std::time::{Duration, Instant};

// ── machine d'états ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Op {
    Ev(Event),
    Due,
    AcquireDone(bool),
    Force,
    Wait(u64),
}

fn mac() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["AA:AA".to_string(), "BB:BB".to_string(), "CC:CC".to_string()])
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => mac().prop_map(|m| Op::Ev(Event::Connected(m))),
        3 => mac().prop_map(|m| Op::Ev(Event::Disconnected(m))),
        1 => Just(Op::Ev(Event::BatterySignal)),
        1 => Just(Op::Ev(Event::NoBluez)),
        2 => prop::collection::vec(mac(), 0..4).prop_map(|v| Op::Ev(Event::Reconcile(v))),
        4 => Just(Op::Due),
        4 => any::<bool>().prop_map(Op::AcquireDone),
        1 => Just(Op::Force),
        3 => (0u64..20_000).prop_map(Op::Wait),
    ]
}

fn check_machine(m: &mut Machine, now: Instant) -> Result<(), TestCaseError> {
    if !m.is_connected() {
        prop_assert!(m.mac().is_none(), "déconnectée mais mac suivie");
        prop_assert!(m.standby().is_empty(), "déconnectée mais file d'attente non vide");
        prop_assert!(m.next_deadline().is_none(), "échéance sans connexion");
        prop_assert!(!m.is_acquired());
        prop_assert!(m.due(now).is_empty(), "action due sans connexion");
    } else {
        // jamais bloquée : une échéance existe toujours
        prop_assert!(m.next_deadline().is_some(), "connectée sans échéance (machine bloquée)");
    }
    if let Some(f) = m.mac() {
        prop_assert!(!m.standby().iter().any(|s| s == f), "suivie aussi en attente");
    }
    let st = m.standby();
    for (i, a) in st.iter().enumerate() {
        prop_assert!(!st[i + 1..].contains(a), "doublon en attente");
    }
    if m.is_acquired() {
        prop_assert!(m.is_connected());
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 3000, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn machine_never_reaches_an_impossible_state(ops in prop::collection::vec(op(), 1..80)) {
        let t0 = Instant::now();
        let mut now = t0;
        let mut m = Machine::new();
        for o in ops {
            match o {
                Op::Ev(e) => {
                    let a = m.on_event(&e, now);
                    if let Some(a) = a { prop_assert_eq!(a, Action::Clear); }
                }
                Op::Due => { let _ = m.due(now); }
                Op::AcquireDone(ok) => m.acquire_done(ok, now),
                Op::Force => m.force_refresh(now),
                Op::Wait(s) => now += Duration::from_secs(s),
            }
            check_machine(&mut m, now)?;
        }
    }

    /// Même chose avec un pilote qui exécute les actions comme le démon.
    #[test]
    fn machine_with_driver_never_issues_actions_when_disconnected(
        ops in prop::collection::vec(op(), 1..80), ok in prop::collection::vec(any::<bool>(), 80),
    ) {
        let mut now = Instant::now();
        let mut m = Machine::new();
        let mut k = 0usize;
        for o in ops {
            match o {
                Op::Ev(e) => { let _ = m.on_event(&e, now); }
                Op::Wait(s) => now += Duration::from_secs(s),
                Op::Force => m.force_refresh(now),
                _ => {}
            }
            let was_connected = m.is_connected();
            for a in m.due(now) {
                prop_assert!(was_connected, "action {a:?} alors que déconnectée");
                if a == Action::Acquire { m.acquire_done(ok[k % ok.len()], now); k += 1; }
            }
            check_machine(&mut m, now)?;
        }
    }

    // ── récupération de liaison ────────────────────────────────────────────
    #[test]
    fn recovery_never_panics_and_never_spams(ops in prop::collection::vec(0u8..12, 1..120), waits in prop::collection::vec(0u64..4000, 120)) {
        let mut now = Instant::now();
        let mut r = Recovery::new();
        let mut last_connect: Option<Instant> = None;
        for (i, o) in ops.into_iter().enumerate() {
            now += Duration::from_secs(waits[i]);
            match o {
                0 => r.start(true, true, now),
                1 => r.start(false, true, now),
                2 => r.on_connected(now),
                3 => r.on_disconnected(DisconnectReason::Timeout, now),
                4 => r.on_disconnected(DisconnectReason::Authentication, now),
                5 => r.on_disconnected(DisconnectReason::Remote, now),
                6 => r.on_connect_result(Ok(()), now),
                7 => r.on_connect_result(Err(ConnectError::classify("org.bluez.Error.InProgress", "busy")), now),
                8 => r.on_sleep(now),
                9 => r.on_resume(false, now),
                10 => r.on_adapter(false, now),
                _ => r.on_bond_lost(now),
            }
            for a in r.poll(now) {
                if a == RAction::Connect {
                    if let Some(p) = last_connect {
                        // MIN_SPACING = 20 s entre deux tentatives
                        prop_assert!(now.duration_since(p) >= Duration::from_secs(20) || now == p,
                            "deux Connect à moins de 20 s");
                    }
                    last_connect = Some(now);
                }
            }
        }
    }

    // ── alertes ───────────────────────────────────────────────────────────
    #[test]
    fn alerts_never_fire_twice_without_rearm(seq in prop::collection::vec(prop_oneof![Just(f64::NAN), Just(f64::INFINITY), -50.0f64..200.0], 1..200)) {
        let mut s = AlertState::new(AlertConfig::default());
        let mut fired: Vec<u8> = Vec::new();
        for p in seq {
            if let Some(c) = s.update(p) {
                prop_assert!(c.pct.is_finite());
                prop_assert!(!fired.contains(&c.threshold) || s.is_armed(c.threshold) == Some(false));
                fired.push(c.threshold);
            }
        }
    }

    // ── chimie ────────────────────────────────────────────────────────────
    #[test]
    fn chemistry_estimate_bounded_and_monotone(a in any::<u32>(), b in any::<u32>()) {
        for chem in [Chemistry::Alkaline, Chemistry::Nimh, Chemistry::Lithium, Chemistry::Unknown] {
            if let Some(e) = estimate_charge(a, chem) {
                prop_assert!(e.pct.is_finite() && (0.0..=100.0).contains(&e.pct));
                prop_assert!(e.low.is_finite() && e.high.is_finite());
                prop_assert!(e.low >= 0.0 && e.high <= 100.0 && e.low <= e.pct && e.pct <= e.high);
            }
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            if let (Some(l), Some(h)) = (estimate_charge(lo, chem), estimate_charge(hi, chem)) {
                prop_assert!(l.pct <= h.pct, "{chem:?}: f({lo})={} > f({hi})={}", l.pct, h.pct);
            }
            let _ = assess(Some(a), Some(b), chem, None);
        }
    }

    #[test]
    fn chemistry_parse_never_panics(s in any::<String>()) {
        let _ = Chemistry::parse(&s);
    }

    // ── historique / prévision : jamais NaN, ∞, négatif ──────────────────
    #[test]
    fn history_estimate_remaining_is_finite(entries in prop::collection::vec(entry(), 0..60)) {
        if let Some((rate, hours)) = history::estimate_remaining(&entries) {
            prop_assert!(rate.is_finite() && rate >= 0.0, "taux {rate}");
            prop_assert!(hours.is_finite() && hours >= 0.0, "heures {hours}");
            let _ = history::format_remaining(rate, hours);
        }
    }

    #[test]
    fn forecast_is_finite_and_does_not_overflow(entries in prop::collection::vec(entry(), 0..60)) {
        if let Ok(f) = forecast::estimate(&entries) {
            prop_assert!(f.rate_pct_per_day.is_finite() && f.rate_pct_per_day > 0.0);
            prop_assert!(f.fitted_pct.is_finite() && (0.0..=100.0).contains(&f.fitted_pct));
            prop_assert!(f.remaining_days(0).is_finite());
            let _ = forecast::format_days(f.remaining_s(0));
        }
    }

    #[test]
    fn batteries_and_format_never_panic(entries in prop::collection::vec(entry(), 0..60)) {
        let _ = batteries::replacements(&entries);
        let _ = batteries::battery_sets(&entries);
        let _ = batteries::current_set_start(&entries);
        for e in &entries { let _ = history::format_utc(e.ts); }
    }

    /// Historique plausible (ts réalistes) : la prévision reste cohérente.
    #[test]
    fn forecast_realistic_history(n in 3usize..200, start in 1_600_000_000u64..1_900_000_000, step in 600u64..20_000, drop in 0.0f64..0.5) {
        let entries: Vec<HistoryEntry> = (0..n).map(|i| HistoryEntry::sample(start + i as u64 * step, (100.0 - i as f64 * drop).max(0.0), Some(3.0))).collect();
        if let Ok(f) = forecast::estimate(&entries) {
            prop_assert!(f.empty_at >= start);
            prop_assert!(f.rate_pct_per_day > 0.0 && f.rate_pct_per_day.is_finite());
        }
    }
}

fn entry() -> impl Strategy<Value = HistoryEntry> {
    let ts = prop_oneof![
        3 => 1_700_000_000u64..1_800_000_000,
        1 => 0u64..100_000,
        1 => (u64::MAX - 10_000_000)..=u64::MAX,
    ];
    let pct = prop_oneof![
        4 => 0.0f64..100.0, 1 => Just(f64::NAN), 1 => Just(f64::INFINITY), 1 => -1e308f64..1e308,
    ];
    let volt = prop_oneof![
        2 => Just(None),
        3 => (0.0f64..5.0).prop_map(Some),
        1 => (-1e308f64..1e308).prop_map(Some),
        1 => Just(Some(f64::NAN)),
    ];
    (ts, pct, volt).prop_map(|(ts, pct, v)| HistoryEntry::sample(ts, pct, v))
}

// ── reproducteurs déterministes des défauts confirmés ─────────────────────

/// `forecast::estimate` : `last_ts + secondes` déborde (panique en debug, wrap en
/// release) quand le dernier horodatage d'un historique est proche de u64::MAX.
#[test]
fn forecast_does_not_overflow_on_extreme_timestamps() {
    let t = u64::MAX - 400_000;
    let e: Vec<HistoryEntry> = (0..5)
        .map(|i| HistoryEntry::sample(t + i * 100_000, 90.0 - 20.0 * i as f64, None))
        .collect();
    let r = std::panic::catch_unwind(|| forecast::estimate(&e));
    assert!(r.is_ok(), "forecast::estimate a paniqué (add with overflow)");
}

/// `history::parse` n'applique pas `valid_sample` : une ligne avec une tension
/// absurde donne une autonomie infinie.
#[test]
fn estimate_remaining_is_finite_on_absurd_voltages_from_parse() {
    let content = "{\"ts\":1000000,\"pct\":50,\"voltage\":1e306,\"schema\":2}\n\
                   {\"ts\":1010000,\"pct\":49,\"voltage\":1.0,\"schema\":2}\n";
    let entries = history::parse(content);
    if let Some((rate, hours)) = history::estimate_remaining(&entries) {
        assert!(rate.is_finite() && hours.is_finite(), "taux={rate} heures={hours}");
    }
}

/// Un seul octet non UTF-8 dans history.jsonl : tout l'historique devient
/// invisible (`read_to_string` échoue) et `rotate()` renvoie une erreur à chaque
/// appel, le fichier grossit sans fin.
#[test]
fn history_survives_one_invalid_utf8_byte() {
    use akm_core::history::{Clock, History};
    struct C;
    impl Clock for C {
        fn now(&self) -> u64 {
            2_000_000
        }
    }
    let dir = std::env::temp_dir().join(format!("akm-audit-utf8-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("history.jsonl");
    let mut data = Vec::new();
    for i in 0..10u64 {
        data.extend_from_slice(format!("{{\"ts\":{},\"pct\":{}}}\n", 1_000_000 + i * 60, 90 - i).as_bytes());
    }
    data.extend_from_slice(b"\xff\xfe garbage\n"); // une ligne corrompue
    std::fs::write(&p, &data).unwrap();
    let h = History::new(&p, C);
    let n = h.read().len();
    let rot = h.rotate(10);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(n, 10, "les 10 lignes saines doivent rester lisibles (lues: {n})");
    assert!(rot.is_ok(), "rotate() échoue sur un fichier avec un octet invalide: {rot:?}");
}
