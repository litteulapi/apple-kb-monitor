//! `waybar` and `metrics`: deterministic renderings of the daemon snapshot.

use akm_core::Snapshot;
use serde_json::{json, Map, Value};

/// Health of the 2 x AA pack from its voltage (same bands as the old CLI).
pub fn voltage_health(v: f64) -> &'static str {
    match v {
        v if v >= 2.8 => "Fresh",
        v if v >= 2.4 => "Good",
        v if v >= 2.1 => "Low",
        v if v >= 1.8 => "Critical",
        _ => "Dead",
    }
}

/// Waybar class of a battery percentage.
pub fn waybar_class(pct: Option<u32>, connected: bool) -> &'static str {
    match pct {
        Some(p) if connected => match p {
            0..=15 => "critical",
            16..=50 => "warning",
            _ => "good",
        },
        _ => "disconnected",
    }
}

/// JSON object of a waybar `custom` module (`return-type: json`). `snap` is
/// `None` when the daemon is not on the bus. `percentage` is left out when
/// unknown (never an invented 0).
pub fn waybar(snap: Option<&Snapshot>) -> Value {
    let Some(s) = snap else {
        return json!({"text": "", "tooltip": "apple-kb-monitord is not running", "class": "disconnected", "alt": "disconnected"});
    };
    let pct = s.battery_pct().map(|p| p.round().clamp(0.0, 100.0) as u32);
    let class = waybar_class(pct, s.connected);
    let mut tip = vec![s.display_name().or(s.model()).unwrap_or("Apple keyboard").to_string()];
    if !s.connected {
        tip.push("Disconnected".into());
    }
    if let Some(p) = pct {
        tip.push(format!("Battery: {p} % (keyboard indication)"));
    }
    if let Some(e) = s.keyboard.as_ref().and_then(|k| k.battery.charge_estimate.as_ref()) {
        tip.push(format!("Estimate: ~{:.0} % ({:.0} to {:.0} %), {}", e.pct, e.low, e.high, e.chemistry.as_str()));
    }
    if let Some(v) = s.voltage().filter(|v| v.is_finite()) {
        tip.push(format!("Voltage: {v:.2} V ({})", voltage_health(v)));
    }
    if let Some(t) = akm_core::signal::text(s.rssi(), false) {
        tip.push(t);
    }
    if let Some(r) = s.remaining_display.as_deref() {
        tip.push(format!("Autonomy: {r}"));
    }
    if let Some(e) = s.kb_error.as_deref().or(s.last_error.as_deref()) {
        tip.push(format!("Error: {e}"));
    }
    let mut o = Map::new();
    o.insert("text".into(), pct.map_or(String::new(), |p| format!("{p}%")).into());
    o.insert("tooltip".into(), tip.join("\n").into());
    o.insert("class".into(), class.into());
    o.insert("alt".into(), class.into());
    if let Some(p) = pct {
        o.insert("percentage".into(), p.into());
    }
    Value::Object(o)
}

