//! Battery replacement detection and per-set lifetime log (#85, F04).
//!
//! A replacement is a rise of at least [`MIN_PCT_RISE`] points of % or
//! [`MIN_MV_RISE`] mV between a sample and the lowest value of the hour
//! before it. AA cells do not recharge by themselves: such a rise only
//! happens when the cells are swapped (or the keyboard is put on fresh
//! rechargeables). When the keyboard was off in between (no sample in the
//! last hour: typically the old cells died), the last sample before the gap
//! is the baseline.
//!
//! The daemon marks the detecting sample with `"event":"battery_replaced"`
//! in `history.jsonl`; such lines survive rotation, so the lifetime of every
//! set stays known after the 90-day retention of ordinary samples.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::history::{HistoryEntry, HistoryEvent};

/// Minimum rise of the percentage (points).
pub const MIN_PCT_RISE: f64 = 20.0;
/// Minimum rise of the voltage (mV).
pub const MIN_MV_RISE: f64 = 300.0;
/// Look-back window for the baseline.
pub const WINDOW_S: u64 = 3600;

/// A detected (or recorded) battery replacement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Replacement {
    /// Unix time of the first sample on the new batteries.
    pub ts: u64,
    /// Lowest % of the baseline (None for a recorded marker without baseline).
    pub pct_before: Option<f64>,
    pub pct_after: f64,
    pub voltage_before: Option<f64>,
    pub voltage_after: Option<f64>,
}

/// Streaming detector; feed samples in chronological order.
#[derive(Debug, Clone, Default)]
pub struct Detector {
    recent: VecDeque<HistoryEntry>,
    /// Time of the last replacement: no new one is reported within
    /// [`WINDOW_S`] of it (first readings on fresh cells can still climb).
    last_replacement: Option<u64>,
}

impl Detector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prime with existing history (no events are returned).
    pub fn primed(entries: &[HistoryEntry]) -> Self {
        let mut d = Self::new();
        let mut sorted: Vec<&HistoryEntry> = entries.iter().collect();
        sorted.sort_by_key(|e| e.ts);
        for e in sorted {
            d.observe(e);
        }
        d
    }

    /// Timestamp of the last replacement this detector has seen.
    pub fn last_replacement(&self) -> Option<u64> {
        self.last_replacement
    }

    /// Would `e` be a replacement? Pure check, the state is not changed.
    pub fn check(&self, e: &HistoryEntry) -> Option<Replacement> {
        // Out-of-range samples (raw byte 255 %, 211 V) are never a replacement.
        if !crate::history::valid_sample(e.pct, e.voltage) {
            return None;
        }
        let since = e.ts.saturating_sub(WINDOW_S);
        // Samples of the last hour, plus the last one before it (gap case).
        let start = self.recent.iter().rposition(|p| p.ts < since).unwrap_or(0);
        let base: Vec<&HistoryEntry> = self
            .recent
            .iter()
            .skip(start)
            .filter(|p| p.ts <= e.ts)
            .collect();
        let pct_before = base
            .iter()
            .map(|p| p.pct)
            .filter(|p| p.is_finite())
            .reduce(f64::min);
        let voltage_before = base
            .iter()
            .filter_map(|p| p.reliable_voltage())
            .filter(|v| crate::history::VOLTAGE_RANGE.contains(v))
            .reduce(f64::min);
        let pct_rise = pct_before.is_some_and(|b| e.pct - b >= MIN_PCT_RISE);
        let mv_rise = matches!((voltage_before, e.reliable_voltage()), (Some(b), Some(a)) if (a - b) * 1000.0 >= MIN_MV_RISE - 1e-6);
        let marked = e.event == Some(HistoryEvent::BatteryReplaced);
        let cooling = self
            .last_replacement
            .is_some_and(|t| e.ts < t.saturating_add(WINDOW_S));
        ((pct_rise || mv_rise) && !cooling || marked).then_some(Replacement {
            ts: e.ts,
            pct_before,
            pct_after: e.pct,
            voltage_before,
            voltage_after: e.reliable_voltage(),
        })
    }

    /// Feed a sample; returns the replacement it reveals, if any.
    pub fn observe(&mut self, e: &HistoryEntry) -> Option<Replacement> {
        if !crate::history::valid_sample(e.pct, e.voltage) {
            return None; // not a sample: neither baseline nor trigger
        }
        let r = self.check(e);
        if r.is_some() {
            self.last_replacement = Some(e.ts);
            // New set: the old cells must not serve as baseline any more.
            self.recent.clear();
        }
        self.recent.push_back(e.clone());
        let since = e.ts.saturating_sub(WINDOW_S);
        while self.recent.len() >= 2 && self.recent[1].ts < since {
            self.recent.pop_front();
        }
        r
    }
}

/// Every replacement in a history (any order).
pub fn replacements(entries: &[HistoryEntry]) -> Vec<Replacement> {
    let mut sorted: Vec<&HistoryEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.ts);
    let mut d = Detector::new();
    sorted.into_iter().filter_map(|e| d.observe(e)).collect()
}

