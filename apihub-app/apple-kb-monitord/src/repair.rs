//! Link keeper: keeps the keyboard's Bluetooth link alive across link losses,
//! reboots and system sleep, and says clearly when a re-pairing is really
//! needed (#142, #144, #145). Also reconciles the acquisition machine with
//! BlueZ's view of the world (#165) and launches the guided repair (#147).
//!
//! Layout:
//! * [`Keeper`] — the decision core, generic over [`LinkBus`] (effects), one
//!   [`Recovery`] per paired keyboard. Tested without hardware with a fake bus.
//! * [`spawn`] — the real thing: a listener thread on the system bus (BlueZ
//!   `Device1` / `Adapter1` properties, `Device1.Disconnected(reason)`,
//!   `NameOwnerChanged(org.bluez)`), logind sleep events ([`crate::sleep`]),
//!   and the keeper thread that calls `Device1.Connect` asynchronously.
//! * [`LinkIface`] — session D-Bus object `/com/agenceapi/AppleKbMonitor1/Link`
//!   (`Status()` JSON, `Reconnect()`), read by `akmctl doctor` and the tray.
//!
//! The keeper never removes a pairing: it only connects, reports and
//! notifies. Removal is `akmctl repair`, after an explicit typed confirmation.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use akm_core::alerts::Urgency;
use akm_core::machine::Event;
use akm_core::model::is_keyboard_device;
use akm_core::recovery::{Action, ConnectError, DisconnectReason, Notice, Recovery};
use zbus::blocking::{fdo::DBusProxy, Connection, MessageIterator};
use zbus::zvariant::OwnedValue;
use zbus::MatchRule;

use crate::actor::{Mailbox, Msg};
use crate::sleep::SleepEvent;

pub const LINK_PATH: &str = "/com/agenceapi/AppleKbMonitor1/Link";
pub const LINK_INTERFACE: &str = "com.agenceapi.AppleKbMonitor1.Link";
/// Fresh BlueZ enumeration pushed to the acquisition machine this often.
pub const RECONCILE_PERIOD: Duration = Duration::from_secs(5 * 60);
/// Longest idle wait of the keeper loop.
const IDLE_WAIT: Duration = Duration::from_secs(60);

// ── Facts and effects ──────────────────────────────────────────────────────

/// A keyboard as BlueZ describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevInfo {
    pub path: String,
    pub mac: String,
    pub name: String,
    pub connected: bool,
    pub paired: bool,
}

/// Messages to the keeper.
#[derive(Debug, Clone, PartialEq)]
pub enum KMsg {
    /// Full enumeration (start, BlueZ back, listener resubscribed).
    Sync(Vec<DevInfo>),
    /// `org.bluez` left the bus.
    BluezGone,
    Connected(String, bool),
    Paired(String, bool),
    Disconnected(String, DisconnectReason),
    Adapter(bool),
    Sleep(SleepEvent),
    /// Result of an asynchronous `Device1.Connect` (by MAC).
    ConnectDone(String, Result<(), ConnectError>),
    /// User asked for an immediate attempt (tray, akmctl).
    Request,
    Quit,
}

/// Side effects of the keeper (real buses, or a fake in tests).
pub trait LinkBus {
    /// Start `Device1.Connect` on `path`; the result must come back as
    /// [`KMsg::ConnectDone`]. Must not block.
    fn connect(&mut self, path: &str, mac: &str);
    fn notify(&mut self, summary: &str, body: &str, urgency: Urgency);
    /// Same, knowing which notice it is (KDE event, buttons); defaults to
    /// the plain notification.
    fn notify_notice(&mut self, _notice: &Notice, summary: &str, body: &str, urgency: Urgency) {
        self.notify(summary, body, urgency);
    }
    /// Tell the acquisition machine which keyboards are really connected.
    fn reconcile(&mut self, connected: Vec<String>);
    /// Fresh enumeration; `None` if BlueZ cannot be reached.
    fn enumerate(&mut self) -> Option<Vec<DevInfo>>;
}

/// Published per keyboard (D-Bus `Status()`, `akmctl doctor`).
#[derive(Debug, Clone, PartialEq)]
pub struct LinkStatus {
    pub mac: String,
    pub name: String,
    pub health: String,
    /// Unix time the current episode started (0 = connected).
    pub since: u64,
    pub attempts: u32,
    pub failures: u32,
    pub last_error: String,
    pub last_reason: String,
    /// Unix time of the status.
    pub updated: u64,
}

