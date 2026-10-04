//! Who sent a bus signal. Match rules only filter broadcasts: any peer may send a signal straight
//! to our unique name, so a listener keeps a signal only when it is a broadcast from the bus daemon
//! (`NameOwnerChanged`) or from the unique name currently owning a watched service.

use zbus::blocking::{fdo::DBusProxy, Connection};
use zbus::names::BusName;
use zbus::Message;

/// The bus daemon's own name, sender of `NameOwnerChanged`.
pub const BUS_DAEMON: &str = "org.freedesktop.DBus";

/// The owners of the watched well-known services.
#[derive(Debug, Clone)]
pub struct Origin {
    conn: Connection,
    owners: Vec<(&'static str, Option<String>)>,
}

impl Origin {
    /// Follow `services`, asking their owners on `conn` (a connection for method calls).
    #[must_use]
    pub fn new(conn: &Connection, services: &[&'static str]) -> Self {
        let mut o = Self {
            conn: conn.clone(),
            owners: services.iter().map(|s| (*s, None)).collect(),
        };
        o.refresh();
        o
    }

    fn refresh(&mut self) {
        let Ok(dbus) = DBusProxy::new(&self.conn) else {
            return;
        };
        for (service, owner) in &mut self.owners {
            *owner = BusName::try_from(*service)
                .ok()
                .and_then(|n| dbus.get_name_owner(n).ok())
                .map(|o| o.to_string());
        }
    }

    fn owned_by(&self, sender: &str) -> bool {
        self.owners
            .iter()
            .any(|(_, o)| o.as_deref() == Some(sender))
    }

    /// `true` for a genuine broadcast signal: `NameOwnerChanged` from the bus daemon, anything
    /// else from the current owner of a watched service.
    pub fn accept(&mut self, msg: &Message) -> bool {
        let hdr = msg.header();
        if hdr.destination().is_some() {
            return false;
        }
        let Some(sender) = hdr.sender().map(|s| s.as_str().to_owned()) else {
            return false;
        };
        if hdr.interface().is_some_and(|i| i.as_str() == BUS_DAEMON)
            && hdr
                .member()
                .is_some_and(|m| m.as_str() == "NameOwnerChanged")
        {
            if sender != BUS_DAEMON {
                return false;
            }
            if let Ok((name, _, new)) = msg.body().deserialize::<(String, String, String)>() {
                for (service, owner) in &mut self.owners {
                    if name == *service {
                        *owner = (!new.is_empty()).then(|| new.clone());
                    }
                }
            }
            return true;
        }
        if self.owned_by(&sender) {
            return true;
        }
        // A service restarted without a NameOwnerChanged we follow.
        self.refresh();
        self.owned_by(&sender)
    }
}
