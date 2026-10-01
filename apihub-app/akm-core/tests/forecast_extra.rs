//! Tests numériques de `forecast` : valeurs exactes recalculées à la main
//! sur des droites parfaites, bornes de décision, entrées extrêmes (#223).

use akm_core::forecast::{self, Unavailable, MIN_RATE_PCT_PER_DAY, MIN_SPAN_S, WINDOW_S};
use akm_core::history::{HistoryEntry, HistoryEvent};

/// Multiple de 3600 : un échantillon par heure tombe dans son propre seau.
const T0: u64 = 472_222 * 3600;
const H: u64 = 3600;
const DAY: u64 = 86_400;

fn e(ts: u64, pct: f64) -> HistoryEntry {
    HistoryEntry::sample(ts, pct, None)
}

/// pct = start - rate * (ts - T0) / jour, un point toutes les `step` s.
fn line(from: u64, n: u64, step: u64, start: f64, rate: f64) -> Vec<HistoryEntry> {
    (0..n)
        .map(|i| e(from + i * step, start - rate * (i * step) as f64 / DAY as f64))
        .collect()
}

fn close(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} != {b} (+-{tol})");
}

#[test]
fn exact_line_two_pct_per_day() {
    // 120 h, 120 points, 100 % -> 90.083 %, 2 %/jour.
    let h = line(T0, 120, H, 100.0, 2.0);
    let f = forecast::estimate(&h).unwrap();
    close(f.rate_pct_per_day, 2.0, 1e-6);
    let last = T0 + 119 * H;
    let fitted = 100.0 - 2.0 * (119 * H) as f64 / DAY as f64;
    close(f.fitted_pct, fitted, 1e-6);
    assert_eq!(f.span_s, 119 * H);
    assert_eq!(f.buckets, 120);
    let expect = last + (fitted / 2.0 * DAY as f64).round() as u64;
    assert!(f.empty_at.abs_diff(expect) <= 1, "{} vs {expect}", f.empty_at);
    // Restant : à `last`, (fitted / 2) jours.
    close(f.remaining_days(last), fitted / 2.0, 1e-4);
    assert_eq!(f.remaining_s(f.empty_at), 0);
    assert_eq!(f.remaining_s(f.empty_at + 10), 0);
    assert_eq!(f.remaining_s(f.empty_at - 10), 10);
}

#[test]
fn input_order_does_not_matter() {
    let mut h = line(T0, 120, H, 100.0, 2.0);
    let a = forecast::estimate(&h).unwrap();
    h.reverse();
    assert_eq!(forecast::estimate(&h).unwrap(), a);
}

#[test]
fn hourly_averaging_keeps_the_exact_slope() {
    // 3 points par heure (+0, +600, +1200 s) sur 100 h : la moyenne horaire d'une
    // droite est sur la droite, la pente reste exacte et il y a 100 seaux.
    let mut h = Vec::new();
    for i in 0..100u64 {
        for k in 0..3u64 {
            let ts = T0 + i * H + k * 600;
            h.push(e(ts, 90.0 - 3.0 * (ts - T0) as f64 / DAY as f64));
        }
    }
    let f = forecast::estimate(&h).unwrap();
    close(f.rate_pct_per_day, 3.0, 1e-6);
    assert_eq!(f.buckets, 100);
    assert_eq!(f.span_s, 99 * H + 1200);
    // Ajustement au dernier échantillon.
    close(f.fitted_pct, 90.0 - 3.0 * (99 * H + 1200) as f64 / DAY as f64, 1e-6);
}

#[test]
fn span_and_bucket_thresholds() {
    // Exactement 48 h, 3 seaux : accepté.
    let ok = line(T0, 3, 24 * H, 100.0, 1.0);
    let f = forecast::estimate(&ok).unwrap();
    assert_eq!(f.span_s, MIN_SPAN_S);
    close(f.rate_pct_per_day, 1.0, 1e-9);
    // 47 h 59 min 59 s : refusé.
    let short = vec![e(T0, 100.0), e(T0 + 24 * H, 99.0), e(T0 + MIN_SPAN_S - 1, 98.0)];
    assert_eq!(forecast::estimate(&short), Err(Unavailable::NotEnoughData));
    // 48 h mais 2 seaux seulement.
    let two = vec![e(T0, 100.0), e(T0 + MIN_SPAN_S, 98.0)];
    assert_eq!(forecast::estimate(&two), Err(Unavailable::NotEnoughData));
    assert_eq!(forecast::estimate(&[]), Err(Unavailable::NotEnoughData));
    assert_eq!(forecast::estimate(&[e(T0, 50.0)]), Err(Unavailable::NotEnoughData));
    // Tous les points dans le même seau : variance de temps nulle.
    let same: Vec<_> = (0..5).map(|i| e(T0 + i, 50.0)).collect();
    assert_eq!(forecast::estimate(&same), Err(Unavailable::NotEnoughData));
}

