//! Pure presentation logic of the window (no egui context, unit-tested):
//! unknown-value formatting, semantic levels and the light/dark palette.

use eframe::egui::Color32;

/// BlueZ / the daemon report 127 when a radio value is unavailable.
pub const RADIO_UNKNOWN: i32 = 127;
/// Shown instead of any unknown value.
pub const DASH: &str = "—";

/// Relative BR/EDR RSSI in dB (0 = ideal reception range), **not** dBm
/// (#174). `None` when unknown (absent, 127, out of range); 0 and positive
/// values are legal.
pub fn rssi_valid(r: Option<i32>) -> Option<i32> {
    akm_core::signal::valid_rel(r)
}

/// `"excellent (0)"`, `"good (−3)"`, `"weak (−12)"`: words first, the raw
/// value in parentheses, no unit.
pub fn rssi_text(r: Option<i32>) -> String {
    match rssi_valid(r) {
        Some(v) => format!(
            "{} ({})",
            akm_core::signal::quality(v).en(),
            akm_core::signal::raw_text(v)
        ),
        None => DASH.to_string(),
    }
}

/// Estimated charge by chemistry, always marked as an estimate (#178).
pub fn estimate_text(b: &akm_core::report::KbBattery) -> Option<String> {
    if b.new_batteries {
        return Some("new batteries, no estimate yet".to_string());
    }
    let e = b.charge_estimate.as_ref()?;
    Some(format!(
        "\u{2248} {:.0}% ({:.0} to {:.0}%), {}",
        e.pct,
        e.low,
        e.high,
        e.chemistry.as_str()
    ))
}

/// Age of the last reading. The kernel percentage only steps down at
/// reconnections, so this age says how stale the indication can be (#179).
pub fn age_text(age_s: Option<u64>) -> String {
    match age_s {
        None => DASH.to_string(),
        Some(a) if a < 60 => format!("{a} s ago"),
        Some(a) if a < 3600 => format!("{} min ago", a / 60),
        Some(a) if a < 86_400 => format!("{} h ago", a / 3600),
        Some(a) => format!("{} d ago", a / 86_400),
    }
}

/// TX power may legitimately be positive; only 127 / absent mean unknown.
pub fn tx_power_text(t: Option<i32>) -> String {
    match t.filter(|v| *v != RADIO_UNKNOWN) {
        Some(v) => format!("{v} dBm"),
        None => DASH.to_string(),
    }
}

/// Battery percentage; unknown is a dash, never "0%".
pub fn pct_text(p: Option<f64>, decimals: usize) -> String {
    match p.filter(|v| v.is_finite()) {
        Some(v) => format!("{v:.decimals$}%"),
        None => DASH.to_string(),
    }
}

/// Fraction 0..=1 for a progress bar (unknown = empty).
pub fn pct_fraction(p: Option<f64>) -> f32 {
    p.filter(|v| v.is_finite()).map(|v| (v / 100.0).clamp(0.0, 1.0) as f32).unwrap_or(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Good,
    Warn,
    Bad,
    Unknown,
}

pub fn battery_level(p: Option<f64>) -> Level {
    match p.filter(|v| v.is_finite()) {
        None => Level::Unknown,
        Some(v) if v > 50.0 => Level::Good,
        Some(v) if v > 20.0 => Level::Warn,
        Some(_) => Level::Bad,
    }
}

/// Measured battery voltage, rounded to 0.01 V.
pub fn volts_text(v: f64) -> String {
    format!("{v:.2} V")
}

pub fn voltage_level(v: f64) -> Level {
    if v > 2.8 {
        Level::Good
    } else if v > 2.4 {
        Level::Warn
    } else {
        Level::Bad
    }
}

pub fn rssi_level(r: Option<i32>) -> Level {
    match rssi_valid(r) {
        None => Level::Unknown,
        Some(v) if v >= -5 => Level::Good,
        Some(v) if v >= -15 => Level::Warn,
        Some(_) => Level::Bad,
    }
}

/// Lit bars (0-4) of the signal gauge on the relative scale; unknown = 0.
/// Drawn with the painter: the block glyphs U+2581-2588 are missing from
/// egui's fonts and showed as empty squares (#198).
pub fn rssi_bar_count(r: Option<i32>) -> u8 {
    match rssi_valid(r) {
        None => 0,
        Some(v) if v >= 0 => 4,
        Some(v) if v >= -2 => 3,
        Some(v) if v >= -5 => 2,
        Some(v) if v >= -10 => 1,
        Some(_) => 0,
    }
}

/// Where the big percentage comes from: the keyboard's own indication
/// (kernel / report 0x47) or a voltage estimate (#198).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PctSource {
    Indication(f64),
    Estimate(f64),
    Unknown,
}

