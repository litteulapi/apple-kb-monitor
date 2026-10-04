//! D-Bus integration test on a PRIVATE session bus.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use akm_core::history::{Clock, History};
use akm_core::{KbReport, Snapshot, Watch};
use apple_kb_monitord::actor::{Mailbox, Msg};
use apple_kb_monitord::{client, service};
use zbus::blocking::{Connection, MessageIterator};
use zbus::MatchRule;

const INNER: &str = "AKM_DBUS_TEST_INNER";

struct FixedClock(u64);
impl Clock for FixedClock {
    fn now(&self) -> u64 {
        self.0
    }
}

fn keyboard(pct: f64) -> Snapshot {
    let mut k = KbReport::default();
    k.battery.percentage_fine = Some(pct);
    k.battery.voltage = Some(2.81);
    k.radio.rssi_dbm = Some(-52);
    k.device.mac = Some("AA:BB:CC:DD:EE:F1".into());
    k.device.model = Some("Apple Wireless Keyboard (A1314, aluminum, ISO)".into());
    Snapshot {
        connected: true,
        keyboard: Some(k),
        last_update: 1_700_000_000,
        ..Default::default()
    }
}

fn busctl_like_get(conn: &Connection, prop: &str) -> zbus::zvariant::OwnedValue {
    conn.call_method(
        Some(service::BUS_NAME),
        service::OBJECT_PATH,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(service::INTERFACE, prop),
    )
    .unwrap()
    .body()
    .deserialize()
    .unwrap()
}

