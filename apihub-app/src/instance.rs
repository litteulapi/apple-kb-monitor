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
pub const APP_PATH: &str = "/com/agenceapi/AppleKbMonitor";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

struct Application {
    on_activate: Box<dyn Fn(Option<String>) + Send + Sync>,
}

/// Wayland/X11 activation token sent by the caller in `platform_data`
/// (`activation-token`, else the legacy `desktop-startup-id`).
pub fn activation_token(platform_data: &HashMap<String, OwnedValue>) -> Option<String> {
    ["activation-token", "desktop-startup-id"].iter().find_map(|k| {
        let v = platform_data.get(*k)?;
        let s: String = v.try_clone().ok()?.try_into().ok()?;
        (!s.is_empty()).then_some(s)
    })
}

#[interface(name = "org.freedesktop.Application")]
impl Application {
    fn activate(&self, platform_data: HashMap<String, OwnedValue>) {
        (self.on_activate)(activation_token(&platform_data));
    }
    fn open(&self, _uris: Vec<String>, platform_data: HashMap<String, OwnedValue>) {
        (self.on_activate)(activation_token(&platform_data));
    }
    fn activate_action(&self, _name: String, _params: Vec<OwnedValue>, platform_data: HashMap<String, OwnedValue>) {
        (self.on_activate)(activation_token(&platform_data));
    }
}

pub enum Claim {
    /// We own the name; keep the connection alive for the whole run.
    Primary(Connection),
    /// Another instance exists and has been asked to show its window.
    Existing,
    /// Another instance owns the name but never answered: never open a
    /// second window next to it (#197).
    Unreachable,
    /// No session bus: run without single-instance protection.
    NoBus,
}

/// Outcome of one attempt to own the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Owned,
    Taken,
    Failed,
}

/// Single-instance protocol, independent of D-Bus (unit-tested): own the
/// name, else activate its owner; if the owner does not answer (it is
/// exiting, dumping core or hung), retry so that a name freed in between is
/// taken by us. Never returns "open anyway" while another owner may live.
pub fn negotiate(
    attempts: u32,
    mut try_own: impl FnMut() -> Step,
    mut activate: impl FnMut() -> Result<(), String>,
    mut pause: impl FnMut(),
) -> Result<bool, String> {
    let mut last = String::from("no attempt");
    for i in 0..attempts.max(1) {
        if i > 0 {
            pause();
        }
        match try_own() {
            Step::Owned => return Ok(true),
            Step::Failed => return Err("cannot request the name".into()),
            Step::Taken => match activate() {
                Ok(()) => return Ok(false),
                Err(e) => last = e,
            },
        }
    }
    Err(last)
}

pub fn claim(on_activate: impl Fn(Option<String>) + Send + Sync + 'static) -> Claim {
    let Ok(conn) = Connection::session() else { return Claim::NoBus };
    if conn.object_server().at(APP_PATH, Application { on_activate: Box::new(on_activate) }).is_err() {
        return Claim::NoBus;
    }
    let Ok(name) = WellKnownName::try_from(APP_ID) else { return Claim::NoBus };
    use zbus::fdo::RequestNameFlags::DoNotQueue;
    let try_own = || match conn.request_name_with_flags(name.clone(), DoNotQueue.into()) {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Step::Owned,
        // zbus maps "Exists" to NameTaken; both mean another instance owns the name.
        Ok(_) | Err(zbus::Error::NameTaken) => Step::Taken,
        Err(e) => {
            eprintln!("[apihub] cannot claim {APP_ID}: {e}");
            Step::Failed
        }
    };
    // ~20 s at worst: long enough for an exiting instance to release the name.
    match negotiate(8, try_own, || activate_existing(&conn), || std::thread::sleep(std::time::Duration::from_millis(500))) {
        Ok(true) => Claim::Primary(conn),
        Ok(false) => Claim::Existing,
        Err(e) if conn.unique_name().is_some() && e != "cannot request the name" => {
            eprintln!("[apihub] existing instance unreachable ({e}): not opening a second window");
            Claim::Unreachable
        }
        Err(_) => Claim::NoBus,
    }
}

/// `Activate` on the running instance, bounded in time: a hung owner must
/// not block this launch for the bus' 25 s reply timeout at every attempt.
fn activate_existing(conn: &Connection) -> Result<(), String> {
    // Forward the token we were started with, so the running window may take focus.
    let token = std::env::var("XDG_ACTIVATION_TOKEN").ok().filter(|t| !t.is_empty());
    let conn = conn.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("activate".into()).spawn(move || {
        let mut args: HashMap<&str, Value<'_>> = HashMap::new();
        if let Some(t) = &token {
            args.insert("activation-token", Value::from(t.as_str()));
        }
        let r = conn
            .call_method(Some(APP_ID), APP_PATH, Some("org.freedesktop.Application"), "Activate", &(args,))
            .map(|_| ())
            .map_err(|e| e.to_string());
        let _ = tx.send(r);
    });
    if let Err(e) = spawned {
        return Err(e.to_string());
    }
    rx.recv_timeout(ACTIVATE_TIMEOUT).unwrap_or_else(|_| Err("no answer to Activate".into()))
}

const ACTIVATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Bus name of a registered SNI item (`"<name>/<path>"`, or just a name).
fn item_name(item: &str) -> &str {
    item.split('/').next().unwrap_or("")
}

