//! Passive input listening on a PRIVATE session bus.

use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use akm_core::passive::Config;
use akm_core::{KbReport, Snapshot, Watch};
use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::passive::{self, INPUT_INTERFACE};
use apple_kb_monitord::service::{self, ServeOptions};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::OwnedValue;
use zbus::MatchRule;

const INNER: &str = "AKM_PASSIVE_INNER";
const MAC: &str = "AA:BB:CC:DD:EE:F1";
const DEV: &str = "/com/agenceapi/AppleKbMonitor1/devices/AA_BB_CC_DD_EE_F1";

fn hex(s: &str) -> Vec<u8> {
    let c: String = s.split_whitespace().collect();
    (0..c.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&c[i..i + 2], 16).unwrap())
        .collect()
}

fn try_get(c: &Connection, prop: &str) -> Option<OwnedValue> {
    c.call_method(
        Some(service::BUS_NAME),
        DEV,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(INPUT_INTERFACE, prop),
    )
    .ok()?
    .body()
    .deserialize()
    .ok()
}

fn get(c: &Connection, prop: &str) -> OwnedValue {
    try_get(c, prop).unwrap_or_else(|| panic!("Get {prop}: unavailable"))
}

fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    panic!("timeout: {what}");
}

#[allow(clippy::too_many_lines)] // one end-to-end scenario, read top to bottom
fn inner() {
    let dir = std::env::temp_dir().join(format!("akm-passive-dbus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let node = dir.join("hidraw9");
    let cpath = std::ffi::CString::new(node.to_str().unwrap()).unwrap();
    // SAFETY: valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);

    let mut k = KbReport::default();
    k.device.mac = Some(MAC.into());
    k.device.model = Some("A1314".into());
    let watch = Arc::new(Watch::new());
    watch.publish(Snapshot {
        connected: true,
        keyboard: Some(k),
        last_update: 1_790_000_000,
        ..Default::default()
    });
    let opts = ServeOptions::new(watch.clone(), Mailbox::new(), None);
    let server = service::serve_with(&opts).expect("serve");

    let sub = Connection::session().unwrap();
    let proxy = zbus::blocking::fdo::DBusProxy::new(&sub).unwrap();
    proxy
        .add_match_rule(
            MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .path(DEV)
                .unwrap()
                .build(),
        )
        .unwrap();
    let it = MessageIterator::from(sub);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for m in it.flatten() {
            let h = m.header();
            if let Some(member) = h.member() {
                let _ = tx.send(member.to_string());
            }
        }
    });

    let cfg = Config {
        lock: dir.join("hid.lock"),
        retry: Duration::from_millis(50),
        poll_ms: 40,
    };
    let n2: PathBuf = node.clone();
    let _p = passive::start(server.clone(), watch, cfg, move || Some(n2.clone())).unwrap();

    let c = Connection::session().unwrap();
    until("listening", || {
        try_get(&c, "Listening")
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(false)
    });
    assert_eq!(i32::try_from(get(&c, "FnLock")).unwrap(), -1);
    assert_eq!(u64::try_from(get(&c, "WakeCount")).unwrap(), 0);

    let mut w = std::fs::OpenOptions::new().write(true).open(&node).unwrap();
    let send = |w: &mut std::fs::File, s: &str| {
        w.write_all(&hex(s)).unwrap();
        std::thread::sleep(Duration::from_millis(120)); // one report per read
    };
    send(&mut w, "05 02");
    until("FnLock", || i32::try_from(get(&c, "FnLock")).unwrap() == 2);
    send(&mut w, "13 03");
    until("WakeCount", || {
        u64::try_from(get(&c, "WakeCount")).unwrap() == 1
    });
    send(&mut w, "04 01");
    until("LastSleepEvent", || {
        u64::try_from(get(&c, "LastSleepEvent")).unwrap() > 0
    });
    send(&mut w, "11 08");
    until("EjectPressed", || {
        bool::try_from(get(&c, "EjectPressed")).unwrap()
    });
    send(&mut w, "11 00");
    until("Eject released", || {
        !bool::try_from(get(&c, "EjectPressed")).unwrap()
    });
    send(&mut w, "30 00");
    until("BatteryStatus", || {
        i32::try_from(get(&c, "BatteryStatus")).unwrap() == 0
    });
    assert!(!bool::try_from(get(&c, "PoweredOff")).unwrap());
    send(&mut w, "13 00");
    until("PoweredOff", || {
        bool::try_from(get(&c, "PoweredOff")).unwrap()
    });
    send(&mut w, "30 01");
    until("BatteryStatus low", || {
        i32::try_from(get(&c, "BatteryStatus")).unwrap() == 1
    });
    send(&mut w, "01 02 00 1a 1b 00 00 00 00");
    let json: String = String::try_from(get(&c, "State")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["fn_lock"], 2);
    assert_eq!(v["wake_count"], 1, "a switch-off is not a wake");
    assert_eq!(v["eject_count"], 1);
    assert_eq!(
        (v["powered_off"].as_bool(), v["keyboard_off_count"].as_u64()),
        (Some(true), Some(1))
    );
    assert_eq!(v["battery_state"], "low");
    assert!(!json.contains("1a") && v.get("keys").is_none());

    let mut seen = vec![];
    while let Ok(m) = rx.recv_timeout(Duration::from_millis(400)) {
        seen.push(m);
    }
    for want in [
        "Wake",
        "SleepEvent",
        "EjectChanged",
        "FnLockUpdated",
        "KeyboardOff",
        "BatteryAlert",
        "PropertiesChanged",
    ] {
        assert!(seen.iter().any(|m| m == want), "{want} in {seen:?}");
    }

    drop(w);
    until("not listening", || {
        !bool::try_from(get(&c, "Listening")).unwrap()
    });
    assert_eq!(i32::try_from(get(&c, "FnLock")).unwrap(), -1);
    assert_eq!(u64::try_from(get(&c, "WakeCount")).unwrap(), 1);
    assert!(
        bool::try_from(get(&c, "PoweredOff")).unwrap(),
        "the Off state is what the disconnection becomes"
    );
    until("listening again", || {
        bool::try_from(get(&c, "Listening")).unwrap()
    });
    let mut w = std::fs::OpenOptions::new().write(true).open(&node).unwrap();
    send(&mut w, "13 02");
    until("WakeCount 2", || {
        u64::try_from(get(&c, "WakeCount")).unwrap() == 2
    });
    assert!(
        !bool::try_from(get(&c, "PoweredOff")).unwrap(),
        "powered on again"
    );
    // A forget withdraws Input with Device; the next event of the keyboard exports it again.
    opts.events
        .publish(&apple_kb_monitord::events::DeviceEvent::Forgotten { mac: MAC.into() });
    until("Input withdrawn", || try_get(&c, "Listening").is_none());
    send(&mut w, "13 02");
    until("Input back", || {
        try_get(&c, "WakeCount").is_some_and(|v| u64::try_from(v).unwrap() == 3)
    });
    drop(w);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn passive_input_on_private_bus() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    apple_kb_monitord::testbus::rerun_test("passive_input_on_private_bus", INNER, &[]);
}
