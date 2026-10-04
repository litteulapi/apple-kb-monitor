//! Stable JSON / text views of the daemon snapshot.

use akm_core::tr;
use akm_core::Snapshot;
use serde_json::{json, Value};
use std::fmt::Write as _;

use crate::{fnmode, shutdown_notify};

pub const SCHEMA: u32 = 1;

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
        "name_on_keyboard": s.keyboard.as_ref().and_then(|k| k.device.name_on_keyboard.clone()),
        "kernel_name": s.kernel_name(),
        "mac": s.mac(),
        "battery_pct": s.battery_pct().map(crate::render::pct_u32),
        "voltage": s.voltage().filter(|v| v.is_finite()),
        "rssi_dbm": rel,
        "rssi_rel_db": rel,
        "rssi_kind": rel.map(|_| akm_core::signal::KIND_BREDR),
        "rssi_quality": rel.map(|r| akm_core::signal::quality(r).as_str()),
        "battery_estimate_pct": est.map(|e| e.pct),
        "battery_estimate_low": est.map(|e| e.low),
        "battery_estimate_high": est.map(|e| e.high),
        "battery_chemistry": est.map(|e| e.chemistry.as_str()),
        "new_batteries": battery.map(|b| b.new_batteries),
        "battery_kept": battery.map(|b| b.kept),
        "battery_apple_display_pct": battery.and_then(|b| b.apple_display_pct).map(crate::render::pct_u32),
        "battery_thresholds": battery.and_then(|b| b.thresholds).map(|t| json!({
            "full_mv": t.full_mv, "low_mv": t.low_mv, "critical_mv": t.critical_mv, "empty_mv": t.empty_mv,
            "level": battery.and_then(|b| b.threshold_level.clone()),
            "margins_mv": battery.and_then(|b| b.threshold_margins_mv),
        })),
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
        "will_shutdown": shutdown_notify::status_json(shutdown_notify::configured()),
    })
}

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
    v.map_or_else(|| tr!("n/a"), |x| format!("{x}{unit}"))
}

fn signal_value(s: &Snapshot) -> String {
    if let Some(t) = akm_core::signal::text(s.rssi()).map(|t| after_label(&t).to_string()) {
        return t;
    }
    let issue = s
        .connected
        .then(|| s.keyboard.as_ref()?.radio.rssi_error.as_ref())
        .flatten();
    match issue {
        Some(i) => {
            let line = akm_core::rssi::short_line(&i.code);
            let short = after_label(&line);
            let (_, fix) = akm_core::rssi::explain(&i.code);
            let why = short
                .find('(')
                .map_or(short, |i| short[i + 1..].trim_end_matches(')'));
            if fix.starts_with(why) {
                tr!("not measured; fix: {fix}", fix = fix)
            } else {
                tr!("{short}; fix: {fix}", short = short, fix = fix)
            }
        }
        None => tr!("n/a"),
    }
}

/// Text after the first colon: `"Signal: good (0)"` gives `"good (0)"` in every language.
fn after_label(t: &str) -> &str {
    t.split_once(':').map_or(t, |(_, r)| r.trim_start())
}

