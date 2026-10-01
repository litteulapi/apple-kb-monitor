//! D-Bus integration test of `com.agenceapi.AppleKbMonitor1` on a PRIVATE
//! session bus (`dbus-run-session`), without keyboard and without touching
//! the user's session bus: the test re-executes itself under
//! `dbus-run-session` and the inner run talks only to that bus.

use std::process::Command;
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
    k.device.mac = Some("04:DB:56:CA:42:EE".into());
    k.device.model = Some("Apple Wireless Keyboard (A1314, aluminum, ISO)".into());
    Snapshot { connected: true, keyboard: Some(k), last_update: 1_700_000_000, ..Default::default() }
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

fn inner() {
    let addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("dbus-run-session sets the address");
    assert!(!addr.is_empty());

    // Server side: watch + mailbox + history in a temp dir, no actor (no hardware).
    let dir = std::env::temp_dir().join(format!("akm-dbus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let h = History::new(dir.join("h.jsonl"), FixedClock(1_000));
    h.append(91.0, Some(2.82)).unwrap();
    let h2 = History::new(dir.join("h.jsonl"), FixedClock(2_000));
    h2.append(90.0, None).unwrap();

    let watch = Arc::new(Watch::new());
    watch.publish(keyboard(90.0));
    let mailbox = Mailbox::new();
    let _server = service::serve(watch.clone(), mailbox.clone(), Some(Arc::new(History::new(dir.join("h.jsonl"), akm_core::history::SystemClock)))).expect("serve");

    // Second instance must be refused (single owner of the keyboard).
    let other = Connection::session().unwrap();
    let w2 = Arc::new(Watch::new());
    assert!(matches!(
        service::serve_on(&other, w2, Mailbox::new(), None),
        Err(service::ServeError::NameTaken)
    ));

    // Client side.
    let c = Connection::session().unwrap();
    assert!(client::daemon_present(&c));
    assert_eq!(i32::try_from(busctl_like_get(&c, "Battery")).unwrap(), 90);
    assert_eq!(f64::try_from(busctl_like_get(&c, "Voltage")).unwrap(), 2.81);
    assert_eq!(i32::try_from(busctl_like_get(&c, "Rssi")).unwrap(), -52);
    assert!(bool::try_from(busctl_like_get(&c, "Connected")).unwrap());
    assert!(String::try_from(busctl_like_get(&c, "Model")).unwrap().contains("A1314"));
    let snap = client::fetch_snapshot(&c).unwrap();
    assert_eq!(snap.battery_pct(), Some(90.0));
    let (s, src) = client::snapshot(false).unwrap();
    assert_eq!((s.battery_pct(), src), (Some(90.0), client::Source::Daemon));

    // History(since)
    assert_eq!(client::fetch_history(&c, 0).unwrap().len(), 2);
    let recent = client::fetch_history(&c, 1_500).unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].pct, 90.0);

    // Refresh() without a running actor is an explicit error...
    assert!(client::request_refresh(&c).is_err());
    // ...and reaches the actor once one is installed.
    let (tx, rx) = mpsc::channel();
    mailbox.install(tx);
    client::request_refresh(&c).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), Msg::Refresh);

    // Signals: subscribe, publish a change, expect PropertiesChanged(Battery)
    // and StateChanged with the new revision.
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
            let member = m.header().member().map(|x| x.to_string()).unwrap_or_default();
            if member == "PropertiesChanged" {
                let (iface, changed, _): (String, std::collections::HashMap<String, zbus::zvariant::OwnedValue>, Vec<String>) =
                    m.body().deserialize().unwrap();
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
    let got = done_rx.recv_timeout(Duration::from_secs(5)).expect("PropertiesChanged + StateChanged received");
    assert_eq!(got, rev);
    assert_eq!(i32::try_from(busctl_like_get(&c, "Battery")).unwrap(), 42);

    // Unknown battery is exposed as the documented sentinel -1.
    watch.publish(Snapshot::default());
    assert_eq!(i32::try_from(busctl_like_get(&c, "Battery")).unwrap(), -1);
    assert!(!bool::try_from(busctl_like_get(&c, "Connected")).unwrap());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn session_interface_on_private_bus() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    if Command::new("dbus-run-session").arg("--version").output().is_err() {
        eprintln!("SKIP: dbus-run-session not installed");
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let out = Command::new("dbus-run-session")
        .arg("--")
        .arg(exe)
        .args(["--exact", "session_interface_on_private_bus", "--nocapture", "--test-threads=1"])
        .env(INNER, "1")
        .output()
        .expect("run under dbus-run-session");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "inner run failed:\n{}", text.chars().rev().take(3000).collect::<String>().chars().rev().collect::<String>());
    assert!(text.contains("1 passed"), "inner test did not run:\n{text}");
}
