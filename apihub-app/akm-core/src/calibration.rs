//! Battery calibration (BCM2042) — **estimates only** (#136).
//!
//! Evidence (docs/AUDIT-DECODAGE-HID.md, A1314 ISO, 2026-10-01):
//! - [mesuré] report 0x5A carries 4 big-endian u16, strictly decreasing
//!   (`0b8a 09ca 0964 0806` = 2954, 2506, 2404, 2054), identical to 0x60 and 0xEB.
//! - [hypothèse] these are mV thresholds for [100 %, 75 %, 50 %, 25 %].
//! - [mesuré] the cell voltage is the u16 LE of report 0x46 (2986-2991 mV) and
//!   the u16 BE of bytes 1-2 of report 0xFF (same value); 0x49 is a u16 LE
//!   (2953 then 2950 mV, [hypothèse] filtered voltage). 0xF5 (900) is constant
//!   across a battery change: it is **not** a voltage (#139).
//! - [hypothèse] the estimate is made on the filtered voltage (0x49, else
//!   0x46): 0x49 = 2950 mV gives 99.78 %, i.e. 99 = 0x47 = kernel.
//! - [hypothèse] below the 25 % threshold the charge decays linearly down to
//!   [`CUTOFF_MV`] (0 %), not to 0 mV (a keyboard at 1.8 V is out of service).
//!
//! No interpolation is made without a curve read from the device: the former
//! default curve `[2900, 2450, 2350, 2000]` had no source. The displayed
//! percentage comes from the kernel (`power`) or report 0x47.

/// 0 % point of the estimate: 2 × 0.9 V, the usual end-of-discharge of two
/// alkaline AA cells in series [hypothèse].
pub const CUTOFF_MV: u16 = 1800;
/// Plausible range (mV) of a 2 x AA cell voltage read from the device;
/// anything outside is a corrupt read and is dropped.
pub const PLAUSIBLE_MV: std::ops::RangeInclusive<u16> = 1000..=4500;
/// Max gap (mV) between 0x46 and 0xFF before a sample is flagged doubtful.
pub const MAX_SOURCE_GAP_MV: u16 = 20;

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

/// Round to one decimal (0.1 %).
fn tenth(p: f64) -> f64 {
    (p * 10.0).round() / 10.0
}

/// Estimated battery % (0.1 % resolution) of a voltage in mV on the curve
/// read from the device (report 0x5A as [100, 75, 50, 25] % thresholds, linear
/// between points, linear down to [`CUTOFF_MV`] = 0 %) [hypothèse]. `None` if
/// the curve is invalid or the voltage is not finite. Always within 0..=100.
pub fn estimate_percentage_mv(mv: f64, thresholds_mv: &[u16; 4]) -> Option<f64> {
    if !calibration_valid(thresholds_mv) || !mv.is_finite() {
        return None;
    }
    let cutoff = f64::from(CUTOFF_MV.min(thresholds_mv[3]));
    let levels_pct = [100.0, 75.0, 50.0, 25.0, 0.0];
    let levels_mv = [
        f64::from(thresholds_mv[0]),
        f64::from(thresholds_mv[1]),
        f64::from(thresholds_mv[2]),
        f64::from(thresholds_mv[3]),
        cutoff,
    ];
    if mv >= levels_mv[0] {
        return Some(100.0);
    }
    if mv <= cutoff {
        return Some(0.0);
    }
    for i in 0..4 {
        let (hi, lo) = (levels_mv[i], levels_mv[i + 1]);
        if mv >= lo {
            if hi <= lo {
                return Some(levels_pct[i + 1]);
            }
            let frac = (mv - lo) / (hi - lo);
            let p = levels_pct[i + 1] + frac * (levels_pct[i] - levels_pct[i + 1]);
            return Some(tenth(p).clamp(0.0, 100.0));
        }
    }
    Some(0.0)
}

/// Same as [`estimate_percentage_mv`] for a voltage in volts.
pub fn interpolate_battery(voltage_v: f64, thresholds_mv: &[u16; 4]) -> Option<f64> {
    estimate_percentage_mv(voltage_v * 1000.0, thresholds_mv)
}

/// Guess of the battery chemistry from the voltage estimate (2 x AA in
/// series). [hypothèse] thresholds without source, applied to a voltage that
/// is itself an estimate: display as an estimate only.
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

    /// [mesuré] Report 0x5A of the A1314 ISO, 2026-10-01.
    const REAL: [u16; 4] = [2954, 2506, 2404, 2054];

    #[test]
    fn calibration_validation() {
        assert!(calibration_valid(&REAL));
        assert!(!calibration_valid(&[0, 0, 0, 0]));
        assert!(!calibration_valid(&[2000, 2450, 2350, 2900]));
        assert!(!calibration_valid(&[2900, 2900, 2350, 2000]));
    }

    #[test]
    fn calibration_report_parsing() {
        let f = [0x5A, 0x0B, 0x8A, 0x09, 0xCA, 0x09, 0x64, 0x08, 0x06];
        assert_eq!(parse_calibration(&f), Some(REAL));
        assert_eq!(parse_calibration(&[0x5A, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(parse_calibration(&f[..8]), None);
    }

    #[test]
    fn interpolation_bounds_and_midpoints() {
        let c = REAL;
        let p = |v: f64| (interpolate_battery(v, &c).unwrap() * 1e6).round() / 1e6;
        assert_eq!(p(3.3), 100.0);
        assert_eq!(p(2.954), 100.0);
        assert_eq!(p(2.506), 75.0);
        assert_eq!(p(2.404), 50.0);
        assert_eq!(p(2.054), 25.0);
        // Below the last threshold: down to the cut-off, never to 0 mV (#136).
        assert_eq!(p(1.8), 0.0);
        assert_eq!(p(0.0), 0.0);
        assert_eq!(p(-1.0), 0.0);
        assert!((p(1.927) - 12.5).abs() < 0.01);
        // [mesuré] 0x49 = 2950 mV -> 99.8 % (99.78, floor = 0x47 = 99).
        assert_eq!(estimate_percentage_mv(2950.0, &c), Some(99.8));
        assert_eq!(estimate_percentage_mv(2991.0, &c), Some(100.0));
        assert_eq!(estimate_percentage_mv(f64::INFINITY, &c), None);
        assert!(interpolate_battery(f64::NAN, &c).is_none());
    }

    #[test]
    fn no_interpolation_without_a_valid_curve() {
        for bad in [[0u16; 4], [100, 200, 300, 400], [2900, 2900, 2900, 2900]] {
            assert!(interpolate_battery(2.6, &bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn curve_below_cutoff_is_bounded() {
        let low = [1700, 1600, 1500, 1400];
        for mv in [0.0, 1.0, 1.45, 1.65, 2.0] {
            let p = interpolate_battery(mv, &low).unwrap();
            assert!((0.0..=100.0).contains(&p), "{mv} -> {p}");
        }
    }

    #[test]
    fn battery_type_thresholds() {
        assert_eq!(detect_battery_type(3.2), "Lithium (fresh)");
        assert_eq!(detect_battery_type(2.9), "Alkaline (fresh)");
        assert_eq!(detect_battery_type(1.0), "Critical — replace");
    }
}
