//! Quality of the Bluetooth link from the BR/EDR RSSI.

use serde::{Deserialize, Serialize};

/// `radio.rssi_kind` of a BR/EDR link.
pub const KIND_BREDR: &str = "bredr-golden-range";

/// Plausible range of the relative value (dB).
pub const REL_RANGE: std::ops::RangeInclusive<i32> = -127..=20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalQuality {
    /// In or above the ideal range (>= 0).
    Excellent,
    /// -1 to -5 dB under the range.
    Good,
    /// More than 5 dB under the range.
    Weak,
}

impl SignalQuality {
    /// Stable machine name (`radio.rssi_quality`, JSON, CLI).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Excellent => "excellent",
            Self::Good => "good",
            Self::Weak => "weak",
        }
    }
}

/// Relative value if it is a plausible measurement (not `127`, in range).
#[must_use]
pub fn valid_rel(rel: Option<i32>) -> Option<i32> {
    rel.filter(|r| *r != 127 && REL_RANGE.contains(r))
}

/// Quality of a relative RSSI; positive values are legal and excellent.
#[must_use]
pub fn quality(rel: i32) -> SignalQuality {
    match rel {
        r if r >= 0 => SignalQuality::Excellent,
        -5..=-1 => SignalQuality::Good,
        _ => SignalQuality::Weak,
    }
}

/// Raw value for display: real minus sign, `+` for a positive value, no unit.
#[must_use]
pub fn raw_text(rel: i32) -> String {
    match rel {
        r if r < 0 => format!("\u{2212}{}", -r),
        r if r > 0 => format!("+{r}"),
        _ => "0".to_string(),
    }
}

/// `"Signal : bon (−2)"` / `"Signal: good (−2)"`; `None` if unknown.
#[must_use]
pub fn text(rel: Option<i32>) -> Option<String> {
    use crate::tr;
    let r = valid_rel(rel)?;
    let q = match quality(r) {
        SignalQuality::Excellent => tr!("excellent"),
        SignalQuality::Good => tr!("good"),
        SignalQuality::Weak => tr!("weak"),
    };
    Some(tr!(
        "Signal: {quality} ({value})",
        quality = q,
        value = raw_text(r)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_bands_accept_positive_values() {
        assert_eq!(quality(0), SignalQuality::Excellent);
        assert_eq!(quality(3), SignalQuality::Excellent);
        assert_eq!(quality(-1), SignalQuality::Good);
        assert_eq!(quality(-5), SignalQuality::Good);
        assert_eq!(quality(-6), SignalQuality::Weak);
        assert_eq!(quality(-90), SignalQuality::Weak);
    }

    #[test]
    fn text_has_no_dbm_and_zero_is_a_real_value() {
        assert_eq!(text(Some(0)).unwrap(), "Signal: excellent (0)");
        assert_eq!(text(Some(-3)).unwrap(), "Signal: good (\u{2212}3)");
        assert_eq!(text(Some(-8)).unwrap(), "Signal: weak (\u{2212}8)");
        assert_eq!(text(Some(2)).unwrap(), "Signal: excellent (+2)");
        for r in [0, -3, 2, -40] {
            assert!(!text(Some(r)).unwrap().to_lowercase().contains("dbm"));
        }
    }

    #[test]
    fn unknown_values_are_absent_never_127() {
        assert_eq!(text(None), None);
        assert_eq!(text(Some(127)), None);
        assert_eq!(text(Some(-200)), None);
        assert_eq!(valid_rel(Some(21)), None);
        assert_eq!(valid_rel(Some(0)), Some(0));
        assert_eq!(SignalQuality::Weak.as_str(), "weak");
    }
}
