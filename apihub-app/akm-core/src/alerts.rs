//! Multi-threshold low-battery alerts with hysteresis (#82, F01).
//!
//! Each threshold `t` fires once when the battery goes down to `t` % or
//! below, and is re-armed only when the battery climbs back to `t + hysteresis`
//! (new batteries, or a noisy reading that recovered for real). A drop that
//! crosses several thresholds at once raises a single alert, for the lowest
//! one. Pure: no clock, no I/O, unit-tested without hardware.

use serde::{Deserialize, Serialize};

/// Default thresholds, in %, highest first.
pub const DEFAULT_THRESHOLDS: [u8; 3] = [30, 15, 5];
/// Default re-arm margin, in points of %.
pub const DEFAULT_HYSTERESIS: f64 = 3.0;

/// Notification urgency (`org.freedesktop.Notifications` `urgency` hint).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

impl Urgency {
    /// Value of the `urgency` hint (byte).
    pub fn hint(self) -> u8 {
        match self {
            Urgency::Low => 0,
            Urgency::Normal => 1,
            Urgency::Critical => 2,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Urgency::Low => "low",
            Urgency::Normal => "normal",
            Urgency::Critical => "critical",
        }
    }
}

/// Alert settings (`[alerts]` of `config.toml`).
#[derive(Debug, Clone, PartialEq)]
pub struct AlertConfig {
    /// Thresholds in %, kept sorted highest first, deduplicated, in 1..=99.
    thresholds: Vec<u8>,
    /// Re-arm margin (points of %), >= 1.
    pub hysteresis: f64,
    /// Thresholds at or below this value are `critical`, the others `normal`.
    pub critical_at: u8,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self::new(DEFAULT_THRESHOLDS.to_vec(), DEFAULT_HYSTERESIS, 5)
    }
}

impl AlertConfig {
    /// Normalises the input: out-of-range thresholds dropped, sorted, unique;
    /// hysteresis clamped to 1..=20.
    pub fn new(thresholds: Vec<u8>, hysteresis: f64, critical_at: u8) -> Self {
        let mut t: Vec<u8> = thresholds
            .into_iter()
            .filter(|t| (1..=99).contains(t))
            .collect();
        t.sort_unstable_by(|a, b| b.cmp(a));
        t.dedup();
        let hysteresis = if hysteresis.is_finite() {
            hysteresis.clamp(1.0, 20.0)
        } else {
            DEFAULT_HYSTERESIS
        };
        Self {
            thresholds: t,
            hysteresis,
            critical_at,
        }
    }

    /// Thresholds, highest first.
    pub fn thresholds(&self) -> &[u8] {
        &self.thresholds
    }

    pub fn urgency(&self, threshold: u8) -> Urgency {
        if threshold <= self.critical_at {
            Urgency::Critical
        } else {
            Urgency::Normal
        }
    }
}

/// Largest drop (points) at a reconnection still taken for a firmware step
/// (#179, C14); a larger drop is a discharge and alerts.
pub const MAX_RECONNECT_STEP: f64 = 5.0;

/// A threshold that was just crossed downwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossing {
    pub threshold: u8,
    pub pct: f64,
    pub urgency: Urgency,
}

/// Armed state of every threshold. Starts armed: a session that starts at
/// 10 % is warned once.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertState {
    cfg: AlertConfig,
    armed: Vec<bool>,
    /// Last value fed (to tell a drop from a steady reading).
    last: Option<f64>,
}

impl AlertState {
    pub fn new(cfg: AlertConfig) -> Self {
        let armed = vec![true; cfg.thresholds.len()];
        Self {
            cfg,
            armed,
            last: None,
        }
    }

    pub fn config(&self) -> &AlertConfig {
        &self.cfg
    }

    /// Feed a battery reading; returns the alert to raise, if any.
    pub fn update(&mut self, pct: f64) -> Option<Crossing> {
        self.update_after(pct, false)
    }