/// Unix time at which the current set of batteries was installed, if a
/// replacement is known.
pub fn current_set_start(entries: &[HistoryEntry]) -> Option<u64> {
    replacements(entries).last().map(|r| r.ts)
}

/// One set of batteries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatterySet {
    /// First sample on this set.
    pub start_ts: u64,
    /// Last sample on this set.
    pub last_ts: u64,
    /// The set began at a detected replacement (false: history starts with it).
    pub start_known: bool,
    /// The set was replaced (false: still in the keyboard).
    pub ended: bool,
    pub start_pct: f64,
    pub last_pct: f64,
    pub start_voltage: Option<f64>,
    pub last_voltage: Option<f64>,
    /// `last_ts - start_ts`; for an ended set, up to the replacement instead
    /// (the cells powered the keyboard until then).
    pub duration_s: u64,
}

/// Split a history into battery sets, oldest first.
pub fn battery_sets(entries: &[HistoryEntry]) -> Vec<BatterySet> {
    let mut sorted: Vec<&HistoryEntry> = entries.iter().filter(|e| e.pct.is_finite()).collect();
    sorted.sort_by_key(|e| e.ts);
    let mut d = Detector::new();
    let mut sets: Vec<BatterySet> = Vec::new();
    for e in sorted {
        let replaced = d.observe(e).is_some();
        match sets.last_mut() {
            Some(cur) if !replaced => {
                cur.last_ts = e.ts;
                cur.last_pct = e.pct;
                if e.reliable_voltage().is_some() {
                    cur.last_voltage = e.reliable_voltage();
                }
                cur.duration_s = cur.last_ts - cur.start_ts;
            }
            _ => {
                if let Some(prev) = sets.last_mut() {
                    prev.ended = true;
                    prev.duration_s = e.ts - prev.start_ts;
                }
                sets.push(BatterySet {
                    start_ts: e.ts,
                    last_ts: e.ts,
                    start_known: replaced,
                    ended: false,
                    start_pct: e.pct,
                    last_pct: e.pct,
                    start_voltage: e.reliable_voltage(),
                    last_voltage: e.reliable_voltage(),
                    duration_s: 0,
                });
            }
        }
    }
    sets
}

