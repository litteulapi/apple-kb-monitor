//! Event sources of the keyboard actor (#66).
//!
//! * `spawn_signal_watcher` subscribes to the system bus (BlueZ
//!   `Device1.PropertiesChanged`, `NameOwnerChanged` of `org.bluez`, UPower
//!   `Device.PropertiesChanged`) and forwards [`Event`]s on a channel. It never
//!   touches the keyboard itself.
//! * The pure scheduling logic ([`Machine`]) lives in `akm_core::machine`.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::actor::Msg;
pub use akm_core::machine::*;
use akm_core::model::{is_apple_modalias, is_keyboard_upower_path};

use zbus::blocking::{fdo::DBusProxy, Connection, MessageIterator};
use zbus::zvariant::OwnedValue;
use zbus::MatchRule;

// ── D-Bus signal watcher ────────────────────────────────────────────────────

type Props = HashMap<String, OwnedValue>;

fn prop_str(p: &Props, k: &str) -> Option<String> {
    p.get(k)
        .and_then(|v| <&str>::try_from(v).ok().map(str::to_string))
}

fn prop_bool(p: &Props, k: &str) -> Option<bool> {
    p.get(k).and_then(|v| bool::try_from(v).ok())
}

/// Spawn the watcher thread. It reconnects to the bus on failure (10 s) and
/// emits `NoBluez` once if BlueZ cannot be reached at all.
pub fn spawn_signal_watcher(tx: Sender<Msg>) {
    let _ = std::thread::Builder::new()
        .name("kb-watch".into())
        .spawn(move || {
            let mut told_no_bluez = false;
            loop {
                match watch_once(&tx) {
                    Ok(()) => return, // receiver gone
                    Err(e) => {
                        tracing::warn!("D-Bus error: {e} — retry in 10s");
                        if !told_no_bluez {
                            told_no_bluez = true;
                            if tx.send(Msg::Bus(Event::NoBluez)).is_err() {
                                return;
                            }
                        }
                        std::thread::sleep(Duration::from_secs(10));
                    }
                }
            }
        });
}

fn add_rules(conn: &Connection) -> zbus::Result<()> {
    let dbus = DBusProxy::new(conn)?;
    let props =
        |sender: &'static str, arg0: Option<&'static str>| -> zbus::Result<MatchRule<'static>> {
            let mut b = MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(sender)?
                .interface("org.freedesktop.DBus.Properties")?
                .member("PropertiesChanged")?;
            if let Some(a) = arg0 {
                b = b.arg(0, a)?;
            }
            Ok(b.build())
        };
    dbus.add_match_rule(props("org.bluez", Some("org.bluez.Device1"))?)?;
    dbus.add_match_rule(props("org.freedesktop.UPower", None)?)?;
    let owner = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, "org.bluez")?
        .build();
    dbus.add_match_rule(owner)?;
    Ok(())
}

/// Known BlueZ keyboards: object path -> MAC, plus last connected flag.
#[derive(Default)]
struct Known(HashMap<String, (String, bool)>);

fn device_props(conn: &Connection, path: &str) -> zbus::Result<Props> {
    conn.call_method(
        Some("org.bluez"),
        path,
        Some("org.freedesktop.DBus.Properties"),
        "GetAll",
        &("org.bluez.Device1",),
    )?
    .body()
    .deserialize()
}

/// Enumerate BlueZ devices and emit the current state of every Apple keyboard.
fn initial_sync(conn: &Connection, known: &mut Known, tx: &Sender<Msg>) -> zbus::Result<bool> {
    let om = zbus::blocking::fdo::ObjectManagerProxy::builder(conn)
        .destination("org.bluez")?
        .path("/")?
        .build()?;
    let mut sent = false;
    for (path, ifaces) in om.get_managed_objects()? {
        let Some(d) = ifaces
            .iter()
            .find(|(k, _)| k.as_str() == "org.bluez.Device1")
            .map(|(_, v)| v)
        else {
            continue;
        };
        if !prop_str(d, "Modalias").is_some_and(|m| is_apple_modalias(&m)) {
            continue;
        }
        let Some(mac) = prop_str(d, "Address").map(|a| a.to_ascii_uppercase()) else {
            continue;
        };
        let connected = prop_bool(d, "Connected").unwrap_or(false);
        known.0.insert(path.to_string(), (mac.clone(), connected));
        if connected {
            sent = true;
            if tx.send(Msg::Bus(Event::Connected(mac))).is_err() {
                return Ok(sent);
            }
        }
    }
    Ok(sent)
}

fn watch_once(tx: &Sender<Msg>) -> zbus::Result<()> {
    let conn = Connection::system()?;
    let calls = Connection::system()?; // separate connection for method calls
    add_rules(&conn)?;
    let mut known = Known::default();
    let mut it = MessageIterator::from(conn.clone());
    // Subscribe first, then enumerate: no event can fall in the gap.
    initial_sync(&calls, &mut known, tx)?;

    for msg in &mut it {
        let msg = msg?;
        let hdr = msg.header();
        let member = hdr
            .member()
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        let path = hdr
            .path()
            .map(|p| p.as_str().to_string())
            .unwrap_or_default();
        match member.as_str() {
            "NameOwnerChanged" => {
                // bluetoothd restarted: resync from scratch.
                let (_, _, new): (String, String, String) = msg.body().deserialize()?;
                let old: Vec<(String, bool)> =
                    known.0.drain().map(|(_, (mac, c))| (mac, c)).collect();
                for (mac, c) in old {
                    if c && tx.send(Msg::Bus(Event::Disconnected(mac))).is_err() {
                        return Ok(());
                    }
                }
                if !new.is_empty() {
                    std::thread::sleep(Duration::from_millis(500));
                    initial_sync(&calls, &mut known, tx)?;
                }
            }
            "PropertiesChanged" => {
                let sender_bluez = path.starts_with("/org/bluez/");
                let (iface, changed, _inv): (String, Props, Vec<String>) =
                    msg.body().deserialize()?;
                if sender_bluez && iface == "org.bluez.Device1" {
                    let Some(connected) = prop_bool(&changed, "Connected") else {
                        continue;
                    };
                    let entry = match known.0.get(&path) {
                        Some((m, _)) => Some(m.clone()),
                        None => device_props(&calls, &path).ok().and_then(|p| {
                            prop_str(&p, "Modalias")
                                .filter(|m| is_apple_modalias(m))
                                .and(prop_str(&p, "Address"))
                                .map(|a| a.to_ascii_uppercase())
                        }),
                    };
                    let Some(mac) = entry else { continue };
                    known.0.insert(path.clone(), (mac.clone(), connected));
                    let ev = if connected {
                        Event::Connected(mac)
                    } else {
                        Event::Disconnected(mac)
                    };
                    if tx.send(Msg::Bus(ev)).is_err() {
                        return Ok(());
                    }
                } else if path.starts_with("/org/freedesktop/UPower/devices/")
                    && is_keyboard_upower_path(&path)
                    && changed.contains_key("Percentage")
                    && tx.send(Msg::Bus(Event::BatterySignal)).is_err()
                {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
    Err(zbus::Error::Failure("D-Bus message stream ended".into()))
}