/// Escape a Prometheus label value.
fn label(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

/// Prometheus text exposition (version 0.0.4), one page. A metric whose value
/// is unknown is left out. `apple_kb_daemon_up` is always present.
pub fn metrics(snap: Option<&Snapshot>, fn_mode: Option<u8>) -> String {
    let mut out = String::new();
    let mut emit = |name: &str, help: &str, kind: &str, labels: &str, val: String| {
        out.push_str(&format!("# HELP {name} {help}\n# TYPE {name} {kind}\n{name}{labels} {val}\n"));
    };
    emit("apple_kb_daemon_up", "1 if apple-kb-monitord answers on the session bus.", "gauge", "", u8::from(snap.is_some()).to_string());
    if let Some(m) = fn_mode {
        emit("apple_kb_fnmode", "hid_apple fnmode (0 disabled, 1 fkeyslast, 2 fkeysfirst, 3 auto).", "gauge", "", m.to_string());
    }
    let Some(s) = snap else { return out };
    let l = format!("{{mac=\"{}\",model=\"{}\"}}", label(s.mac().unwrap_or("")), label(s.model().unwrap_or("")));
    emit("apple_kb_connected", "1 if the keyboard is connected.", "gauge", &l, u8::from(s.connected).to_string());
    if let Some(p) = s.battery_pct().filter(|p| p.is_finite()) {
        emit("apple_kb_battery_percent", "Battery percentage reported by the keyboard.", "gauge", &l, format!("{}", p.round()));
    }
    let bat = s.keyboard.as_ref().map(|k| &k.battery);
    if let Some(e) = bat.and_then(|b| b.charge_estimate.as_ref()) {
        emit("apple_kb_battery_estimate_percent", "Charge estimate from the voltage and the declared chemistry (hypothesis).", "gauge", &l, format!("{}", e.pct));
    }
    if let Some(v) = s.voltage().filter(|v| v.is_finite()) {
        emit("apple_kb_battery_voltage", "Cell voltage in volts.", "gauge", &l, format!("{v:.3}"));
    }
    if let Some(r) = s.rssi() {
        emit("apple_kb_rssi_rel_db", "Relative BR/EDR RSSI in dB (0 = ideal range, not dBm).", "gauge", &l, r.to_string());
    }
    if let Some(b) = bat {
        emit("apple_kb_new_batteries", "1 if new batteries were detected and no estimate exists yet.", "gauge", &l, u8::from(b.new_batteries).to_string());
    }
    if s.last_update > 0 {
        emit("apple_kb_last_update_timestamp_seconds", "Unix time of the last successful reading.", "gauge", &l, s.last_update.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"schema":1,"version":28,"connected":true,"remaining_display":"~42 days","keyboard":{"device":{"model":"Apple Wireless Keyboard (A1314)","mac":"04:DB:56:CA:42:EE","alias":"Clavier de maria #1"},"battery":{"percentage":42.0,"voltage":2.46},"radio":{"rssi_dbm":-2}},"last_update":1790000000}"#;

    fn snap() -> Snapshot {
        serde_json::from_str(SAMPLE).unwrap()
    }

    #[test]
    fn classes_follow_the_thresholds() {
        assert_eq!(waybar_class(Some(15), true), "critical");
        assert_eq!(waybar_class(Some(16), true), "warning");
        assert_eq!(waybar_class(Some(50), true), "warning");
        assert_eq!(waybar_class(Some(51), true), "good");
        assert_eq!(waybar_class(Some(80), false), "disconnected");
        assert_eq!(waybar_class(None, true), "disconnected");
    }

    #[test]
    fn voltage_bands() {
        assert_eq!(voltage_health(2.95), "Fresh");
        assert_eq!(voltage_health(2.46), "Good");
        assert_eq!(voltage_health(2.2), "Low");
        assert_eq!(voltage_health(1.9), "Critical");
        assert_eq!(voltage_health(1.0), "Dead");
    }

    #[test]
    fn waybar_full_output() {
        let v = waybar(Some(&snap()));
        assert_eq!(v["text"], "42%");
        assert_eq!(v["class"], "warning");
        assert_eq!(v["alt"], "warning");
        assert_eq!(v["percentage"], 42);
        assert_eq!(
            v["tooltip"],
            "Clavier de maria #1\nBattery: 42 % (keyboard indication)\nVoltage: 2.46 V (Good)\nSignal: good (\u{2212}2)\nAutonomy: ~42 days"
        );
    }

    #[test]
    fn waybar_degraded_states() {
        let v = waybar(None);
        assert_eq!(v["class"], "disconnected");
        assert_eq!(v["text"], "");
        let v = waybar(Some(&Snapshot::default()));
        assert_eq!(v["class"], "disconnected");
        assert!(v.get("percentage").is_none(), "unknown, not 0");
        assert!(v["tooltip"].as_str().unwrap().contains("Disconnected"));
        let mut s = snap();
        s.connected = false;
        assert_eq!(waybar(Some(&s))["class"], "disconnected");
    }

    #[test]
    fn metrics_page_is_exact() {
        let m = metrics(Some(&snap()), Some(2));
        let l = r#"{mac="04:DB:56:CA:42:EE",model="Apple Wireless Keyboard (A1314)"}"#;
        let expect = format!(
            "# HELP apple_kb_daemon_up 1 if apple-kb-monitord answers on the session bus.\n# TYPE apple_kb_daemon_up gauge\napple_kb_daemon_up 1\n\
# HELP apple_kb_fnmode hid_apple fnmode (0 disabled, 1 fkeyslast, 2 fkeysfirst, 3 auto).\n# TYPE apple_kb_fnmode gauge\napple_kb_fnmode 2\n\
# HELP apple_kb_connected 1 if the keyboard is connected.\n# TYPE apple_kb_connected gauge\napple_kb_connected{l} 1\n\
# HELP apple_kb_battery_percent Battery percentage reported by the keyboard.\n# TYPE apple_kb_battery_percent gauge\napple_kb_battery_percent{l} 42\n\
# HELP apple_kb_battery_voltage Cell voltage in volts.\n# TYPE apple_kb_battery_voltage gauge\napple_kb_battery_voltage{l} 2.460\n\
# HELP apple_kb_rssi_rel_db Relative BR/EDR RSSI in dB (0 = ideal range, not dBm).\n# TYPE apple_kb_rssi_rel_db gauge\napple_kb_rssi_rel_db{l} -2\n\
# HELP apple_kb_new_batteries 1 if new batteries were detected and no estimate exists yet.\n# TYPE apple_kb_new_batteries gauge\napple_kb_new_batteries{l} 0\n\
# HELP apple_kb_last_update_timestamp_seconds Unix time of the last successful reading.\n# TYPE apple_kb_last_update_timestamp_seconds gauge\napple_kb_last_update_timestamp_seconds{l} 1790000000\n"
        );
        assert_eq!(m, expect);
    }

    #[test]
    fn metrics_without_daemon_or_data() {
        let m = metrics(None, None);
        assert!(m.ends_with("apple_kb_daemon_up 0\n"), "{m}");
        assert_eq!(m.matches("# TYPE").count(), 1);
        let m = metrics(Some(&Snapshot::default()), None);
        assert!(m.contains("apple_kb_connected{mac=\"\",model=\"\"} 0"));
        assert!(!m.contains("battery_percent") && !m.contains("voltage") && !m.contains("rssi"));
    }

    #[test]
    fn label_values_are_escaped() {
        assert_eq!(label("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }
}
