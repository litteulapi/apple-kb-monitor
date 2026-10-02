//! Pure presentation logic of the window (no egui context, unit-tested):
//! unknown-value formatting, semantic levels and the light/dark palette.

use eframe::egui::Color32;

use crate::i18n::{is_french, tr, tr_in, trf, trf_in};

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
        Some(v) => {
            use akm_core::signal::SignalQuality as Q;
            let word = match akm_core::signal::quality(v) {
                Q::Excellent => tr("excellent"),
                Q::Good => tr("good"),
                Q::Weak => tr("weak"),
            };
            format!("{} ({})", word, akm_core::signal::raw_text(v))
        }
        None => DASH.to_string(),
    }
}

/// Estimated charge by chemistry, always marked as an estimate (#178).
pub fn estimate_text(b: &akm_core::report::KbBattery) -> Option<String> {
    estimate_text_in(is_french(), b)
}

pub fn estimate_text_in(fr: bool, b: &akm_core::report::KbBattery) -> Option<String> {
    if b.new_batteries {
        return Some(tr_in(fr, "new batteries, no estimate yet").to_string());
    }
    let e = b.charge_estimate.as_ref()?;
    Some(trf_in(
        fr,
        "\u{2248} {}% ({} to {}%), {}",
        &[&format!("{:.0}", e.pct), &format!("{:.0}", e.low), &format!("{:.0}", e.high), &e.chemistry.as_str()],
    ))
}

/// Decimal comma in French (`2.99` -> `2,99`), unchanged otherwise.
fn dec_in(fr: bool, s: String) -> String {
    if fr {
        s.replace('.', ",")
    } else {
        s
    }
}

/// The firmware line of the window and its level (#227): `Firmware : 0x0050 —
/// à jour (dernière version publique connue d'Apple)` in French. `None` while
/// the version was not read.
pub fn firmware_line(fw: &akm_core::report::KbFirmware, fr: bool) -> Option<(String, Level)> {
    let text = if fr {
        akm_core::firmware::summary_fr(fw)
    } else {
        akm_core::firmware::summary_en(fw)
    }?;
    let level = match fw.status.as_str() {
        "up_to_date" => Level::Good,
        "update_available" => Level::Warn,
        _ => Level::Unknown,
    };
    Some((text, level))
}

/// Percentage as macOS shows it, labelled (#213).
pub fn apple_display_text(b: &akm_core::report::KbBattery) -> Option<String> {
    let p = b.apple_display_pct.filter(|p| p.is_finite())?;
    Some(trf("{}% (macOS-style display)", &[&format!("{p:.0}")]))
}

/// Battery thresholds read from the keyboard (report 0x60) and where the
/// voltage sits against them.
pub fn thresholds_text(b: &akm_core::report::KbBattery) -> Option<String> {
    thresholds_text_in(is_french(), b)
}

pub fn thresholds_text_in(fr: bool, b: &akm_core::report::KbBattery) -> Option<String> {
    let t = b.thresholds?;
    Some(match b.threshold_margins_mv {
        Some([_, low, crit, _]) => trf_in(
            fr,
            "Full {} / Low {} / Critical {} / Empty {} mV \u{b7} {} mV to Low, {} mV to Critical",
            &[&t.full_mv, &t.low_mv, &t.critical_mv, &t.empty_mv, &format!("{low:+}"), &format!("{crit:+}")],
        ),
        None => trf_in(
            fr,
            "Full {} / Low {} / Critical {} / Empty {} mV",
            &[&t.full_mv, &t.low_mv, &t.critical_mv, &t.empty_mv],
        ),
    })
}

/// Age of the last reading. The kernel percentage only steps down at
/// reconnections, so this age says how stale the indication can be (#179).
pub fn age_text(age_s: Option<u64>) -> String {
    age_text_in(is_french(), age_s)
}

