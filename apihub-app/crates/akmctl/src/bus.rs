//! Session-bus access to `com.agenceapi.AppleKbMonitor1` (blocking zbus).

use akm_core::tr;
use akm_core::Snapshot;
use zbus::blocking::{fdo::DBusProxy, Connection, Proxy};
use zbus::names::WellKnownName;

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
    Connection::session().map_err(|e| BusError::Absent(tr!("no session bus: {e}", e = e)))
}

pub fn daemon_present(conn: &Connection) -> bool {
    let Ok(p) = DBusProxy::new(conn) else {
        return false;
    };
    p.name_has_owner(WellKnownName::from_static_str_unchecked(BUS_NAME).into())
        .unwrap_or(false)
}

pub fn proxy(conn: &Connection) -> Result<Proxy<'_>, BusError> {
    Proxy::new(conn, BUS_NAME, OBJECT_PATH, INTERFACE)
        .map_err(|e| BusError::Failed(format!("proxy: {e}")))
}

pub fn parse_snapshot(json: &str) -> Result<Snapshot, BusError> {
    serde_json::from_str(json).map_err(|e| BusError::Failed(tr!("bad snapshot JSON: {e}", e = e)))
}

/// Proxy to the daemon; `Absent` when it is not on the bus (never D-Bus-activated).
pub fn daemon(conn: &Connection) -> Result<Proxy<'_>, BusError> {
    if !daemon_present(conn) {
        return Err(BusError::Absent(tr!(
            "daemon not running ({name} absent on the session bus)",
            name = BUS_NAME
        )));
    }
    proxy(conn)
}

/// Current snapshot; `Absent` when the daemon is not on the bus.
pub fn get_state(conn: &Connection) -> Result<Snapshot, BusError> {
    let json: String = daemon(conn)?
        .call("GetState", &())
        .map_err(|e| BusError::Failed(format!("GetState: {e}")))?;
    parse_snapshot(&json)
}

pub fn set_alias(conn: &Connection, mac: &str, name: &str) -> Result<String, BusError> {
    daemon(conn)?
        .call("SetAlias", &(mac, name))
        .map_err(|e| BusError::Failed(format!("SetAlias: {e}")))
}

pub fn expect_disconnect(conn: &Connection) -> Result<bool, BusError> {
    daemon(conn)?
        .call("ExpectDisconnect", &())
        .map_err(|e| BusError::Failed(format!("ExpectDisconnect: {e}")))
}

pub fn reread_name(conn: &Connection) -> Result<(bool, String), BusError> {
    let reply = daemon(conn)?
        .call_method("RereadName", &())
        .map_err(|e| BusError::Failed(format!("RereadName: {e}")))?;
    reread_reply(reply.body().deserialize::<(bool, String)>())
}

/// A reply that is not `(bs)` is a protocol mismatch, never a success.
fn reread_reply(body: zbus::Result<(bool, String)>) -> Result<(bool, String), BusError> {
    body.map_err(|e| BusError::Failed(format!("RereadName: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_reread_reply_is_a_failure() {
        let bad = reread_reply(Err(zbus::Error::Failure("bad signature".into())));
        assert!(matches!(bad, Err(BusError::Failed(m)) if m.contains("bad signature")));
        assert_eq!(
            reread_reply(Ok((false, "x".into()))).unwrap(),
            (false, "x".into())
        );
    }

    #[test]
    fn parse_snapshot_rejects_garbage() {
        assert!(parse_snapshot("not json").is_err());
        assert!(parse_snapshot("{}").is_ok());
    }
}
