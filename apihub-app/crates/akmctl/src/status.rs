//! Stable JSON / text views of the daemon snapshot.

use akm_core::Snapshot;
use serde_json::{json, Value};

use crate::{fnmode, shutdown_notify};

/// Version of the `akmctl` JSON output schema.
pub const SCHEMA: u32 = 1;

/// The JSON object printed by `status --json` and, per signal, by `watch`.
/// Unknown values are `null` (never an invented 0). `revision` is the daemon's
/// snapshot counter (`StateChanged.revision`) when known.
pub fn to_json(s: &Snapshot, fn_mode: Option<u8>, revision: Option<u64>) -> Value {
    let rel = s.rssi();
    let battery = s.keyboard.as_ref().map(|k| &k.battery);
    let est = battery.and_then(|b| b.charge_estimate.as_ref());
    json!({
        "schema": SCHEMA,
        "daemon": true,
        "revision": revision.unwrap_or(s.version),
        "connected": s.connected,
        "model": s.model(),
        "name": s.display_name(),
        "alias": s.alias(),
        // Name the kernel registered the input device under (HID_NAME): what
        // KWin and System Settings > Keyboard show (#248).
        "kernel_name": s.kernel_name(),
        "mac": s.mac(),
        "battery_pct": s.battery_pct().map(|p| p.round().clamp(0.0, 100.0) as u32),
        "voltage": s.voltage().filter(|v| v.is_finite()),
        // `rssi_dbm` is the former name, kept for compatibility: on BR/EDR it
        // is a relative value in dB (0 = ideal range), not dBm (#174).
        "rssi_dbm": rel,
        "rssi_rel_db": rel,
        "rssi_kind": rel.map(|_| akm_core::signal::KIND_BREDR),
        "rssi_quality": rel.map(|r| akm_core::signal::quality(r).as_str()),
        // `battery_pct` is the keyboard's own indication; the estimate is a
        // [hypothèse] by declared chemistry (#178).
        "battery_estimate_pct": est.map(|e| e.pct),
        "battery_estimate_low": est.map(|e| e.low),
        "battery_estimate_high": est.map(|e| e.high),
        "battery_chemistry": est.map(|e| e.chemistry.as_str()),
        "new_batteries": battery.map(|b| b.new_batteries),
        // Percentage as macOS shows it (#213): a labelled secondary figure.
        "battery_apple_display_pct": battery.and_then(|b| b.apple_display_pct).map(|p| p.round() as u32),
        // Thresholds the keyboard reports (report 0x60, once per connection).
        "battery_thresholds": battery.and_then(|b| b.thresholds).map(|t| json!({
            "full_mv": t.full_mv, "low_mv": t.low_mv, "critical_mv": t.critical_mv, "empty_mv": t.empty_mv,
            "level": battery.and_then(|b| b.threshold_level.clone()),
            "margins_mv": battery.and_then(|b| b.threshold_margins_mv),
        })),
        // Firmware check against the embedded table (#227).
        "firmware": s.firmware().map(|f| json!({
            "version_hex": f.version_hex.clone().or_else(|| f.version.clone()),
            "latest_known": f.latest_known,
            "status": if f.status.is_empty() { "unknown" } else { f.status.as_str() },
            "source": f.source,
            "table_date": f.table_date,
        })),
        "last_update": s.last_update,
        "last_error": s.last_error,
        "kb_error": s.kb_error,
        "fnmode": fn_mode,
        "fnmode_label": fn_mode.map(fnmode::label),
        // `[apple] will_shutdown`: what is sent to the keyboard at shutdown (#191).
        "will_shutdown": shutdown_notify::status_json(shutdown_notify::configured()),
    })
}

