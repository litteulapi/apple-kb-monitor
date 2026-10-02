//! #104 end to end on a private bus playing the system bus (fake `org.bluez`)
//! and the session bus (fake notification server): "Disconnect" and
//! "Reconnect" call BlueZ, "Forget" removes the pairing only after the
//! button of its confirmation notification was pressed, by the notification
//! server itself. No real BlueZ, no real desktop, no keyboard.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::{forget, repair};
use zbus::blocking::{Connection, ConnectionBuilder};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const INNER: &str = "AKM_TRAY_BT_INNER";
const WAIT: Duration = Duration::from_secs(5);
const SHORT: Duration = Duration::from_millis(700);
const MAC: &str = "AA:BB:CC:DD:EE:F1";
const PATH: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_F1";
const ADAPTER: &str = "/org/bluez/hci0";

static CONNECTS: AtomicUsize = AtomicUsize::new(0);
static DISCONNECTS: AtomicUsize = AtomicUsize::new(0);
static REMOVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Device1 {
    connected: bool,
}

#[zbus::interface(name = "org.bluez.Device1")]
impl Device1 {
    #[zbus(property)]
    fn address(&self) -> String {
        MAC.into()
    }
    #[zbus(property)]
    fn alias(&self) -> String {
        "Clavier de alice".into()
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
        true
    }
    fn connect(&self) {
        CONNECTS.fetch_add(1, Ordering::SeqCst);
    }
    fn disconnect(&self) {
        DISCONNECTS.fetch_add(1, Ordering::SeqCst);
    }
}

struct Adapter1;

#[zbus::interface(name = "org.bluez.Adapter1")]
impl Adapter1 {
    #[zbus(property)]
    fn powered(&self) -> bool {
        true
    }
    fn remove_device(&self, device: OwnedObjectPath) {
        REMOVED.lock().unwrap().push(device.to_string());
    }
}

#[derive(Debug)]
struct Call {
    id: u32,
    event: String,
    actions: Vec<String>,
}

struct Notifications {
    tx: Mutex<Sender<Call>>,
    next: u32,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &mut self,
        _app: String,
        _replaces: u32,
        _icon: String,
        _summary: String,
        _body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        _timeout: i32,
    ) -> u32 {
        self.next += 1;
        let event = hints
            .get("x-kde-eventId")
            .and_then(|v| <&str>::try_from(v).ok())
            .unwrap_or_default()
            .to_string();
        let _ = self.tx.lock().unwrap().send(Call {
            id: self.next,
            event,
            actions,
        });
        self.next
    }
}