#[test]
fn discharge_rate_threshold() {
    assert_eq!(MIN_RATE_PCT_PER_DAY, 0.05);
    assert!(forecast::estimate(&line(T0, 100, H, 80.0, 0.06)).is_ok());
    assert_eq!(forecast::estimate(&line(T0, 100, H, 80.0, 0.04)), Err(Unavailable::NotDischarging));
    assert_eq!(forecast::estimate(&line(T0, 100, H, 80.0, 0.0)), Err(Unavailable::NotDischarging));
    // Pourcentage qui monte (charge).
    assert_eq!(forecast::estimate(&line(T0, 100, H, 50.0, -1.0)), Err(Unavailable::NotDischarging));
}

#[test]
fn invalid_percentages_are_ignored() {
    let mut h = line(T0, 120, H, 100.0, 2.0);
    h.push(e(T0 + 50 * H + 5, f64::NAN));
    h.push(e(T0 + 51 * H + 5, -5.0));
    h.push(e(T0 + 52 * H + 5, 100.5));
    h.push(e(T0 + 53 * H + 5, f64::INFINITY));
    let f = forecast::estimate(&h).unwrap();
    close(f.rate_pct_per_day, 2.0, 1e-6);
    assert_eq!(f.buckets, 120);
    // Bornes incluses.
    let mut h = line(T0, 3, 24 * H, 100.0, 50.0);
    h[0].pct = 100.0;
    h[2].pct = 0.0;
    assert!(forecast::estimate(&h).is_ok());
}

#[test]
fn only_the_current_battery_set_and_window_count() {
    // Vieux échantillons hors fenêtre (31 j avant le dernier) très différents.
    let last = T0 + 120 * H;
    let mut h = vec![e(last - WINDOW_S - DAY, 5.0), e(last - WINDOW_S - 2 * DAY, 1.0)];
    h.extend(line(T0, 121, H, 100.0, 2.0));
    let f = forecast::estimate(&h).unwrap();
    close(f.rate_pct_per_day, 2.0, 1e-6);
    assert_eq!(f.span_s, 120 * H);
    // Remplacement de piles au milieu : seule la suite compte.
    let mut h = line(T0, 60, H, 100.0, 9.0);
    let mut ev = e(T0 + 60 * H, 100.0);
    ev.event = Some(HistoryEvent::BatteryReplaced);
    h.push(ev);
    h.extend((1..80u64).map(|i| e(T0 + 60 * H + i * H, 100.0 - 1.5 * (i * H) as f64 / DAY as f64)));
    let f = forecast::estimate(&h).unwrap();
    close(f.rate_pct_per_day, 1.5, 1e-6);
    assert_eq!(f.span_s, 79 * H);
}

#[test]
fn extreme_timestamps_neither_panic_nor_wrap() {
    // Dernier ts près de u64::MAX : ignoré (hors bornes), pas de débordement.
    let t = u64::MAX - 400_000;
    let h: Vec<_> = (0..5).map(|i| e(t + i * 100_000, 90.0 - 20.0 * i as f64)).collect();
    assert!(forecast::estimate(&h).is_err());
    // Ts plausibles mais pente minuscule et pct élevé : horizon borné, sans wrap.
    let h = line(T0, 100, H, 100.0, 0.0501);
    let f = forecast::estimate(&h).unwrap();
    assert!(f.empty_at > T0 + 100 * H, "empty_at {}", f.empty_at);
    assert!(f.empty_at < T0 + 100 * H + 2_000 * DAY);
}

#[test]
fn format_days_text() {
    assert_eq!(forecast::format_days(0), "\u{2248} 0 h left");
    assert_eq!(forecast::format_days(20 * H), "\u{2248} 20 h left");
    assert_eq!(forecast::format_days(2 * DAY - 1), "\u{2248} 48 h left");
    assert_eq!(forecast::format_days(2 * DAY), "\u{2248} 2 days left");
    assert_eq!(forecast::format_days(41 * DAY), "\u{2248} 41 days left");
}