impl LinkStatus {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "mac": self.mac,
            "name": self.name,
            "health": self.health,
            "since": self.since,
            "attempts": self.attempts,
            "failures": self.failures,
            "last_error": self.last_error,
            "last_reason": self.last_reason,
            "updated": self.updated,
        })
    }
}

pub type SharedStatus = Arc<Mutex<Vec<LinkStatus>>>;

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn french() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .is_some_and(|v| v.to_ascii_lowercase().starts_with("fr"))
}

/// Notification text for a notice (summary, body, urgency).
pub fn notice_text(n: &Notice, name: &str, now: Instant, fr: bool) -> (String, String, Urgency) {
    match n {
        Notice::Unreachable { since } => {
            let min = now.saturating_duration_since(*since).as_secs() / 60;
            if fr {
                (
                    "Clavier injoignable".into(),
                    format!(
                        "« {name} » ne répond plus depuis {min} min. Appuyez sur une touche ; \
                         sinon éteignez puis rallumez-le (le pairage n'est pas en cause). \
                         Ensuite : akmctl doctor"
                    ),
                    Urgency::Normal,
                )
            } else {
                (
                    "Keyboard unreachable".into(),
                    format!(
                        "\u{201c}{name}\u{201d} has not answered for {min} min. Press a key; \
                         otherwise switch it off and on (the pairing is not at fault). \
                         Then: akmctl doctor"
                    ),
                    Urgency::Normal,
                )
            }
        }
        Notice::RepairNeeded { why } => {
            if fr {
                (
                    "Clavier : ré-appairage nécessaire".into(),
                    format!(
                        "« {name} » : {why}. Lancez « akmctl repair » \
                         (ou menu de l'icône › Réparer la liaison…)."
                    ),
                    Urgency::Critical,
                )
            } else {
                (
                    "Keyboard: re-pairing needed".into(),
                    format!(
                        "\u{201c}{name}\u{201d}: {why}. Run \u{201c}akmctl repair\u{201d} \
                         (or tray menu \u{203a} Repair the link\u{2026})."
                    ),
                    Urgency::Critical,
                )
            }
        }
        Notice::Recovered => {
            if fr {
                (
                    "Clavier reconnecté".into(),
                    format!("« {name} » est de nouveau connecté."),
                    Urgency::Low,
                )
            } else {
                (
                    "Keyboard reconnected".into(),
                    format!("\u{201c}{name}\u{201d} is connected again."),
                    Urgency::Low,
                )
            }
        }
    }
}

// ── Decision core ──────────────────────────────────────────────────────────

struct Dev {
    path: String,
    name: String,
    connected: bool,
    rec: Recovery,
}

pub struct Keeper<B: LinkBus> {
    bus: B,
    devs: BTreeMap<String, Dev>,
    shared: SharedStatus,
    bluez: bool,
    sleeping: bool,
    adapter_on: bool,
    last_reconcile: Option<Instant>,
    notify: bool,
    fr: bool,
}

impl<B: LinkBus> Keeper<B> {
    pub fn new(bus: B, shared: SharedStatus, notify: bool) -> Self {
        Self {
            bus,
            devs: BTreeMap::new(),
            shared,
            bluez: false,
            sleeping: false,
            adapter_on: true,
            last_reconcile: None,
            notify,
            fr: french(),
        }
    }

    pub fn bus(&mut self) -> &mut B {
        &mut self.bus
    }

    pub fn set_french(&mut self, fr: bool) {
        self.fr = fr;
    }

    fn mac_of(&self, path: &str) -> Option<String> {
        self.devs
            .iter()
            .find(|(_, d)| d.path == path)
            .map(|(m, _)| m.clone())
    }

