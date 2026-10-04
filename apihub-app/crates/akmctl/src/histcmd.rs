//! `history`, `history export --csv`, `graph`: pure views of the battery history.

use akm_core::history::{format_utc, HistoryEntry, HistoryEvent};
use akm_core::tr;
use serde_json::{json, Value};
use std::fmt::Write as _;

pub fn select(
    entries: &[HistoryEntry],
    since: Option<u64>,
    until: Option<u64>,
    last: Option<usize>,
) -> Vec<&HistoryEntry> {
    let mut v: Vec<&HistoryEntry> = entries
        .iter()
        .filter(|e| since.is_none_or(|s| e.ts >= s) && until.is_none_or(|u| e.ts <= u))
        .collect();
    v.sort_by_key(|e| e.ts);
    if let Some(n) = last {
        let skip = v.len().saturating_sub(n);
        v.drain(..skip);
    }
    v
}

fn event_name(e: &HistoryEntry) -> &'static str {
    match e.event {
        Some(HistoryEvent::BatteryReplaced) => "battery_replaced",
        None => "",
    }
}

pub fn to_text(entries: &[&HistoryEntry]) -> String {
    let mut out = format!(
        "{:<17} {:>6} {:>8} {:>6} {:>6}  {}\n",
        tr!("Time (UTC)"),
        tr!("Bat %"),
        tr!("Voltage"),
        "mV 46",
        "mV 49",
        tr!("Event")
    );
    for e in entries {
        let volt = e
            .reliable_voltage()
            .map_or("-".to_string(), |v| format!("{v:.3} V"));
        let mv = |v: Option<u32>| v.map_or("-".to_string(), |x| x.to_string());
        let _ = writeln!(
            out,
            "{:<17} {:>6} {:>8} {:>6} {:>6}  {}",
            format_utc(e.ts),
            format!("{:.0}", e.pct),
            volt,
            mv(e.mv_0x46),
            mv(e.mv_0x49),
            event_name(e)
        );
    }
    out
}

pub fn to_json(entries: &[&HistoryEntry]) -> Value {
    Value::Array(
        entries
            .iter()
            .map(|e| {
                json!({
                    "ts": e.ts,
                    "time_utc": format_utc(e.ts),
                    "battery_pct": e.pct,
                    "voltage": e.reliable_voltage(),
                    "mv_0x46": e.mv_0x46,
                    "mv_0x49": e.mv_0x49,
                    "event": (!event_name(e).is_empty()).then(|| event_name(e)),
                })
            })
            .collect(),
    )
}

pub fn to_csv(entries: &[&HistoryEntry]) -> String {
    let mut out = String::from("time_utc,ts,battery_pct,voltage,mv_0x46,mv_0x49,event\n");
    for e in entries {
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{}",
            format_utc(e.ts),
            e.ts,
            e.pct,
            e.reliable_voltage()
                .map_or(String::new(), |v| format!("{v:.3}")),
            e.mv_0x46.map_or(String::new(), |v| v.to_string()),
            e.mv_0x49.map_or(String::new(), |v| v.to_string()),
            event_name(e)
        );
    }
    out
}

const SPARK: [char; 8] = [
    '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}',
];
pub const WIDTH: usize = 60;

fn buckets(points: &[(u64, f64)], start: u64, end: u64, width: usize) -> Vec<Option<f64>> {
    let span = (end - start).max(1);
    let mut sum = vec![0.0; width];
    let mut n = vec![0u32; width];
    for &(ts, v) in points {
        let i = ((u128::from(ts - start) * width as u128) / (u128::from(span) + 1)) as usize;
        sum[i.min(width - 1)] += v;
        n[i.min(width - 1)] += 1;
    }
    (0..width)
        .map(|i| (n[i] > 0).then(|| sum[i] / f64::from(n[i])))
        .collect()
}

#[allow(clippy::cast_sign_loss)] // index clamped to 0..=7
#[allow(clippy::cast_possible_truncation)] // index clamped to 0..=7
fn spark_line(b: &[Option<f64>], lo: f64, hi: f64) -> String {
    let range = hi - lo;
    b.iter()
        .map(|v| match v {
            None => ' ',
            Some(_) if range <= 0.0 => SPARK[3],
            Some(x) => SPARK[(((x - lo) / range) * 7.0).round().clamp(0.0, 7.0) as usize],
        })
        .collect()
}

fn chart(
    title: &str,
    pts: &[(u64, f64)],
    start: u64,
    end: u64,
    fixed: Option<(f64, f64)>,
    unit: &str,
    dec: usize,
) -> String {
    if pts.is_empty() {
        return format!("{title}\n  {}\n\n", tr!("no sample in this window"));
    }
    let min = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let (lo, hi) = fixed.unwrap_or((min, max));
    let b = buckets(pts, start, end, WIDTH);
    let last = pts.last().map_or(0.0, |p| p.1);
    let axis = format!(
        "{:<w$}{:>w2$}",
        format_utc(start),
        format!("{} UTC", format_utc(end)),
        w = WIDTH / 2,
        w2 = WIDTH - WIDTH / 2,
    );
    let summary = tr!(
        "min {min}{unit}  max {max}{unit}  last {last}{unit}  ({n} samples)",
        min = format!("{min:.dec$}"),
        max = format!("{max:.dec$}"),
        last = format!("{last:.dec$}"),
        unit = unit,
        n = pts.len()
    );
    format!(
        "{title}\n  {}\n  {axis}\n  {summary}\n\n",
        spark_line(&b, lo, hi)
    )
}

