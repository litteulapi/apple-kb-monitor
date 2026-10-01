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
}

impl AlertState {
    pub fn new(cfg: AlertConfig) -> Self {
        let armed = vec![true; cfg.thresholds.len()];
        Self { cfg, armed }
    }

    pub fn config(&self) -> &AlertConfig {
        &self.cfg
    }

    /// Feed a battery reading; returns the alert to raise, if any.
    pub fn update(&mut self, pct: f64) -> Option<Crossing> {
        if !pct.is_finite() {
            return None;
        }
        let mut lowest = None;
        for (i, &t) in self.cfg.thresholds.iter().enumerate() {
            let t_f = f64::from(t);
            if pct >= t_f + self.cfg.hysteresis {
                self.armed[i] = true;
            } else if self.armed[i] && pct <= t_f {
                self.armed[i] = false;
                lowest = Some(t); // thresholds are sorted highest first
            }
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
}
