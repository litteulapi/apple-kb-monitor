//! Passive input state (`...Input` interface of the keyboard object) for `status
//! --json`: Fn-lock byte, last sleep event, wake count, Eject.

use serde_json::Value;
use zbus::blocking::{Connection, Proxy};

use crate::bus::BUS_NAME;

const INTERFACE: &str = "com.agenceapi.AppleKbMonitor1.Input";

pub fn device_path(mac: &str) -> Option<String> {
    let parts: Vec<&str> = mac.split(':').collect();
    let ok = parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()));
    ok.then(|| {
        format!(
            "/com/agenceapi/AppleKbMonitor1/devices/{}",
            mac.to_ascii_uppercase().replace(':', "_")
        )
    })
}

pub fn fetch(conn: &Connection, mac: Option<&str>) -> Value {
    let Some(path) = mac.and_then(device_path) else {
        return Value::Null;
    };
    let Ok(p) = Proxy::new(conn, BUS_NAME, path, INTERFACE) else {
        return Value::Null;
    };
    p.get_property::<String>("State")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(
            device_path("AA:BB:CC:DD:EE:F1").as_deref(),
            Some("/com/agenceapi/AppleKbMonitor1/devices/AA_BB_CC_DD_EE_F1")
        );
        assert_eq!(device_path("nope"), None);
        assert_eq!(device_path("AA:BB:CC:DD:EE:EG"), None);
    }

    #[test]
    fn no_mac_is_null() {
        if let Ok(c) = Connection::session() {
            assert_eq!(fetch(&c, None), Value::Null);
            assert_eq!(fetch(&c, Some("bad")), Value::Null);
        }
    }
}