    fn sync(&mut self, list: Vec<DevInfo>, now: Instant) {
        self.bluez = true;
        let mut seen = Vec::new();
        for info in &list {
            if !info.paired {
                if let Some(d) = self.devs.get_mut(&info.mac) {
                    d.rec.on_bond_lost(now);
                    seen.push(info.mac.clone());
                }
                continue;
            }
            seen.push(info.mac.clone());
            match self.devs.get_mut(&info.mac) {
                Some(d) => {
                    d.path = info.path.clone();
                    d.name = info.name.clone();
                    if info.connected && !d.connected {
                        d.rec.on_connected(now);
                    } else if !info.connected && d.connected {
                        d.rec.on_disconnected(DisconnectReason::Unknown, now);
                    }
                    d.connected = info.connected;
                }
                None => {
                    let mut rec = Recovery::new();
                    rec.start(info.connected, true, now);
                    if self.sleeping {
                        rec.on_sleep(now);
                    }
                    if !self.adapter_on {
                        rec.on_adapter(false, now);
                    }
                    self.devs.insert(
                        info.mac.clone(),
                        Dev {
                            path: info.path.clone(),
                            name: info.name.clone(),
                            connected: info.connected,
                            rec,
                        },
                    );
                }
            }
        }
        // Removed from BlueZ (forgotten by the user): stop following, silently.
        self.devs.retain(|m, _| seen.contains(m));
        let connected = list
            .iter()
            .filter(|d| d.connected)
            .map(|d| d.mac.clone())
            .collect();
        self.bus.reconcile(connected);
        self.last_reconcile = Some(now);
    }

    fn resync(&mut self, now: Instant) {
        if let Some(list) = self.bus.enumerate() {
            self.sync(list, now);
        }
    }

    pub fn handle(&mut self, m: KMsg, now: Instant) {
        match m {
            KMsg::Sync(list) => self.sync(list, now),
            KMsg::BluezGone => {
                self.bluez = false;
                for d in self.devs.values_mut() {
                    d.rec.on_adapter(false, now);
                }
            }
            KMsg::Connected(path, c) => match self.mac_of(&path) {
                Some(mac) => {
                    let d = self.devs.get_mut(&mac).expect("known");
                    d.connected = c;
                    if c {
                        d.rec.on_connected(now);
                    } else {
                        d.rec.on_disconnected(DisconnectReason::Unknown, now);
                    }
                }
                None if c => self.resync(now), // a keyboard paired meanwhile
                None => {}
            },
            KMsg::Paired(path, p) => match (self.mac_of(&path), p) {
                (Some(mac), false) => {
                    if let Some(d) = self.devs.get_mut(&mac) {
                        d.rec.on_bond_lost(now);
                    }
                }
                (None, true) => self.resync(now),
                _ => {}
            },
            KMsg::Disconnected(path, reason) => {
                if let Some(d) = self.mac_of(&path).and_then(|m| self.devs.get_mut(&m)) {
                    d.connected = false;
                    d.rec.on_disconnected(reason, now);
                }
            }
            KMsg::Adapter(on) => {
                self.adapter_on = on;
                for d in self.devs.values_mut() {
                    d.rec.on_adapter(on, now);
                }
            }
            KMsg::Sleep(SleepEvent::Sleeping) => {
                self.sleeping = true;
                for d in self.devs.values_mut() {
                    d.rec.on_sleep(now);
                }
            }
            KMsg::Sleep(SleepEvent::Resumed) => {
                self.sleeping = false;
                // Fresh view first: what survived the sleep?
                let fresh = self.bus.enumerate();
                if let Some(list) = fresh.as_ref() {
                    for info in list {
                        if let Some(d) = self.devs.get_mut(&info.mac) {
                            d.connected = info.connected;
                        }
                    }
                }
                for d in self.devs.values_mut() {
                    d.rec.on_resume(d.connected, now);
                }
                if let Some(list) = fresh {
                    self.sync(list, now);
                }
            }
            KMsg::ConnectDone(mac, r) => {
                if let Some(d) = self.devs.get_mut(&mac) {
                    if let Err(e) = &r {
                        tracing::info!("link: connect {mac}: {}", e.describe());
                    }
                    d.rec.on_connect_result(r, now);
                }
            }
            KMsg::Request => {
                for d in self.devs.values_mut() {
                    d.rec.request_now(now);
                }
            }
            KMsg::Quit => {}
        }
        self.publish(now);
    }