pub fn age_text_in(fr: bool, age_s: Option<u64>) -> String {
    match age_s {
        None => DASH.to_string(),
        Some(a) if a < 60 => trf_in(fr, "{} s ago", &[&a]),
        Some(a) if a < 3600 => trf_in(fr, "{} min ago", &[&(a / 60)]),
        Some(a) if a < 86_400 => trf_in(fr, "{} h ago", &[&(a / 3600)]),
        Some(a) => trf_in(fr, "{} d ago", &[&(a / 86_400)]),
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
        Some(v) => {
            let n = dec_in(is_french(), format!("{v:.decimals$}"));
            // French typography: narrow no-break space before the percent sign.
            if is_french() { format!("{n}\u{202f}%") } else { format!("{n}%") }
        }
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
    let fr = is_french();
    format!("{}{}V", dec_in(fr, format!("{v:.2}")), if fr { "\u{202f}" } else { " " })
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
            Self::Indication(_) => tr("keyboard indication"),
            Self::Estimate(_) => tr("estimate from the voltage curve"),
            Self::Unknown => tr("battery level unknown"),
        }
    }
}

/// Battery chemistry line: the one declared to the daemon when it made an
/// estimate, else a guess from the voltage, always marked as such.
pub fn chemistry_text(b: &akm_core::report::KbBattery) -> Option<String> {
    if let Some(e) = &b.charge_estimate {
        return Some(trf("{} (declared)", &[&e.chemistry.as_str()]));
    }
    b.voltage
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| trf("{} (guess from the voltage)", &[&akm_core::calibration::detect_battery_type(v)]))
}

/// Paired state. The daemon never fills `bluetooth.paired` (always false
/// while BlueZ says Paired=true, #198): only the paired host read from the
/// keyboard (0x4C) or an explicit `true` prove it; otherwise unknown.
pub fn paired_text(b: &akm_core::report::KbBluetooth) -> Option<&'static str> {
    (b.paired || b.paired_host_addr.is_some()).then(|| tr("Yes"))
}

/// Autonomy left: the daemon's forecast, else the legacy text.
pub fn remaining_text(s: &akm_core::Snapshot, now: u64) -> Option<String> {
    s.remaining_s(now)
        .map(format_days)
        .or_else(|| s.remaining_display.clone().filter(|t| !t.trim().is_empty()))
}

/// `"≈ 41 days left"`, `"≈ 20 h left"` (same rule as the core's English
/// text, translated here).
fn format_days(remaining_s: u64) -> String {
    let days = remaining_s as f64 / 86_400.0;
    if days >= 2.0 {
        trf("\u{2248} {} days left", &[&format!("{days:.0}")])
    } else {
        trf("\u{2248} {} h left", &[&format!("{:.0}", remaining_s as f64 / 3600.0)])
    }
}

/// Last wake event of the keyboard (input report 0x13), passive listening.
pub fn wake_text(w: &akm_core::report::KbWake) -> Option<String> {
    let age = w.last_age_s.filter(|a| a.is_finite() && *a >= 0.0)?;
    Some(trf("{} ({} since start)", &[&age_text(Some(age as u64)), &w.count]))
}

/// Two side-by-side tiles only when each gets a usable width; below, the
/// tiles are stacked (#195).
pub const TWO_COLUMNS_MIN_WIDTH: f32 = 660.0;
pub fn two_columns(width: f32) -> bool {
    width >= TWO_COLUMNS_MIN_WIDTH
}

/// Period shown by the history chart (#96). The daemon keeps 90 days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Range {
    #[default]
    Day,
    Week,
    Month,
    Quarter,
}

impl Range {
    pub const ALL: [Range; 4] = [Range::Day, Range::Week, Range::Month, Range::Quarter];

    pub fn seconds(self) -> f64 {
        86_400.0
            * match self {
                Range::Day => 1.0,
                Range::Week => 7.0,
                Range::Month => 30.0,
                Range::Quarter => 90.0,
            }
    }

    /// Short text of the selector button.
    pub fn button(self) -> &'static str {
        match self {
            Range::Day => tr("24 h"),
            Range::Week => tr("7 d"),
            Range::Month => tr("30 d"),
            Range::Quarter => tr("90 d"),
        }
    }

    /// The period in words, for the messages and the screen reader.
    pub fn text(self) -> &'static str {
        match self {
            Range::Day => tr("24 h"),
            Range::Week => tr("7 days"),
            Range::Month => tr("30 days"),
            Range::Quarter => tr("90 days"),
        }
    }
}

