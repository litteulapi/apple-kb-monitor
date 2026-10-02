//! #169: the FIRST connection of a keyboard after the daemon started with an
//! empty snapshot must be signalled on its device object (private session bus).

use std::sync::Arc;
use std::time::Duration;

use akm_core::{KbReport, Snapshot, Watch};
use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::service::{self, ServeOptions};
use zbus::blocking::{Connection, MessageIterator};
use zbus::MatchRule;

const INNER: &str = "AKM_FIRST_CONNECT_INNER";
const MAC: &str = "AA:BB:CC:DD:EE:F1";
const DEV: &str = "/com/agenceapi/AppleKbMonitor1/devices/AA_BB_CC_DD_EE_F1";

fn keyboard() -> Snapshot {
    let mut k = KbReport::default();
    k.battery.percentage_fine = Some(80.0);
    k.device.mac = Some(MAC.into());
    k.device.model = Some("Apple Wireless Keyboard (A1314, aluminum, ISO)".into());
    Snapshot {
        connected: true,
        keyboard: Some(k),
        last_update: 1_790_000_000,
        ..Default::default()
    }
}

fn inner() {
    let watch = Arc::new(Watch::new());
    let so = ServeOptions::new(watch.clone(), Mailbox::new(), None);
    let _server = service::serve_with(so).expect("serve");

    let sub = Connection::session().unwrap();
    zbus::blocking::fdo::DBusProxy::new(&sub)
        .unwrap()
        .add_match_rule(
            MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .path(DEV)
                .unwrap()
                .build(),
        )
        .unwrap();
    let it = MessageIterator::from(sub);
    std::thread::sleep(Duration::from_millis(200));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for m in it.flatten() {
            let member = m.header().member().map(|x| x.to_string()).unwrap_or_default();
            let _ = tx.send(member);
        }
    });

    watch.publish(keyboard());
    let mut seen = Vec::new();
    while let Ok(m) = rx.recv_timeout(Duration::from_secs(3)) {
        seen.push(m);
    }
    assert!(
        seen.iter().any(|m| m == "ConnectionChanged"),
        "no ConnectionChanged on first connect: {seen:?}"
    );
}

#[test]
fn first_connection_is_signalled() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    let Some((_bus, mut cmd)) = apple_kb_monitord::testbus::rerun() else {
        eprintln!("SKIP: dbus-daemon not installed");
        return;
    };
    let out = cmd
        .args(["--exact", "first_connection_is_signalled", "--nocapture"])
        .env(INNER, "1")
        .output()
        .expect("run on the private bus");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "inner run failed:\n{text}");
    assert!(text.contains("1 passed"), "inner test did not run:\n{text}");
}
