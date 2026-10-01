//! Battery autonomy forecast in days (#83, F02).
//!
//! Model: least-squares line of the battery percentage against time, on the
//! samples of the **current set of batteries** (after the last replacement,
//! see [`crate::batteries`]) within the last [`WINDOW_S`]. Samples are first
//! averaged per hour so that a day of typing (one sample every 5 min) does
//! not outweigh the nights the keyboard was off, and the 1 % quantisation of
//! the kernel value averages out. Below [`MIN_SPAN_S`] of data, or without a
//! measurable discharge, the estimate is unavailable (never invented).

use serde::{Deserialize, Serialize};

use crate::batteries;
use crate::history::HistoryEntry;

/// Minimum time covered by the samples (48 h).
pub const MIN_SPAN_S: u64 = 48 * 3600;
/// Samples older than this (relative to the last one) are ignored.
pub const WINDOW_S: u64 = 30 * 24 * 3600;
/// Below this discharge rate (%/day) the batteries are considered flat-lined:
/// no estimate (it would be years).
pub const MIN_RATE_PCT_PER_DAY: f64 = 0.05;
/// Minimum number of distinct hourly buckets.
const MIN_BUCKETS: usize = 3;
const BUCKET_S: u64 = 3600;
/// Longest horizon of an estimate (s).
const MAX_HORIZON_S: f64 = 1e10;

/// Result of a successful estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    /// Discharge rate, in points of % per day (> 0).
    pub rate_pct_per_day: f64,
    /// Unix time at which the fitted line reaches 0 %.
    pub empty_at: u64,
    /// Battery % given by the fitted line at the last sample.
    pub fitted_pct: f64,
    /// Time covered by the samples used (s).
    pub span_s: u64,
    /// Hourly buckets used for the fit.
    pub buckets: usize,
}

impl Forecast {
    /// Seconds left at unix time `now` (0 once past `empty_at`).
    pub fn remaining_s(&self, now: u64) -> u64 {
        self.empty_at.saturating_sub(now)
    }
    /// Days left at unix time `now`.
    pub fn remaining_days(&self, now: u64) -> f64 {
        self.remaining_s(now) as f64 / 86_400.0
    }
}

/// Why there is no estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// Less than [`MIN_SPAN_S`] of data on the current batteries.
    NotEnoughData,
    /// The percentage does not go down (measurably).
    NotDischarging,
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Unavailable::NotEnoughData => "estimate unavailable (less than 48 h of data)",
            Unavailable::NotDischarging => "estimate unavailable (no measurable discharge)",
        })
    }
}

/// Estimate from a history (any order, any length).
pub fn estimate(entries: &[HistoryEntry]) -> Result<Forecast, Unavailable> {
    let mut sorted: Vec<&HistoryEntry> = entries
        .iter()
        .filter(|e| {
            e.pct.is_finite() && (0.0..=100.0).contains(&e.pct) && e.ts <= crate::history::MAX_TS
        })
        .collect();
    sorted.sort_by_key(|e| e.ts);
    let Some(last) = sorted.last() else {
        return Err(Unavailable::NotEnoughData);
    };
    let last_ts = last.ts;
    // Current set of batteries only.
    let set_start = batteries::current_set_start(entries).unwrap_or(0);
    let from = last_ts.saturating_sub(WINDOW_S).max(set_start);
    let used: Vec<&HistoryEntry> = sorted.into_iter().filter(|e| e.ts >= from).collect();
    let first_ts = used.first().map_or(last_ts, |e| e.ts);
    let span_s = last_ts - first_ts;
    if span_s < MIN_SPAN_S {
        return Err(Unavailable::NotEnoughData);
    }

    // Hourly means.
    let mut buckets: Vec<(f64, f64, u32)> = Vec::new(); // (sum t, sum pct, n)
    let mut cur_key = None;
    for e in &used {
        let key = e.ts / BUCKET_S;
        if cur_key != Some(key) {
            cur_key = Some(key);
            buckets.push((0.0, 0.0, 0));
        }
        let b = buckets.last_mut().expect("just pushed");
        b.0 += (e.ts - first_ts) as f64;
        b.1 += e.pct;
        b.2 += 1;
    }
    if buckets.len() < MIN_BUCKETS {
        return Err(Unavailable::NotEnoughData);
    }
    let pts: Vec<(f64, f64)> = buckets
        .iter()
        .map(|&(st, sp, n)| (st / f64::from(n), sp / f64::from(n)))
        .collect();
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    if sxx <= 0.0 {
        return Err(Unavailable::NotEnoughData);
    }
    let slope = sxy / sxx; // % per second
    let rate = -slope * 86_400.0;
    if !rate.is_finite() || rate < MIN_RATE_PCT_PER_DAY {
        return Err(Unavailable::NotDischarging);
    }
    let fitted = (my + slope * (span_s as f64 - mx)).clamp(0.0, 100.0);
    // Bounded (~317 years) and saturating: a corrupt timestamp near u64::MAX
    // must not wrap `empty_at` to a date in 1970 ("empty now", #223).
    let secs_left = (fitted / rate * 86_400.0).clamp(0.0, MAX_HORIZON_S);
    let empty_at = last_ts.saturating_add(secs_left.round() as u64);
    Ok(Forecast {
        rate_pct_per_day: rate,
        empty_at,
        fitted_pct: fitted,
        span_s,
        buckets: pts.len(),
    })
}

