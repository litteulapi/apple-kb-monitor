//! Stable JSON / text views of the daemon snapshot.

use akm_core::Snapshot;
use serde_json::{json, Value};

use crate::fnmode;

/// Version of the `akmctl` JSON output schema.
pub const SCHEMA: u32 = 1;

/// The JSON object printed by `status --json` and, per signal, by `watch`.
/// Unknown values are `null` (never an invented 0). `revision` is the daemon's
/// snapshot counter (`StateChanged.revision`) when known.
pub fn to_json(s: &Snapshot, fn_mode: Option<u8>, revision: Option<u64>) -> Value {
    json!({
        "schema": SCHEMA,
        "daemon": true,
        "revision": revision.unwrap_or(s.version),
        "connected": s.connected,
        "model": s.model(),
        "mac": s.mac(),
        "battery_pct": s.battery_pct().map(|p| p.round().clamp(0.0, 100.0) as u32),
        "voltage": s.voltage().filter(|v| v.is_finite()),
        "rssi_dbm": s.rssi(),
        "last_update": s.last_update,
        "last_error": s.last_error,
        "kb_error": s.kb_error,
        "fnmode": fn_mode,
        "fnmode_label": fn_mode.map(fnmode::label),
    })
}

/// Output of `status --json` when the daemon is not on the bus.
pub fn absent_json(fn_mode: Option<u8>) -> Value {
    json!({
        "schema": SCHEMA,
        "daemon": false,
        "fnmode": fn_mode,
        "fnmode_label": fn_mode.map(fnmode::label),
    })
}

fn opt<T: std::fmt::Display>(v: Option<T>, unit: &str) -> String {
    v.map_or_else(|| "n/a".into(), |x| format!("{x}{unit}"))
}

/// Human-readable multi-line status.
pub fn to_text(s: &Snapshot, fn_mode: Option<u8>) -> String {
    let mut out = String::new();
    let mut line = |k: &str, v: String| out.push_str(&format!("{k:<11}{v}\n"));
    line("Keyboard:", s.model().unwrap_or("n/a").to_string());
    line("MAC:", s.mac().unwrap_or("n/a").to_string());
    line("Connected:", if s.connected { "yes" } else { "no" }.into());
    line("Battery:", opt(s.battery_pct().map(|p| p.round() as i64), " %"));
    line("Voltage:", opt(s.voltage().map(|v| format!("{v:.2}")), " V"));
    line("RSSI:", opt(s.rssi(), " dBm"));
    line(
        "Fn mode:",
        fn_mode.map_or_else(|| "n/a".into(), |m| format!("{m} - {}", fnmode::label(m))),
    );
    if let Some(e) = s.kb_error.as_deref().or(s.last_error.as_deref()) {
        line("Error:", e.to_string());
    }
    out
}

pub fn absent_text(fn_mode: Option<u8>) -> String {
    format!(
        "Daemon:    not running (com.agenceapi.AppleKbMonitor1 absent on the session bus)\nFn mode:   {}\n",
        fn_mode.map_or_else(|| "n/a".into(), |m| format!("{m} - {}", fnmode::label(m)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"schema":1,"version":28,"connected":true,"keyboard":{"device":{"model":"Apple Wireless Keyboard (A1314)","mac":"04:DB:56:CA:42:EE"},"battery":{"percentage":99.0,"voltage":2.9},"radio":{"rssi_dbm":0}},"last_update":1790000000}"#;

    #[test]
    fn json_fields_and_nulls() {
        let s: Snapshot = serde_json::from_str(SAMPLE).unwrap();
        let v = to_json(&s, Some(2), Some(30));
        assert_eq!(v["schema"], 1);
        assert_eq!(v["daemon"], true);
        assert_eq!(v["revision"], 30);
        assert_eq!(v["battery_pct"], 99);
        assert_eq!(v["rssi_dbm"], 0);
        assert_eq!(v["mac"], "04:DB:56:CA:42:EE");
        assert_eq!(v["fnmode"], 2);
        assert!(v["fnmode_label"].as_str().unwrap().starts_with("fkeysfirst"));
        assert!(v["last_error"].is_null());
    }

    #[test]
    fn disconnected_keeps_nulls_not_zeros() {
        let v = to_json(&Snapshot::default(), None, None);
        assert_eq!(v["connected"], false);
        assert!(v["battery_pct"].is_null());
        assert!(v["voltage"].is_null());
        assert!(v["rssi_dbm"].is_null());
        assert!(v["fnmode"].is_null());
        assert_eq!(v["revision"], 0);
    }

    #[test]
    fn absent_is_valid_json_object() {
        let v = absent_json(Some(1));
        assert_eq!(v["daemon"], false);
        assert_eq!(v["fnmode"], 1);
        assert!(serde_json::to_string(&v).unwrap().starts_with('{'));
    }

    #[test]
    fn text_has_expected_lines() {
        let s: Snapshot = serde_json::from_str(SAMPLE).unwrap();
        let t = to_text(&s, Some(1));
        assert!(t.contains("Battery:   99 %"));
        assert!(t.contains("Connected: yes"));
        assert!(t.contains("Fn mode:   1 - fkeyslast"));
        assert!(to_text(&Snapshot::default(), None).contains("Battery:   n/a"));
    }
}