/// Human-readable table (CLI `--batteries`).
pub fn format_sets(sets: &[BatterySet]) -> String {
    use std::fmt::Write;
    if sets.is_empty() {
        return "no battery history\n".into();
    }
    let mut out = String::from("#  installed            days   from  ->  to    state\n");
    for (i, s) in sets.iter().enumerate() {
        let days = s.duration_s as f64 / 86_400.0;
        let installed = if s.start_known {
            crate::history::format_utc(s.start_ts)
        } else {
            format!("<={}", crate::history::format_utc(s.start_ts))
        };
        let _ = writeln!(
            out,
            "{:<2} {:<20} {:>6.1} {:>5.0}% -> {:>3.0}%  {}",
            i + 1,
            installed,
            days,
            s.start_pct,
            s.last_pct,
            if s.ended { "replaced" } else { "in use" }
        );
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn out_of_range_sample_is_ignored_by_detector() {
        // #163
        let mut d = Detector::new();
        d.observe(&HistoryEntry::sample(1_000, 40.0, None));
        assert_eq!(d.observe(&HistoryEntry::sample(1_300, 255.0, None)), None);
        assert_eq!(d.observe(&HistoryEntry::sample(1_400, 41.0, Some(211.0))), None);
        assert_eq!(d.last_replacement, None);
        // baseline not polluted: a real jump is still seen
        assert!(d.observe(&HistoryEntry::sample(1_500, 100.0, None)).is_some());
    }

    use super::*;

    const T0: u64 = 1_790_000_000;

    fn e(ts: u64, pct: f64, v: Option<f64>) -> HistoryEntry {
        HistoryEntry::sample(ts, pct, v)
    }

    #[test]
    fn legacy_voltage_never_triggers_nor_feeds_a_replacement() {
        // #180: the legacy `voltage` was a constant (or its jump an artefact of
        // the ADC byte), never a measurement.
        let legacy = |ts, pct, v| {
            let mut x = e(ts, pct, Some(v));
            x.voltage_valid = Some(false);
            x
        };
        let mut d = Detector::new();
        assert!(d.observe(&legacy(T0, 50.0, 2.50)).is_none());
        // +0.6 V on a flat percentage: no replacement from an unreliable voltage.
        assert!(d.observe(&legacy(T0 + 300, 50.0, 3.10)).is_none());
        // Reliable lines still detect the jump and report their voltages.
        let mut d = Detector::new();
        assert!(d.observe(&e(T0, 50.0, Some(2.50))).is_none());
        let r = d.observe(&e(T0 + 300, 50.0, Some(3.10))).unwrap();
        assert_eq!(r.voltage_after, Some(3.10));
        // A replacement seen from a legacy line carries no voltage.
        let mut d = Detector::new();
        d.observe(&legacy(T0, 12.0, 2.50));
        let r = d.observe(&legacy(T0 + 300, 99.0, 3.0)).unwrap();
        assert_eq!((r.voltage_before, r.voltage_after), (None, None));
    }

    #[test]
    fn pct_jump_within_an_hour_is_a_replacement() {
        let mut d = Detector::new();
        assert!(d.observe(&e(T0, 12.0, None)).is_none());
        assert!(d.observe(&e(T0 + 300, 11.0, None)).is_none());
        let r = d.observe(&e(T0 + 1800, 31.0, None)).expect("20-point rise");
        assert_eq!(
            (r.pct_before, r.pct_after, r.ts),
            (Some(11.0), 31.0, T0 + 1800)
        );
        // first readings on the new cells can still climb: same replacement
        assert!(d.observe(&e(T0 + 2100, 98.0, None)).is_none());
        assert!(d.observe(&e(T0 + 2400, 97.0, None)).is_none());
    }

    #[test]
    fn small_rises_and_noise_are_not_replacements() {
        let mut d = Detector::new();
        for (i, p) in [50.0, 51.0, 49.0, 60.0, 55.0, 68.0].iter().enumerate() {
            assert!(
                d.observe(&e(T0 + i as u64 * 300, *p, None)).is_none(),
                "{p}"
            );
        }
        // +19.9 points is still below the threshold
        assert!(d.check(&e(T0 + 2000, 49.0 + 19.9, None)).is_none());
    }

    #[test]
    fn voltage_jump_counts_even_if_pct_is_flat() {
        let mut d = Detector::new();
        d.observe(&e(T0, 100.0, Some(2.55)));
        let r = d.observe(&e(T0 + 600, 100.0, Some(2.85))).expect("300 mV");
        assert_eq!(r.voltage_before, Some(2.55));
        assert!(d.observe(&e(T0 + 900, 100.0, Some(2.70))).is_none());
        let mut d = Detector::new();
        d.observe(&e(T0, 100.0, Some(2.60)));
        assert!(
            d.observe(&e(T0 + 600, 100.0, Some(2.89))).is_none(),
            "290 mV"
        );
    }

    #[test]
    fn rise_spread_over_more_than_an_hour_is_ignored() {
        let mut d = Detector::new();
        // +6 points per 30 min: 0 -> 36 over 3 h (never 20 within an hour)
        for i in 0..7 {
            assert!(d
                .observe(&e(T0 + i * 1800, 10.0 + i as f64 * 6.0 / 2.0, None))
                .is_none());
        }
    }

    #[test]
    fn keyboard_off_for_days_then_new_cells() {
        let mut d = Detector::new();
        d.observe(&e(T0, 3.0, Some(2.05)));
        d.observe(&e(T0 + 300, 2.0, Some(2.02)));
        // dead for 3 days, then fresh cells
        let r = d
            .observe(&e(T0 + 3 * 86_400, 100.0, Some(3.05)))
            .expect("gap");
        assert_eq!(r.pct_before, Some(2.0));
    }

    #[test]
    fn marker_line_is_a_boundary_even_without_baseline() {
        let mut m = e(T0, 100.0, Some(3.0));
        m.event = Some(HistoryEvent::BatteryReplaced);
        let r = replacements(&[m]);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].pct_before, None);
    }

    #[test]
    fn sets_and_lifetimes() {
        let day = 86_400;
        let mut h = Vec::new();
        // set 1: from 80 % (start unknown) for 40 days down to 4 %
        for d in 0..=40u64 {
            h.push(e(T0 + d * day, 80.0 - d as f64 * 1.9, None));
        }
        // replaced 2 days later, set 2 for 10 days
        let t2 = T0 + 42 * day;
        for d in 0..=10u64 {
            h.push(e(t2 + d * day, 100.0 - d as f64, None));
        }
        h.reverse(); // order does not matter
        let sets = battery_sets(&h);
        assert_eq!(sets.len(), 2);
        assert!(!sets[0].start_known && sets[0].ended);
        assert_eq!(sets[0].duration_s, 42 * day);
        assert!(sets[1].start_known && !sets[1].ended);
        assert_eq!(sets[1].start_ts, t2);
        assert_eq!(sets[1].duration_s, 10 * day);
        assert_eq!(current_set_start(&h), Some(t2));
        let txt = format_sets(&sets);
        assert!(txt.contains("replaced") && txt.contains("in use"), "{txt}");
        assert_eq!(format_sets(&[]), "no battery history\n");
    }

    #[test]
    fn fixture_with_two_replacements() {
        let h = crate::history::parse(include_str!("../tests/fixtures/history_replacements.jsonl"));
        let r = replacements(&h);
        assert_eq!(r.len(), 2, "{r:?}");
        let sets = battery_sets(&h);
        assert_eq!(sets.len(), 3);
        assert!(sets[1].ended && sets[1].start_known);
    }

    #[test]
    fn primed_detector_does_not_report_old_events() {
        let h = vec![e(T0, 10.0, None), e(T0 + 60, 90.0, None)];
        let d = Detector::primed(&h);
        // A new sample consistent with the new set is not a replacement.
        assert!(d.check(&e(T0 + 400, 89.0, None)).is_none());
    }
}
