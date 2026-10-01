//! Pure presentation logic of the window (no egui context, unit-tested):
//! unknown-value formatting, semantic levels and the light/dark palette.

use eframe::egui::Color32;

/// BlueZ / the daemon report 127 when a radio value is unavailable.
pub const RADIO_UNKNOWN: i32 = 127;
/// Shown instead of any unknown value.
pub const DASH: &str = "—";

/// RSSI in dBm, `None` when unknown (absent, 127 or not a plausible negative value).
pub fn rssi_valid(r: Option<i32>) -> Option<i32> {
    r.filter(|v| *v != RADIO_UNKNOWN && (-127..0).contains(v))
}

pub fn rssi_text(r: Option<i32>) -> String {
    match rssi_valid(r) {
        Some(v) => format!("{v} dBm"),
        None => DASH.to_string(),
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
        Some(v) if v > -60 => Level::Good,
        Some(v) if v > -80 => Level::Warn,
        Some(_) => Level::Bad,
    }
}

/// Four-step signal bars; unknown is empty.
pub fn rssi_bars(r: Option<i32>) -> &'static str {
    match rssi_valid(r) {
        None => "\u{2581}\u{2581}\u{2581}\u{2581}",
        Some(v) if v > -50 => "\u{2582}\u{2584}\u{2586}\u{2588}",
        Some(v) if v > -60 => "\u{2582}\u{2584}\u{2586}\u{2581}",
        Some(v) if v > -70 => "\u{2582}\u{2584}\u{2581}\u{2581}",
        Some(v) if v > -80 => "\u{2582}\u{2581}\u{2581}\u{2581}",
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
        assert_eq!(rssi_text(Some(0)), "—");
        assert_eq!(rssi_text(Some(-55)), "-55 dBm");
        assert_eq!(rssi_level(Some(127)), Level::Unknown);
        assert_eq!(rssi_bars(Some(127)), rssi_bars(None));
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
        assert_eq!(rssi_level(Some(-59)), Level::Good);
        assert_eq!(rssi_level(Some(-85)), Level::Bad);
    }

    #[test]
    fn palettes_differ_between_themes() {
        assert_ne!(Palette::new(true).good, Palette::new(false).good);
        assert!(Palette::new(true).color(Level::Unknown).is_none());
    }
}