/// Most points one series of the chart ever draws, whatever the period and
/// the size of the history (#96): 90 days of readings are reduced to this
/// before reaching the painter.
pub const CHART_POINTS_MAX: usize = 720;

/// Reduce a time-sorted series to at most `max` points, keeping the first
/// and last points and the lowest and highest value of each slice, so that a
/// dip or a battery change stays visible. Unchanged when it already fits.
pub fn downsample(pts: Vec<(f64, f64)>, max: usize) -> Vec<(f64, f64)> {
    if pts.len() <= max || max < 4 {
        return pts;
    }
    let inner = &pts[1..pts.len() - 1];
    let buckets = (max - 2) / 2;
    let mut out = Vec::with_capacity(max);
    out.push(pts[0]);
    for b in 0..buckets {
        let s = &inner[b * inner.len() / buckets..(b + 1) * inner.len() / buckets];
        let (mut lo, mut hi) = (0, 0);
        for (i, p) in s.iter().enumerate() {
            if p.1 < s[lo].1 {
                lo = i;
            }
            if p.1 > s[hi].1 {
                hi = i;
            }
        }
        if let Some(first) = s.get(lo.min(hi)) {
            out.push(*first);
            if lo != hi {
                out.push(s[lo.max(hi)]);
            }
        }
    }
    out.push(pts[pts.len() - 1]);
    out
}
/// Clock skew tolerated after "now" before a point counts as future (#236).
pub const CLOCK_SLACK_S: f64 = 600.0;

/// History points that can be drawn: finite, percentage in 0..=100, positive
/// voltage, inside the window of `window` seconds starting at `cutoff` (a
/// point dated in the future would squash the real period into a few pixels,
/// #236), sorted by time.
pub fn chart_points(pts: &[(f64, f64)], cutoff: f64, window: f64, pct: bool) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = pts
        .iter()
        .copied()
        .filter(|(t, y)| t.is_finite() && y.is_finite() && *t >= cutoff && *t <= cutoff + window + CLOCK_SLACK_S)
        .filter(|(_, y)| if pct { (0.0..=100.0).contains(y) } else { *y > 0.0 && *y < 10.0 })
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v
}

/// Longest text the chart ever lays out (legend, tick); a `{:.2}` of
/// `f64::MAX` was a 309-digit legend in 3.1.0 (#230).
pub const CHART_LABEL_MAX: usize = 40;
/// At most this many time ticks under the chart.
pub const CHART_TICKS_MAX: usize = 6;

/// Everything the history chart draws, computed without egui (unit-tested):
/// the UI only maps it to pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartModel {
    pub batt: Vec<(f64, f64)>,
    pub volt: Vec<(f64, f64)>,
    pub t_min: f64,
    pub t_max: f64,
    /// Voltage axis (low, high), padded; `None` when fewer than 2 voltages.
    pub volt_axis: Option<(f64, f64)>,
    /// (timestamp, label) under the plot, e.g. "6 h ago", "now".
    pub x_ticks: Vec<(f64, String)>,
    /// One text per drawn series, battery first.
    pub legend: Vec<String>,
    pub summary: String,
}

