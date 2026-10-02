//! `akmctl info`: the register map of the keyboard (docs/RE-*.md) with, for
//! each report, its name, size, safety class and the decoded value **when the
//! daemon has it in its snapshot**, "never read" otherwise.
//!
//! This command NEVER talks to the keyboard: it only reads the daemon's
//! snapshot and passive state over D-Bus (and works offline from the static
//! table when the daemon is absent). The one command that requests reports
//! is `akmctl dump`, restricted to the same registry.

use akm_core::registry::{self, Decoded, Direction, Entry};
use akm_core::Snapshot;
use serde_json::{json, Value};

/// Text of the value of `e` known by the daemon, if any.
pub fn cached_value(e: &Entry, snap: Option<&Snapshot>, passive: &Value) -> Option<String> {
    let kb = snap.and_then(|s| s.keyboard.as_ref());
    match e.dir {
        Direction::Feature => {
            let kb = kb?;
            // Decoded fields first (they survive the raw map), then raw payloads.
            match e.id {
                0x4F if e.dir == Direction::Feature => {
                    return kb.firmware.version.clone();
                }
                0x60 => {
                    return kb.battery.thresholds.map(|t| Decoded::Thresholds(t).to_string());
                }
                _ => {}
            }
            let hex = kb.raw.get(&format!("{:#04x}", e.id))?;
            let bytes = parse_hex(hex)?;
            let d = e.decode_payload(&bytes)?;
            Some(e.render(&d))
        }
        Direction::Input => {
            let n = |k: &str| passive.get(k).and_then(Value::as_u64);
            match e.id {
                0x30 => n("battery_status").map(|b| {
                    let st = registry::BatteryState::from_byte(u8::try_from(b).unwrap_or(u8::MAX));
                    format!("{} ({b})", st.as_str())
                }),
                0x05 => n("fn_lock").map(|v| format!("{v}")),
                0x04 => n("last_sleep_code").map(|v| format!("{v}")),
                0x13 => {
                    let ready = passive.get("last_wake_ready").and_then(Value::as_bool)?;
                    let req = passive.get("last_wake_conn_request").and_then(Value::as_bool)?;
                    Some(format!("ready={ready} connection_request={req}"))
                }
                _ => None,
            }
        }
        Direction::Output => None,
    }
}

fn parse_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn len_text(e: &Entry) -> String {
    e.len.map_or_else(|| "-".to_string(), |l| l.to_string())
}

/// Aligned table, one line per report, then a legend.
pub fn to_text(snap: Option<&Snapshot>, passive: &Value) -> String {
    let mut out = String::from(
        "Register map of the Apple A1314 (BCM2042). Values come from the daemon's cache: this command never reads the keyboard.\n\n",
    );
    out.push_str(&format!(
        "{:<5} {:<8} {:>3}  {:<18} {:<28} {}\n",
        "ID", "Dir", "Len", "Class", "Name", "Value"
    ));
    for e in registry::sorted() {
        let name = e.apple_name.unwrap_or(e.name);
        let val = cached_value(e, snap, passive).map_or_else(
            || "never read".to_string(),
            |v| format!("{v} {}", e.proof.tag()),
        );
        out.push_str(&format!(
            "{:#04x}  {:<8} {:>3}  {:<18} {:<28} {}\n",
            e.id,
            e.dir.as_str(),
            len_text(e),
            e.safety.as_str(),
            name,
            val
        ));
    }
    out.push_str(
        "\nClasses: SafeRead = routine read; OncePerConnection = read once per connection; PassiveInput = listened to, never requested;\n\
ManualOnly = readable, never requested by the daemon; NeverRead = never requested (secret or freezes the firmware);\n\
WriteApple = written only by a named Apple operation: Shutdown = 0x40 WillShutdown (id only, what macOS sends),\n\
  DeviceName = 0x55 LongDeviceName (64 bytes, Lion; akmctl rename --device-name: pre-flight, backup, one confirmation, read back);\n\
NeverWrite = command / write-only register, no write path exists; Unknown = not understood.\n\
Details per report: akmctl info --json (meaning, unit, endianness, decoder, proof, source).\n",
    );
    out
}