    /// Like [`Self::update`]. `after_reconnect`: this is the first reading
    /// since the keyboard reconnected. The firmware percentage only steps down
    /// at reconnections (docs/VERIF-BATTERIE.md §1.2bis: 99 -> 96 without any
    /// consumption), so a **drop** seen on such a reading is a link artefact,
    /// not a discharge: the thresholds it crosses are disarmed **without**
    /// raising an alert (#179). A reading that does not drop is handled
    /// normally, and so is the very first reading of the session.
    pub fn update_after(&mut self, pct: f64, after_reconnect: bool) -> Option<Crossing> {
        if !pct.is_finite() {
            return None;
        }
        // A firmware step is a few points (99 -> 96, 32 -> 28 measured): a
        // larger drop is a real discharge during the disconnection (C14).
        let stepped = after_reconnect
            && self
                .last
                .is_some_and(|prev| pct < prev && prev - pct <= MAX_RECONNECT_STEP);
        self.last = Some(pct);
        let mut lowest = None;
        let mut crossed = 0;
        for (i, &t) in self.cfg.thresholds.iter().enumerate() {
            let t_f = f64::from(t);
            if pct >= t_f + self.cfg.hysteresis {
                self.armed[i] = true;
            } else if self.armed[i] && pct <= t_f {
                self.armed[i] = false;
                crossed += 1;
                lowest = Some(t); // thresholds are sorted highest first
            }
        }
        // The step hides at most the ONE threshold it crosses; a jump over
        // several thresholds is warned at the lowest (C14).
        if stepped && crossed == 1 {
            lowest = None;
        }
        lowest.map(|threshold| Crossing {
            threshold,
            pct,
            urgency: self.cfg.urgency(threshold),
        })
    }

    /// Re-arm every threshold (batteries replaced).
    pub fn rearm_all(&mut self) {
        self.armed.iter_mut().for_each(|a| *a = true);
    }

    /// Is the threshold armed? (`None` if not configured).
    pub fn is_armed(&self, threshold: u8) -> Option<bool> {
        self.cfg
            .thresholds
            .iter()
            .position(|&t| t == threshold)
            .map(|i| self.armed[i])
    }
}

// ── no double alert: percentage thresholds vs the keyboard's own 0x30 (#189) ─

/// An alert of the keyboard and an alert by percentage that follows it (or
/// precedes it) within this many seconds are one event for the user.
pub const DEDUPE_WINDOW_S: u64 = 3600;

/// Rank of an alert for the dedupe: `1` low, `2` critical.
pub fn rank_of(u: Urgency) -> u8 {
    if u == Urgency::Critical {
        2
    } else {
        1
    }
}

/// Keeps the two sources of "battery low / critical" from announcing the same
/// thing twice (pure: the caller gives the clock).
///
/// * the keyboard is authoritative (macOS acts on `0x30` only): once it has
///   announced rank `R`, percentage alerts of rank `<= R` are muted until it
///   reports normal again or new batteries are detected; a higher rank still
///   goes through;
/// * a keyboard alert of rank `R` is muted if a percentage alert of rank
///   `>= R` was shown within [`DEDUPE_WINDOW_S`] (same event, seen twice).
///
/// Without any `0x30` ever received the percentage thresholds work unchanged
/// (the fallback of #189).
#[derive(Debug, Default)]
pub struct AlertDedupe {
    kb_rank: u8,
    pct: Option<(u64, u8)>,
}

impl AlertDedupe {
    pub const fn new() -> Self {
        Self { kb_rank: 0, pct: None }
    }

    /// May a percentage alert of `rank` be shown at `now`? Records it if so.
    pub fn allow_percent(&mut self, rank: u8, now: u64) -> bool {
        if self.kb_rank >= rank {
            return false;
        }
        self.pct = Some((now, rank));
        true
    }

    /// May a keyboard alert of `rank` be shown at `now`? Records it if so.
    pub fn allow_keyboard(&mut self, rank: u8, now: u64) -> bool {
        if self.kb_rank >= rank {
            return false;
        }
        if let Some((t, r)) = self.pct {
            if r >= rank && now.saturating_sub(t) <= DEDUPE_WINDOW_S {
                // the user already has this alert: remember the keyboard said it
                self.kb_rank = rank;
                return false;
            }
        }
        self.kb_rank = rank;
        true
    }

    /// The keyboard reports a normal state again (`0x30` = 0), or new
    /// batteries were detected: everything is armed again.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

static DEDUPE: std::sync::Mutex<AlertDedupe> = std::sync::Mutex::new(AlertDedupe::new());

/// The process-wide dedupe shared by the percentage alerts (acquisition
/// thread) and the keyboard-driven ones (passive listener thread).
pub fn dedupe() -> std::sync::MutexGuard<'static, AlertDedupe> {
    DEDUPE.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(state: &mut AlertState, seq: impl IntoIterator<Item = f64>) -> Vec<Crossing> {
        seq.into_iter().filter_map(|p| state.update(p)).collect()
    }

