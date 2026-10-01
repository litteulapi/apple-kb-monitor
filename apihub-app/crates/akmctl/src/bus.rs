//! Session-bus access to `com.agenceapi.AppleKbMonitor1` (blocking zbus).

use akm_core::Snapshot;
use zbus::blocking::{fdo::DBusProxy, Connection, Proxy};

pub const BUS_NAME: &str = "com.agenceapi.AppleKbMonitor1";
pub const OBJECT_PATH: &str = "/com/agenceapi/AppleKbMonitor1";
pub const INTERFACE: &str = "com.agenceapi.AppleKbMonitor1";

#[derive(Debug)]
pub enum BusError {
    /// No session bus, or the daemon does not own the name.
    Absent(String),
    Failed(String),
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BusError::Absent(m) | BusError::Failed(m) => f.write_str(m),
        }
    }
}

pub fn connect() -> Result<Connection, BusError> {
    Connection::session().map_err(|e| BusError::Absent(format!("no session bus: {e}")))
}

pub fn daemon_present(conn: &Connection) -> bool {
    let Ok(p) = DBusProxy::new(conn) else {
        return false;
    };
    p.name_has_owner(BUS_NAME.try_into().expect("valid bus name"))
        .unwrap_or(false)
}

pub fn proxy(conn: &Connection) -> Result<Proxy<'_>, BusError> {
    Proxy::new(conn, BUS_NAME, OBJECT_PATH, INTERFACE)
        .map_err(|e| BusError::Failed(format!("proxy: {e}")))
}

/// Parse a snapshot JSON string (`GetState` / `StateChanged.json`).
pub fn parse_snapshot(json: &str) -> Result<Snapshot, BusError> {
    serde_json::from_str(json).map_err(|e| BusError::Failed(format!("bad snapshot JSON: {e}")))
}

/// Current snapshot; `Absent` when the daemon is not on the bus (no
/// activation attempt: a status command must not start services).
pub fn get_state(conn: &Connection) -> Result<Snapshot, BusError> {
    if !daemon_present(conn) {
        return Err(BusError::Absent(format!(
            "daemon not running ({BUS_NAME} absent on the session bus)"
        )));
    }
    let json: String = proxy(conn)?
        .call("GetState", &())
        .map_err(|e| BusError::Failed(format!("GetState: {e}")))?;
    parse_snapshot(&json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_snapshot_rejects_garbage() {
        assert!(parse_snapshot("not json").is_err());
        assert!(parse_snapshot("{}").is_ok());
    }
}
