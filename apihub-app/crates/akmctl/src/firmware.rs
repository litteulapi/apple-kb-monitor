//! `akmctl firmware`: the firmware version of the keyboard against the
//! embedded table (#227). Reads the daemon's snapshot only (the version is
//! read once per connection by the daemon); no network, no request to the
//! keyboard, no flash.

use akm_core::report::KbFirmware;
use akm_core::Snapshot;
use serde_json::{json, Value};

fn status_text(fw: &KbFirmware) -> String {
    match (fw.status.as_str(), fw.latest_known.as_deref()) {
        ("up_to_date", _) => "up to date (latest public version known)".into(),
        ("update_available", Some(l)) => {
            format!("update available from Apple (latest known: {l}); this program never flashes")
        }
        ("update_available", None) => "update available from Apple".into(),
        _ => "unknown (model or version not in the table)".into(),
    }
}

pub fn to_json(s: &Snapshot) -> Value {
    let fw = s.firmware().cloned().unwrap_or_default();
    json!({
        "schema": 1,
        "daemon": true,
        "model": s.model(),
        "firmware": {
            "version_hex": fw.version_hex.or(fw.version),
            "latest_known": fw.latest_known,
            "status": if fw.status.is_empty() { "unknown" } else { fw.status.as_str() },
            "source": fw.source,
            "table_date": fw.table_date.unwrap_or_else(|| akm_core::firmware::TABLE_DATE.to_string()),
        },
    })
}

pub fn to_text(s: &Snapshot) -> String {
    let fw = s.firmware().cloned().unwrap_or_default();
    let mut out = String::new();
    let mut line = |k: &str, v: String| out.push_str(&format!("{k:<14}{v}\n"));
    line("Keyboard:", s.model().unwrap_or("n/a").to_string());
    line(
        "Version:",
        fw.version.clone().unwrap_or_else(|| "not read yet (read once per connection)".into()),
    );
    line("Latest known:", fw.latest_known.clone().unwrap_or_else(|| "n/a (model not in the table)".into()));
    line("Status:", status_text(&fw));
    line("Source:", fw.source.clone().unwrap_or_else(|| "n/a".into()));
    line(
        "Table date:",
        fw.table_date.clone().unwrap_or_else(|| akm_core::firmware::TABLE_DATE.to_string()),
    );
    out
}

/// Static part, when the daemon is absent: what the embedded table knows.
pub fn table_text() -> String {
    let mut out = format!("Embedded table (reviewed {}):\n", akm_core::firmware::TABLE_DATE);
    for k in akm_core::firmware::KNOWN_FIRMWARE {
        let pids: Vec<String> = k.pids.iter().map(|p| format!("{p:#06x}")).collect();
        out.push_str(&format!(
            "  {}: latest {} ({})\n",
            pids.join(", "),
            akm_core::firmware::hex(k.latest),
            k.source
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(version: &str, pid: u32) -> Snapshot {
        let mut k = akm_core::KbReport::default();
        k.device.model = Some("Apple Wireless Keyboard (A1314)".into());
        k.firmware.version = Some(version.into());
        akm_core::firmware::assess_report(Some(pid), &mut k.firmware);
        Snapshot {
            keyboard: Some(k),
            connected: true,
            ..Default::default()
        }
    }

    #[test]
    fn current_firmware() {
        let s = snap("0x0050", 0x0256);
        let v = to_json(&s);
        assert_eq!(v["firmware"]["version_hex"], "0x0050");
        assert_eq!(v["firmware"]["status"], "up_to_date");
        assert_eq!(v["firmware"]["latest_known"], "0x0050");
        assert!(v["firmware"]["source"].as_str().unwrap().contains("A1314"));
        let t = to_text(&s);
        assert!(t.contains("Version:      0x0050"), "{t}");
        assert!(t.contains("Status:       up to date"), "{t}");
        assert!(t.contains("Table date:   2026-10-01"), "{t}");
    }

    #[test]
    fn old_and_unknown() {
        let t = to_text(&snap("0x0044", 0x0239));
        assert!(t.contains("update available from Apple"), "{t}");
        assert!(t.contains("never flashes"));
        let v = to_json(&Snapshot::default());
        assert_eq!(v["firmware"]["status"], "unknown");
        assert!(v["firmware"]["version_hex"].is_null());
        assert!(to_text(&Snapshot::default()).contains("not read yet"));
        assert!(table_text().contains("0x0256"));
    }
}