    #[test]
    fn full_discharge_raises_exactly_one_alert_per_threshold() {
        let mut s = AlertState::new(AlertConfig::default());
        let got = run(&mut s, (0..=100).rev().map(f64::from));
        let th: Vec<u8> = got.iter().map(|c| c.threshold).collect();
        assert_eq!(th, vec![30, 15, 5]);
        assert_eq!(got[0].pct, 30.0);
        assert_eq!(got[0].urgency, Urgency::Normal);
        assert_eq!(got[1].urgency, Urgency::Normal);
        assert_eq!(got[2].urgency, Urgency::Critical);
    }

    #[test]
    fn oscillation_around_a_threshold_is_silent() {
        let mut s = AlertState::new(AlertConfig::default());
        assert!(run(&mut s, [50.0, 40.0]).is_empty());
        assert_eq!(run(&mut s, [30.0, 20.0]).len(), 1);
        let first = run(&mut s, [16.0, 15.0]);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].threshold, 15);
        // 15 <-> 16 <-> 17 for a long time: below the 3-point re-arm margin.
        let noise = (0..200).map(|i| [15.0, 16.0, 17.0, 16.4][i % 4]);
        assert!(run(&mut s, noise).is_empty());
        // Recovery to 18 re-arms; the next drop alerts again.
        assert!(run(&mut s, [18.0]).is_empty());
        assert_eq!(s.is_armed(15), Some(true));
        assert_eq!(run(&mut s, [15.0]).len(), 1);
    }

    #[test]
    fn big_drop_raises_only_the_lowest_threshold() {
        let mut s = AlertState::new(AlertConfig::default());
        let got = run(&mut s, [80.0, 4.0]);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].threshold, got[0].urgency), (5, Urgency::Critical));
        // all crossed thresholds are disarmed: no late alert for 30 / 15
        assert!(run(&mut s, [4.0, 3.0, 2.0]).is_empty());
    }

    #[test]
    fn start_below_threshold_warns_once_and_rearm_all_resets() {
        let mut s = AlertState::new(AlertConfig::default());
        assert_eq!(run(&mut s, [10.0, 10.0, 9.0]).len(), 1);
        s.rearm_all();
        assert_eq!(s.is_armed(30), Some(true));
        assert_eq!(run(&mut s, [9.0]).len(), 1);
        assert_eq!(s.is_armed(42), None);
        assert!(s.update(f64::NAN).is_none());
    }

    #[test]
    fn step_down_at_a_reconnection_raises_no_alert() {
        // #179: 32 -> 28 on the first reading after a reconnection.
        let mut s = AlertState::new(AlertConfig::default());
        assert!(s.update(32.0).is_none());
        assert!(s.update_after(28.0, true).is_none());
        assert_eq!(s.is_armed(30), Some(false), "disarmed, not forgotten");
        // The same drop during a continuous session does alert.
        let mut s = AlertState::new(AlertConfig::default());
        assert!(s.update(32.0).is_none());
        assert_eq!(s.update_after(28.0, false).unwrap().threshold, 30);
        // A real discharge after the step still raises the lower thresholds.
        let mut s = AlertState::new(AlertConfig::default());
        s.update(32.0);
        s.update_after(28.0, true);
        assert_eq!(s.update(14.0).unwrap().threshold, 15);
    }

    #[test]
    fn a_real_discharge_across_a_disconnection_is_never_swallowed() {
        // C14: 41 % -> 14 % across a long disconnection: two thresholds
        // crossed, warned at the lowest.
        let mut s = AlertState::new(AlertConfig::default());
        assert!(s.update(41.0).is_none());
        assert_eq!(s.update_after(14.0, true).unwrap().threshold, 15);
        // A drop of more than a step over a single threshold alerts too.
        let mut s = AlertState::new(AlertConfig::default());
        s.update(38.0);
        assert_eq!(s.update_after(29.0, true).unwrap().threshold, 30);
        // A small step over two close thresholds is no artefact either.
        let mut s = AlertState::new(AlertConfig::new(vec![20, 18], 2.0, 5));
        s.update(21.0);
        assert_eq!(s.update_after(17.0, true).unwrap().threshold, 18);
    }

    #[test]
    fn reconnection_without_a_drop_or_first_reading_is_normal() {
        // First reading of the session, below a threshold: warned once.
        let mut s = AlertState::new(AlertConfig::default());
        assert_eq!(s.update_after(10.0, true).unwrap().threshold, 15);
        // Reconnection with a steady value already armed: the usual rules.
        let mut s = AlertState::new(AlertConfig::default());
        s.update(40.0);
        assert!(s.update_after(40.0, true).is_none());
        assert!(s.update_after(41.0, true).is_none());
    }

    #[test]
    fn descending_voltage_estimate_raises_three_alerts_in_order_once() {
        // #178: 2950 -> 2000 mV on the alkaline curve, alerts on the estimate.
        use crate::chemistry::{alert_pct, estimate_charge, Chemistry};
        let mut s = AlertState::new(AlertConfig::default());
        let mut got = Vec::new();
        for mv in (2000..=2950).rev().step_by(5) {
            let e = estimate_charge(mv, Chemistry::Alkaline);
            let (pct, _) = alert_pct(e.as_ref(), 100.0);
            if let Some(c) = s.update(pct) {
                got.push((c.threshold, mv));
            }
        }
        let th: Vec<u8> = got.iter().map(|g| g.0).collect();
        assert_eq!(th, vec![30, 15, 5], "{got:?}");
        // 30 % real alkaline is about 2460 mV (firmware 64 %), not 2124 mV.
        assert!((2450..=2470).contains(&got[0].1), "{got:?}");
    }

    #[test]
    fn config_is_normalised() {
        let c = AlertConfig::new(vec![5, 30, 0, 15, 30, 100], 0.2, 5);
        assert_eq!(c.thresholds(), &[30, 15, 5]);
        assert_eq!(c.hysteresis, 1.0);
        let c = AlertConfig::new(vec![20], f64::NAN, 0);
        assert_eq!(c.hysteresis, DEFAULT_HYSTERESIS);
        assert_eq!(c.urgency(20), Urgency::Normal);
        let mut s = AlertState::new(AlertConfig::new(vec![], 3.0, 5));
        assert!(run(&mut s, [1.0]).is_empty());
        assert_eq!(Urgency::Critical.hint(), 2);
        assert_eq!(Urgency::Low.as_str(), "low");
    }

    #[test]
    fn keyboard_and_percentage_alerts_do_not_double_up() {
        let (low, crit) = (rank_of(Urgency::Normal), rank_of(Urgency::Critical));
        assert_eq!((low, crit), (1, 2));
        // percentage first, then the keyboard says the same thing: muted
        let mut d = AlertDedupe::new();
        assert!(d.allow_percent(low, 1000));
        assert!(!d.allow_keyboard(low, 1600), "same event within the window");
        assert!(d.allow_keyboard(crit, 1700), "a higher rank is news");
        // keyboard first: percentage alerts of the same or lower rank are muted
        let mut d = AlertDedupe::new();
        assert!(d.allow_keyboard(low, 10));
        assert!(!d.allow_percent(low, 20), "30 % after the keyboard's low");
        assert!(!d.allow_keyboard(low, 30), "a repeat from the keyboard");
        assert!(d.allow_percent(crit, 40), "5 % critical is still news");
        // the keyboard's critical covers every percentage alert
        let mut d = AlertDedupe::new();
        assert!(d.allow_keyboard(crit, 10));
        assert!(!d.allow_percent(low, 11) && !d.allow_percent(crit, 12));
        // back to normal / new batteries: everything is armed again
        d.reset();
        assert!(d.allow_percent(low, 13) && d.allow_keyboard(crit, 14));
        // percentage alert long before: not the same event any more
        let mut d = AlertDedupe::new();
        assert!(d.allow_percent(low, 0));
        assert!(d.allow_keyboard(low, DEDUPE_WINDOW_S + 1));
        // no 0x30 ever: percentage thresholds all go through
        let mut d = AlertDedupe::new();
        assert!(d.allow_percent(low, 1) && d.allow_percent(low, 2) && d.allow_percent(crit, 3));
    }
}
