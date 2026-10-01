//! Client side of `com.agenceapi.AppleKbMonitor1` (UI, `--json`), plus the
//! direct one-shot read used when the daemon is absent.

use akm_core::history::{self, HistoryEntry};
use akm_core::{hidraw, led, Snapshot};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use crate::service::{BUS_NAME, INTERFACE, OBJECT_PATH};

/// Where a snapshot came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Daemon,
    Direct,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Daemon => "daemon",
            Source::Direct => "direct",
        }
    }
}

/// Is the daemon on this bus?
pub fn daemon_present(conn: &Connection) -> bool {
    let Ok(p) = zbus::blocking::fdo::DBusProxy::new(conn) else { return false };
    p.name_has_owner(BUS_NAME.try_into().expect("valid name")).unwrap_or(false)
}

/// Start the daemon by D-Bus activation if a service file declares it
/// (`dbus/com.agenceapi.AppleKbMonitor1.service`). True if it is now running.
pub fn activate(conn: &Connection) -> bool {
    let Ok(p) = zbus::blocking::fdo::DBusProxy::new(conn) else { return false };
    let activatable = p
        .list_activatable_names()
        .map(|names| names.iter().any(|n| n.as_str() == BUS_NAME))
        .unwrap_or(false);
    activatable
        && p.start_service_by_name(BUS_NAME.try_into().expect("valid name"), 0)
            .map_err(|e| tracing::warn!("activation of {BUS_NAME} failed: {e}"))
            .is_ok()
}

fn get_prop(conn: &Connection, name: &str) -> zbus::Result<OwnedValue> {
    conn.call_method(Some(BUS_NAME), OBJECT_PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(INTERFACE, name))?
        .body()
        .deserialize()
}

/// Current snapshot from the daemon (`Json` property).
pub fn fetch_snapshot(conn: &Connection) -> zbus::Result<Snapshot> {
    let v = get_prop(conn, "Json")?;
    let s: String = v.try_into().map_err(zbus::Error::Variant)?;
    serde_json::from_str(&s).map_err(|e| zbus::Error::Failure(format!("bad Json property: {e}")))
}

/// History entries since `since` from the daemon.
pub fn fetch_history(conn: &Connection, since: u64) -> zbus::Result<Vec<HistoryEntry>> {
    let s: String = conn
        .call_method(Some(BUS_NAME), OBJECT_PATH, Some(INTERFACE), "History", &(since,))?
        .body()
        .deserialize()?;
    serde_json::from_str(&s).map_err(|e| zbus::Error::Failure(format!("bad History reply: {e}")))
}

/// Ask the daemon for a full read.
pub fn request_refresh(conn: &Connection) -> zbus::Result<()> {
    conn.call_method(Some(BUS_NAME), OBJECT_PATH, Some(INTERFACE), "Refresh", &())?;
    Ok(())
}

/// One-shot direct read (daemon absent): HID if readable, else kernel
/// power_supply; no BlueZ provider, no history write. Closes the hidraw fd.
pub fn direct_snapshot() -> Snapshot {
    let now = akm_core::history::Clock::now(&akm_core::history::SystemClock);
    let mut err = None;
    let kb = hidraw::read_keyboard().or_else(|| {
        let r = hidraw::find_apple_keyboard_mac_in(std::path::Path::new("/sys")).and_then(|m| hidraw::report_from_sysfs(&m));
        if r.is_some() {
            err = Some("HID diagnostics unavailable (hidraw not readable): kernel battery only".to_string());
        }
        r
    });
    hidraw::set_wake_monitor_enabled(false);
    hidraw::close_hid_fd();
    let (caps, num) = if kb.is_some() { led::read_led_state() } else { (false, false) };
    let remaining = history::estimate_remaining(&history::History::open_default().read())
        .map(|(r, h)| history::format_remaining(r, h));
    Snapshot {
        connected: kb.is_some(),
        kb_error: kb.is_none().then(|| "Keyboard: not found".to_string()),
        last_update: if kb.is_some() { now } else { 0 },
        keyboard: kb,
        caps_lock: caps,
        num_lock: num,
        remaining_display: remaining,
        last_error: err,
        ..Default::default()
    }
}

/// Daemon first; direct read only if allowed and the daemon is absent.
pub fn snapshot(allow_direct: bool) -> Result<(Snapshot, Source), String> {
    let daemon_err = match Connection::session() {
        Ok(conn) if daemon_present(&conn) || activate(&conn) => match fetch_snapshot(&conn) {
            Ok(s) => return Ok((s, Source::Daemon)),
            Err(e) => format!("daemon error: {e}"),
        },
        Ok(_) => format!("{BUS_NAME} not running"),
        Err(e) => format!("no session bus: {e}"),
    };
    if allow_direct {
        Ok((direct_snapshot(), Source::Direct))
    } else {
        Err(daemon_err)
    }
}

/// `--json` output: the snapshot plus its source and the daemon version.
pub fn to_json(s: &Snapshot, src: Source) -> String {
    let mut v = serde_json::to_value(s).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.insert("source".into(), src.as_str().into());
        o.insert("battery_percent".into(), s.battery_pct().map(|p| p.round()).into());
    }
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_output_carries_source_and_summary() {
        let mut k = akm_core::KbReport::default();
        k.battery.percentage = Some(90.0);
        let s = Snapshot { connected: true, keyboard: Some(k), ..Default::default() };
        let v: serde_json::Value = serde_json::from_str(&to_json(&s, Source::Daemon)).unwrap();
        assert_eq!(v["source"], "daemon");
        assert_eq!(v["battery_percent"], 90.0);
        assert_eq!(v["schema"], akm_core::snapshot::SCHEMA_VERSION);
        assert_eq!(v["keyboard"]["battery"]["percentage"], 90.0);
        let v: serde_json::Value = serde_json::from_str(&to_json(&Snapshot::default(), Source::Direct)).unwrap();
        assert!(v["battery_percent"].is_null());
    }
}
