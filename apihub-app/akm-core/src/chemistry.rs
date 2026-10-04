//! Charge estimate corrected by battery chemistry.

use serde::{Deserialize, Serialize};

/// A new set of batteries is "new" for this long.
pub const NEW_SET_S: u64 = 2 * 24 * 3600;

/// Battery chemistry declared in `config.toml` (`[battery] chemistry`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Chemistry {
    /// Alkaline (or zinc-carbon) AA.
    #[default]
    Alkaline,
    /// `NiMH` rechargeable (Eneloop type).
    Nimh,
    /// Lithium Li-FeS2 (Energizer Ultimate Lithium).
    Lithium,
    /// Not declared: no estimate, the keyboard's own percentage is used.
    Unknown,
}

impl Chemistry {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "alkaline" | "alcaline" => Some(Self::Alkaline),
            "nimh" | "ni-mh" => Some(Self::Nimh),
            "lithium" => Some(Self::Lithium),
            "unknown" | "inconnue" => Some(Self::Unknown),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alkaline => "alkaline",
            Self::Nimh => "nimh",
            Self::Lithium => "lithium",
            Self::Unknown => "unknown",
        }
    }

    fn curve(self) -> &'static [(f64, f64)] {
        match self {
            // E91 / MN1500, 1 mA: V per cell x 2 for 100 - depth of discharge.
            Self::Alkaline => &[
                (3160.0, 100.0),
                (3060.0, 95.0),
                (3000.0, 90.0),
                (2920.0, 80.0),
                (2840.0, 70.0),
                (2760.0, 60.0),
                (2660.0, 50.0),
                (2560.0, 40.0),
                (2460.0, 30.0),
                (2340.0, 20.0),
                (2200.0, 10.0),
                (2060.0, 5.0),
                (1800.0, 0.0),
            ],
            // Eneloop-type NiMH: a very flat plateau at 1.20-1.25 V per cell.
            Self::Nimh => &[
                (2800.0, 100.0),
                (2640.0, 95.0),
                (2580.0, 90.0),
                (2540.0, 80.0),
                (2500.0, 60.0),
                (2460.0, 40.0),
                (2400.0, 20.0),
                (2340.0, 10.0),
                (2240.0, 5.0),
                (2000.0, 0.0),
            ],
            // L91 Li-FeS2: plateau at 1.5-1.65 V per cell, then a collapse.
            Self::Lithium => &[
                (3560.0, 100.0),
                (3400.0, 95.0),
                (3360.0, 90.0),
                (3300.0, 70.0),
                (3240.0, 50.0),
                (3160.0, 30.0),
                (3040.0, 15.0),
                (2940.0, 10.0),
                (2700.0, 5.0),
                (1800.0, 0.0),
            ],
            Self::Unknown => &[],
        }
    }

    /// Half-width of the range around the estimate, in points of %.
    #[must_use]
    pub fn spread(self) -> f64 {
        match self {
            Self::Alkaline => 10.0,
            Self::Nimh | Self::Lithium => 15.0,
            Self::Unknown => 0.0,
        }
    }
}

/// Estimated real charge, with its range. **\[hypothesis\]**, see the module doc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChargeEstimate {
    /// Estimated charge, % (rounded to the unit).
    pub pct: f64,
    /// Low end of the range, %.
    pub low: f64,
    /// High end of the range, %.
    pub high: f64,
    pub chemistry: Chemistry,
    /// Pair voltage the estimate is computed from (0x49, else 0x46), mV.
    pub basis_mv: u32,
}

/// Projects a pair voltage on the curve of `chem`.
#[must_use]
pub fn estimate_charge(mv: u32, chem: Chemistry) -> Option<ChargeEstimate> {
    let curve = chem.curve();
    let first = *curve.first()?;
    let last = *curve.last()?;
    let v = f64::from(mv);
    let pct = if v >= first.0 {
        first.1
    } else if v <= last.0 {
        last.1
    } else {
        curve.windows(2).find(|w| v >= w[1].0).map(|w| {
            let ((v_hi, p_hi), (v_lo, p_lo)) = (w[0], w[1]);
            p_lo + (v - v_lo) / (v_hi - v_lo) * (p_hi - p_lo)
        })?
    };
    let pct = pct.round().clamp(0.0, 100.0);
    let s = chem.spread();
    Some(ChargeEstimate {
        pct,
        low: (pct - s).max(0.0),
        high: (pct + s).min(100.0),
        chemistry: chem,
        basis_mv: mv,
    })
}

/// What the UI and the alerts get for one battery reading.
#[derive(Debug, Clone, PartialEq)]
pub struct Assessment {
    /// Estimate by chemistry; `None` for `Unknown`, without voltage or on a new set of batteries.
    pub estimate: Option<ChargeEstimate>,
    /// Batteries installed less than [`NEW_SET_S`] ago: no figure is drawn from the voltage.
    pub new_batteries: bool,
}

