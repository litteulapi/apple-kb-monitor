//! #252: the keyboard is forgotten from OUTSIDE the daemon (Plasma "Forget" =
//! `Adapter1.RemoveDevice`, `bluetoothctl remove`). BlueZ then emits
//! `Disconnected`, `Paired=false` and finally `InterfacesRemoved`. The daemon
//! must drop the ghost keyboard at once, never page it, never claim a
//! re-pairing is needed, and say once "removed from this computer".
//!
//! Private bus (`dbus-run-session`) playing both the system bus (via
//! `DBUS_SYSTEM_BUS_ADDRESS`) and the session bus, with a fake `org.bluez`
//! and a fake notification server. Nothing reaches the real desktop.

use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::time::Duration;

use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::repair;
use zbus::blocking::ConnectionBuilder;
use zbus::zvariant::{OwnedValue, Value};

const INNER: &str = "AKM_BLUEZ_FORGET_INNER";
const WAIT: Duration = Duration::from_secs(5);
const PATH: &str = "/org/bluez/hci0/dev_04_DB_56_CA_42_EE";

struct Device1 {
    connected: bool,
    paired: bool,
}

#[zbus::interface(name = "org.bluez.Device1")]
impl Device1 {
    #[zbus(property)]
    fn address(&self) -> String {
        "04:DB:56:CA:42:EE".into()
    }
    #[zbus(property)]
    fn alias(&self) -> String {
        "alex".into()
    }
    #[zbus(property)]
    fn modalias(&self) -> String {
        "usb:v05ACp0256d0000".into()
    }
    #[zbus(property)]
    fn class(&self) -> u32 {
        0x0005_0540
    }
    #[zbus(property)]
    fn connected(&self) -> bool {
        self.connected
    }
    #[zbus(property)]
    fn paired(&self) -> bool {
        self.paired
    }
    fn connect(&self) {
        CONNECTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

static CONNECTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct Notifications(Mutex<Sender<(String, HashMap<String, String>)>>);

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        _app: String,
        _replaces: u32,
        _icon: String,
        summary: String,
        _body: String,
        _actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        _timeout: i32,
    ) -> u32 {
        let h = hints
            .into_iter()
            .filter_map(|(k, v)| match &*v {
                Value::Str(s) => Some((k, s.to_string())),
                _ => None,
            })
            .collect();
        let _ = self.0.lock().unwrap().send((summary, h));
        7
    }
    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".into(), "body".into()]
    }
    fn get_server_information(&self) -> (String, String, String, String) {
        ("fake".into(), "t".into(), "1".into(), "1.2".into())
    }
}

fn inner() {
    // The "system" bus of the daemon is this private bus.
    let addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap();
    std::env::set_var("DBUS_SYSTEM_BUS_ADDRESS", &addr);

    let (ntx, notes) = mpsc::channel();
    let _server = ConnectionBuilder::session()
        .unwrap()
        .name("org.freedesktop.Notifications")
        .unwrap()
        .serve_at("/org/freedesktop/Notifications", Notifications(Mutex::new(ntx)))
        .unwrap()
        .build()
        .unwrap();

    let bluez = ConnectionBuilder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .build()
        .unwrap();
    bluez
        .object_server()
        .at(PATH, Device1 { connected: true, paired: true })
        .unwrap();

    let handle = repair::spawn(Mailbox::new(), true);
    let status = |h: &repair::KeeperHandle| h.shared.lock().unwrap().clone();
    let deadline = std::time::Instant::now() + WAIT;
    while status(&handle).is_empty() {
        assert!(std::time::Instant::now() < deadline, "keyboard never followed");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(status(&handle)[0].name, "alex");

    // Plasma "Forget": disconnect, Paired=false, then the object disappears.
    let iface = bluez.object_server().interface::<_, Device1>(PATH).unwrap();
    let ctx = iface.signal_context().clone();
    zbus::block_on(async {
        let mut d = iface.get_mut();
        d.connected = false;
        d.connected_changed(&ctx).await.unwrap();
        d.paired = false;
        d.paired_changed(&ctx).await.unwrap();
    });
    std::thread::sleep(Duration::from_millis(60));
    bluez.object_server().remove::<Device1, _>(PATH).unwrap();

    let deadline = std::time::Instant::now() + WAIT;
    while !status(&handle).is_empty() {
        assert!(std::time::Instant::now() < deadline, "ghost keyboard still listed");
        std::thread::sleep(Duration::from_millis(30));
    }
    let (summary, hints) = notes.recv_timeout(WAIT).expect("removal notice");
    assert_eq!(hints["x-kde-eventId"], "KeyboardRemoved");
    assert!(summary.contains("supprim") || summary.contains("removed"), "{summary}");
    // Wait past the bond-loss grace: nothing else is said, nothing is paged.
    assert!(
        notes.recv_timeout(repair::BOND_GRACE + Duration::from_millis(1500)).is_err(),
        "a second notification (re-pairing?) was raised"
    );
    assert_eq!(CONNECTS.load(std::sync::atomic::Ordering::SeqCst), 0, "paged a forgotten device");
    assert!(status(&handle).is_empty());

    // The device comes back (re-paired): InterfacesAdded -> followed again.
    bluez
        .object_server()
        .at(PATH, Device1 { connected: true, paired: true })
        .unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    while status(&handle).is_empty() {
        assert!(std::time::Instant::now() < deadline, "re-added keyboard not followed");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn forgetting_from_plasma_drops_the_keyboard_and_notifies_once() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    if Command::new("dbus-run-session").arg("--version").output().is_err() {
        eprintln!("SKIP: dbus-run-session not installed");
        return;
    }
    let out = Command::new("dbus-run-session")
        .arg("--")
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "forgetting_from_plasma_drops_the_keyboard_and_notifies_once", "--nocapture"])
        .env(INNER, "1")
        .env("LC_ALL", "fr_FR.UTF-8")
        .output()
        .expect("run under dbus-run-session");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "inner run failed:\n{text}");
    assert!(text.contains("1 passed"), "inner test did not run:\n{text}");
}