#[allow(clippy::too_many_lines)] // text rendered line by line
pub fn to_text(s: &Snapshot, fn_mode: Option<u8>) -> String {
    let mut out = String::new();
    let mut line = |k: String, v: String| {
        let _ = writeln!(out, "{k:<10} {v}");
    };
    line(
        tr!("Keyboard:"),
        s.model().map_or_else(|| tr!("n/a"), str::to_string),
    );
    line(
        tr!("Name:"),
        s.display_name().map_or_else(|| tr!("n/a"), str::to_string),
    );
    line(
        tr!("On kb:"),
        s.keyboard
            .as_ref()
            .and_then(|k| k.device.name_on_keyboard.clone())
            .map_or_else(
                || tr!("n/a (not read yet)"),
                |n| tr!("{n} (name stored in the keyboard)", n = n),
            ),
    );
    if let Some(note) = kernel_name_note(s) {
        line(tr!("Kernel:"), note);
    }
    line(
        tr!("MAC:"),
        s.mac().map_or_else(|| tr!("n/a"), str::to_string),
    );
    line(
        tr!("Connected:"),
        if s.connected { tr!("yes") } else { tr!("no") },
    );
    line(
        tr!("Battery:"),
        s.battery_pct().map_or_else(
            || tr!("n/a"),
            |p| {
                let kept = s.keyboard.as_ref().is_some_and(|k| k.battery.kept);
                let p = format!("{p:.0}");
                if kept {
                    tr!(
                        "{p} % (keyboard indication, kept from the last read)",
                        p = p
                    )
                } else {
                    tr!("{p} % (keyboard indication)", p = p)
                }
            },
        ),
    );
    if let Some(b) = s.keyboard.as_ref().map(|k| &k.battery) {
        if b.new_batteries {
            line(tr!("Estimate:"), tr!("new batteries, no estimate yet"));
        } else if let Some(e) = &b.charge_estimate {
            line(
                tr!("Estimate:"),
                tr!(
                    "~{pct} % ({low} to {high} %) from the voltage, {chemistry}",
                    pct = format!("{:.0}", e.pct),
                    low = format!("{:.0}", e.low),
                    high = format!("{:.0}", e.high),
                    chemistry = e.chemistry.as_str()
                ),
            );
        }
    }
    // One percentage in the text, as in the widget: the keyboard's own; the voltage estimate
    // is labelled as such; the macOS-style value stays in --json only (R13).
    if let Some(t) = s.keyboard.as_ref().and_then(|k| k.battery.thresholds) {
        line(
            tr!("Limits:"),
            tr!(
                "Full {full} / Low {low} / Critical {critical} / Empty {empty} mV",
                full = t.full_mv,
                low = t.low_mv,
                critical = t.critical_mv,
                empty = t.empty_mv
            ),
        );
    }
    if let Some(fw) = s.firmware().and_then(akm_core::firmware::summary) {
        line(tr!("Firmware:"), after_label(&fw).to_string());
    }
    line(
        tr!("Voltage:"),
        opt(s.voltage().map(|v| format!("{v:.2}")), " V"),
    );
    line(tr!("Signal:"), signal_value(s));
    let now = akm_core::history::Clock::now(&akm_core::history::SystemClock);
    if let Some(age) = s.update_age_s(now) {
        line(tr!("Updated:"), tr!("{age} s ago", age = age));
    }
    line(
        tr!("Fn mode:"),
        fn_mode.map_or_else(|| tr!("n/a"), |m| format!("{m} - {}", fnmode::label(m))),
    );
    line(
        tr!("Shutdown:"),
        shutdown_notify::status_text(shutdown_notify::configured()),
    );
    if let Some(e) = s.kb_error.as_deref().or(s.last_error.as_deref()) {
        line(tr!("Error:"), e.to_string());
    }
    out
}

pub fn kernel_name_note(s: &Snapshot) -> Option<String> {
    let alias = s.alias()?;
    let kernel = s.kernel_name()?;
    (alias != kernel).then(|| {
        tr!(
            "{kernel} - KWin and System Settings > Keyboard show this kernel name; \
             Bluetooth and Battery show the alias \"{alias}\". The kernel name changes only \
             after the keyboard reconnects (switch it off and on).",
            kernel = kernel,
            alias = alias
        )
    })
}