/// `set_age_s`: age of the current set of batteries, if the installation was detected.
#[must_use]
pub fn assess(
    mv_slow: Option<u32>,
    mv_inst: Option<u32>,
    chem: Chemistry,
    set_age_s: Option<u64>,
) -> Assessment {
    let new_batteries = set_age_s.is_some_and(|a| a < NEW_SET_S);
    let estimate = if new_batteries {
        None
    } else {
        mv_slow.or(mv_inst).and_then(|mv| estimate_charge(mv, chem))
    };
    Assessment {
        estimate,
        new_batteries,
    }
}

/// What a threshold alert is computed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertBasis {
    /// Charge estimated by chemistry.
    Estimate,
    /// The keyboard's own percentage (no estimate available).
    Firmware,
}

/// Percentage the alerts are computed on.
#[must_use]
pub fn alert_pct(estimate: Option<&ChargeEstimate>, firmware_pct: f64) -> (f64, AlertBasis) {
    match estimate {
        Some(e) => (e.pct, AlertBasis::Estimate),
        None => (firmware_pct, AlertBasis::Firmware),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pct(mv: u32, c: Chemistry) -> f64 {
        estimate_charge(mv, c).unwrap().pct
    }

    #[test]
    fn alkaline_matches_the_table_of_verif_batterie() {
        for (mv, want) in [
            (2950, 84.0),
            (2775, 62.0),
            (2600, 44.0),
            (2506, 35.0),
            (2404, 25.0),
            (2200, 10.0),
            (2054, 5.0),
        ] {
            let got = pct(mv, Chemistry::Alkaline);
            assert!((got - want).abs() <= 1.5, "{mv} mV: {got} != ~{want}");
        }
        let got = pct(2124, Chemistry::Alkaline);
        assert!((got - 7.0).abs() <= 1.5, "{got}");
    }

    #[allow(clippy::float_cmp)] // reason: exact values are the property under test
    #[test]
    fn other_chemistries_and_bounds() {
        assert_eq!(pct(2404, Chemistry::Nimh), 21.0);
        assert!(pct(2950, Chemistry::Lithium) <= 11.0);
        assert_eq!(pct(5000, Chemistry::Alkaline), 100.0);
        assert_eq!(pct(1000, Chemistry::Alkaline), 0.0);
        assert!(estimate_charge(2500, Chemistry::Unknown).is_none());
    }

    #[test]
    fn range_is_clamped_and_monotonic() {
        let e = estimate_charge(2054, Chemistry::Alkaline).unwrap();
        assert_eq!((e.low, e.high), (0.0, 15.0));
        let e = estimate_charge(3200, Chemistry::Alkaline).unwrap();
        assert_eq!((e.low, e.high), (90.0, 100.0));
        let e = estimate_charge(2404, Chemistry::Nimh).unwrap();
        assert_eq!((e.low, e.high), (6.0, 36.0));
        let mut last = 101.0;
        for mv in (1800..=3300).rev().step_by(10) {
            let p = pct(mv, Chemistry::Alkaline);
            assert!(p <= last, "not monotonic at {mv}");
            last = p;
        }
    }

    #[test]
    fn parse_names_and_fallback() {
        assert_eq!(Chemistry::parse(" NiMH "), Some(Chemistry::Nimh));
        assert_eq!(Chemistry::parse("alcaline"), Some(Chemistry::Alkaline));
        assert_eq!(Chemistry::parse("zinc"), None);
        assert_eq!(Chemistry::default(), Chemistry::Alkaline);
        for c in [
            Chemistry::Alkaline,
            Chemistry::Nimh,
            Chemistry::Lithium,
            Chemistry::Unknown,
        ] {
            assert_eq!(Chemistry::parse(c.as_str()), Some(c));
        }
    }

    #[test]
    fn assessment_prefers_slow_voltage_and_skips_new_sets() {
        let a = assess(Some(2404), Some(2440), Chemistry::Alkaline, None);
        assert_eq!(a.estimate.as_ref().unwrap().basis_mv, 2404);
        let a = assess(None, Some(2440), Chemistry::Alkaline, Some(10 * 86_400));
        assert_eq!(a.estimate.as_ref().unwrap().basis_mv, 2440);
        assert!(!a.new_batteries);
        let a = assess(Some(2950), Some(2990), Chemistry::Alkaline, Some(3600));
        assert!(a.new_batteries && a.estimate.is_none());
        let a = assess(None, None, Chemistry::Alkaline, None);
        assert!(a.estimate.is_none() && !a.new_batteries);
        assert!(assess(Some(2400), None, Chemistry::Unknown, None)
            .estimate
            .is_none());
    }

    #[test]
    fn alert_basis_falls_back_on_the_firmware() {
        let e = estimate_charge(2404, Chemistry::Alkaline).unwrap();
        assert_eq!(alert_pct(Some(&e), 50.0), (25.0, AlertBasis::Estimate));
        assert_eq!(alert_pct(None, 50.0), (50.0, AlertBasis::Firmware));
    }
}
