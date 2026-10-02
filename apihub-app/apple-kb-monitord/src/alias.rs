//! Rename the keyboard on this computer (#141): BlueZ `Device1.Alias`.
//!
//! The alias is a property BlueZ persists next to the pairing data; changing
//! it neither disconnects nor re-pairs the keyboard, and nothing is written
//! into the keyboard (its own stored name is untouched, see
//! docs/RENOMMER-CLAVIER.md). An empty alias restores the keyboard's own name.
//! The write goes through a [`AliasBackend`], replaceable in tests.

use std::sync::{Arc, Mutex};

use akm_core::alias;
use akm_core::model::is_apple_modalias;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

use crate::actor::{Mailbox, Msg};
use crate::settings::SetError;

const BLUEZ: &str = "org.bluez";
const DEVICE1: &str = "org.bluez.Device1";

/// Reads and writes the alias of a keyboard.
pub trait AliasBackend: Send + Sync + std::fmt::Debug {
    /// Current alias (BlueZ returns the keyboard's own name when none was set).
    fn get(&self, mac: &str) -> Option<String>;
    /// Apply an alias already validated; `""` resets it.
    fn set(&self, mac: &str, alias: &str) -> Result<(), SetError>;
    /// Remember the alias now in effect and who asked for it, for the
    /// self-test (`alias.json`). Only the real backend keeps it: test
    /// backends never touch the user's state directory.
    fn remember(&self, _mac: &str, _alias: &str, _caller: &str) {}
}

/// Real backend: BlueZ on the system bus.
#[derive(Debug, Default)]
pub struct BluezAlias {
    conn: Mutex<Option<Connection>>,
}

impl BluezAlias {
    fn connection(&self) -> zbus::Result<Connection> {
        let mut g = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            *g = Some(Connection::system()?);
        }
        Ok(g.clone().expect("just set"))
    }

    /// Drop the cached connection after a bus error (bluetoothd/dbus restart).
    fn forget(&self) {
        *self.conn.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Object path of the paired *Apple* device with this MAC.
    fn find(conn: &Connection, mac: &str) -> zbus::Result<Option<String>> {
        let om = zbus::blocking::fdo::ObjectManagerProxy::builder(conn)
            .destination(BLUEZ)?
            .path("/")?
            .build()?;
        for (path, ifaces) in om.get_managed_objects()? {
            let Some(d) = ifaces.get(DEVICE1) else {
                continue;
            };
            let s = |k: &str| {
                d.get(k)
                    .and_then(|v| <&str>::try_from(v).ok().map(str::to_string))
            };
            if s("Address").is_some_and(|a| a.eq_ignore_ascii_case(mac))
                && s("Modalias").is_some_and(|m| is_apple_modalias(&m))
            {
                return Ok(Some(path.to_string()));
            }
        }
        Ok(None)
    }

    fn try_get(&self, mac: &str) -> zbus::Result<Option<String>> {
        let conn = self.connection()?;
        let Some(path) = Self::find(&conn, mac)? else {
            return Ok(None);
        };
        let v: OwnedValue = conn
            .call_method(
                Some(BLUEZ),
                path.as_str(),
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(DEVICE1, "Alias"),
            )?
            .body()
            .deserialize()?;
        Ok(<&str>::try_from(&v).ok().map(str::to_string))
    }
}

impl AliasBackend for BluezAlias {
    fn get(&self, mac: &str) -> Option<String> {
        match self.try_get(mac) {
            Ok(a) => a,
            Err(e) => {
                tracing::debug!("alias: cannot read: {e}");
                self.forget();
                None
            }
        }
    }

    fn set(&self, mac: &str, alias: &str) -> Result<(), SetError> {
        let fail = |e: zbus::Error| SetError::Failed(format!("BlueZ: {e}"));
        let conn = self.connection().map_err(fail)?;
        let path = Self::find(&conn, mac)
            .map_err(|e| {
                self.forget();
                fail(e)
            })?
            .ok_or_else(|| SetError::Invalid(format!("no paired Apple device {mac} in BlueZ")))?;
        conn.call_method(
            Some(BLUEZ),
            path.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Set",
            &(DEVICE1, "Alias", Value::from(alias)),
        )
        .map_err(fail)?;
        Ok(())
    }

    fn remember(&self, mac: &str, alias: &str, caller: &str) {
        let path = alias::AliasMemory::default_path();
        let mut m = alias::AliasMemory::load(&path);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        m.remember(mac, alias, caller, now);
        if let Err(e) = m.save(&path) {
            tracing::warn!("alias: cannot save {}: {e}", path.display());
        }
    }
}