    /// Run what is due (connections, notifications, periodic reconciliation).
    pub fn tick(&mut self, now: Instant) {
        if self.bluez
            && !self.sleeping
            && self
                .last_reconcile
                .is_none_or(|t| now.saturating_duration_since(t) >= RECONCILE_PERIOD)
        {
            self.resync(now);
            self.last_reconcile = Some(now);
        }
        let mut connects = Vec::new();
        let mut notes = Vec::new();
        for (mac, d) in self.devs.iter_mut() {
            let before = d.rec.health();
            for a in d.rec.poll(now) {
                match a {
                    Action::Connect => connects.push((d.path.clone(), mac.clone())),
                    Action::Notify(n) => notes.push((n, d.name.clone())),
                }
            }
            if d.rec.health() != before {
                tracing::info!(
                    "link: {mac} {} -> {}",
                    before.as_str(),
                    d.rec.health().as_str()
                );
            }
        }
        for (path, mac) in connects {
            tracing::info!("link: paging {mac}");
            self.bus.connect(&path, &mac);
        }
        for (n, name) in notes {
            let (s, b, u) = notice_text(&n, &name, now, self.fr);
            tracing::warn!("link: {s} — {b}");
            if self.notify {
                self.bus.notify_notice(&n, &s, &b, u);
            }
        }
        self.publish(now);
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        let reconcile = self
            .bluez
            .then(|| self.last_reconcile.map(|t| t + RECONCILE_PERIOD))
            .flatten();
        self.devs
            .values()
            .filter_map(|d| d.rec.next_deadline())
            .chain(reconcile)
            .min()
    }

    pub fn status(&self, now: Instant) -> Vec<LinkStatus> {
        let unow = unix_now();
        self.devs
            .iter()
            .map(|(mac, d)| LinkStatus {
                mac: mac.clone(),
                name: d.name.clone(),
                health: d.rec.health().as_str().into(),
                since: d.rec.since().map_or(0, |s| {
                    unow.saturating_sub(now.saturating_duration_since(s).as_secs())
                }),
                attempts: d.rec.attempts(),
                failures: d.rec.failures(),
                last_error: d.rec.last_error().unwrap_or_default().into(),
                last_reason: d.rec.last_reason().map_or("", |r| r.as_str()).into(),
                updated: unow,
            })
            .collect()
    }

    fn publish(&self, now: Instant) {
        let s = self.status(now);
        *self.shared.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }
}

// ── Real buses ─────────────────────────────────────────────────────────────

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

/// Paired-or-not Apple keyboards known to BlueZ.
pub fn enumerate(calls: &Connection) -> zbus::Result<Vec<DevInfo>> {
    let om = zbus::blocking::fdo::ObjectManagerProxy::builder(calls)
        .destination("org.bluez")?
        .path("/")?
        .build()?;
    let mut out = Vec::new();
    for (path, ifaces) in om.get_managed_objects()? {
        let Some(d) = ifaces
            .iter()
            .find(|(k, _)| k.as_str() == "org.bluez.Device1")
            .map(|(_, v)| v)
        else {
            continue;
        };
        let keyboard =
            prop_str(d, "Modalias").is_some_and(|m| is_keyboard_device(&m, prop_u32(d, "Class")));
        let Some(mac) = prop_str(d, "Address").map(|a| a.to_ascii_uppercase()) else {
            continue;
        };
        if !keyboard {
            continue;
        }
        out.push(DevInfo {
            path: path.to_string(),
            name: prop_str(d, "Alias")
                .or_else(|| prop_str(d, "Name"))
                .unwrap_or_else(|| mac.clone()),
            mac,
            connected: prop_bool(d, "Connected").unwrap_or(false),
            paired: prop_bool(d, "Paired").unwrap_or(false)
                || prop_bool(d, "Bonded").unwrap_or(false),
        });
    }
    Ok(out)
}

struct SystemBus {
    calls: Option<Connection>,
    tx: Sender<KMsg>,
    mailbox: Arc<Mailbox>,
}

impl SystemBus {
    fn calls(&mut self) -> Option<&Connection> {
        if self.calls.is_none() {
            self.calls = Connection::system().ok();
        }
        self.calls.as_ref()
    }
}

impl LinkBus for SystemBus {
    fn connect(&mut self, path: &str, mac: &str) {
        let (path, mac, tx) = (path.to_string(), mac.to_string(), self.tx.clone());
        let Some(conn) = self.calls().cloned() else {
            let _ = tx.send(KMsg::ConnectDone(mac, Err(ConnectError::AdapterOff)));
            return;
        };
        let _ = std::thread::Builder::new()
            .name("kb-link-connect".into())
            .spawn(move || {
                let r = conn
                    .call_method(
                        Some("org.bluez"),
                        path.as_str(),
                        Some("org.bluez.Device1"),
                        "Connect",
                        &(),
                    )
                    .map(|_| ())
                    .map_err(|e| match e {
                        zbus::Error::MethodError(name, msg, _) => {
                            ConnectError::classify(name.as_str(), msg.as_deref().unwrap_or(""))
                        }
                        other => ConnectError::Other(other.to_string()),
                    });
                let _ = tx.send(KMsg::ConnectDone(mac, r));
            });
    }

