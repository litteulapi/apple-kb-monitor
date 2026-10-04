//! Rename the keyboard on this computer: `BlueZ` `Device1.Alias`.

use std::sync::Mutex;

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
    fn get(&self, mac: &str) -> Option<String>;
    /// # Errors
    /// The reason `BlueZ` refused or failed the write.
    fn set(&self, mac: &str, alias: &str) -> Result<(), SetError>;
    fn remember(&self, _mac: &str, _alias: &str, _caller: &str) {}
}

/// Real backend: `BlueZ` on the system bus.
#[derive(Debug, Default)]
pub struct BluezAlias {
    conn: Mutex<Option<Connection>>,
}

impl BluezAlias {
    fn connection(&self) -> zbus::Result<Connection> {
        let mut g = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(c) = g.as_ref() {
            return Ok(c.clone());
        }
        let c = Connection::system()?;
        *g = Some(c.clone());
        Ok(c)
    }

    fn forget(&self) {
        *self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

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
        let fail = |e: zbus::Error| SetError::Failed(tr!("BlueZ: {e}", e = e));
        let conn = self.connection().map_err(fail)?;
        let path = Self::find(&conn, mac)
            .map_err(|e| {
                self.forget();
                fail(e)
            })?
            .ok_or_else(|| {
                SetError::Invalid(tr!("no paired Apple device {mac} in BlueZ", mac = mac))
            })?;
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

/// Longest wait for the alias read of the acquisition thread (zbus 4 has no per-call timeout).
pub const READ_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// `backend.get(mac)` on its own thread: `None` when it did not answer within [`READ_WAIT`].
pub fn get_bounded(backend: &std::sync::Arc<dyn AliasBackend>, mac: &str) -> Option<String> {
    let (b, m) = (std::sync::Arc::clone(backend), mac.to_owned());
    let got = crate::repair::with_timeout(READ_WAIT, move || b.get(&m));
    if got.is_none() {
        tracing::debug!("alias: BlueZ did not answer within {READ_WAIT:?}");
    }
    got.flatten()
}

/// Validate, write, and tell the acquisition thread so the new name is published at once.
///
/// # Errors
/// [`SetError`] when the name is invalid or `BlueZ` refuses the write.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{mpsc, Arc};

    #[derive(Debug, Default)]
    pub struct Fake(pub Mutex<HashMap<String, String>>);
    impl AliasBackend for Fake {
        fn get(&self, mac: &str) -> Option<String> {
            self.0
                .lock()
                .unwrap()
                .get(&mac.to_ascii_uppercase())
                .cloned()
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

    #[derive(Debug)]
    struct Stuck;
    impl AliasBackend for Stuck {
        fn get(&self, _: &str) -> Option<String> {
            std::thread::sleep(std::time::Duration::from_secs(30));
            Some("late".into())
        }
        fn set(&self, _: &str, _: &str) -> Result<(), SetError> {
            Ok(())
        }
    }

    #[test]
    fn a_silent_bluez_does_not_hold_the_alias_read() {
        let t = std::time::Instant::now();
        let stuck: Arc<dyn AliasBackend> = Arc::new(Stuck);
        assert_eq!(get_bounded(&stuck, "AA:BB:CC:DD:EE:F1"), None);
        assert!(t.elapsed() < READ_WAIT * 3, "{:?}", t.elapsed());
        let f = Fake::default();
        f.set("AA:BB:CC:DD:EE:F1", "Desk").unwrap();
        let ok: Arc<dyn AliasBackend> = Arc::new(f);
        assert_eq!(
            get_bounded(&ok, "aa:bb:cc:dd:ee:f1").as_deref(),
            Some("Desk")
        );
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
        let r = rename(&f, &mb, "aa:bb:cc:dd:ee:f1", "  Desk  ", "test").unwrap();
        assert_eq!(r.as_deref(), Some("Desk"));
        assert!(
            matches!(rx.try_recv().unwrap(), Msg::Alias(m, Some(a)) if m == MAC && a == "Desk")
        );
    }

    #[test]
    fn empty_resets_to_the_native_name() {
        let f = Fake::default();
        let (mb, rx) = mailbox();
        rename(&f, &mb, MAC, "Desk", "test").unwrap();
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
        rename(&s, &mb, MAC, "Desk", ":1.42 pid 77 (plasmashell)").unwrap();
        rename(&s, &mb, MAC, "", "tray").unwrap();
        assert!(rename(&s, &mb, MAC, "a\nb", "tray").is_err());
        assert_eq!(
            *s.1.lock().unwrap(),
            [
                (
                    MAC.into(),
                    "Desk".into(),
                    ":1.42 pid 77 (plasmashell)".into()
                ),
                (MAC.into(), "Native name".into(), "tray".into()),
            ]
        );
        Fake::default().remember(MAC, "x", "y");
    }

    #[test]
    fn works_without_a_running_actor() {
        let f = Fake::default();
        assert!(rename(&f, &Mailbox::default(), MAC, "Desk", "test").is_ok());
    }
}
