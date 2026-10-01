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
use akm_core::model::{is_keyboard_device, upower_path_matches_mac};

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

fn prop_u32(p: &Props, k: &str) -> Option<u32> {
    p.get(k).and_then(|v| u32::try_from(v).ok())
}

/// A BlueZ device is followed only if it is one of our keyboards: modalias in
/// the model table and, when known, a keyboard Class of Device (#124). AirPods,
/// Magic Mouse / Trackpad and phones share Apple's vendor ID and must never
/// take the keyboard's place.
fn is_keyboard(p: &Props) -> bool {
    prop_str(p, "Modalias").is_some_and(|m| is_keyboard_device(&m, prop_u32(p, "Class")))
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
        if !is_keyboard(d) {
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
                    // Renamed from anywhere (akmctl, bluetoothctl, KDE settings).
                    if let (Some(alias), Some((mac, _))) =
                        (prop_str(&changed, "Alias"), known.0.get(&path))
                    {
                        if tx.send(Msg::Alias(mac.clone(), Some(alias))).is_err() {
                            return Ok(());
                        }
                    }
                    let Some(connected) = prop_bool(&changed, "Connected") else {
                        continue;
                    };
                    let entry = match known.0.get(&path) {
                        Some((m, _)) => Some(m.clone()),
                        None => device_props(&calls, &path).ok().and_then(|p| {
                            is_keyboard(&p)
                                .then(|| prop_str(&p, "Address"))
                                .flatten()
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
                    && known
                        .0
                        .values()
                        .any(|(mac, connected)| *connected && upower_path_matches_mac(&path, mac))
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

// ── Disconnection asked by Apple's breaker (#251) ───────────────────────────

/// The only call the breaker makes: `org.bluez.Device1.Disconnect`, Linux's
/// equivalent of `SetHIDDriverReady(false)` -> bluetoothd disconnects
/// (`docs/RE-GHIDRA-KEXT.md` §2.2). The pairing is kept: never the adapter's
/// device removal.
pub const DISCONNECT_CALL: (&str, &str) = ("org.bluez.Device1", "Disconnect");

/// Ask BlueZ to disconnect the keyboard `mac`, waiting at most `timeout`
/// (the call goes on in its own thread if BlueZ is slower). Called once per
/// connection by the actor.
pub fn request_disconnect(mac: &str, timeout: Duration) -> Result<(), String> {
    bounded(timeout, mac.to_ascii_uppercase(), |mac| {
        let conn = Connection::system().map_err(|e| e.to_string())?;
        let dev = crate::repair::enumerate(&conn)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|d| d.mac == mac)
            .ok_or_else(|| format!("{mac} is unknown to BlueZ"))?;
        conn.call_method(
            Some("org.bluez"),
            dev.path.as_str(),
            Some(DISCONNECT_CALL.0),
            DISCONNECT_CALL.1,
            &(),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
}

/// Run `call(arg)` in a thread, wait at most `timeout` for its result.
fn bounded(
    timeout: Duration,
    arg: String,
    call: impl FnOnce(String) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("kb-disconnect".into())
        .spawn(move || {
            let _ = tx.send(call(arg));
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(timeout)
        .map_err(|_| format!("no answer from BlueZ within {} ms", timeout.as_millis()))?
}

#[cfg(test)]
mod disconnect_tests {
    use super::*;

    #[test]
    fn the_breaker_disconnects_and_never_removes_the_pairing() {
        assert_eq!(DISCONNECT_CALL, ("org.bluez.Device1", "Disconnect"));
        // No device removal anywhere in the actor or this watcher.
        let removal = concat!("Remove", "Device");
        for (name, src) in [("watcher.rs", include_str!("watcher.rs")), ("actor.rs", include_str!("actor.rs"))] {
            assert!(!src.contains(removal), "{name} names {removal}");
        }
    }

    #[test]
    fn the_call_is_bounded_in_time() {
        let t = std::time::Instant::now();
        let r = bounded(Duration::from_millis(50), "AA".into(), |_| {
            std::thread::sleep(Duration::from_millis(400));
            Ok(())
        });
        assert!(r.unwrap_err().contains("within 50 ms"));
        assert!(t.elapsed() < Duration::from_millis(300));
        assert_eq!(bounded(Duration::from_secs(1), "AA".into(), |m| if m == "AA" { Ok(()) } else { Err(m) }), Ok(()));
        assert_eq!(bounded(Duration::from_secs(1), "x".into(), Err), Err("x".into()));
    }
}