    fn notify(&mut self, summary: &str, body: &str, urgency: Urgency) {
        crate::notify::send_with(
            summary,
            body,
            "input-keyboard",
            urgency,
            urgency == Urgency::Low,
        );
    }

    fn notify_notice(&mut self, notice: &Notice, summary: &str, body: &str, urgency: Urgency) {
        use crate::notify::Event;
        let event = match notice {
            Notice::Unreachable { .. } => Event::KeyboardUnreachable,
            Notice::RepairNeeded { .. } => Event::RepairNeeded,
            Notice::Recovered => Event::KeyboardReconnected,
        };
        crate::notify::notice(event, summary, body, urgency);
    }

    fn reconcile(&mut self, connected: Vec<String>) {
        self.mailbox.send(Msg::Bus(Event::Reconcile(connected)));
    }

    fn enumerate(&mut self) -> Option<Vec<DevInfo>> {
        let r = self.calls().map(enumerate);
        match r {
            Some(Ok(l)) => Some(l),
            Some(Err(e)) => {
                tracing::debug!("link: enumeration failed: {e}");
                self.calls = None;
                None
            }
            None => None,
        }
    }
}

fn add_rules(conn: &Connection) -> zbus::Result<()> {
    let dbus = DBusProxy::new(conn)?;
    for iface in ["org.bluez.Device1", "org.bluez.Adapter1"] {
        dbus.add_match_rule(
            MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender("org.bluez")?
                .interface("org.freedesktop.DBus.Properties")?
                .member("PropertiesChanged")?
                .arg(0, iface)?
                .build(),
        )?;
    }
    dbus.add_match_rule(
        MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.bluez")?
            .interface("org.bluez.Device1")?
            .member("Disconnected")?
            .build(),
    )?;
    dbus.add_match_rule(
        MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg(0, "org.bluez")?
            .build(),
    )?;
    Ok(())
}

fn listen_once(tx: &Sender<KMsg>) -> zbus::Result<()> {
    let conn = Connection::system()?;
    let calls = Connection::system()?;
    add_rules(&conn)?;
    let it = MessageIterator::from(conn);
    // Subscribed first, then enumerated: nothing falls in the gap.
    match enumerate(&calls) {
        Ok(l) => {
            if tx.send(KMsg::Sync(l)).is_err() {
                return Ok(());
            }
        }
        Err(_) => {
            if tx.send(KMsg::BluezGone).is_err() {
                return Ok(());
            }
        }
    }
    for msg in it {
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
        let iface = hdr
            .interface()
            .map(|i| i.as_str().to_string())
            .unwrap_or_default();
        let out = match (member.as_str(), iface.as_str()) {
            ("NameOwnerChanged", _) => {
                let Ok((_, _, new)) = msg.body().deserialize::<(String, String, String)>() else {
                    continue;
                };
                if new.is_empty() {
                    vec![KMsg::BluezGone]
                } else {
                    std::thread::sleep(Duration::from_millis(500));
                    match enumerate(&calls) {
                        Ok(l) => vec![KMsg::Sync(l)],
                        Err(_) => vec![KMsg::BluezGone],
                    }
                }
            }
            ("Disconnected", "org.bluez.Device1") => {
                let Ok((name, _)) = msg.body().deserialize::<(String, String)>() else {
                    continue;
                };
                vec![KMsg::Disconnected(
                    path,
                    DisconnectReason::from_bluez(&name),
                )]
            }
            ("PropertiesChanged", _) => {
                let Ok((i, changed, _)) = msg.body().deserialize::<(String, Props, Vec<String>)>()
                else {
                    continue;
                };
                let mut v = Vec::new();
                if i == "org.bluez.Device1" {
                    if let Some(c) = prop_bool(&changed, "Connected") {
                        v.push(KMsg::Connected(path.clone(), c));
                    }
                    if let Some(p) = prop_bool(&changed, "Paired") {
                        v.push(KMsg::Paired(path.clone(), p));
                    }
                } else if i == "org.bluez.Adapter1" {
                    if let Some(p) = prop_bool(&changed, "Powered") {
                        v.push(KMsg::Adapter(p));
                    }
                }
                v
            }
            _ => Vec::new(),
        };
        for m in out {
            if tx.send(m).is_err() {
                return Ok(());
            }
        }
    }
    Err(zbus::Error::Failure("system bus stream ended".into()))
}