/// Output of `status --json` when the daemon is not on the bus.
pub fn absent_json(fn_mode: Option<u8>) -> Value {
    json!({
        "schema": SCHEMA,
        "daemon": false,
        "fnmode": fn_mode,
        "fnmode_label": fn_mode.map(fnmode::label),
        "will_shutdown": shutdown_notify::status_json(shutdown_notify::configured()),
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
    line("Name:", s.display_name().unwrap_or("n/a").to_string());
    if let Some(note) = kernel_name_note(s) {
        line("Kernel:", note);
    }
    line("MAC:", s.mac().unwrap_or("n/a").to_string());
    line("Connected:", if s.connected { "yes" } else { "no" }.into());
    line(
        "Battery:",
        s.battery_pct().map_or_else(
            || "n/a".into(),
            |p| format!("{:.0} % (keyboard indication)", p),
        ),
    );
    if let Some(b) = s.keyboard.as_ref().map(|k| &k.battery) {
        if b.new_batteries {
            line("Estimate:", "new batteries, no estimate yet".into());
        } else if let Some(e) = &b.charge_estimate {
            line(
                "Estimate:",
                format!(
                    "~{:.0} % ({:.0} to {:.0} %), {}",
                    e.pct,
                    e.low,
                    e.high,
                    e.chemistry.as_str()
                ),
            );
        }
    }
    if let Some(a) = s.keyboard.as_ref().and_then(|k| k.battery.apple_display_pct) {
        line("Apple:", format!("{a:.0} % (macOS-style display)"));
    }
    if let Some(t) = s.keyboard.as_ref().and_then(|k| k.battery.thresholds) {
        line(
            "Limits:",
            format!("Full {} / Low {} / Critical {} / Empty {} mV", t.full_mv, t.low_mv, t.critical_mv, t.empty_mv),
        );
    }
    if let Some(fw) = s.firmware().and_then(akm_core::firmware::summary_en) {
        line("Firmware:", fw.trim_start_matches("Firmware: ").to_string());
    }
    line("Voltage:", opt(s.voltage().map(|v| format!("{v:.2}")), " V"));
    // Relative BR/EDR value, no unit (#174).
    line(
        "Signal:",
        akm_core::signal::text(s.rssi(), false)
            .and_then(|t| t.strip_prefix("Signal: ").map(str::to_string))
            .unwrap_or_else(|| "n/a".into()),
    );
    // Age of the last reading: the kernel percentage only steps down at
    // reconnections (#179).
    let now = akm_core::history::Clock::now(&akm_core::history::SystemClock);
    if let Some(age) = s.update_age_s(now) {
        line("Updated:", format!("{age} s ago"));
    }
    line(
        "Fn mode:",
        fn_mode.map_or_else(|| "n/a".into(), |m| format!("{m} - {}", fnmode::label(m))),
    );
    line("Shutdown:", shutdown_notify::status_text(shutdown_notify::configured()));
    if let Some(e) = s.kb_error.as_deref().or(s.last_error.as_deref()) {
        line("Error:", e.to_string());
    }
    out
}

/// Explains why KDE shows two names (#248): the Bluetooth and Battery
/// applets read the BlueZ alias, while KWin and System Settings > Keyboard
/// read the kernel's input device name (`HID_NAME`), which only changes when
/// the keyboard reconnects. `None` when both names are the same.
pub fn kernel_name_note(s: &Snapshot) -> Option<String> {
    let alias = s.alias()?;
    let kernel = s.kernel_name()?;
    (alias != kernel).then(|| {
        format!(
            "{kernel} - KWin and System Settings > Keyboard show this kernel name; \
             Bluetooth and Battery show the alias \"{alias}\". The kernel name changes only \
             after the keyboard reconnects (switch it off and on)."
        )
    })
}

pub fn absent_text(fn_mode: Option<u8>) -> String {
    format!(
        "Daemon:    not running (com.agenceapi.AppleKbMonitor1 absent on the session bus)\nFn mode:   {}\nShutdown:  {}\n",
        fn_mode.map_or_else(|| "n/a".into(), |m| format!("{m} - {}", fnmode::label(m))),
        shutdown_notify::status_text(shutdown_notify::configured())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"schema":1,"version":28,"connected":true,"keyboard":{"device":{"model":"Apple Wireless Keyboard (A1314)","mac":"04:DB:56:CA:42:EE","alias":"Clavier de maria #1"},"battery":{"percentage":99.0,"voltage":2.9},"radio":{"rssi_dbm":0}},"last_update":1790000000}"#;

    #[test]
    fn json_fields_and_nulls() {
        let s: Snapshot = serde_json::from_str(SAMPLE).unwrap();
        let v = to_json(&s, Some(2), Some(30));
        assert_eq!(v["schema"], 1);
        assert_eq!(v["daemon"], true);
        assert_eq!(v["revision"], 30);
        assert_eq!(v["battery_pct"], 99);
        assert_eq!(v["rssi_dbm"], 0, "compat name");
        assert_eq!(v["rssi_rel_db"], 0);
        assert_eq!(v["rssi_kind"], "bredr-golden-range");
        assert_eq!(v["rssi_quality"], "excellent");
        assert!(v["battery_estimate_pct"].is_null(), "no estimate in this sample");
        assert_eq!(v["mac"], "04:DB:56:CA:42:EE");
        assert_eq!(v["name"], "Clavier de maria #1");
        assert_eq!(v["alias"], "Clavier de maria #1");
        assert_eq!(v["fnmode"], 2);
        assert!(v["fnmode_label"].as_str().unwrap().starts_with("fkeysfirst"));
        assert!(v["last_error"].is_null());
    }

    #[test]
    fn a_line_explains_two_names_only_when_they_differ() {
        let mk = |alias: &str, name: &str| -> Snapshot {
            let mut s: Snapshot = serde_json::from_str(SAMPLE).unwrap();
            let d = &mut s.keyboard.as_mut().unwrap().device;
            d.alias = Some(alias.into());
            d.name = Some(name.into());
            s
        };
        let s = mk("alex", "Clavier de maria #1");
        let note = kernel_name_note(&s).expect("names differ");
        assert!(note.starts_with("Clavier de maria #1") && note.contains("alex") && note.contains("reconnects"));
        let text = to_text(&s, None);
        assert!(text.contains("Kernel:") && text.contains("KWin"), "{text}");
        assert_eq!(to_json(&s, None, None)["kernel_name"], "Clavier de maria #1");
        let same = mk("Clavier de maria #1", "Clavier de maria #1");
        assert!(kernel_name_note(&same).is_none());
        assert!(!to_text(&same, None).contains("Kernel:"));
        let mut no_alias = mk("x", "Clavier de maria #1");
        no_alias.keyboard.as_mut().unwrap().device.alias = None;
        assert!(kernel_name_note(&no_alias).is_none());
    }

    #[test]
    fn disconnected_keeps_nulls_not_zeros() {
        let v = to_json(&Snapshot::default(), None, None);
        assert_eq!(v["connected"], false);
        assert!(v["battery_pct"].is_null());
        assert!(v["name"].is_null() && v["alias"].is_null());
        assert!(v["voltage"].is_null());
        assert!(v["rssi_dbm"].is_null() && v["rssi_rel_db"].is_null());
        assert!(v["rssi_quality"].is_null() && v["battery_estimate_pct"].is_null());
        assert!(v["fnmode"].is_null());
        assert_eq!(v["revision"], 0);
    }

    #[test]
    fn estimate_is_a_separate_labelled_field() {
        // #178
        let mut s: Snapshot = serde_json::from_str(SAMPLE).unwrap();
        let b = &mut s.keyboard.as_mut().unwrap().battery;
        b.percentage = Some(62.0);
        b.charge_estimate =
            akm_core::chemistry::estimate_charge(2460, akm_core::chemistry::Chemistry::Alkaline);
        let v = to_json(&s, None, None);
        assert_eq!(v["battery_pct"], 62, "the indication is untouched");
        assert_eq!(v["battery_estimate_pct"], 30.0);
        assert_eq!(v["battery_estimate_low"], 20.0);
        assert_eq!(v["battery_estimate_high"], 40.0);
        assert_eq!(v["battery_chemistry"], "alkaline");
        let t = to_text(&s, None);
        assert!(t.contains("Estimate:  ~30 % (20 to 40 %), alkaline"), "{t}");
        s.keyboard.as_mut().unwrap().battery.new_batteries = true;
        assert!(to_text(&s, None).contains("new batteries"));
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
        assert!(t.contains("Battery:   99 % (keyboard indication)"));
        assert!(t.contains("Signal:    excellent (0)"), "{t}");
        assert!(!t.contains("dBm"));
        assert!(t.contains("Updated:"));
        assert!(t.contains("Connected: yes"));
        assert!(t.contains("Name:      Clavier de maria #1"));
        assert!(t.contains("Fn mode:   1 - fkeyslast"));
        assert!(to_text(&Snapshot::default(), None).contains("Battery:   n/a"));
    }

    #[test]
    fn firmware_thresholds_and_apple_display_in_status() {
        let mut s: Snapshot = serde_json::from_str(SAMPLE).unwrap();
        let k = s.keyboard.as_mut().unwrap();
        k.firmware.version = Some("0x0050".into());
        akm_core::firmware::assess_report(Some(0x0256), &mut k.firmware);
        k.battery.apple_display_pct = Some(100.0);
        let t = akm_core::registry::Thresholds::parse(&[0x0b, 0x8a, 0x09, 0xca, 0x09, 0x64, 0x08, 0x06]).unwrap();
        k.battery.thresholds = Some(t);
        k.battery.threshold_level = Some("ok".into());
        k.battery.threshold_margins_mv = Some(t.margins(2986));
        let v = to_json(&s, None, None);
        assert_eq!(v["firmware"]["version_hex"], "0x0050");
        assert_eq!(v["firmware"]["status"], "up_to_date");
        assert_eq!(v["firmware"]["latest_known"], "0x0050");
        assert_eq!(v["battery_apple_display_pct"], 100);
        assert_eq!(v["battery_thresholds"]["low_mv"], 2506);
        assert_eq!(v["battery_thresholds"]["level"], "ok");
        let txt = to_text(&s, None);
        assert!(txt.contains("Firmware:  0x0050 - up to date"), "{txt}");
        assert!(txt.contains("Apple:     100 % (macOS-style display)"), "{txt}");
        assert!(txt.contains("Limits:    Full 2954"), "{txt}");
        // Unknown firmware stays null / "unknown", never invented.
        let v = to_json(&Snapshot::default(), None, None);
        assert!(v["firmware"].is_null() && v["battery_thresholds"].is_null());
    }
}
