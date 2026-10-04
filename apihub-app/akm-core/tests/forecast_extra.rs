//! Numeric tests of `forecast`.

use akm_core::conv::{f64_from_u64, u64_from_f64_round};
use akm_core::forecast::{self, Unavailable, MIN_RATE_PCT_PER_DAY, MIN_SPAN_S, WINDOW_S};
use akm_core::history::{HistoryEntry, HistoryEvent};

const T0: u64 = 472_222 * 3600;
const H: u64 = 3600;
const DAY: u64 = 86_400;

fn e(ts: u64, pct: f64) -> HistoryEntry {
    HistoryEntry::sample(ts, pct, None)
}

fn line(from: u64, n: u64, step: u64, start: f64, rate: f64) -> Vec<HistoryEntry> {
    (0..n)
        .map(|i| {
            e(
                from + i * step,
                start - rate * f64_from_u64(i * step) / f64_from_u64(DAY),
            )
        })
        .collect()
}

fn close(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} != {b} (+-{tol})");
}

#[test]
fn exact_line_two_pct_per_day() {
    let h = line(T0, 120, H, 100.0, 2.0);
    let f = forecast::estimate(&h).unwrap();
    close(f.rate_pct_per_day, 2.0, 1e-6);
    let last = T0 + 119 * H;
    let fitted = 100.0 - 2.0 * f64_from_u64(119 * H) / f64_from_u64(DAY);
    close(f.fitted_pct, fitted, 1e-6);
    assert_eq!(f.span_s, 119 * H);
    assert_eq!(f.buckets, 120);
    let expect = last + u64_from_f64_round(fitted / 2.0 * f64_from_u64(DAY));
    assert!(
        f.empty_at.abs_diff(expect) <= 1,
        "{} vs {expect}",
        f.empty_at
    );
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
    let mut h = Vec::new();
    for i in 0..100u64 {
        for k in 0..3u64 {
            let ts = T0 + i * H + k * 600;
            h.push(e(
                ts,
                90.0 - 3.0 * f64_from_u64(ts - T0) / f64_from_u64(DAY),
            ));
        }
    }
    let f = forecast::estimate(&h).unwrap();
    close(f.rate_pct_per_day, 3.0, 1e-6);
    assert_eq!(f.buckets, 100);
    assert_eq!(f.span_s, 99 * H + 1200);
    close(
        f.fitted_pct,
        90.0 - 3.0 * f64_from_u64(99 * H + 1200) / f64_from_u64(DAY),
        1e-6,
    );
}

#[test]
fn span_and_bucket_thresholds() {
    let ok = line(T0, 3, 24 * H, 100.0, 1.0);
    let f = forecast::estimate(&ok).unwrap();
    assert_eq!(f.span_s, MIN_SPAN_S);
    close(f.rate_pct_per_day, 1.0, 1e-9);
    let short = vec![
        e(T0, 100.0),
        e(T0 + 24 * H, 99.0),
        e(T0 + MIN_SPAN_S - 1, 98.0),
    ];
    assert_eq!(forecast::estimate(&short), Err(Unavailable::NotEnoughData));
    let two = vec![e(T0, 100.0), e(T0 + MIN_SPAN_S, 98.0)];
    assert_eq!(forecast::estimate(&two), Err(Unavailable::NotEnoughData));
    assert_eq!(forecast::estimate(&[]), Err(Unavailable::NotEnoughData));
    assert_eq!(
        forecast::estimate(&[e(T0, 50.0)]),
        Err(Unavailable::NotEnoughData)
    );
    let same: Vec<_> = (0..5).map(|i| e(T0 + i, 50.0)).collect();
    assert_eq!(forecast::estimate(&same), Err(Unavailable::NotEnoughData));
}

#[allow(clippy::float_cmp)] // reason: exact values are the property under test
#[test]
fn discharge_rate_threshold() {
    assert_eq!(MIN_RATE_PCT_PER_DAY, 0.05);
    assert!(forecast::estimate(&line(T0, 100, H, 80.0, 0.06)).is_ok());
    assert_eq!(
        forecast::estimate(&line(T0, 100, H, 80.0, 0.04)),
        Err(Unavailable::NotDischarging)
    );
    assert_eq!(
        forecast::estimate(&line(T0, 100, H, 80.0, 0.0)),
        Err(Unavailable::NotDischarging)
    );
    assert_eq!(
        forecast::estimate(&line(T0, 100, H, 50.0, -1.0)),
        Err(Unavailable::NotDischarging)
    );
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
    let mut h = line(T0, 3, 24 * H, 100.0, 50.0);
    h[0].pct = 100.0;
    h[2].pct = 0.0;
    assert!(forecast::estimate(&h).is_ok());
}

#[test]
fn only_the_current_battery_set_and_window_count() {
    let last = T0 + 120 * H;
    let mut h = vec![
        e(last - WINDOW_S - DAY, 5.0),
        e(last - WINDOW_S - 2 * DAY, 1.0),
    ];
    h.extend(line(T0, 121, H, 100.0, 2.0));
    let f = forecast::estimate(&h).unwrap();
    close(f.rate_pct_per_day, 2.0, 1e-6);
    assert_eq!(f.span_s, 120 * H);
    let mut h = line(T0, 60, H, 100.0, 9.0);
    let mut ev = e(T0 + 60 * H, 100.0);
    ev.event = Some(HistoryEvent::BatteryReplaced);
    h.push(ev);
    h.extend((1..80u64).map(|i| {
        e(
            T0 + 60 * H + i * H,
            100.0 - 1.5 * f64_from_u64(i * H) / f64_from_u64(DAY),
        )
    }));
    let f = forecast::estimate(&h).unwrap();
    close(f.rate_pct_per_day, 1.5, 1e-6);
    assert_eq!(f.span_s, 79 * H);
}

#[test]
fn extreme_timestamps_neither_panic_nor_wrap() {
    let t = u64::MAX - 400_000;
    let h: Vec<_> = (0..5)
        .map(|i| e(t + i * 100_000, 90.0 - 20.0 * f64_from_u64(i)))
        .collect();
    assert!(forecast::estimate(&h).is_err());
    let h = line(T0, 100, H, 100.0, 0.0501);
    let f = forecast::estimate(&h).unwrap();
    assert!(f.empty_at > T0 + 100 * H, "empty_at {}", f.empty_at);
    assert!(f.empty_at < T0 + 100 * H + 2_000 * DAY);
}
