//! Signals sent straight to the listener by another peer are dropped.

use std::sync::mpsc;
use std::time::Duration;

use apple_kb_monitord::repair::{listen_on, KMsg};
use apple_kb_monitord::testbus::{connect, private_bus};
use zbus::blocking::Connection;

const WAIT: Duration = Duration::from_secs(5);
const SPOOFED: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF";
const GENUINE: &str = "/org/bluez/hci0/dev_11_22_33_44_55_66";

fn removed(from: &Connection, to: Option<&str>, path: &str) {
    let path = zbus::zvariant::ObjectPath::try_from(path).unwrap();
    from.emit_signal(
        to,
        "/",
        "org.freedesktop.DBus.ObjectManager",
        "InterfacesRemoved",
        &(path, vec!["org.bluez.Device1"]),
    )
    .unwrap();
}

#[test]
fn unicast_signals_from_other_peers_are_rejected() {
    let Some(bus) = private_bus() else {
        return;
    };
    let bluez = connect(&bus.addr).unwrap();
    bluez
        .object_server()
        .at("/", zbus::fdo::ObjectManager)
        .unwrap();
    bluez.request_name("org.bluez").unwrap();

    let conn = connect(&bus.addr).unwrap();
    let calls = connect(&bus.addr).unwrap();
    let listener = conn.unique_name().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || listen_on(&conn, &calls, &tx));
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), KMsg::Sync(Vec::new()));

    let spoofer = connect(&bus.addr).unwrap();
    removed(&spoofer, Some(&listener), SPOOFED);
    spoofer
        .emit_signal(
            Some(listener.as_str()),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameOwnerChanged",
            &("org.bluez", ":1.1", ""),
        )
        .unwrap();
    // Even the owner of org.bluez is not trusted on a unicast signal.
    removed(&bluez, Some(&listener), SPOOFED);
    removed(&spoofer, None, SPOOFED);
    removed(&bluez, None, GENUINE);

    assert_eq!(
        rx.recv_timeout(WAIT).unwrap(),
        KMsg::Removed(GENUINE.into())
    );
    assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
}