/// Handle on the running keeper.
#[derive(Clone)]
pub struct KeeperHandle {
    pub tx: Sender<KMsg>,
    pub shared: SharedStatus,
}

/// Start the keeper (listener + logind + decision threads).
pub fn spawn(mailbox: Arc<Mailbox>, notify: bool) -> KeeperHandle {
    let (tx, rx) = mpsc::channel::<KMsg>();
    let shared: SharedStatus = Arc::new(Mutex::new(Vec::new()));
    let ltx = tx.clone();
    let _ = std::thread::Builder::new()
        .name("kb-link-listen".into())
        .spawn(move || loop {
            match listen_once(&ltx) {
                Ok(()) => return,
                Err(e) => {
                    tracing::warn!("link: system bus error: {e} — resubscribing in 10 s");
                    std::thread::sleep(Duration::from_secs(10));
                }
            }
        });
    let stx = tx.clone();
    crate::sleep::spawn(move |e| {
        let _ = stx.send(KMsg::Sleep(e));
    });
    let bus = SystemBus {
        calls: None,
        tx: tx.clone(),
        mailbox,
    };
    let sh = shared.clone();
    let _ = std::thread::Builder::new()
        .name("kb-link".into())
        .spawn(move || run(Keeper::new(bus, sh, notify), rx));
    KeeperHandle { tx, shared }
}

fn run<B: LinkBus>(mut k: Keeper<B>, rx: Receiver<KMsg>) {
    loop {
        let now = Instant::now();
        let wait = k.next_deadline().map_or(IDLE_WAIT, |d| {
            d.saturating_duration_since(now).min(IDLE_WAIT)
        });
        match rx.recv_timeout(wait) {
            Ok(KMsg::Quit) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(m) => k.handle(m, Instant::now()),
            Err(RecvTimeoutError::Timeout) => {}
        }
        k.tick(Instant::now());
    }
}

// ── Session D-Bus object ───────────────────────────────────────────────────

pub struct LinkIface {
    handle: KeeperHandle,
}

#[zbus::interface(name = "com.agenceapi.AppleKbMonitor1.Link")]
impl LinkIface {
    /// JSON array of [`LinkStatus`], one per paired keyboard.
    fn status(&self) -> String {
        let s = self
            .handle
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        serde_json::Value::Array(s.iter().map(LinkStatus::to_json).collect()).to_string()
    }

    /// Page the keyboard now (still rate-limited to one attempt per 20 s,
    /// never while sleeping or when the pairing is refused).
    fn reconnect(&self) -> bool {
        self.handle.tx.send(KMsg::Request).is_ok()
    }
}

/// Export the link object on the daemon's session connection.
pub fn export(conn: &Connection, handle: KeeperHandle) -> zbus::Result<()> {
    conn.object_server().at(LINK_PATH, LinkIface { handle })?;
    Ok(())
}

// ── Guided repair launcher (tray) ──────────────────────────────────────────

/// Terminal command running `akmctl repair` (first terminal found).
pub fn repair_command(which: impl Fn(&str) -> bool) -> Option<Vec<String>> {
    const TERMS: [(&str, &[&str]); 5] = [
        ("konsole", &["--hold", "-e"]),
        ("gnome-terminal", &["--"]),
        ("kitty", &["--hold"]),
        ("alacritty", &["--hold", "-e"]),
        ("xterm", &["-hold", "-e"]),
    ];
    TERMS.iter().find(|(t, _)| which(t)).map(|(t, args)| {
        let mut v = vec![t.to_string()];
        v.extend(args.iter().map(|a| a.to_string()));
        v.push("akmctl".into());
        v.push("repair".into());
        v
    })
}

fn in_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| {
            let f = d.join(bin);
            std::fs::metadata(&f).is_ok_and(|m| m.is_file())
        })
    })
}