fn pct_ok(p: Option<f64>) -> Option<f64> {
    p.filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
}

pub fn pct_source(b: &akm_core::report::KbBattery) -> PctSource {
    if let Some(p) = pct_ok(b.percentage).or(pct_ok(b.percentage_fine)) {
        PctSource::Indication(p)
    } else if let Some(p) = pct_ok(b.percentage_estimate).or(pct_ok(b.percentage_interpolated)) {
        PctSource::Estimate(p)
    } else {
        PctSource::Unknown
    }
}

impl PctSource {
    pub fn value(self) -> Option<f64> {
        match self {
            Self::Indication(p) | Self::Estimate(p) => Some(p),
            Self::Unknown => None,
        }
    }
    pub fn caption(self) -> &'static str {
        match self {
            Self::Indication(_) => "keyboard indication",
            Self::Estimate(_) => "estimate from the voltage curve",
            Self::Unknown => "battery level unknown",
        }
    }
}

/// Battery chemistry line: the one declared to the daemon when it made an
/// estimate, else a guess from the voltage, always marked as such.
pub fn chemistry_text(b: &akm_core::report::KbBattery) -> Option<String> {
    if let Some(e) = &b.charge_estimate {
        return Some(format!("{} (declared)", e.chemistry.as_str()));
    }
    b.voltage
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| format!("{} (guess from the voltage)", akm_core::calibration::detect_battery_type(v)))
}

/// Paired state. The daemon never fills `bluetooth.paired` (always false
/// while BlueZ says Paired=true, #198): only the paired host read from the
/// keyboard (0x4C) or an explicit `true` prove it; otherwise unknown.
pub fn paired_text(b: &akm_core::report::KbBluetooth) -> Option<&'static str> {
    (b.paired || b.paired_host_addr.is_some()).then_some("Yes")
}

/// Autonomy left: the daemon's forecast, else the legacy text.
pub fn remaining_text(s: &akm_core::Snapshot, now: u64) -> Option<String> {
    s.remaining_s(now)
        .map(akm_core::forecast::format_days)
        .or_else(|| s.remaining_display.clone().filter(|t| !t.trim().is_empty()))
}

/// Last wake event of the keyboard (input report 0x13), passive listening.
pub fn wake_text(w: &akm_core::report::KbWake) -> Option<String> {
    let age = w.last_age_s.filter(|a| a.is_finite() && *a >= 0.0)?;
    Some(format!("{} ({} since start)", age_text(Some(age as u64)), w.count))
}

/// Two side-by-side tiles only when each gets a usable width; below, the
/// tiles are stacked (#195).
pub const TWO_COLUMNS_MIN_WIDTH: f32 = 660.0;
pub fn two_columns(width: f32) -> bool {
    width >= TWO_COLUMNS_MIN_WIDTH
}

/// History points that can be drawn: finite, percentage in 0..=100, voltage
/// > 0, at or after `cutoff`, sorted by time.
pub fn chart_points(pts: &[(f64, f64)], cutoff: f64, pct: bool) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = pts
        .iter()
        .copied()
        .filter(|(t, y)| t.is_finite() && y.is_finite() && *t >= cutoff)
        .filter(|(_, y)| if pct { (0.0..=100.0).contains(y) } else { *y > 0.0 && *y < 10.0 })
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v
}

