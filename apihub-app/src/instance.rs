//! Single instance (`org.freedesktop.Application` on `com.agenceapi.AppleKbMonitor`)
//! and detection of the tray provided by the daemon.

use std::collections::HashMap;

use apple_kb_monitord::service::BUS_NAME as DAEMON_NAME;
use zbus::blocking::Connection;
use zbus::fdo::RequestNameReply;
use zbus::interface;
use zbus::names::WellKnownName;
use zbus::zvariant::{OwnedValue, Value};

/// Wayland app_id, D-Bus name and desktop-file id of the window.
pub const APP_ID: &str = "com.agenceapi.AppleKbMonitor";
const APP_PATH: &str = "/com/agenceapi/AppleKbMonitor";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

struct Application {
    on_activate: Box<dyn Fn() + Send + Sync>,
}

#[interface(name = "org.freedesktop.Application")]
impl Application {
    fn activate(&self, _platform_data: HashMap<String, OwnedValue>) {
        (self.on_activate)();
    }
    fn open(&self, _uris: Vec<String>, _platform_data: HashMap<String, OwnedValue>) {
        (self.on_activate)();
    }
    fn activate_action(&self, _name: String, _params: Vec<OwnedValue>, _platform_data: HashMap<String, OwnedValue>) {
        (self.on_activate)();
    }
}

pub enum Claim {
    /// We own the name; keep the connection alive for the whole run.
    Primary(Connection),
    /// Another instance exists and has been asked to show its window.
    Existing,
    /// No session bus: run without single-instance protection.
    NoBus,
}

pub fn claim(on_activate: impl Fn() + Send + Sync + 'static) -> Claim {
    let Ok(conn) = Connection::session() else { return Claim::NoBus };
    if conn.object_server().at(APP_PATH, Application { on_activate: Box::new(on_activate) }).is_err() {
        return Claim::NoBus;
    }
    let Ok(name) = WellKnownName::try_from(APP_ID) else { return Claim::NoBus };
    use zbus::fdo::RequestNameFlags::DoNotQueue;
    match conn.request_name_with_flags(name, DoNotQueue.into()) {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Claim::Primary(conn),
        // zbus maps "Exists" to NameTaken; both mean another instance owns the name.
        Ok(_) | Err(zbus::Error::NameTaken) => activate_existing(&conn),
        Err(e) => {
            eprintln!("[apihub] cannot claim {APP_ID}: {e}");
            Claim::NoBus
        }
    }
}

fn activate_existing(conn: &Connection) -> Claim {
    let args: HashMap<&str, Value<'_>> = HashMap::new();
    match conn.call_method(Some(APP_ID), APP_PATH, Some("org.freedesktop.Application"), "Activate", &(args,)) {
        Ok(_) => Claim::Existing,
        Err(e) => {
            eprintln!("[apihub] existing instance unreachable ({e}): opening anyway");
            Claim::NoBus
        }
    }
}

/// Does one of the registered SNI items belong to `owner` (a unique name)?
/// `resolve` maps a well-known name to its current owner.
pub fn items_owned_by(items: &[String], owner: &str, resolve: impl Fn(&str) -> Option<String>) -> bool {
    items.iter().any(|item| {
        let name = item.split('/').next().unwrap_or("");
        if name.starts_with(':') {
            name == owner
        } else {
            resolve(name).as_deref() == Some(owner)
        }
    })
}

/// True when the daemon is on the bus AND has registered its own tray item
/// with the StatusNotifierWatcher.
pub fn daemon_tray_present() -> bool {
    let Ok(conn) = Connection::session() else { return false };
    let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return false };
    let owner_of = |n: &str| -> Option<String> {
        let name = zbus::names::BusName::try_from(n).ok()?;
        dbus.get_name_owner(name).ok().map(|o| o.to_string())
    };
    let Some(daemon) = owner_of(DAEMON_NAME) else { return false };
    let Ok(reply) = conn.call_method(
        Some(WATCHER),
        "/StatusNotifierWatcher",
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(WATCHER, "RegisteredStatusNotifierItems"),
    ) else {
        return false;
    };
    let Ok(v) = reply.body().deserialize::<OwnedValue>() else { return false };
    let Ok(items) = Vec::<String>::try_from(v) else { return false };
    items_owned_by(&items, &daemon, owner_of)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_daemon_item_by_owner() {
        let items = vec![
            "org.kde.StatusNotifierItem-4242-1/StatusNotifierItem".to_string(),
            ":1.77/StatusNotifierItem".to_string(),
        ];
        let resolve = |n: &str| (n == "org.kde.StatusNotifierItem-4242-1").then(|| ":1.50".to_string());
        assert!(items_owned_by(&items, ":1.50", resolve));
        assert!(items_owned_by(&items, ":1.77", resolve));
        assert!(!items_owned_by(&items, ":1.99", resolve));
        assert!(!items_owned_by(&[], ":1.50", resolve));
    }
}