/// Open a terminal running the guided repair (tray action).
pub fn launch_repair() {
    match repair_command(in_path) {
        Some(argv) => {
            match std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .stdin(std::process::Stdio::null())
                .spawn()
            {
                Ok(_) => tracing::info!("tray: guided repair started in {}", argv[0]),
                Err(e) => tracing::warn!("tray: cannot start {}: {e}", argv[0]),
            }
        }
        None => tracing::warn!("tray: no terminal found for akmctl repair"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akm_core::machine::{Action as MAction, Machine};
    use akm_core::recovery::{Health, MIN_SPACING};

    const MAC: &str = "04:DB:56:CA:42:EE";
    const PATH: &str = "/org/bluez/hci0/dev_04_DB_56_CA_42_EE";

    /// Fake BlueZ + desktop + acquisition machine (the real `Machine`).
    struct Fake {
        world: Vec<DevInfo>,
        bluez_up: bool,
        connects: Vec<Instant>,
        notes: Vec<(String, Urgency)>,
        machine: Machine,
        clears: usize,
        now: Instant,
    }

    impl LinkBus for Fake {
        fn connect(&mut self, _path: &str, _mac: &str) {
            self.connects.push(self.now);
        }
        fn notify(&mut self, summary: &str, _body: &str, u: Urgency) {
            self.notes.push((summary.to_string(), u));
        }
        fn reconcile(&mut self, connected: Vec<String>) {
            if self
                .machine
                .on_event(&Event::Reconcile(connected), self.now)
                == Some(MAction::Clear)
            {
                self.clears += 1;
            }
        }
        fn enumerate(&mut self) -> Option<Vec<DevInfo>> {
            self.bluez_up.then(|| self.world.clone())
        }
    }

    fn kb(connected: bool) -> DevInfo {
        DevInfo {
            path: PATH.into(),
            mac: MAC.into(),
            name: "Clavier de maria #1".into(),
            connected,
            paired: true,
        }
    }

    fn keeper(connected: bool, t0: Instant) -> Keeper<Fake> {
        let fake = Fake {
            world: vec![kb(connected)],
            bluez_up: true,
            connects: Vec::new(),
            notes: Vec::new(),
            machine: Machine::new(),
            clears: 0,
            now: t0,
        };
        let mut k = Keeper::new(fake, Arc::new(Mutex::new(Vec::new())), true);
        k.set_french(true);
        k
    }

    /// Advance second by second; every page goes unanswered.
    fn run_for(k: &mut Keeper<Fake>, from: Instant, secs: u64) -> Instant {
        let mut t = from;
        for _ in 0..secs {
            t += Duration::from_secs(1);
            k.bus().now = t;
            let before = k.bus().connects.len();
            k.tick(t);
            if k.bus().connects.len() > before {
                k.handle(
                    KMsg::ConnectDone(MAC.into(), Err(ConnectError::NoAnswer)),
                    t,
                );
            }
        }
        t
    }

    fn health(k: &Keeper<Fake>, t: Instant) -> String {
        k.status(t)[0].health.clone()
    }

    #[test]
    fn reboot_scenario_pages_then_alerts_then_recovers() {
        // The PC boots, the keyboard sleeps (the 2026-10-01 case).
        let t0 = Instant::now();
        let mut k = keeper(false, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        assert_eq!(health(&k, t0), "dormant");
        let t = run_for(&mut k, t0, 15 * 60);
        assert_eq!(health(&k, t), "unreachable");
        let n = &k.bus().notes;
        assert_eq!(n.len(), 1, "{n:?}");
        assert_eq!(n[0].0, "Clavier injoignable");
        let c = &k.bus().connects;
        assert!(c.len() >= 4 && c.len() <= 9, "{} pages in 15 min", c.len());
        for w in c.windows(2) {
            assert!(w[1] - w[0] >= MIN_SPACING);
        }
        // a key press: BlueZ reports the keyboard connected
        k.bus().world = vec![kb(true)];
        k.handle(KMsg::Connected(PATH.into(), true), t);
        k.tick(t);
        assert_eq!(health(&k, t), "connected");
        assert_eq!(k.bus().notes.last().unwrap().0, "Clavier reconnecté");
        let pages = k.bus().connects.len();
        run_for(&mut k, t, 3600);
        assert_eq!(k.bus().connects.len(), pages, "no paging while connected");
    }

    #[test]
    fn authentication_failure_requests_repair_and_stops_paging() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        k.bus().world = vec![kb(false)];
        k.handle(
            KMsg::Disconnected(PATH.into(), DisconnectReason::Authentication),
            t0,
        );
        k.handle(KMsg::Connected(PATH.into(), false), t0);
        let t = run_for(&mut k, t0, 6 * 3600);
        assert_eq!(health(&k, t), "auth-failed");
        assert_eq!(k.bus().connects.len(), 0);
        let n = &k.bus().notes;
        assert_eq!(n.len(), 1);
        assert_eq!(
            n[0],
            (
                "Clavier : ré-appairage nécessaire".into(),
                Urgency::Critical
            )
        );
    }

    #[test]
    fn suspend_resume_pauses_then_pages_quickly() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        k.handle(KMsg::Sleep(SleepEvent::Sleeping), t0);
        k.handle(
            KMsg::Disconnected(PATH.into(), DisconnectReason::Suspend),
            t0,
        );
        let t = run_for(&mut k, t0, 8 * 3600);
        assert_eq!(health(&k, t), "suspended");
        assert!(k.bus().connects.is_empty(), "nothing while asleep");
        k.bus().world = vec![kb(false)];
        k.handle(KMsg::Sleep(SleepEvent::Resumed), t);
        assert_eq!(health(&k, t), "dormant");
        run_for(&mut k, t, 10);
        assert_eq!(
            k.bus().connects.len(),
            1,
            "first page within the grace delay"
        );
    }

    #[test]
    fn keyboard_sleep_is_silent() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        k.bus().world = vec![kb(false)];
        k.handle(
            KMsg::Disconnected(PATH.into(), DisconnectReason::Remote),
            t0,
        );
        k.handle(KMsg::Connected(PATH.into(), false), t0);
        let t = run_for(&mut k, t0, 12 * 3600);
        assert_eq!(health(&k, t), "dormant");
        assert!(k.bus().notes.is_empty());
        assert!(k.bus().connects.len() <= 12 + 44 + 2);
    }

    #[test]
    fn issue_165_bluez_back_without_keyboard_stops_the_probing() {
        // BlueZ absent at start: the watcher sent NoBluez, the machine probes.
        let t0 = Instant::now();
        let mut k = keeper(false, t0);
        k.bus().bluez_up = false;
        k.bus().machine.on_event(&Event::NoBluez, t0);
        assert!(k.bus().machine.is_connected());
        k.handle(KMsg::BluezGone, t0);
        // bluetoothd appears, keyboard off
        k.bus().bluez_up = true;
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0 + Duration::from_secs(30));
        assert!(
            !k.bus().machine.is_connected(),
            "reconciled: nothing connected"
        );
        assert_eq!(k.bus().clears, 1);
    }

    #[test]
    fn issue_165_missed_disconnection_is_caught_by_the_periodic_check() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        k.bus().machine.on_event(&Event::Connected(MAC.into()), t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0 + Duration::from_secs(5));
        assert!(k.bus().machine.is_connected());
        // the keyboard leaves while every signal is lost (bus outage)
        k.bus().world = vec![kb(false)];
        let t = run_for(
            &mut k,
            t0 + Duration::from_secs(5),
            RECONCILE_PERIOD.as_secs(),
        );
        assert!(
            !k.bus().machine.is_connected(),
            "machine reconciled within one period"
        );
        assert_eq!(health(&k, t), "dormant", "keeper recovering");
    }

    #[test]
    fn forgotten_keyboard_is_dropped_silently_and_unpaired_one_asks_for_repair() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        k.handle(KMsg::Paired(PATH.into(), false), t0);
        k.tick(t0);
        assert_eq!(health(&k, t0), Health::AuthFailed.as_str());
        assert_eq!(k.bus().notes.len(), 1);
        k.handle(KMsg::Sync(vec![]), t0);
        assert!(k.status(t0).is_empty());
    }

    #[test]
    fn repair_terminal_choice() {
        let v = repair_command(|b| b == "konsole").unwrap();
        assert_eq!(v, ["konsole", "--hold", "-e", "akmctl", "repair"]);
        let v = repair_command(|b| b == "xterm").unwrap();
        assert_eq!(v.last().unwrap(), "repair");
        assert!(repair_command(|_| false).is_none());
    }

    #[test]
    fn notice_texts_are_explicit() {
        let t = Instant::now();
        let (s, b, u) = notice_text(&Notice::RepairNeeded { why: "x".into() }, "Kb", t, false);
        assert!(s.contains("re-pairing") && b.contains("akmctl repair"));
        assert_eq!(u, Urgency::Critical);
        let (_, b, _) = notice_text(
            &Notice::Unreachable { since: t },
            "Kb",
            t + Duration::from_secs(660),
            true,
        );
        assert!(b.contains("11 min") && b.contains("akmctl doctor"));
    }
}