/// One object per report.
pub fn to_json(snap: Option<&Snapshot>, passive: &Value) -> Value {
    let rows: Vec<Value> = registry::sorted()
        .into_iter()
        .map(|e| {
            let v = cached_value(e, snap, passive);
            json!({
                "id": format!("{:#04x}", e.id),
                "direction": e.dir.as_str(),
                "length": e.len,
                "apple_name": e.apple_name,
                "name": e.name,
                "meaning": e.meaning,
                "unit": e.unit,
                "endianness": e.endian.as_str(),
                "proof": e.proof.tag(),
                "safety": e.safety.as_str(),
                "source": e.source,
                "value": v,
                "read": v.is_some(),
            })
        })
        .collect();
    json!({ "schema": 1, "daemon": snap.is_some(), "reports": rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snapshot {
        let mut k = akm_core::KbReport::default();
        k.raw.insert("0x47".into(), "63".into());
        k.raw.insert("0x46".into(), "aa0b".into());
        k.raw.insert("0x49".into(), "860b".into());
        k.raw.insert("0x4f".into(), "5000".into());
        k.firmware.version = Some("0x0050".into());
        k.battery.thresholds = akm_core::registry::Thresholds::parse(&[0x0b, 0x8a, 0x09, 0xca, 0x09, 0x64, 0x08, 0x06]);
        Snapshot {
            keyboard: Some(k),
            connected: true,
            ..Default::default()
        }
    }

    #[test]
    fn lists_every_report_and_marks_unread_ones() {
        let t = to_text(None, &Value::Null);
        for e in registry::all() {
            assert!(t.contains(&format!("{:#04x}", e.id)), "{:#04x}", e.id);
        }
        assert!(t.contains("never read"));
        assert!(!t.contains("[mesuré]"), "no value without a daemon");
        let n = t.lines().filter(|l| l.starts_with("0x") && l.contains("never read")).count();
        assert_eq!(n, registry::all().len());
    }

    #[test]
    fn decodes_cached_values_only() {
        let s = snap();
        let p = json!({"battery_status": 1, "fn_lock": 2});
        let t = to_text(Some(&s), &p);
        let line = |id: &str| t.lines().find(|l| l.starts_with(id) && l.contains("Feature")).unwrap().to_string();
        assert!(line("0x47").contains("99 % [mesuré]"), "{}", line("0x47"));
        assert!(line("0x46").contains("2986 mV"));
        assert!(line("0x49").contains("2950 mV"));
        assert!(line("0x4f").contains("0x0050"));
        assert!(line("0x60").contains("Full 2954 / Low 2506 / Critical 2404 / Empty 2054 mV"));
        assert!(line("0x4c").contains("never read"));
        assert!(line("0xfe").contains("never read"));
        let inp = |id: &str| t.lines().find(|l| l.starts_with(id) && l.contains("Input")).unwrap().to_string();
        assert!(inp("0x30").contains("low (1)"));
        assert!(inp("0x05").contains('2'));
        assert!(inp("0x13").contains("never read"));
    }

    #[test]
    fn json_has_the_documented_fields() {
        let v = to_json(Some(&snap()), &Value::Null);
        assert_eq!(v["daemon"], true);
        let rows = v["reports"].as_array().unwrap();
        assert_eq!(rows.len(), registry::all().len());
        let r47 = rows.iter().find(|r| r["id"] == "0x47" && r["direction"] == "Feature").unwrap();
        assert_eq!(r47["apple_name"], "BatteryPercent");
        assert_eq!(r47["safety"], "SafeRead");
        assert_eq!(r47["value"], "99 %");
        assert_eq!(r47["read"], true);
        let r4c = rows.iter().find(|r| r["id"] == "0x4c").unwrap();
        assert!(r4c["value"].is_null() && r4c["read"] == false);
        for r in rows {
            for k in ["meaning", "unit", "endianness", "proof", "source", "name", "length"] {
                assert!(r.get(k).is_some(), "{k}");
            }
        }
    }

    #[test]
    fn corrupt_cache_never_panics() {
        let mut s = snap();
        let k = s.keyboard.as_mut().unwrap();
        for e in registry::all() {
            k.raw.insert(format!("{:#04x}", e.id), "zz".into());
        }
        k.raw.insert("0x47".into(), "ff".into());
        k.raw.insert("0x46".into(), "a".into());
        let _ = to_text(Some(&s), &json!({"battery_status": 99999999999u64}));
        let _ = to_json(Some(&s), &json!("x"));
    }

    /// The command can not reach the hardware: its source mentions none of the
    /// reading entry points (the string literals of this test aside).
    #[test]
    fn info_source_has_no_hardware_access() {
        let src = include_str!("info.rs");
        let body = src.split("#[cfg(test)]").next().unwrap();
        for banned in ["hidraw", "Hidraw", "read_safe", "SafeSource", "hid_read_feature", "ioctl", "/dev/"] {
            assert!(!body.contains(banned), "info.rs touches {banned}");
        }
    }
}
