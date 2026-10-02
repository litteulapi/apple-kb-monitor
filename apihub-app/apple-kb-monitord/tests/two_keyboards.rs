//! #94 / #119 on a private bus with TWO simulated keyboards: each has its
//! D-Bus object, `GetState` lists both, the second one leaving and coming
//! back is signalled on its own object and changes nothing for the first.
//! API 1 clients (root properties) keep seeing the keyboard the daemon reads.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use akm_core::roster::DeviceSummary;
use akm_core::{KbReport, Snapshot, Watch};
use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::devices::{device_path, DEVICE_INTERFACE};
use apple_kb_monitord::service;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::MatchRule;

const INNER: &str = "AKM_TWO_KEYBOARDS_INNER";
const A: &str = "AA:BB:CC:DD:EE:F1";
const B: &str = "AA:BB:CC:DD:EE:F2";
const WAIT: Duration = Duration::from_secs(5);

fn snapshot(second: Option<(bool, f64)>) -> Snapshot {
    let mut k = KbReport::default();
    k.battery.percentage_fine = Some(80.0);
    k.device.mac = Some(A.into());
    k.device.alias = Some("Bureau".into());
    k.device.model = Some("Apple Wireless Keyboard (A1314, aluminum, ISO)".into());
    let mut devices = vec![DeviceSummary {
        mac: A.into(),
        name: "Bureau".into(),
        connected: true,
        battery: Some(80.0),
        primary: true,
    }];
    if let Some((connected, pct)) = second {
        devices.push(DeviceSummary {
            mac: B.into(),
            name: "Salon".into(),
            connected,
            battery: Some(pct),
            primary: false,
        });
    }
    Snapshot {
        connected: true,
        keyboard: Some(k),
        last_update: 1_790_000_000,
        devices,
        ..Default::default()
    }
}

fn get(c: &Connection, path: &str, iface: &str, prop: &str) -> OwnedValue {
    c.call_method(
        Some(service::BUS_NAME),
        path,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(iface, prop),
    )
    .unwrap()
    .body()
    .deserialize()
    .unwrap()
}

fn devices(c: &Connection) -> Vec<String> {
    let paths: Vec<OwnedObjectPath> = c
        .call_method(
            Some(service::BUS_NAME),
            service::OBJECT_PATH,
            Some(service::INTERFACE),
            "GetDevices",
            &(),
        )
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    paths.iter().map(|p| p.to_string()).collect()
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let end = std::time::Instant::now() + WAIT;
    while !f() {
        assert!(std::time::Instant::now() < end, "timeout: {what}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn inner() {
    let watch = Arc::new(Watch::new());
    watch.publish(snapshot(None));
    let _server = service::serve(watch.clone(), Mailbox::new(), None).expect("serve");
    let c = Connection::session().unwrap();
    let (pa, pb) = (
        device_path(A).unwrap().to_string(),
        device_path(B).unwrap().to_string(),
    );
    assert_eq!(devices(&c), [pa.clone()], "one keyboard at first");

    // Listen to the second keyboard's object before it exists.
    let sub = Connection::session().unwrap();
    zbus::blocking::fdo::DBusProxy::new(&sub)
        .unwrap()
        .add_match_rule(
            MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface(DEVICE_INTERFACE)
                .unwrap()
                .member("ConnectionChanged")
                .unwrap()
                .build(),
        )
        .unwrap();
    let (tx, changes) = mpsc::channel::<(String, bool, i32)>();
    let it = MessageIterator::from(sub);
    std::thread::spawn(move || {
        for m in it.flatten() {
            let path = m.header().path().map(|p| p.to_string()).unwrap_or_default();
            if let Ok((connected, battery)) = m.body().deserialize::<(bool, i32)>() {
                let _ = tx.send((path, connected, battery));
            }
        }
    });
    std::thread::sleep(Duration::from_millis(150));

    // 1. A second keyboard appears, at 12 %.
    watch.publish(snapshot(Some((true, 12.0))));
    wait_until("second object exported", || devices(&c).len() == 2);
    assert_eq!(devices(&c), [pa.clone(), pb.clone()]);
    let battery = |p: &str| i32::try_from(get(&c, p, DEVICE_INTERFACE, "Battery")).unwrap();
    let connected = |p: &str| bool::try_from(get(&c, p, DEVICE_INTERFACE, "Connected")).unwrap();
    let name = |p: &str| String::try_from(get(&c, p, DEVICE_INTERFACE, "Name")).unwrap();
    assert_eq!(
        (battery(&pb), connected(&pb), name(&pb).as_str()),
        (12, true, "Salon")
    );
    assert_eq!(
        (battery(&pa), connected(&pa), name(&pa).as_str()),
        (80, true, "Bureau")
    );
    assert_eq!(
        changes
            .recv_timeout(WAIT)
            .expect("the new keyboard is announced"),
        (pb.clone(), true, 12)
    );
    // GetState lists both, the keyboard read by the daemon first.
    let state: String = c
        .call_method(
            Some(service::BUS_NAME),
            service::OBJECT_PATH,
            Some(service::INTERFACE),
            "GetState",
            &(),
        )
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    let s: Snapshot = serde_json::from_str(&state).unwrap();
    assert_eq!(s.devices.len(), 2);
    assert!(s.devices[0].primary && !s.devices[1].primary);
    assert_eq!(akm_core::roster::weakest(&s.devices).unwrap().mac, B);
    // API 1: the root object still is the keyboard the daemon reads.
    let root = |prop: &str| get(&c, service::OBJECT_PATH, service::INTERFACE, prop);
    assert_eq!(i32::try_from(root("Battery")).unwrap(), 80);
    assert_eq!(String::try_from(root("Mac")).unwrap(), A);

    // 2. The second keyboard leaves: signalled on ITS object only.
    watch.publish(snapshot(Some((false, 12.0))));
    assert_eq!(
        changes.recv_timeout(WAIT).expect("removal"),
        (pb.clone(), false, 12)
    );
    assert!(!connected(&pb));
    assert_eq!(
        (battery(&pa), connected(&pa)),
        (80, true),
        "the first is unaffected"
    );
    assert!(
        changes.recv_timeout(Duration::from_millis(400)).is_err(),
        "nothing was signalled for the first keyboard"
    );
    assert_eq!(devices(&c).len(), 2, "its object stays, disconnected");

    // 3. It comes back, weaker.
    watch.publish(snapshot(Some((true, 9.0))));
    assert_eq!(
        changes.recv_timeout(WAIT).expect("reconnection"),
        (pb.clone(), true, 9)
    );
    assert_eq!((battery(&pb), connected(&pb)), (9, true));
    assert_eq!((battery(&pa), connected(&pa)), (80, true));
}

#[test]
fn two_keyboards_on_the_bus() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    apple_kb_monitord::testbus::rerun_test("two_keyboards_on_the_bus", INNER, &[]);
}