/// The chart of the last `range` (#96), each series reduced to
/// [`CHART_POINTS_MAX`] points. `Err` = the text shown instead of a chart.
pub fn chart_model(
    battery: &[(f64, f64)],
    voltage: &[(f64, f64)],
    now: f64,
    range: Range,
) -> Result<ChartModel, String> {
    let window = range.seconds();
    let batt = chart_points(battery, now - window, window, true);
    if batt.is_empty() {
        return Err(if battery.is_empty() {
            tr("No history data yet.").into()
        } else {
            trf("No data in the last {}.", &[&range.text()])
        });
    }
    let (t_min, t_max) = (batt[0].0, batt[batt.len() - 1].0);
    if batt.len() < 2 || t_max - t_min < 1.0 {
        let last = pct_text(Some(batt[batt.len() - 1].1), 0);
        return Err(trf("Only one reading in the last {}: {}.", &[&range.text(), &last]));
    }
    let volt: Vec<(f64, f64)> = chart_points(voltage, t_min, window, false).into_iter().filter(|p| p.0 <= t_max).collect();
    // The summary counts the real readings; the painter gets a bounded series.
    let readings = batt.len();
    let batt = downsample(batt, CHART_POINTS_MAX);
    let volt = downsample(volt, CHART_POINTS_MAX);
    let mut legend = vec![tr("Battery %").to_string()];
    let volt_axis = if volt.len() >= 2 {
        let lo = volt.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let hi = volt.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        // Constant voltage: a flat line in the middle, not a division by 0.
        let pad = ((hi - lo) * 0.05).max(0.05);
        let fr = is_french();
        legend.push(clip_label(&trf(
            "Voltage {}\u{2013}{} V",
            &[&dec_in(fr, format!("{lo:.2}")), &dec_in(fr, format!("{hi:.2}"))],
        )));
        Some((lo - pad, hi + pad))
    } else {
        None
    };
    let hours = (t_max - t_min) / 3600.0;
    Ok(ChartModel {
        x_ticks: time_ticks(t_min, t_max, now),
        summary: trf("{} points over {}", &[&readings, &span_text(hours)]),
        batt,
        volt,
        t_min,
        t_max,
        volt_axis,
        legend,
    })
}

fn clip_label(s: &str) -> String {
    if s.chars().count() <= CHART_LABEL_MAX {
        s.to_string()
    } else {
        s.chars().take(CHART_LABEL_MAX - 1).chain(std::iter::once('\u{2026}')).collect()
    }
}

fn span_text(hours: f64) -> String {
    if hours < 1.0 {
        trf("{} min", &[&format!("{:.0}", (hours * 60.0).max(1.0))])
    } else if hours < 48.0 {
        trf("{} h", &[&dec_in(is_french(), format!("{hours:.1}"))])
    } else {
        trf("{} d", &[&dec_in(is_french(), format!("{:.1}", hours / 24.0))])
    }
}

