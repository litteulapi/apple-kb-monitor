//! Battery calibration (BCM2042) — **estimates only**.

/// 0 % point of the estimate.
pub const CUTOFF_MV: u16 = 1800;
/// Plausible range (mV) of a 2 x AA cell voltage read from the device.
pub const PLAUSIBLE_MV: std::ops::RangeInclusive<u16> = 1000..=4500;
/// Max gap (mV) between 0x46 and 0xFF before a sample is flagged doubtful.
pub const MAX_SOURCE_GAP_MV: u16 = 20;

/// A calibration curve is usable only if it is strictly decreasing and non-zero.
#[must_use]
pub fn calibration_valid(t: &[u16; 4]) -> bool {
    t[3] > 0 && t[0] > t[1] && t[1] > t[2] && t[2] > t[3]
}

/// Parse report 0x5A (`[id, 4 x u16 BE]`); `None` if short or invalid.
#[must_use]
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

fn tenth(p: f64) -> f64 {
    (p * 10.0).round() / 10.0
}

/// Estimated battery % of a voltage in mV on the curve read from the device \[hypothesis\].
#[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