pub fn absent_text(fn_mode: Option<u8>) -> String {
    let row = |k: String, v: String| format!("{k:<10} {v}\n");
    row(
        tr!("Daemon:"),
        tr!(
            "not running ({name} absent on the session bus)",
            name = "com.agenceapi.AppleKbMonitor1"
        ),
    ) + &row(
        tr!("Fn mode:"),
        fn_mode.map_or_else(|| tr!("n/a"), |m| format!("{m} - {}", fnmode::label(m))),
    ) + &row(
        tr!("Shutdown:"),
        shutdown_notify::status_text(shutdown_notify::configured()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"schema":1,"version":28,"connected":true,"keyboard":{"device":{"model":"Apple Wireless Keyboard (A1314)","mac":"AA:BB:CC:DD:EE:F1","alias":"Alice's keyboard #1"},"battery":{"percentage":99.0,"voltage":2.9},"radio":{"rssi_dbm":0}},"last_update":1790000000}"#;

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
        assert!(
            v["battery_estimate_pct"].is_null(),
            "no estimate in this sample"
        );
        assert_eq!(v["mac"], "AA:BB:CC:DD:EE:F1");
        assert_eq!(v["name"], "Alice's keyboard #1");
        assert_eq!(v["alias"], "Alice's keyboard #1");
        assert_eq!(v["fnmode"], 2);
        assert!(v["fnmode_label"]
            .as_str()
            .unwrap()
            .starts_with("fkeysfirst"));
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
        let s = mk("alex", "Alice's keyboard #1");
        let note = kernel_name_note(&s).expect("names differ");
        assert!(
            note.starts_with("Alice's keyboard #1")
                && note.contains("alex")
                && note.contains("reconnects")
        );
        let text = to_text(&s, None);
        assert!(text.contains("Kernel:") && text.contains("KWin"), "{text}");
        assert_eq!(
            to_json(&s, None, None)["kernel_name"],
            "Alice's keyboard #1"
        );
        let same = mk("Alice's keyboard #1", "Alice's keyboard #1");
        assert!(kernel_name_note(&same).is_none());
        assert!(!to_text(&same, None).contains("Kernel:"));
        let mut no_alias = mk("x", "Alice's keyboard #1");
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
        assert!(
            t.contains("Estimate:  ~30 % (20 to 40 %) from the voltage, alkaline"),
            "{t}"
        );
        s.keyboard.as_mut().unwrap().battery.new_batteries = true;
        assert!(to_text(&s, None).contains("new batteries"));
    }

    #[test]
    fn signal_says_why_it_is_not_measured() {
        let mut s: Snapshot = serde_json::from_str(SAMPLE).unwrap();
        let radio = &mut s.keyboard.as_mut().unwrap().radio;
        radio.rssi_rel_db = None;
        radio.rssi_dbm = None;
        radio.rssi_error = Some(akm_core::rssi::RssiIssue {
            code: akm_core::rssi::CODE_HELPER_MISSING.into(),
            detail: "rssi-helper not installed".into(),
        });
        s.connected = true;
        let t = to_text(&s, None);
        assert!(
            t.contains("Signal:    not measured (rssi-helper not installed); fix: reinstall the apple-kb-monitor package"),
            "{t}"
        );
        s.keyboard.as_mut().unwrap().radio.rssi_error = Some(akm_core::rssi::RssiIssue {
            code: akm_core::rssi::CODE_HELPER_FAILED.into(),
            detail: String::new(),
        });
        let t = to_text(&s, None);
        assert!(
            t.contains("Signal:    not measured (see akmctl doctor); fix: reinstall the apple-kb-monitor package, then run akmctl doctor"),
            "{t}"
        );
        s.connected = false;
        assert!(to_text(&s, None).contains("Signal:    n/a"));
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
        assert!(t.contains("Name:      Alice's keyboard #1"));
        assert!(t.contains("Fn mode:   1 - fkeyslast"));
        assert!(to_text(&Snapshot::default(), None).contains("Battery:   n/a"));
        let mut k: Snapshot = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(to_json(&k, None, None)["battery_kept"], false);
        k.keyboard.as_mut().unwrap().battery.kept = true;
        assert!(to_text(&k, None)
            .contains("Battery:   99 % (keyboard indication, kept from the last read)"));
        assert_eq!(to_json(&k, None, None)["battery_kept"], true);
    }

    #[test]
    fn firmware_thresholds_and_apple_display_in_status() {
        let mut s: Snapshot = serde_json::from_str(SAMPLE).unwrap();
        let k = s.keyboard.as_mut().unwrap();
        k.firmware.version = Some("0x0050".into());
        akm_core::firmware::assess_report(Some(0x0239), &mut k.firmware);
        k.battery.apple_display_pct = Some(100.0);
        let t = akm_core::registry::Thresholds::parse(&[
            0x0b, 0x8a, 0x09, 0xca, 0x09, 0x64, 0x08, 0x06,
        ])
        .unwrap();
        k.battery.thresholds = Some(t);
        k.battery.threshold_level = Some("ok".into());
        k.battery.threshold_margins_mv = Some(t.margins(2986));
        let v = to_json(&s, None, None);
        assert_eq!(v["firmware"]["version_hex"], "0x0050");
        assert_eq!(v["firmware"]["status"], "up_to_date");
        assert!(v["name_on_keyboard"].is_null());
        assert!(to_text(&s, None).contains("On kb:     n/a (not read yet)"));
        let mut s2 = s.clone();
        s2.keyboard.as_mut().unwrap().device.name_on_keyboard = Some("Alice's keyboard #1".into());
        assert_eq!(
            to_json(&s2, None, None)["name_on_keyboard"],
            "Alice's keyboard #1"
        );
        assert!(to_text(&s2, None)
            .contains("On kb:     Alice's keyboard #1 (name stored in the keyboard)"));
        assert_eq!(v["firmware"]["latest_known"], "0x0050");
        assert_eq!(v["battery_apple_display_pct"], 100);
        assert_eq!(v["battery_thresholds"]["low_mv"], 2506);
        assert_eq!(v["battery_thresholds"]["level"], "ok");
        let txt = to_text(&s, None);
        assert!(txt.contains("Firmware:  0x0050 - up to date"), "{txt}");
        assert!(
            !txt.contains("Apple:"),
            "R13: one percentage in the text: {txt}"
        );
        assert!(txt.contains("Limits:    Full 2954"), "{txt}");
        // Unknown firmware stays null / "unknown", never invented.
        let v = to_json(&Snapshot::default(), None, None);
        assert!(v["firmware"].is_null() && v["battery_thresholds"].is_null());
    }
}