/// Short human text: `"≈ 41 days left"`, `"≈ 20 h left"`.
pub fn format_days(remaining_s: u64) -> String {
    let days = remaining_s as f64 / 86_400.0;
    if days >= 2.0 {
        format!("\u{2248} {days:.0} days left")
    } else {
        format!("\u{2248} {:.0} h left", remaining_s as f64 / 3600.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{parse, HistoryEvent};

    const T0: u64 = 1_790_000_000;
    const DAY: u64 = 86_400;

    fn entry(ts: u64, pct: f64) -> HistoryEntry {
        HistoryEntry::sample(ts, pct, None)
    }

    /// 1 %/day from 80 %, kernel-quantised to whole %, sampled every 5 min
    /// only while "in use" (9:00-18:00), plus deterministic ±0.6 noise.
    fn synthetic(days: u64, rate_per_day: f64) -> Vec<HistoryEntry> {
        let mut v = Vec::new();
        let mut seed: u32 = 12345;
        for d in 0..days {
            for m in (9 * 60..18 * 60).step_by(5) {
                let ts = T0 + d * DAY + m * 60;
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let noise = (f64::from(seed >> 16 & 0x7fff) / 32767.0 - 0.5) * 1.2;
                let real = 80.0 - rate_per_day * (ts - T0) as f64 / DAY as f64;
                v.push(entry(ts, (real + noise).floor().clamp(0.0, 100.0)));
            }
        }
        v
    }

    #[test]
    fn constant_slope_one_pct_per_day_is_within_ten_percent() {
        let h = synthetic(10, 1.0);
        let f = estimate(&h).expect("estimate");
        assert!(
            (f.rate_pct_per_day - 1.0).abs() <= 0.1,
            "rate {}",
            f.rate_pct_per_day
        );
        let last = h.last().unwrap();
        // truth: 80 - 10 days ≈ 70.6 % left at 1 %/day ≈ 70.6 days
        let truth_days = (80.0 - (last.ts - T0) as f64 / DAY as f64) / 1.0;
        let got = f.remaining_days(last.ts);
        assert!(
            (got - truth_days).abs() / truth_days <= 0.10,
            "got {got} days, truth {truth_days}"
        );
        assert!(f.span_s >= MIN_SPAN_S && f.buckets >= 50);
    }

    #[test]
    fn other_rates_and_shuffled_input() {
        let mut h = synthetic(6, 3.0);
        h.reverse();
        let f = estimate(&h).unwrap();
        assert!((f.rate_pct_per_day - 3.0).abs() <= 0.3, "{f:?}");
    }

    #[test]
    fn less_than_48h_is_unavailable() {
        let h: Vec<_> = (0..(47 * 12))
            .map(|i| entry(T0 + i * 300, 90.0 - i as f64 * 0.01))
            .collect();
        assert_eq!(estimate(&h), Err(Unavailable::NotEnoughData));
        assert_eq!(estimate(&[]), Err(Unavailable::NotEnoughData));
        assert!(Unavailable::NotEnoughData.to_string().contains("48 h"));
    }

    #[test]
    fn flat_or_rising_is_not_discharging() {
        let h: Vec<_> = (0..10).map(|d| entry(T0 + d * DAY, 77.0)).collect();
        assert_eq!(estimate(&h), Err(Unavailable::NotDischarging));
        let h: Vec<_> = (0..10)
            .map(|d| entry(T0 + d * DAY, 50.0 + d as f64))
            .collect();
        assert_eq!(estimate(&h), Err(Unavailable::NotDischarging));
    }

    #[test]
    fn only_the_current_batteries_are_used() {
        // Old set: 10 days at 5 %/day, then replaced (marker), then 3 days at 1 %/day.
        let mut h: Vec<_> = (0..10 * 24)
            .map(|i| entry(T0 + i * 3600, 60.0 - i as f64 * 5.0 / 24.0))
            .collect();
        let t1 = T0 + 11 * DAY;
        let mut marker = entry(t1, 100.0);
        marker.event = Some(HistoryEvent::BatteryReplaced);
        h.push(marker);
        h.extend((1..3 * 24).map(|i| entry(t1 + i * 3600, 100.0 - i as f64 / 24.0)));
        let f = estimate(&h).unwrap();
        assert!((f.rate_pct_per_day - 1.0).abs() < 0.05, "{f:?}");
    }

    #[test]
    fn fixture_file_is_estimated() {
        let h = parse(include_str!("../tests/fixtures/history_1pct_day.jsonl"));
        assert!(h.len() > 100);
        let f = estimate(&h).unwrap();
        assert!((f.rate_pct_per_day - 1.0).abs() <= 0.1, "{f:?}");
    }

    #[test]
    fn remaining_and_format() {
        let f = Forecast {
            rate_pct_per_day: 1.0,
            empty_at: T0 + 41 * DAY,
            fitted_pct: 41.0,
            span_s: 0,
            buckets: 0,
        };
        assert_eq!(f.remaining_s(T0), 41 * DAY);
        assert_eq!(f.remaining_s(T0 + 50 * DAY), 0);
        assert_eq!(format_days(41 * DAY), "\u{2248} 41 days left");
        assert_eq!(format_days(20 * 3600), "\u{2248} 20 h left");
    }
}