pub fn accent_color(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// Semantic colours, readable on both the light and the dark background.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub good: Color32,
    pub warn: Color32,
    pub bad: Color32,
    pub info: Color32,
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                good: Color32::from_rgb(80, 220, 100),
                warn: Color32::from_rgb(255, 200, 50),
                bad: Color32::from_rgb(255, 90, 90),
                info: Color32::from_rgb(100, 180, 255),
            }
        } else {
            Self {
                good: Color32::from_rgb(20, 130, 50),
                warn: Color32::from_rgb(160, 100, 0),
                bad: Color32::from_rgb(190, 30, 30),
                info: Color32::from_rgb(20, 90, 190),
            }
        }
    }

    /// `None` = neutral (caller uses the text colour of the theme).
    pub fn color(&self, l: Level) -> Option<Color32> {
        match l {
            Level::Good => Some(self.good),
            Level::Warn => Some(self.warn),
            Level::Bad => Some(self.bad),
            Level::Unknown => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voltage_and_curve_texts() {
        assert_eq!(volts_text(2.987), "2.99 V");
        assert_eq!(volts_text(2.9), "2.90 V");
    }

    #[test]
    fn unknown_rssi_is_a_dash_never_127() {
        assert_eq!(rssi_text(None), "—");
        assert_eq!(rssi_text(Some(127)), "—");
        assert_eq!(rssi_text(Some(-200)), "—");
        // #174: 0 is the ideal range, a real value; no dBm unit.
        assert_eq!(rssi_text(Some(0)), "excellent (0)");
        assert_eq!(rssi_text(Some(-3)), "good (\u{2212}3)");
        assert_eq!(rssi_text(Some(-12)), "weak (\u{2212}12)");
        assert_eq!(rssi_text(Some(2)), "excellent (+2)");
        assert_eq!(rssi_level(Some(127)), Level::Unknown);
        assert_eq!(rssi_bar_count(Some(127)), 0);
        assert_eq!(rssi_bar_count(None), 0);
        assert_eq!(rssi_bar_count(Some(0)), 4);
        assert_eq!(rssi_bar_count(Some(-1)), 3);
    }

    #[test]
    fn estimate_and_age_texts() {
        let mut b = akm_core::report::KbBattery::default();
        assert_eq!(estimate_text(&b), None);
        b.charge_estimate =
            akm_core::chemistry::estimate_charge(2460, akm_core::chemistry::Chemistry::Alkaline);
        assert_eq!(
            estimate_text(&b).unwrap(),
            "\u{2248} 30% (20 to 40%), alkaline"
        );
        b.new_batteries = true;
        assert!(estimate_text(&b).unwrap().starts_with("new batteries"));
        assert_eq!(age_text(None), "—");
        assert_eq!(age_text(Some(12)), "12 s ago");
        assert_eq!(age_text(Some(300)), "5 min ago");
        assert_eq!(age_text(Some(7200)), "2 h ago");
        assert_eq!(age_text(Some(3 * 86_400)), "3 d ago");
    }

    #[test]
    fn tx_power_keeps_positive_values() {
        assert_eq!(tx_power_text(Some(4)), "4 dBm");
        assert_eq!(tx_power_text(Some(127)), "—");
        assert_eq!(tx_power_text(None), "—");
    }

    #[test]
    fn unknown_battery_is_not_zero_percent() {
        assert_eq!(pct_text(None, 0), "—");
        assert_eq!(pct_text(Some(f64::NAN), 0), "—");
        assert_eq!(pct_text(Some(73.4), 0), "73%");
        assert_eq!(pct_text(Some(73.46), 1), "73.5%");
        assert_eq!(pct_fraction(None), 0.0);
        assert_eq!(pct_fraction(Some(150.0)), 1.0);
        assert_eq!(battery_level(None), Level::Unknown);
    }

    #[test]
    fn levels_thresholds() {
        assert_eq!(battery_level(Some(51.0)), Level::Good);
        assert_eq!(battery_level(Some(50.0)), Level::Warn);
        assert_eq!(battery_level(Some(20.0)), Level::Bad);
        assert_eq!(voltage_level(2.9), Level::Good);
        assert_eq!(voltage_level(2.5), Level::Warn);
        assert_eq!(voltage_level(2.3), Level::Bad);
        assert_eq!(rssi_level(Some(0)), Level::Good);
        assert_eq!(rssi_level(Some(-5)), Level::Good);
        assert_eq!(rssi_level(Some(-9)), Level::Warn);
        assert_eq!(rssi_level(Some(-40)), Level::Bad);
        assert_ne!(rssi_bar_count(Some(0)), rssi_bar_count(Some(-40)));
    }

    #[test]
    fn palettes_differ_between_themes() {
        assert_ne!(Palette::new(true).good, Palette::new(false).good);
        assert!(Palette::new(true).color(Level::Unknown).is_none());
    }

    #[test]
    fn big_percentage_says_where_it_comes_from() {
        let mut b = akm_core::report::KbBattery::default();
        assert_eq!(pct_source(&b), PctSource::Unknown);
        b.percentage_estimate = Some(81.3);
        assert_eq!(pct_source(&b), PctSource::Estimate(81.3));
        assert!(pct_source(&b).caption().contains("estimate"));
        b.percentage_interpolated = Some(80.0);
        assert_eq!(pct_source(&b), PctSource::Estimate(81.3));
        b.percentage = Some(f64::NAN);
        assert_eq!(pct_source(&b), PctSource::Estimate(81.3));
        b.percentage = Some(96.0);
        assert_eq!(pct_source(&b), PctSource::Indication(96.0));
        assert_eq!(pct_source(&b).caption(), "keyboard indication");
        b.percentage = Some(250.0);
        b.percentage_fine = None;
        assert_eq!(pct_source(&b).value(), Some(81.3));
    }

    #[test]
    fn chemistry_prefers_the_declared_one() {
        let mut b = akm_core::report::KbBattery::default();
        assert_eq!(chemistry_text(&b), None);
        b.voltage = Some(2.95);
        assert!(chemistry_text(&b).unwrap().contains("guess"));
        b.voltage = Some(f64::NAN);
        assert_eq!(chemistry_text(&b), None);
        b.charge_estimate =
            akm_core::chemistry::estimate_charge(2460, akm_core::chemistry::Chemistry::Nimh);
        if b.charge_estimate.is_some() {
            assert_eq!(chemistry_text(&b).unwrap(), "nimh (declared)");
        }
    }

    #[test]
    fn paired_is_never_a_false_no() {
        let mut bt = akm_core::report::KbBluetooth::default();
        assert_eq!(paired_text(&bt), None);
        bt.paired_host_addr = Some("66:77:88:99:AA:BB".into());
        assert_eq!(paired_text(&bt), Some("Yes"));
        bt.paired_host_addr = None;
        bt.paired = true;
        assert_eq!(paired_text(&bt), Some("Yes"));
    }

    #[test]
    fn remaining_and_wake_texts() {
        let mut s = akm_core::Snapshot::default();
        assert_eq!(remaining_text(&s, 1000), None);
        s.remaining_display = Some("  ".into());
        assert_eq!(remaining_text(&s, 1000), None);
        s.remaining_display = Some("about 3 days".into());
        assert_eq!(remaining_text(&s, 1000).as_deref(), Some("about 3 days"));
        let mut w = akm_core::report::KbWake::default();
        assert_eq!(wake_text(&w), None);
        w.last_age_s = Some(f64::NAN);
        assert_eq!(wake_text(&w), None);
        w.last_age_s = Some(125.4);
        w.count = 3;
        assert_eq!(wake_text(&w).unwrap(), "2 min ago (3 since start)");
    }

    #[test]
    fn chart_drops_unusable_points() {
        let pts = [(10.0, 50.0), (5.0, 60.0), (11.0, f64::NAN), (12.0, 1e308), (13.0, -1.0), (1.0, 70.0), (f64::INFINITY, 1.0)];
        assert_eq!(chart_points(&pts, 2.0, true), vec![(5.0, 60.0), (10.0, 50.0)]);
        let v = [(1.0, 2.9), (2.0, 0.0), (3.0, 1e9)];
        assert_eq!(chart_points(&v, 0.0, false), vec![(1.0, 2.9)]);
    }

    #[test]
    fn layout_switches_to_one_column_when_narrow() {
        assert!(!two_columns(500.0));
        assert!(two_columns(704.0));
    }
}
