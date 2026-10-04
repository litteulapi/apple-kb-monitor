//! Battery autonomy forecast in days (F02).

use serde::{Deserialize, Serialize};

use crate::batteries;
use crate::conv::f64_from_u64;
use crate::history::HistoryEntry;
use crate::{tr, trn};

/// Minimum time covered by the samples (48 h).
pub const MIN_SPAN_S: u64 = 48 * 3600;
/// Samples older than this (relative to the last one) are ignored.
pub const WINDOW_S: u64 = 30 * 24 * 3600;
/// Below this discharge rate the batteries are considered flat-lined: no estimate.
pub const MIN_RATE_PCT_PER_DAY: f64 = 0.05;
const MIN_BUCKETS: usize = 3;
const BUCKET_S: u64 = 3600;
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
    #[must_use]
    pub fn remaining_s(&self, now: u64) -> u64 {
        self.empty_at.saturating_sub(now)
    }
    /// Time left at unix time `now`, translated: "≈ 3 days" from 1.5 days on, else "≈ 5 h".
    #[must_use]
    pub fn remaining_text(&self, now: u64) -> String {
        let s = self.remaining_s(now);
        if s >= 129_600 {
            let n = (s + 43_200) / 86_400;
            trn!("≈ {n} day", "≈ {n} days", n, n = n)
        } else {
            tr!("≈ {h} h", h = (s + 1_800) / 3_600)
        }
    }
    #[cfg(any(test, feature = "testseam"))]
    /// Days left at unix time `now`.
    #[must_use]
    pub fn remaining_days(&self, now: u64) -> f64 {
        crate::conv::f64_from_u64(self.remaining_s(now)) / 86_400.0
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
///
/// # Errors
///
/// [`Unavailable`] saying why no estimate can be made.
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
    let mut buckets: Vec<(u64, f64, f64, u32)> = Vec::new(); // (hour, sum t, sum pct, n)
    for e in &used {
        let key = e.ts / BUCKET_S;
        let t = f64_from_u64(e.ts - first_ts);
        match buckets.last_mut() {
            Some(b) if b.0 == key => {
                b.1 += t;
                b.2 += e.pct;
                b.3 += 1;
            }
            _ => buckets.push((key, t, e.pct, 1)),
        }
    }
    if buckets.len() < MIN_BUCKETS {
        return Err(Unavailable::NotEnoughData);
    }
    let pts: Vec<(f64, f64)> = buckets
        .iter()
        .map(|&(_, st, sp, n)| (st / f64::from(n), sp / f64::from(n)))
        .collect();
    let n = crate::conv::f64_from_usize(pts.len());
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
    let fitted = (my + slope * (f64_from_u64(span_s) - mx)).clamp(0.0, 100.0);
    // Bounded (~317 years) and saturating.
    let secs_left = (fitted / rate * 86_400.0).clamp(0.0, MAX_HORIZON_S);
    let empty_at = last_ts.saturating_add(crate::conv::u64_from_f64_round(secs_left));
    Ok(Forecast {
        rate_pct_per_day: rate,
        empty_at,
        fitted_pct: fitted,
        span_s,
        buckets: pts.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{parse, HistoryEvent};

    #[test]
    fn remaining_text_in_days_then_hours() {
        let f = Forecast {
            rate_pct_per_day: 1.0,
            empty_at: 10_000_000,
            fitted_pct: 50.0,
            span_s: 86_400 * 3,
            buckets: 72,
        };
        assert_eq!(
            f.remaining_text(10_000_000 - 42 * 86_400),
            "\u{2248} 42 days"
        );
        assert_eq!(f.remaining_text(10_000_000 - 129_600), "\u{2248} 2 days");
        assert_eq!(f.remaining_text(10_000_000 - 129_599), "\u{2248} 36 h");
        assert_eq!(f.remaining_text(20_000_000), "\u{2248} 0 h");
    }

    const T0: u64 = 1_790_000_000;
    const DAY: u64 = 86_400;

    fn entry(ts: u64, pct: f64) -> HistoryEntry {
        HistoryEntry::sample(ts, pct, None)
    }

    fn synthetic(days: u64, rate_per_day: f64) -> Vec<HistoryEntry> {
        let mut v = Vec::new();
        let mut seed: u32 = 12345;
        for d in 0..days {
            for m in (9 * 60..18 * 60).step_by(5) {
                let ts = T0 + d * DAY + m * 60;
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let noise = (f64::from(seed >> 16 & 0x7fff) / 32767.0 - 0.5) * 1.2;
                let real = 80.0 - rate_per_day * f64_from_u64(ts - T0) / f64_from_u64(DAY);
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
        let truth_days = 80.0 - f64_from_u64(last.ts - T0) / f64_from_u64(DAY);
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
            .map(|i| entry(T0 + i * 300, 90.0 - f64_from_u64(i) * 0.01))
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
            .map(|d| entry(T0 + d * DAY, 50.0 + f64_from_u64(d)))
            .collect();
        assert_eq!(estimate(&h), Err(Unavailable::NotDischarging));
    }

    #[test]
    fn only_the_current_batteries_are_used() {
        let mut h: Vec<_> = (0..10 * 24)
            .map(|i| entry(T0 + i * 3600, 60.0 - f64_from_u64(i) * 5.0 / 24.0))
            .collect();
        let t1 = T0 + 11 * DAY;
        let mut marker = entry(t1, 100.0);
        marker.event = Some(HistoryEvent::BatteryReplaced);
        h.push(marker);
        h.extend((1..3 * 24).map(|i| entry(t1 + i * 3600, 100.0 - f64_from_u64(i) / 24.0)));
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
    }
}