/// Is one of the registered SNI items served by process `daemon_pid`?
/// The daemon publishes its item on a **second** connection (#196), so the
/// owner's unique name never equals the daemon's: compare process ids.
/// `pid_of` maps any bus name (unique or well-known) to its owner's pid.
pub fn items_of_pid(items: &[String], daemon_pid: u32, pid_of: impl Fn(&str) -> Option<u32>) -> bool {
    items.iter().any(|item| {
        let name = item_name(item);
        !name.is_empty() && pid_of(name) == Some(daemon_pid)
    })
}

/// True when the daemon is on the bus AND has registered its own tray item
/// with the StatusNotifierWatcher.
/// Longest wait for the SNI watcher before opening the window (#233).
pub const TRAY_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Run `f` on a worker thread and wait for it at most `timeout`; `default`
/// when it does not answer in time (zbus 4.4 has no client-side call
/// timeout: a peer that never replies blocks the caller for ever).
pub fn bounded<T: Send + 'static>(timeout: std::time::Duration, default: T, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    if std::thread::Builder::new().name("bounded-call".into()).spawn(move || { let _ = tx.send(f()); }).is_err() {
        return default;
    }
    rx.recv_timeout(timeout).unwrap_or(default)
}

/// True when the daemon's tray icon is registered; false when unknown,
/// including a StatusNotifierWatcher that does not answer within
/// [`TRAY_PROBE_TIMEOUT`] (a frozen plasmashell must not keep the window from
/// opening, #233).
pub fn daemon_tray_present() -> bool {
    let r = bounded(TRAY_PROBE_TIMEOUT, None, || Some(daemon_tray_present_blocking()));
    r.unwrap_or_else(|| {
        eprintln!("[apihub] StatusNotifierWatcher did not answer within {} s", TRAY_PROBE_TIMEOUT.as_secs());
        false
    })
}

fn daemon_tray_present_blocking() -> bool {
    let Ok(conn) = Connection::session() else { return false };
    let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return false };
    let pid_of = |n: &str| -> Option<u32> {
        let name = zbus::names::BusName::try_from(n).ok()?;
        dbus.get_connection_unix_process_id(name).ok()
    };
    let Some(daemon) = pid_of(DAEMON_NAME) else { return false };
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
    items_of_pid(&items, daemon, pid_of)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_call_gives_up_on_a_peer_that_never_answers() {
        let t = std::time::Instant::now();
        let r = bounded(std::time::Duration::from_millis(200), false, || {
            std::thread::sleep(std::time::Duration::from_secs(3600));
            true
        });
        assert!(!r);
        assert!(t.elapsed() < std::time::Duration::from_secs(1));
        assert!(bounded(std::time::Duration::from_secs(5), false, || true));
    }

    #[test]
    fn token_from_platform_data() {
        let mut pd: HashMap<String, OwnedValue> = HashMap::new();
        assert_eq!(activation_token(&pd), None);
        pd.insert("desktop-startup-id".into(), Value::from("legacy").try_into().unwrap());
        assert_eq!(activation_token(&pd).as_deref(), Some("legacy"));
        pd.insert("activation-token".into(), Value::from("tok").try_into().unwrap());
        assert_eq!(activation_token(&pd).as_deref(), Some("tok"));
        pd.insert("activation-token".into(), Value::from("").try_into().unwrap());
        assert_eq!(activation_token(&pd).as_deref(), Some("legacy"));
    }

    #[test]
    fn detects_daemon_item_by_pid_on_a_second_connection() {
        // Measured on the real bus: daemon name on :1.311, its SNI item on
        // :1.312, same process 497861.
        let items = vec![
            "org.kde.StatusNotifierItem-4588-1/StatusNotifierItem".to_string(),
            ":1.56/StatusNotifierItem".to_string(),
            "org.kde.StatusNotifierItem-497861-1/StatusNotifierItem".to_string(),
        ];
        let pid_of = |n: &str| match n {
            "org.kde.StatusNotifierItem-4588-1" => Some(4588),
            ":1.56" => Some(1200),
            "org.kde.StatusNotifierItem-497861-1" => Some(497_861),
            _ => None,
        };
        assert!(items_of_pid(&items, 497_861, pid_of));
        assert!(items_of_pid(&items, 1200, pid_of));
        assert!(!items_of_pid(&items, 42, pid_of));
        assert!(!items_of_pid(&[], 497_861, pid_of));
        assert!(!items_of_pid(&["/StatusNotifierItem".to_string()], 0, |_| Some(0)));
    }

    #[test]
    fn negotiate_owns_activates_or_refuses() {
        // Free name: we own it.
        assert_eq!(negotiate(3, || Step::Owned, || unreachable!(), || {}), Ok(true));
        // Live owner answering: raise it.
        assert_eq!(negotiate(3, || Step::Taken, || Ok(()), || {}), Ok(false));
        // Owner dying during Activate (NoReply), name freed on the 3rd try.
        let mut n = 0;
        let r = negotiate(5, || { n += 1; if n < 3 { Step::Taken } else { Step::Owned } }, || Err("NoReply".into()), || {});
        assert_eq!(r, Ok(true));
        // Owner alive but never answering: error, never "open anyway".
        let mut pauses = 0;
        let r = negotiate(4, || Step::Taken, || Err("NoReply".into()), || pauses += 1);
        assert_eq!(r, Err("NoReply".to_string()));
        assert_eq!(pauses, 3);
        assert!(negotiate(4, || Step::Failed, || Ok(()), || {}).is_err());
    }
}