pub fn graph(entries: &[HistoryEntry], now: u64, span_s: u64) -> String {
    let start = now.saturating_sub(span_s);
    let win = select(entries, Some(start), Some(now), None);
    let label = if span_s > 86_400 && span_s.is_multiple_of(86_400) {
        format!("{} d", span_s / 86_400)
    } else {
        format!("{} h", span_s / 3600)
    };
    if win.is_empty() {
        return tr!("No history sample in the last {label}.", label = label) + "\n";
    }
    let pct: Vec<(u64, f64)> = win.iter().map(|e| (e.ts, e.pct)).collect();
    let volt: Vec<(u64, f64)> = win
        .iter()
        .filter_map(|e| e.reliable_voltage().map(|v| (e.ts, v)))
        .collect();
    let mut out = chart(
        &tr!("BATTERY % - last {label}", label = label),
        &pct,
        start,
        now,
        Some((0.0, 100.0)),
        " %",
        0,
    );
    out.push_str(&chart(
        &tr!("VOLTAGE - last {label}", label = label),
        &volt,
        start,
        now,
        None,
        " V",
        3,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_790_000_000;

    fn fixture() -> Vec<HistoryEntry> {
        vec![
            HistoryEntry::measured(T0, 99.0, Some(3002), Some(2965)),
            HistoryEntry::measured(T0 + 3600, 98.0, Some(2990), None),
            HistoryEntry {
                event: Some(HistoryEvent::BatteryReplaced),
                ..HistoryEntry::measured(T0 + 7200, 100.0, Some(3100), Some(3060))
            },
            HistoryEntry {
                ts: T0 + 100,
                pct: 97.0,
                voltage: Some(0.85),
                voltage_valid: Some(false),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn select_filters_sorts_and_limits() {
        let h = fixture();
        assert_eq!(
            select(&h, None, None, None)
                .iter()
                .map(|e| e.ts)
                .collect::<Vec<_>>(),
            vec![T0, T0 + 100, T0 + 3600, T0 + 7200]
        );
        assert_eq!(select(&h, Some(T0 + 100), Some(T0 + 3600), None).len(), 2);
        let l = select(&h, None, None, Some(2));
        assert_eq!((l[0].ts, l[1].ts), (T0 + 3600, T0 + 7200));
        assert!(
            select(&h, Some(T0 + 99_999), None, None).is_empty(),
            "{:?}",
            select(&h, Some(T0 + 99_999), None, None)
        );
    }

    #[test]
    fn csv_is_deterministic_and_hides_legacy_voltage() {
        let h = fixture();
        let csv = to_csv(&select(&h, None, None, None));
        let expect = "time_utc,ts,battery_pct,voltage,mv_0x46,mv_0x49,event\n\
2026-09-21 14:13,1790000000,99,3.002,3002,2965,\n\
2026-09-21 14:15,1790000100,97,,,,\n\
2026-09-21 15:13,1790003600,98,2.990,2990,,\n\
2026-09-21 16:13,1790007200,100,3.100,3100,3060,battery_replaced\n";
        assert_eq!(csv, expect);
    }

    #[test]
    fn text_and_json_views() {
        let h = fixture();
        let s = select(&h, None, None, None);
        let t = to_text(&s);
        assert!(t.starts_with("Time (UTC)"), "{t}");
        assert!(
            t.contains("2026-09-21 14:13      99  3.002 V   3002   2965"),
            "{t}"
        );
        assert!(
            t.lines().nth(2).unwrap().contains("97        -"),
            "legacy voltage shows '-': {t}"
        );
        assert!(t.contains("battery_replaced"));
        let j = to_json(&s);
        assert_eq!(j.as_array().unwrap().len(), 4);
        assert_eq!(j[0]["voltage"], 3.002);
        assert!(j[1]["voltage"].is_null());
        assert_eq!(j[3]["event"], "battery_replaced");
        assert!(j[0]["event"].is_null());
    }

    #[test]
    fn graph_24h_is_stable() {
        let h = fixture();
        let g = graph(&h, T0 + 7200, 24 * 3600);
        assert!(g.starts_with("BATTERY % - last 24 h\n"), "{g}");
        assert!(
            g.contains("min 97 %  max 100 %  last 100 %  (4 samples)"),
            "{g}"
        );
        assert!(g.contains("VOLTAGE - last 24 h"), "{g}");
        assert!(
            g.lines().nth(2).unwrap().trim_end().ends_with(" UTC"),
            "R17: the axis says UTC: {g}"
        );
        assert!(
            g.contains("min 2.990 V  max 3.100 V  last 3.100 V  (3 samples)"),
            "{g}"
        );
        let line = g.lines().nth(1).unwrap().strip_prefix("  ").unwrap();
        assert_eq!(line.chars().count(), WIDTH);
        assert!(
            line.chars().any(|c| c == '\u{2588}' || c == '\u{2587}'),
            "{line:?}"
        );
        assert_eq!(graph(&h, T0 + 7200, 24 * 3600), g, "deterministic");
    }

    #[test]
    fn graph_empty_window_and_7_days() {
        let h = fixture();
        assert_eq!(
            graph(&h, T0 + 30 * 86_400, 86_400),
            "No history sample in the last 24 h.\n"
        );
        assert_eq!(
            graph(&[], T0, 7 * 86_400),
            "No history sample in the last 7 d.\n"
        );
        assert!(graph(&h, T0 + 7200, 7 * 86_400).starts_with("BATTERY % - last 7 d\n"));
    }

    #[test]
    fn buckets_average_and_leave_gaps() {
        let b = buckets(&[(0, 10.0), (1, 20.0), (99, 50.0)], 0, 99, 4);
        assert_eq!(b, vec![Some(15.0), None, None, Some(50.0)]);
        assert_eq!(spark_line(&b, 0.0, 100.0).chars().nth(1), Some(' '));
    }
}