/// Validate, write, and tell the acquisition thread so the new name is
/// published at once. Returns the alias now in effect (`None` = unknown).
/// `caller` says who asked (D-Bus sender, pid and program, "tray", ...): it
/// is logged with every write and remembered with the alias, so that an
/// alias changed behind the user's back can be traced (`akmctl selftest`).
pub fn rename(
    backend: &dyn AliasBackend,
    mailbox: &Mailbox,
    mac: &str,
    requested: &str,
    caller: &str,
) -> Result<Option<String>, SetError> {
    if crate::devices::device_path(mac).is_none() {
        return Err(SetError::Invalid(format!("invalid MAC address {mac:?}")));
    }
    let name = alias::validate(requested).map_err(|e| SetError::Invalid(e.to_string()))?;
    let before = backend.get(mac);
    tracing::info!(
        mac,
        before = ?before,
        requested = %name,
        caller,
        "BlueZ alias write requested (SetAlias)"
    );
    backend.set(mac, &name)?;
    let now = backend.get(mac);
    tracing::info!(mac, alias = ?now, caller, "BlueZ alias written");
    if let Some(n) = now.as_deref() {
        backend.remember(mac, n, caller);
    }
    mailbox.send(Msg::Alias(mac.to_ascii_uppercase(), now.clone()));
    Ok(now)
}

/// Shared handle type used by the service and the tray.
pub type SharedBackend = Arc<dyn AliasBackend>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::mpsc;

    #[derive(Debug, Default)]
    pub struct Fake(pub Mutex<HashMap<String, String>>);
    impl AliasBackend for Fake {
        fn get(&self, mac: &str) -> Option<String> {
            self.0.lock().unwrap().get(&mac.to_ascii_uppercase()).cloned()
        }
        fn set(&self, mac: &str, alias: &str) -> Result<(), SetError> {
            let mut m = self.0.lock().unwrap();
            let k = mac.to_ascii_uppercase();
            if alias.is_empty() {
                m.insert(k, "Native name".into()); // BlueZ: reset = remote name
            } else {
                m.insert(k, alias.into());
            }
            Ok(())
        }
    }

    fn mailbox() -> (Arc<Mailbox>, mpsc::Receiver<Msg>) {
        let mb = Mailbox::new();
        let (tx, rx) = mpsc::channel();
        mb.install(tx);
        (mb, rx)
    }

    const MAC: &str = "AA:BB:CC:DD:EE:F1";

    #[test]
    fn rename_validates_writes_and_notifies() {
        let f = Fake::default();
        let (mb, rx) = mailbox();
        let r = rename(&f, &mb, "aa:bb:cc:dd:ee:f1", "  Bureau  ", "test").unwrap();
        assert_eq!(r.as_deref(), Some("Bureau"));
        assert_eq!(
            rx.try_recv().unwrap(),
            Msg::Alias(MAC.into(), Some("Bureau".into()))
        );
    }

    #[test]
    fn empty_resets_to_the_native_name() {
        let f = Fake::default();
        let (mb, rx) = mailbox();
        rename(&f, &mb, MAC, "Bureau", "test").unwrap();
        let r = rename(&f, &mb, MAC, "   ", "test").unwrap();
        assert_eq!(r.as_deref(), Some("Native name"));
        assert_eq!(rx.try_iter().count(), 2);
    }

    #[test]
    fn invalid_input_never_reaches_the_backend() {
        let f = Fake::default();
        let (mb, rx) = mailbox();
        for bad in ["a\nb", "x\u{202E}y", &"z".repeat(65)] {
            assert!(
                matches!(rename(&f, &mb, MAC, bad, "test"), Err(SetError::Invalid(_))),
                "{bad:?}"
            );
        }
        assert!(matches!(
            rename(&f, &mb, "not-a-mac", "ok", "test"),
            Err(SetError::Invalid(_))
        ));
        assert!(f.0.lock().unwrap().is_empty());
        assert!(rx.try_recv().is_err());
    }

    /// Every write names its caller and remembers the alias in effect (the
    /// own name after a reset); a refused input remembers nothing. Spy
    /// backend only: the real alias is never touched.
    #[test]
    fn rename_remembers_the_alias_and_its_caller() {
        #[derive(Debug, Default)]
        struct Spy(Fake, Mutex<Vec<(String, String, String)>>);
        impl AliasBackend for Spy {
            fn get(&self, mac: &str) -> Option<String> {
                self.0.get(mac)
            }
            fn set(&self, mac: &str, alias: &str) -> Result<(), SetError> {
                self.0.set(mac, alias)
            }
            fn remember(&self, mac: &str, alias: &str, caller: &str) {
                self.1
                    .lock()
                    .unwrap()
                    .push((mac.into(), alias.into(), caller.into()));
            }
        }
        let s = Spy::default();
        let mb = Mailbox::default();
        rename(&s, &mb, MAC, "Bureau", ":1.42 pid 77 (plasmashell)").unwrap();
        rename(&s, &mb, MAC, "", "tray").unwrap();
        assert!(rename(&s, &mb, MAC, "a\nb", "tray").is_err());
        assert_eq!(
            *s.1.lock().unwrap(),
            [
                (
                    MAC.into(),
                    "Bureau".into(),
                    ":1.42 pid 77 (plasmashell)".into()
                ),
                (MAC.into(), "Native name".into(), "tray".into()),
            ]
        );
        // The trait's default keeps nothing (test backends, fakes).
        Fake::default().remember(MAC, "x", "y");
    }

    #[test]
    fn works_without_a_running_actor() {
        let f = Fake::default();
        assert!(rename(&f, &Mailbox::default(), MAC, "Bureau", "test").is_ok());
    }
}
