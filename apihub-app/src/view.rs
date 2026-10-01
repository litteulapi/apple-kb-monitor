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

/// Percentage interpolated on the discharge curve: always marked as an estimate.
pub fn curve_text(p: f64) -> String {
    format!("{p:.0} % (estimation)")
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

/// Four-step signal bars on the relative scale; unknown is empty.
pub fn rssi_bars(r: Option<i32>) -> &'static str {
    match rssi_valid(r) {
        None => "\u{2581}\u{2581}\u{2581}\u{2581}",
        Some(v) if v >= 0 => "\u{2582}\u{2584}\u{2586}\u{2588}",
        Some(v) if v >= -2 => "\u{2582}\u{2584}\u{2586}\u{2581}",
        Some(v) if v >= -5 => "\u{2582}\u{2584}\u{2581}\u{2581}",
        Some(v) if v >= -10 => "\u{2582}\u{2581}\u{2581}\u{2581}",
        Some(_) => "\u{2581}\u{2581}\u{2581}\u{2581}",
    }
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
        assert_eq!(curve_text(99.78), "100 % (estimation)");
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
        assert_eq!(rssi_bars(Some(127)), rssi_bars(None));
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
        assert_ne!(rssi_bars(Some(0)), rssi_bars(Some(-40)));
    }

    #[test]
    fn palettes_differ_between_themes() {
        assert_ne!(Palette::new(true).good, Palette::new(false).good);
        assert!(Palette::new(true).color(Level::Unknown).is_none());
    }
}