#[allow(clippy::too_many_lines)] // one end-to-end scenario, read top to bottom
#[allow(clippy::many_single_char_names)] // test fixtures with short local names
fn inner() {
    let addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("the private bus address is set");
    assert!(!addr.is_empty(), "{addr:?}");

    let dir = std::env::temp_dir().join(format!("akm-dbus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let h = History::new(dir.join("h.jsonl"), FixedClock(1_000));
    h.append(91.0, Some(2.82)).unwrap();
    let h2 = History::new(dir.join("h.jsonl"), FixedClock(2_000));
    h2.append(90.0, None).unwrap();

    let watch = Arc::new(Watch::new());
    watch.publish(keyboard(90.0));
    let mailbox = Mailbox::new();
    let _server = service::serve(
        watch.clone(),
        mailbox.clone(),
        Some(Arc::new(History::new(
            dir.join("h.jsonl"),
            akm_core::history::SystemClock,
        ))),
    )
    .expect("serve");

    // Second instance must be refused (single owner of the keyboard).
    let other = Connection::session().unwrap();
    let w2 = Arc::new(Watch::new());
    assert!(matches!(
        service::serve_on(&other, w2, Mailbox::new(), None),
        Err(service::ServeError::NameTaken)
    ));

    let c = Connection::session().unwrap();
    assert!(client::daemon_present(&c));
    assert_eq!(i32::try_from(busctl_like_get(&c, "Battery")).unwrap(), 90);
    let volts = f64::try_from(busctl_like_get(&c, "Voltage")).unwrap();
    assert!((volts - 2.81).abs() < 1e-9, "{volts}");
    assert_eq!(i32::try_from(busctl_like_get(&c, "Rssi")).unwrap(), -52);
    assert!(bool::try_from(busctl_like_get(&c, "Connected")).unwrap());
    assert!(String::try_from(busctl_like_get(&c, "Model"))
        .unwrap()
        .contains("A1314"));
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
    assert_eq!(
        serde_json::from_str::<Snapshot>(&state)
            .unwrap()
            .battery_pct(),
        Some(90.0)
    );
    let snap = client::fetch_snapshot(&c).unwrap();
    assert_eq!(snap.battery_pct(), Some(90.0));
    let (s, src) = client::snapshot(false).unwrap();
    assert_eq!((s.battery_pct(), src), (Some(90.0), client::Source::Daemon));

    assert_eq!(client::fetch_history(&c, 0).unwrap().len(), 2);
    let recent = client::fetch_history(&c, 1_500).unwrap();
    assert_eq!(recent.len(), 1);
    assert!((recent[0].pct - 90.0).abs() < 1e-9);

    assert!(client::request_refresh(&c).is_err());
    let (tx, rx) = mpsc::channel();
    mailbox.install(tx);
    let answer = std::thread::spawn(move || {
        for o in [
            akm_core::machine::RefreshOutcome::TooSoon {
                wait: Duration::from_mins(4),
            },
            akm_core::machine::RefreshOutcome::Accepted,
        ] {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Msg::Refresh(reply) => reply.answer(o),
                m => panic!("unexpected {m:?}"),
            }
        }
        rx
    });
    let e = client::request_refresh(&c).unwrap_err().to_string();
    assert!(e.contains("LimitsExceeded") && e.contains("240 s"), "{e}");
    client::request_refresh(&c).unwrap();
    let rx = answer.join().unwrap();

    // RereadName(): unanswered, the caller is told the request is queued, never an error nor
    // a silent "ok".
    let reply = c
        .call_method(
            Some(apple_kb_monitord::service::BUS_NAME),
            apple_kb_monitord::service::OBJECT_PATH,
            Some(apple_kb_monitord::service::INTERFACE),
            "RereadName",
            &(),
        )
        .unwrap();
    let (accepted, text): (bool, String) = reply.body().deserialize().unwrap();
    assert!(accepted && text.contains("queued"), "{text}");
    let msg = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(msg, Msg::RereadName(_)), "{msg:?}");
    let answering = std::thread::spawn(move || {
        if let Ok(Msg::RereadName(reply)) = rx.recv_timeout(Duration::from_secs(5)) {
            reply.answer(akm_core::machine::NameReread::NotConnected);
        }
        rx
    });
    let reply = c
        .call_method(
            Some(apple_kb_monitord::service::BUS_NAME),
            apple_kb_monitord::service::OBJECT_PATH,
            Some(apple_kb_monitord::service::INTERFACE),
            "RereadName",
            &(),
        )
        .unwrap();
    let (accepted, text): (bool, String) = reply.body().deserialize().unwrap();
    assert!(
        !accepted && text.contains("no keyboard connected"),
        "{text}"
    );
    drop(answering.join().unwrap());

    let sub = Connection::session().unwrap();
    let proxy = zbus::blocking::fdo::DBusProxy::new(&sub).unwrap();
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .path(service::OBJECT_PATH)
        .unwrap()
        .build();
    proxy.add_match_rule(rule).unwrap();
    let mut it = MessageIterator::from(sub.clone());
    std::thread::sleep(Duration::from_millis(100));
    let rev = watch.publish(keyboard(42.0));

    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut saw_battery = false;
        let mut saw_state = None;
        for m in &mut it {
            let m = m.unwrap();
            let member = m
                .header()
                .member()
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            if member == "PropertiesChanged" {
                let (iface, changed, _): (
                    String,
                    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
                    Vec<String>,
                ) = m.body().deserialize().unwrap();
                if iface == service::INTERFACE {
                    if let Some(v) = changed.get("Battery") {
                        assert_eq!(i32::try_from(v.try_clone().unwrap()).unwrap(), 42);
                        saw_battery = true;
                    }
                }
            } else if member == "StateChanged" {
                let (r, json): (u64, String) = m.body().deserialize().unwrap();
                let s: Snapshot = serde_json::from_str(&json).unwrap();
                assert_eq!(s.battery_pct(), Some(42.0));
                saw_state = Some(r);
            }
            if let (true, Some(r)) = (saw_battery, saw_state) {
                let _ = done_tx.send(r);
                return;
            }
        }
    });
    let got = done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("PropertiesChanged + StateChanged received");
    assert_eq!(got, rev);
    assert_eq!(i32::try_from(busctl_like_get(&c, "Battery")).unwrap(), 42);

    watch.publish(Snapshot::default());
    assert_eq!(i32::try_from(busctl_like_get(&c, "Battery")).unwrap(), -1);
    assert_eq!(
        i32::try_from(busctl_like_get(&c, "Rssi")).unwrap(),
        service::RSSI_UNKNOWN
    );
    assert!(!bool::try_from(busctl_like_get(&c, "Connected")).unwrap());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn session_interface_on_private_bus() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    apple_kb_monitord::testbus::rerun_test("session_interface_on_private_bus", INNER, &[]);
}
