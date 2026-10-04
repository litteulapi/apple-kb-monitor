//! Numeric and hex conversions shared by the crate.

use std::fmt::Write as _;

/// Lower-case hex, no separator.
#[must_use]
pub fn hex_compact(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// `u64` as `f64`; exact below 2^53, beyond any time span or count handled here.
#[must_use]
#[allow(clippy::cast_precision_loss)] // reason: values stay far below 2^53
pub fn f64_from_u64(x: u64) -> f64 {
    x as f64
}

/// `i64` as `f64`; exact within ±2^53, beyond any value handled here.
#[must_use]
#[allow(clippy::cast_precision_loss)] // reason: values stay far below 2^53
pub fn f64_from_i64(x: i64) -> f64 {
    x as f64
}

/// `usize` as `f64` (counts of samples).
#[must_use]
pub fn f64_from_usize(x: usize) -> f64 {
    f64_from_u64(u64::try_from(x).unwrap_or(u64::MAX))
}

/// Rounded, saturating `f64` → `u64` (negative or NaN = 0).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // reason: `as` saturates, input clamped
pub fn u64_from_f64_round(x: f64) -> u64 {
    x.round().max(0.0) as u64
}

/// Milliseconds of a `Duration`, saturating at `u64::MAX`.
#[must_use]
pub fn millis_u64(d: std::time::Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// UTC civil date of a unix time: (year, month, day, seconds into the day).
#[must_use]
pub fn civil_utc(ts: u64) -> (i64, i64, i64, u64) {
    // Civil-from-days (H. Hinnant).
    let z = (ts / 86_400).cast_signed() + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d, ts % 86_400)
}

/// Broken-down local time of a unix time, `None` if the conversion fails.
#[must_use]
pub fn local_tm(ts: u64) -> Option<libc::tm> {
    let t: libc::time_t = ts.cast_signed();
    // SAFETY: `tm` is plain data fully written by localtime_r, which is
    // given valid pointers and is the thread-safe variant.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        (!libc::localtime_r(&raw const t, &raw mut tm).is_null()).then_some(tm)
    }
}

/// Rounded percentage, clamped to 0..=100.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped to 0..=100 first
pub fn pct_u8(p: f64) -> u8 {
    p.round().clamp(0.0, 100.0) as u8
}

/// [`pct_u8`] as the `i32` of the D-Bus properties.
#[must_use]
pub fn pct_i32(p: f64) -> i32 {
    i32::from(pct_u8(p))
}

/// Volts to rounded millivolts, clamped to the `u32` range.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped to the u32 range first
pub fn millivolts(v: f64) -> u32 {
    (v * 1000.0).round().clamp(0.0, f64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(hex_compact(&[0x00, 0xab, 0x10]), "00ab10");
        assert_eq!(hex_compact(&[]), "");
        assert!((f64_from_u64(86_400) - 86_400.0).abs() < f64::EPSILON);
        assert!((f64_from_i64(-3) + 3.0).abs() < f64::EPSILON);
        assert!((f64_from_usize(7) - 7.0).abs() < f64::EPSILON);
        assert_eq!(u64_from_f64_round(2.5), 3);
        assert_eq!(u64_from_f64_round(-1.0), 0);
        assert_eq!(u64_from_f64_round(f64::NAN), 0);
        assert_eq!(millis_u64(std::time::Duration::from_secs(2)), 2000);
    }

    #[test]
    fn clamps_and_rounds() {
        assert_eq!(pct_u8(42.5), 43);
        assert_eq!(pct_u8(-3.0), 0);
        assert_eq!(pct_u8(140.0), 100);
        assert_eq!(pct_u8(f64::NAN), 0);
        assert_eq!(pct_i32(99.6), 100);
        assert_eq!(millivolts(3.2104), 3210);
        assert_eq!(millivolts(-1.0), 0);
    }
}