fn invoke(from: &Connection, id: u32, key: &str) {
    from.emit_signal(
        None::<&str>,
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "ActionInvoked",
        &(id, key),
    )
    .unwrap();
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + WAIT;
    while !f() {
        assert!(Instant::now() < end, "timeout: {what}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn removed() -> Vec<String> {
    REMOVED.lock().unwrap().clone()
}

fn inner() {
    // Both buses of this process are the private one (testbus::rerun).
    assert_eq!(
        std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap(),
        std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap()
    );
    let (ntx, notes) = mpsc::channel();
    let server = ConnectionBuilder::session()
        .unwrap()
        .name("org.freedesktop.Notifications")
        .unwrap()
        .serve_at(
            "/org/freedesktop/Notifications",
            Notifications {
                tx: Mutex::new(ntx),
                next: 0,
            },
        )
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
    bluez.object_server().at(ADAPTER, Adapter1).unwrap();
    bluez
        .object_server()
        .at(PATH, Device1 { connected: true })
        .unwrap();

    let handle = repair::spawn(Mailbox::new(), true);
    let status = |h: &repair::KeeperHandle| h.shared.lock().unwrap().clone();
    wait_until("keyboard followed", || !status(&handle).is_empty());
    assert_eq!(status(&handle)[0].health, "connected");

    // The Link object, as clients see it.
    let daemon = Connection::session().unwrap();
    repair::export(&daemon, handle.clone()).unwrap();
    daemon
        .request_name("com.agenceapi.AppleKbMonitor1")
        .unwrap();
    let client = Connection::session().unwrap();
    let link = |member: &str, arg: &str| -> bool {
        client
            .call_method(
                Some("com.agenceapi.AppleKbMonitor1"),
                repair::LINK_PATH,
                Some(repair::LINK_INTERFACE),
                member,
                &(arg,),
            )
            .unwrap()
            .body()
            .deserialize()
            .unwrap()
    };

    // ── Forget: asked, never done in one click ─────────────────────────────
    assert!(
        forget::request(MAC, "Clavier de alice"),
        "what the tray entry does"
    );
    let ask = notes.recv_timeout(WAIT).expect("confirmation notification");
    assert_eq!(ask.event, "ForgetConfirm");
    assert_eq!(
        ask.actions,
        ["forget", "Oublier"],
        "one button, no default action"
    );
    std::thread::sleep(SHORT);
    assert!(removed().is_empty(), "removed without confirmation");
    // A click on the body, a button it does not offer, a forged signal: nothing.
    invoke(&server, ask.id, "default");
    invoke(&server, ask.id, "open");
    let rogue = Connection::session().unwrap();
    invoke(&rogue, ask.id, "forget");
    std::thread::sleep(SHORT);
    assert!(removed().is_empty(), "removed without the real button");
    assert_eq!(status(&handle).len(), 1);

    // Same request through D-Bus: only another confirmation.
    assert!(
        !link("RequestForget", "AA:BB:CC:DD:EE:02"),
        "unknown keyboard"
    );
    assert!(link("RequestForget", MAC));
    let ask2 = notes.recv_timeout(WAIT).expect("second confirmation");
    assert_eq!(ask2.event, "ForgetConfirm");
    std::thread::sleep(SHORT);
    assert!(removed().is_empty());

    // ── Disconnect / Reconnect call BlueZ ──────────────────────────────────
    assert!(
        repair::user_disconnect(Some(MAC)),
        "what the tray entry does"
    );
    wait_until("Device1.Disconnect", || {
        DISCONNECTS.load(Ordering::SeqCst) == 1
    });
    // BlueZ reports the keyboard disconnected.
    let iface = bluez.object_server().interface::<_, Device1>(PATH).unwrap();
    let ctx = iface.signal_context().clone();
    zbus::block_on(async {
        let mut d = iface.get_mut();
        d.connected = false;
        d.connected_changed(&ctx).await.unwrap();
    });
    wait_until("dormant", || status(&handle)[0].health == "dormant");
    // Disconnected on purpose: no page by itself.
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(CONNECTS.load(Ordering::SeqCst), 0);
    // "Reconnect": Device1.Connect, once.
    assert!(repair::user_reconnect());
    wait_until("Device1.Connect", || CONNECTS.load(Ordering::SeqCst) == 1);
    // Through D-Bus right after: accepted, but the 20 s spacing holds.
    assert!(client
        .call_method(
            Some("com.agenceapi.AppleKbMonitor1"),
            repair::LINK_PATH,
            Some(repair::LINK_INTERFACE),
            "Reconnect",
            &(),
        )
        .unwrap()
        .body()
        .deserialize::<bool>()
        .unwrap());
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        CONNECTS.load(Ordering::SeqCst),
        1,
        "paged twice within 20 s"
    );
    assert!(
        link("Disconnect", ""),
        "nothing connected: accepted, no call"
    );
    std::thread::sleep(SHORT);
    assert_eq!(DISCONNECTS.load(Ordering::SeqCst), 1);

    // ── The real button, from the server: BlueZ removes the device ─────────
    assert!(removed().is_empty());
    invoke(&server, ask2.id, "forget");
    wait_until("Adapter1.RemoveDevice", || removed() == [PATH]);
    // Once: pressing again (or the first notification) removes nothing more.
    invoke(&server, ask2.id, "forget");
    invoke(&server, ask.id, "forget");
    std::thread::sleep(SHORT);
    assert_eq!(removed(), [PATH]);
}

#[test]
fn bluetooth_actions_of_the_tray() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    apple_kb_monitord::testbus::rerun_test(
        "bluetooth_actions_of_the_tray",
        INNER,
        &[("LC_ALL", "fr_FR.UTF-8")],
    );
}