/// Round time ticks between `t_min` and `t_max`, labelled relative to `now`
/// ("6 h ago", "30 min ago", "now"), never more than [`CHART_TICKS_MAX`].
pub fn time_ticks(t_min: f64, t_max: f64, now: f64) -> Vec<(f64, String)> {
    if !(t_min.is_finite() && t_max.is_finite() && now.is_finite()) || t_max <= t_min {
        return Vec::new();
    }
    const DAY: f64 = 86400.0;
    const STEPS: [f64; 14] = [300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0, 10800.0, 21600.0, 43200.0, DAY, 2.0 * DAY, 7.0 * DAY, 14.0 * DAY, 30.0 * DAY];
    let span = t_max - t_min;
    let step = STEPS.iter().copied().find(|s| span / s <= (CHART_TICKS_MAX - 1) as f64).unwrap_or(30.0 * DAY);
    // Ticks at whole multiples of `step` before `now`, inside [t_min, t_max].
    let mut ticks = Vec::new();
    let mut k = ((now - t_max) / step).ceil().max(0.0);
    loop {
        let t = now - k * step;
        if t < t_min || ticks.len() >= CHART_TICKS_MAX {
            break;
        }
        let ago = (k * step).round() as u64;
        let label = match ago {
            0 => tr("now").to_string(),
            a if a < 3600 => trf("{} min ago", &[&(a / 60)]),
            a if a < 172_800 => trf("{} h ago", &[&(a / 3600)]),
            a => trf("{} d ago", &[&(a / 86_400)]),
        };
        ticks.push((t, label));
        k += 1.0;
    }
    ticks.reverse();
    ticks
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
        assert_eq!(chart_points(&pts, 2.0, 86_400.0, true), vec![(5.0, 60.0), (10.0, 50.0)]);
        let v = [(1.0, 2.9), (2.0, 0.0), (3.0, 1e9)];
        assert_eq!(chart_points(&v, 0.0, 86_400.0, false), vec![(1.0, 2.9)]);
    }

    fn labels_are_short(m: &ChartModel) {
        assert!(m.x_ticks.len() <= CHART_TICKS_MAX, "{:?}", m.x_ticks);
        for t in m.legend.iter().chain(m.x_ticks.iter().map(|t| &t.1)).chain(std::iter::once(&m.summary)) {
            assert!(t.chars().count() <= CHART_LABEL_MAX, "label too long: {t}");
            assert!(!t.contains("inf") && !t.contains("NaN"), "{t}");
        }
    }

    /// 3.1.0 printed `{:.2}` of f64::MAX as the voltage legend when every
    /// voltage was unreliable: a 309-digit string (#230).
    #[test]
    fn real_history_without_reliable_voltage_has_a_short_legend() {
        let now = 1_790_858_400.0;
        let batt: Vec<(f64, f64)> = (0..56).map(|i| (now - 43_200.0 + i as f64 * 785.0, if i < 7 { 90.0 } else { 96.0 })).collect();
        let m = chart_model(&batt, &[], now, Range::Day).unwrap();
        assert_eq!(m.legend, vec!["Battery %".to_string()]);
        assert_eq!(m.volt_axis, None);
        assert_eq!(m.summary, "56 points over 12.0 h");
        labels_are_short(&m);
        assert!(m.x_ticks.len() >= 2);
        assert!(m.x_ticks.iter().all(|(t, _)| *t >= m.t_min && *t <= m.t_max));
    }

    #[test]
    fn chart_handles_empty_single_constant_and_nan() {
        let now = 100_000.0;
        assert_eq!(chart_model(&[], &[], now, Range::Day), Err("No history data yet.".into()));
        assert_eq!(chart_model(&[(1.0, 50.0)], &[], now, Range::Day), Err("No data in the last 24 h.".into()));
        assert_eq!(chart_model(&[(now - 10.0, 42.0)], &[], now, Range::Day), Err("Only one reading in the last 24 h: 42%.".into()));
        let nan = [(now - 100.0, f64::NAN), (now - 50.0, f64::NAN)];
        assert_eq!(chart_model(&nan, &nan, now, Range::Day), Err("No data in the last 24 h.".into()));
        // Constant battery and voltage: a padded axis, no division by zero.
        let batt = [(now - 7200.0, 80.0), (now - 3600.0, 80.0), (now, 80.0)];
        let volt = [(now - 7200.0, 2.9), (now - 3600.0, 2.9), (now, 2.9)];
        let m = chart_model(&batt, &volt, now, Range::Day).unwrap();
        let (lo, hi) = m.volt_axis.unwrap();
        assert!(lo < 2.9 && hi > 2.9 && (hi - lo) < 1.0);
        assert_eq!(m.legend[1], "Voltage 2.90\u{2013}2.90 V");
        assert_eq!(m.x_ticks.last().map(|t| t.1.as_str()), Some("now"));
        labels_are_short(&m);
        // Absurd voltages are dropped, never formatted.
        let m = chart_model(&batt, &[(now - 10.0, f64::MAX), (now - 5.0, f64::MIN)], now, Range::Day).unwrap();
        assert_eq!(m.legend.len(), 1);
        // A point dated in 30 days does not stretch the time axis (#236).
        let mut fut = batt.to_vec();
        fut.push((now + 30.0 * 86_400.0, 50.0));
        let m = chart_model(&fut, &[], now, Range::Day).unwrap();
        assert_eq!(m.t_max, now);
        assert_eq!(m.summary, "3 points over 2.0 h");
    }

    #[test]
    fn time_ticks_are_bounded_and_relative() {
        let now = 1_000_000.0;
        for span in [60.0, 600.0, 3600.0, 5.0 * 3600.0, 12.0 * 3600.0, 24.0 * 3600.0, 1e7] {
            let t = time_ticks(now - span, now, now);
            assert!(!t.is_empty() && t.len() <= CHART_TICKS_MAX, "{span}: {t:?}");
            assert_eq!(t.last().unwrap().1, "now");
        }
        assert_eq!(time_ticks(now - 43_200.0, now, now).iter().map(|t| t.1.as_str()).collect::<Vec<_>>(), ["12 h ago", "9 h ago", "6 h ago", "3 h ago", "now"]);
        assert!(time_ticks(f64::NAN, now, now).is_empty());
        assert!(time_ticks(now, now, now).is_empty());
    }

    /// #96: 7, 30 and 90 days, each drawn with a bounded number of points.
    #[test]
    fn ranges_select_the_period_and_bound_the_drawn_points() {
        let now = 1_790_858_400.0;
        // One reading every 30 s for 90 days: 259 200 points, voltage sagging.
        let n = 259_200;
        let batt: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                (
                    now - 30.0 * (n - 1 - i) as f64,
                    100.0 - 60.0 * i as f64 / n as f64,
                )
            })
            .collect();
        let volt: Vec<(f64, f64)> = batt.iter().map(|&(t, p)| (t, 2.0 + p / 100.0)).collect();
        for (range, readings, span, first_tick) in [
            (Range::Day, 2_881, "24.0 h", "24 h ago"),
            (Range::Week, 20_161, "7.0 d", "6 d ago"),
            (Range::Month, 86_401, "30.0 d", "28 d ago"),
            (Range::Quarter, 259_200, "90.0 d", "60 d ago"),
        ] {
            let m = chart_model(&batt, &volt, now, range).unwrap();
            assert!(
                m.batt.len() <= CHART_POINTS_MAX && m.volt.len() <= CHART_POINTS_MAX,
                "{range:?}: {} points",
                m.batt.len()
            );
            assert!(
                m.batt.len() > CHART_POINTS_MAX / 2,
                "{range:?}: {} points",
                m.batt.len()
            );
            assert!(
                m.batt.windows(2).all(|w| w[0].0 <= w[1].0),
                "{range:?}: not sorted by time"
            );
            assert_eq!(m.summary, format!("{readings} points over {span}"));
            assert_eq!(
                (m.t_min, m.t_max),
                (m.batt[0].0, m.batt[m.batt.len() - 1].0)
            );
            assert!((m.t_max - m.t_min - (readings - 1) as f64 * 30.0).abs() < 1.0);
            assert_eq!(
                m.x_ticks.first().map(|t| t.1.as_str()),
                Some(first_tick),
                "{range:?}: {:?}",
                m.x_ticks
            );
            assert!(m.x_ticks.len() <= CHART_TICKS_MAX);
            labels_are_short(&m);
        }
        assert_eq!(
            chart_model(&batt[..10], &[], now, Range::Week),
            Err("No data in the last 7 days.".into())
        );
        assert_eq!(Range::default(), Range::Day);
        assert_eq!(
            Range::ALL.map(Range::button),
            ["24 h", "7 d", "30 d", "90 d"]
        );
    }

    #[test]
    fn downsampling_keeps_the_ends_and_the_extremes() {
        let mut pts: Vec<(f64, f64)> = (0..50_000).map(|i| (i as f64, 50.0)).collect();
        pts[12_345].1 = 3.0; // a dip one reading wide
        pts[40_000].1 = 99.0; // a spike one reading wide
        let d = downsample(pts.clone(), CHART_POINTS_MAX);
        assert!(d.len() <= CHART_POINTS_MAX, "{}", d.len());
        assert_eq!((d[0], d[d.len() - 1]), (pts[0], pts[49_999]));
        assert!(d.contains(&(12_345.0, 3.0)) && d.contains(&(40_000.0, 99.0)));
        assert!(d.windows(2).all(|w| w[0].0 < w[1].0));
        // Already small enough, or a bound too small to slice: untouched.
        assert_eq!(
            downsample(pts[..700].to_vec(), CHART_POINTS_MAX),
            pts[..700].to_vec()
        );
        assert_eq!(downsample(pts[..10].to_vec(), 3).len(), 10);
        for max in [4, 5, 6, 7, 100, 719, 720] {
            for len in [max + 1, max + 2, 2 * max, 10 * max + 3] {
                assert!(
                    downsample(pts[..len].to_vec(), max).len() <= max,
                    "max {max} len {len}"
                );
            }
        }
    }

    #[test]
    fn layout_switches_to_one_column_when_narrow() {
        assert!(!two_columns(500.0));
        assert!(two_columns(704.0));
    }

    #[test]
    fn dynamic_texts_in_french() {
        assert_eq!(age_text_in(true, Some(12)), "il y a 12\u{202f}s");
        assert_eq!(age_text_in(true, Some(7200)), "il y a 2\u{202f}h");
        assert_eq!(age_text_in(true, None), "\u{2014}");
        let mut b = akm_core::report::KbBattery {
            charge_estimate: akm_core::chemistry::estimate_charge(2460, akm_core::chemistry::Chemistry::Alkaline),
            ..Default::default()
        };
        assert_eq!(estimate_text_in(true, &b).unwrap(), "\u{2248} 30\u{202f}% (20 \u{e0} 40\u{202f}%), alkaline");
        b.new_batteries = true;
        assert_eq!(estimate_text_in(true, &b).unwrap(), "piles neuves, pas encore d\u{2019}estimation");
        let t = akm_core::registry::Thresholds { full_mv: 2954, low_mv: 2506, critical_mv: 2404, empty_mv: 2054 };
        b.thresholds = Some(t);
        b.threshold_margins_mv = Some(t.margins(2986));
        assert_eq!(
            thresholds_text_in(true, &b).unwrap(),
            "Plein 2954 / Bas 2506 / Critique 2404 / Vide 2054\u{202f}mV \u{b7} +480\u{202f}mV avant Bas, +582\u{202f}mV avant Critique"
        );
    }

    #[test]
    fn decimal_comma_in_french_only() {
        assert_eq!(dec_in(true, "2.99".into()), "2,99");
        assert_eq!(dec_in(false, "2.99".into()), "2.99");
    }

    #[test]
    fn firmware_line_apple_display_and_thresholds() {
        let mut fw = akm_core::report::KbFirmware::default();
        assert_eq!(firmware_line(&fw, true), None);
        fw.version = Some("0x0050".into());
        akm_core::firmware::assess_report(Some(0x0256), &mut fw);
        let (t, l) = firmware_line(&fw, true).unwrap();
        assert_eq!(t, "Firmware : 0x0050 \u{2014} \u{e0} jour (derni\u{e8}re version publique connue d'Apple)");
        assert_eq!(l, Level::Good);
        assert!(firmware_line(&fw, false).unwrap().0.contains("up to date"));
        fw.version = Some("0x0044".into());
        akm_core::firmware::assess_report(Some(0x0256), &mut fw);
        assert_eq!(firmware_line(&fw, true).unwrap().1, Level::Warn);
        fw.version = Some("0x0099".into());
        akm_core::firmware::assess_report(Some(0x0256), &mut fw);
        assert_eq!(firmware_line(&fw, true).unwrap().1, Level::Unknown);

        let mut b = akm_core::report::KbBattery::default();
        assert_eq!((apple_display_text(&b), thresholds_text(&b)), (None, None));
        b.apple_display_pct = Some(100.0);
        assert_eq!(apple_display_text(&b).unwrap(), "100% (macOS-style display)");
        b.apple_display_pct = Some(f64::NAN);
        assert_eq!(apple_display_text(&b), None);
        let t = akm_core::registry::Thresholds {
            full_mv: 2954,
            low_mv: 2506,
            critical_mv: 2404,
            empty_mv: 2054,
        };
        b.thresholds = Some(t);
        assert_eq!(thresholds_text(&b).unwrap(), "Full 2954 / Low 2506 / Critical 2404 / Empty 2054 mV");
        b.threshold_margins_mv = Some(t.margins(2986));
        assert!(thresholds_text(&b).unwrap().ends_with("+480 mV to Low, +582 mV to Critical"));
    }
}
