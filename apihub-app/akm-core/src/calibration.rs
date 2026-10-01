//! Battery calibration (BCM2042).
//!
//! # Calibration contract (shared with the Python CLI `apple-kb-monitor`, #80)
//!
//! Report 0x5A carries 4 big-endian u16 thresholds in mV, bytes 1..9, for the
//! battery levels [100 %, 75 %, 50 %, 25 %]. A curve is usable only if it is
//! strictly decreasing and the last value is non-zero (`calibration_valid`);
//! otherwise the default `[2900, 2450, 2350, 2000]` is used. Between two
//! thresholds the percentage is interpolated linearly; at/above the first it is
//! 100, below the 4th it decays linearly to 0 mV = 0 %; the voltage is
//! `adc * 3.3 / 1023` from report 0xF5 (big-endian u16). This value is a
//! diagnostic: the displayed percentage comes from the kernel (`power`).

/// ADC max value (10-bit: 2^10 - 1 = 1023, not 1024)
pub const ADC_MAX: u32 = 1023;
/// ADC reference voltage (V)
pub const ADC_VREF: f64 = 3.3;
/// Default calibration curve [100%, 75%, 50%, 25%] in mV
pub const DEFAULT_CALIBRATION_MV: [u16; 4] = [2900, 2450, 2350, 2000];

/// Voltage of a raw 10-bit ADC reading.
pub fn adc_to_voltage(adc: u32) -> f64 {
    adc as f64 * ADC_VREF / ADC_MAX as f64
}

/// A calibration curve is usable only if it is strictly decreasing and non-zero.
pub fn calibration_valid(t: &[u16; 4]) -> bool {
    t[3] > 0 && t[0] > t[1] && t[1] > t[2] && t[2] > t[3]
}

/// Parse report 0x5A (`[id, 4 x u16 BE]`); `None` if short or invalid.
pub fn parse_calibration(buf: &[u8]) -> Option<[u16; 4]> {
    if buf.len() < 9 {
        return None;
    }
    let mut c = [0u16; 4];
    for (i, v) in c.iter_mut().enumerate() {
        let off = 1 + i * 2;
        *v = u16::from_be_bytes([buf[off], buf[off + 1]]);
    }
    calibration_valid(&c).then_some(c)
}

/// Interpolate battery % from voltage using the BCM2042 calibration curve.
pub fn interpolate_battery(voltage_v: f64, thresholds_mv: &[u16; 4]) -> f64 {
    if !calibration_valid(thresholds_mv) {
        return interpolate_battery(voltage_v, &DEFAULT_CALIBRATION_MV);
    }
    let mv = (voltage_v * 1000.0) as i32;
    let levels_pct = [100.0, 75.0, 50.0, 25.0, 0.0];
    let levels_mv = [
        thresholds_mv[0] as i32,
        thresholds_mv[1] as i32,
        thresholds_mv[2] as i32,
        thresholds_mv[3] as i32,
        0,
    ];
    if mv >= levels_mv[0] {
        return 100.0;
    }
    if mv <= 0 {
        return 0.0;
    }
    for i in 0..4 {
        if mv >= levels_mv[i + 1] {
            let hi_mv = levels_mv[i] as f64;
            let lo_mv = levels_mv[i + 1] as f64;
            if hi_mv == lo_mv {
                return levels_pct[i];
            }
            let frac = (mv as f64 - lo_mv) / (hi_mv - lo_mv);
            return levels_pct[i + 1] + frac * (levels_pct[i] - levels_pct[i + 1]);
        }
    }
    0.0
}

/// Detect battery chemistry from voltage (2xAA cells in series).
pub fn detect_battery_type(voltage: f64) -> &'static str {
    if voltage >= 3.1 {
        "Lithium (fresh)"
    } else if voltage >= 2.85 {
        "Alkaline (fresh)"
    } else if voltage >= 2.5 {
        "Alkaline or NiMH"
    } else if voltage >= 2.3 {
        "NiMH (likely)"
    } else if voltage >= 2.0 {
        "Depleted"
    } else {
        "Critical — replace"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_validation() {
        assert!(calibration_valid(&DEFAULT_CALIBRATION_MV));
        assert!(!calibration_valid(&[0, 0, 0, 0]));
        assert!(!calibration_valid(&[2000, 2450, 2350, 2900]));
        assert!(!calibration_valid(&[2900, 2900, 2350, 2000]));
    }

    #[test]
    fn calibration_report_parsing() {
        // Frame from tests/test_apple_kb.py: 2900, 2450, 2350, 2000 mV.
        let f = [0x5A, 0x0B, 0x54, 0x09, 0x92, 0x09, 0x2E, 0x07, 0xD0];
        assert_eq!(parse_calibration(&f), Some([2900, 2450, 2350, 2000]));
        assert_eq!(parse_calibration(&[0x5A, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(parse_calibration(&f[..8]), None);
    }

    #[test]
    fn interpolation_bounds_and_midpoints() {
        let c = DEFAULT_CALIBRATION_MV;
        assert_eq!(interpolate_battery(3.3, &c), 100.0);
        assert_eq!(interpolate_battery(2.9, &c), 100.0);
        assert_eq!(interpolate_battery(2.45, &c), 75.0);
        assert_eq!(interpolate_battery(2.35, &c), 50.0);
        assert_eq!(interpolate_battery(2.0, &c), 25.0);
        assert_eq!(interpolate_battery(0.0, &c), 0.0);
        assert_eq!(interpolate_battery(-1.0, &c), 0.0);
        let mid = interpolate_battery(2.675, &c);
        assert!((mid - 87.5).abs() < 0.01, "{}", mid);
        assert!((interpolate_battery(1.0, &c) - 12.5).abs() < 0.01);
    }

    #[test]
    fn interpolation_survives_garbled_calibration() {
        for bad in [[0u16; 4], [100, 200, 300, 400], [2900, 2900, 2900, 2900]] {
            for mv in [0.0, 1.0, 2.2, 2.6, 3.3] {
                let p = interpolate_battery(mv, &bad);
                assert!((0.0..=100.0).contains(&p), "{:?} {} -> {}", bad, mv, p);
                assert_eq!(p, interpolate_battery(mv, &DEFAULT_CALIBRATION_MV));
            }
        }
    }

    #[test]
    fn adc_voltage() {
        assert!((adc_to_voltage(1023) - 3.3).abs() < 1e-9);
        assert!((adc_to_voltage(0x0368) - 2.813).abs() < 0.001);
    }

    #[test]
    fn battery_type_thresholds() {
        assert_eq!(detect_battery_type(3.2), "Lithium (fresh)");
        assert_eq!(detect_battery_type(2.9), "Alkaline (fresh)");
        assert_eq!(detect_battery_type(1.0), "Critical — replace");
    }
}
