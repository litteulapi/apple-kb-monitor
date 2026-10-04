//! Link keeper: keeps the keyboard's Bluetooth link alive across link losses, reboots and system
//! sleep, and says clearly when a re-pairing is really needed.

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
use crate::origin::Origin;
use crate::sleep::SleepEvent;

pub const LINK_PATH: &str = "/com/agenceapi/AppleKbMonitor1/Link";
pub const LINK_INTERFACE: &str = "com.agenceapi.AppleKbMonitor1.Link";
/// Fresh `BlueZ` enumeration pushed to the acquisition machine this often.
pub const RECONCILE_PERIOD: Duration = Duration::from_mins(5);
/// `BlueZ` clears `Paired` just before it drops the object when a device is forgotten.
pub const BOND_GRACE: Duration = Duration::from_millis(1500);
/// A disconnection within this delay of the user's request is the user's.
pub const USER_DOWN_WINDOW: Duration = Duration::from_secs(15);
const IDLE_WAIT: Duration = Duration::from_mins(1);

/// A keyboard as `BlueZ` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevInfo {
    pub path: String,
    pub mac: String,
    pub name: String,
    pub connected: bool,
    pub paired: bool,
    /// Battery percentage `BlueZ` (`Battery1`) or `UPower` already holds for it.
    pub battery: Option<u8>,
}

/// Messages to the keeper.
#[derive(Debug, Clone, PartialEq)]
pub enum KMsg {
    /// Full enumeration (start, `BlueZ` back, listener resubscribed).
    Sync(Vec<DevInfo>),
    /// `org.bluez` left the bus.
    BluezGone,
    Connected(String, bool),
    Paired(String, bool),
    /// `org.bluez.Battery1.Percentage` of a device object changed.
    Battery(String, u8),
    /// `BlueZ` removed the device object (Plasma "Forget", `bluetoothctl remove`).
    Removed(String),
    /// A `Device1` object appeared (`InterfacesAdded`).
    Added(String),
    Disconnected(String, DisconnectReason),
    Adapter(bool),
    Sleep(SleepEvent),
    /// Result of an asynchronous `Device1.Connect` (by MAC).
    ConnectDone(String, Result<(), ConnectError>),
    /// User asked for an immediate attempt (tray, akmctl).
    Request,
    /// User asked to disconnect a keyboard: `Device1.Disconnect`, and no page until asked.
    UserDisconnect(Option<String>),
    /// User CONFIRMED the removal of this keyboard (MAC), see [`crate::forget`].
    UserForget(String),
    Quit,
}

/// Side effects of the keeper (real buses, or a fake in tests).
pub trait LinkBus {
    /// Start `Device1.Connect` on `path`; the result must come back as [`KMsg::ConnectDone`].
    fn connect(&mut self, path: &str, mac: &str);
    fn notify(&mut self, summary: &str, body: &str, urgency: Urgency);
    fn notify_notice(&mut self, _notice: &Notice, summary: &str, body: &str, urgency: Urgency) {
        self.notify(summary, body, urgency);
    }
    fn reconcile(&mut self, connected: Vec<String>);
    fn removed(&mut self, _name: &str) {}
    fn forgotten(&mut self, _mac: &str) {}
    fn disconnect(&mut self, _path: &str, _mac: &str) {}
    fn forget(&mut self, _path: &str, _mac: &str) {}
    fn notify_unstable(&mut self, _name: &str, _count: usize) {}
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
    /// Disconnection counts and signal of the last 7 days.
    pub quality: Option<akm_core::linkstats::LinkQuality>,
    /// `BlueZ` says the keyboard is connected.
    pub connected: bool,
    /// Battery percentage known to `BlueZ` / `UPower`, if any.
    pub battery: Option<u8>,
    /// `BlueZ` `Device1.Paired` (or `Bonded`), as last seen.
    pub paired: bool,
    /// A `Device1.Connect` is in flight.
    pub connecting: bool,
}

impl LinkStatus {
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "mac": self.mac,
            "name": self.name,
            "health": self.health,
            "paired": self.paired,
            "connecting": self.connecting,
            "since": self.since,
            "attempts": self.attempts,
            "failures": self.failures,
            "last_error": self.last_error,
            "last_reason": self.last_reason,
            "updated": self.updated,
            "quality": self.quality,
            "connected": self.connected,
            "battery": self.battery,
        })
    }
}

pub type SharedStatus = Arc<Mutex<Vec<LinkStatus>>>;

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Notification text for a notice (summary, body, urgency).
#[must_use]
pub fn notice_text(n: &Notice, name: &str, now: Instant) -> (String, String, Urgency) {
    match n {
        Notice::Unreachable { since } => {
            let min = now.saturating_duration_since(*since).as_secs() / 60;
            (
                tr!("Keyboard unreachable"),
                tr!(
                    "\u{201c}{name}\u{201d} has not answered for {min} min. Press a key; otherwise switch it off and on (the pairing is not at fault). Then: akmctl doctor",
                    name = name,
                    min = min
                ),
                Urgency::Normal,
            )
        }
        Notice::RepairNeeded { why } => (
            tr!("Keyboard: re-pairing needed"),
            tr!(
                "\u{201c}{name}\u{201d}: {why}. Run \u{201c}akmctl repair\u{201d} (or tray menu \u{203a} Repair the link\u{2026}).",
                name = name,
                why = why
            ),
            Urgency::Critical,
        ),
        Notice::Recovered => (
            tr!("Keyboard reconnected"),
            tr!("\u{201c}{name}\u{201d} is connected again.", name = name),
            Urgency::Low,
        ),
    }
}

struct Dev {
    path: String,
    name: String,
    connected: bool,
    battery: Option<u8>,
    rec: Recovery,
    unpair_at: Option<Instant>,
}

#[allow(clippy::struct_excessive_bools)] // independent state flags of the link keeper
pub struct Keeper<B: LinkBus> {
    bus: B,
    devs: BTreeMap<String, Dev>,
    shared: SharedStatus,
    bluez: bool,
    sleeping: bool,
    adapter_on: bool,
    last_reconcile: Option<Instant>,
    notify: bool,
    stats: Option<Arc<crate::linkq::Store>>,
    notify_unstable: bool,
    user_down: HashMap<String, Instant>,
    /// Wall clock of the link statistics, read at the event (`Instant` stops during suspend).
    wall: Box<dyn Fn(Instant) -> u64 + Send>,
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
            stats: None,
            notify_unstable: true,
            user_down: HashMap::new(),
            wall: Box::new(|_| unix_now()),
        }
    }

    /// Record the disconnections in `stats` and raise the "unstable link" alert if `alert`.
    pub fn set_stats(&mut self, stats: Arc<crate::linkq::Store>, alert: bool) {
        self.stats = Some(stats);
        self.notify_unstable = alert;
    }

    fn unix(&self, now: Instant) -> u64 {
        (self.wall)(now)
    }

    fn down_reason(&self, mac: &str, reason: DisconnectReason, now: Instant) -> &'static str {
        let asked = self
            .user_down
            .get(mac)
            .is_some_and(|t| now.saturating_duration_since(*t) <= USER_DOWN_WINDOW);
        if asked {
            "user"
        } else if self.sleeping || reason == DisconnectReason::Suspend {
            "suspend"
        } else if akm_core::link::expected_disconnect_recent() {
            "expected"
        } else if akm_core::link::keyboard_off_recent() {
            "off"
        } else {
            reason.as_str()
        }
    }

    fn note_down(&self, mac: &str, reason: DisconnectReason, now: Instant) {
        if let Some(s) = self.stats.as_ref() {
            s.disconnect(mac, self.unix(now), self.down_reason(mac, reason, now));
        }
    }

    pub fn bus(&mut self) -> &mut B {
        &mut self.bus
    }

    fn mac_of(&self, path: &str) -> Option<String> {
        self.devs
            .iter()
            .find(|(_, d)| d.path == path)
            .map(|(m, _)| m.clone())
    }

    fn sync(&mut self, list: &[DevInfo], now: Instant) {
        self.bluez = true;
        let mut seen = Vec::new();
        // Disconnections only this enumeration reveals (no signal seen).
        let mut downs: Vec<String> = Vec::new();
        for info in list {
            if !info.paired {
                if let Some(d) = self.devs.get_mut(&info.mac) {
                    d.rec.on_bond_lost(now);
                    seen.push(info.mac.clone());
                }
                continue;
            }
            seen.push(info.mac.clone());
            if let Some(d) = self.devs.get_mut(&info.mac) {
                d.path.clone_from(&info.path);
                d.name.clone_from(&info.name);
                if info.battery.is_some() {
                    d.battery = info.battery;
                }
                d.unpair_at = None;
                if info.connected && !d.connected {
                    d.rec.on_connected(now);
                } else if !info.connected && d.connected {
                    d.rec.on_disconnected(DisconnectReason::Unknown, now);
                    downs.push(info.mac.clone());
                }
                d.connected = info.connected;
            } else {
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
                        battery: info.battery,
                        rec,
                        unpair_at: None,
                    },
                );
            }
        }
        for mac in downs {
            self.note_down(&mac, DisconnectReason::Unknown, now);
        }
        // Removed from BlueZ (forgotten by the user): stop following, silently.
        let gone: Vec<String> = self
            .devs
            .keys()
            .filter(|m| !seen.contains(m))
            .cloned()
            .collect();
        self.devs.retain(|m, _| seen.contains(m));
        for mac in gone {
            self.bus.forgotten(&mac);
        }
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
            self.sync(&list, now);
        }
    }

    #[allow(clippy::too_many_lines)] // state machine: one arm per message
    pub fn handle(&mut self, m: KMsg, now: Instant) {
        match m {
            KMsg::Sync(list) => self.sync(&list, now),
            KMsg::BluezGone => {
                self.bluez = false;
                for d in self.devs.values_mut() {
                    d.rec.on_adapter(false, now);
                }
            }
            KMsg::Connected(path, c) => {
                if let Some((mac, d)) = self.devs.iter_mut().find(|(_, d)| d.path == path) {
                    let mac = mac.clone();
                    let was = std::mem::replace(&mut d.connected, c);
                    if c {
                        d.rec.on_connected(now);
                    } else {
                        d.rec.on_disconnected(DisconnectReason::Unknown, now);
                        if was {
                            self.note_down(&mac, DisconnectReason::Unknown, now);
                        }
                    }
                } else if c {
                    self.resync(now); // a keyboard paired meanwhile
                }
            }
            KMsg::Battery(path, pct) => {
                if let Some(d) = self.mac_of(&path).and_then(|m| self.devs.get_mut(&m)) {
                    d.battery = Some(pct.min(100));
                }
            }
            KMsg::Paired(path, p) => match (self.mac_of(&path), p) {
                (Some(mac), false) => {
                    if let Some(d) = self.devs.get_mut(&mac) {
                        d.unpair_at.get_or_insert(now + BOND_GRACE);
                    }
                }
                (Some(mac), true) => {
                    if let Some(d) = self.devs.get_mut(&mac) {
                        d.unpair_at = None;
                    }
                }
                (None, true) => self.resync(now),
                _ => {}
            },
            KMsg::Removed(path) => {
                if let Some(mac) = self.mac_of(&path) {
                    if let Some(d) = self.devs.remove(&mac) {
                        tracing::info!("link: {mac} removed from BlueZ");
                        if self.notify {
                            self.bus.removed(&d.name);
                        }
                    }
                    self.bus.forgotten(&mac);
                    let connected = self
                        .devs
                        .iter()
                        .filter(|(_, d)| d.connected)
                        .map(|(m, _)| m.clone())
                        .collect();
                    self.bus.reconcile(connected);
                }
            }
            KMsg::Added(path) => {
                if self.mac_of(&path).is_none() {
                    self.resync(now);
                }
            }
            KMsg::Disconnected(path, reason) => {
                if let Some(mac) = self.mac_of(&path) {
                    let mut was = false;
                    if let Some(d) = self.devs.get_mut(&mac) {
                        was = std::mem::replace(&mut d.connected, false);
                        d.rec.on_disconnected(reason, now);
                    }
                    if was {
                        self.note_down(&mac, reason, now);
                    } else if let Some(s) = self.stats.as_ref() {
                        // `Connected = false` came first: its record gets the reason.
                        s.refine(&mac, self.unix(now), self.down_reason(&mac, reason, now));
                    }
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
                let mut lost_asleep = Vec::new();
                if let Some(list) = fresh.as_ref() {
                    for info in list {
                        if let Some(d) = self.devs.get_mut(&info.mac) {
                            if d.connected && !info.connected {
                                lost_asleep.push(info.mac.clone());
                            }
                            d.connected = info.connected;
                        }
                    }
                }
                // Links that did not survive the sleep: not an instability.
                for mac in lost_asleep {
                    self.note_down(&mac, DisconnectReason::Suspend, now);
                }
                for d in self.devs.values_mut() {
                    d.rec.on_resume(d.connected, now);
                }
                if let Some(list) = fresh {
                    self.sync(&list, now);
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
            KMsg::UserDisconnect(which) => {
                let targets: Vec<(String, String)> = self
                    .devs
                    .iter()
                    .filter(|(m, d)| {
                        d.connected && which.as_ref().is_none_or(|w| w.eq_ignore_ascii_case(m))
                    })
                    .map(|(m, d)| (m.clone(), d.path.clone()))
                    .collect();
                for (mac, path) in targets {
                    tracing::info!("link: disconnecting {mac} on the user's request");
                    if let Some(d) = self.devs.get_mut(&mac) {
                        d.rec.on_user_disconnect(now);
                    }
                    self.user_down.insert(mac.clone(), now);
                    self.bus.disconnect(&path, &mac);
                }
            }
            KMsg::UserForget(mac) => {
                let mac = mac.to_ascii_uppercase();
                if let Some(d) = self.devs.get_mut(&mac) {
                    tracing::warn!("link: removing {mac} from BlueZ (confirmed by the user)");
                    // No page while BlueZ removes it.
                    d.rec.on_user_disconnect(now);
                    let path = d.path.clone();
                    self.user_down.insert(mac.clone(), now);
                    self.bus.forget(&path, &mac);
                } else {
                    tracing::warn!("link: {mac} is unknown to BlueZ, nothing to forget");
                }
            }
            KMsg::Quit => {}
        }
        self.user_down
            .retain(|_, t| now.saturating_duration_since(*t) <= USER_DOWN_WINDOW);
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
        for (mac, d) in &mut self.devs {
            let before = d.rec.health();
            if d.unpair_at.is_some_and(|t| now >= t) {
                d.unpair_at = None;
                d.rec.on_bond_lost(now);
            }
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
        // Unstable link: more than 3 disconnections within an hour, told once per episode.
        if let Some(stats) = self.stats.clone() {
            let unix = self.unix(now);
            let due: Vec<(String, usize)> = self
                .devs
                .iter()
                .filter_map(|(mac, d)| Some((d.name.clone(), stats.unstable_due(mac, unix)?)))
                .collect();
            for (name, count) in due {
                tracing::warn!("link: {name} unstable, {count} disconnections within an hour");
                if self.notify && self.notify_unstable {
                    self.bus.notify_unstable(&name, count);
                }
            }
        }
        for (n, name) in notes {
            let (s, b, u) = notice_text(&n, &name, now);
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
            .flat_map(|d| d.rec.next_deadline().into_iter().chain(d.unpair_at))
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
                quality: self
                    .stats
                    .as_ref()
                    .and_then(|s| s.quality(mac, self.unix(now))),
                connected: d.connected,
                battery: d.battery,
                paired: d.rec.paired(),
                connecting: d.rec.in_flight(),
            })
            .collect()
    }

    fn publish(&self, now: Instant) {
        let s = self.status(now);
        *self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = s;
    }
}

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

/// Paired-or-not Apple keyboards known to `BlueZ`.
///
/// # Errors
/// The D-Bus error of `GetManagedObjects`.
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
        // What BlueZ already holds: read with the device, nothing is asked from the keyboard.
        let battery = ifaces
            .iter()
            .find(|(k, _)| k.as_str() == "org.bluez.Battery1")
            .and_then(|(_, b)| b.get("Percentage"))
            .and_then(|v| u8::try_from(v).ok())
            .map(|p| p.min(100));
        out.push(DevInfo {
            path: path.to_string(),
            name: prop_str(d, "Alias")
                .or_else(|| prop_str(d, "Name"))
                .unwrap_or_else(|| mac.clone()),
            mac,
            connected: prop_bool(d, "Connected").unwrap_or(false),
            paired: prop_bool(d, "Paired").unwrap_or(false)
                || prop_bool(d, "Bonded").unwrap_or(false),
            battery,
        });
    }
    Ok(out)
}

/// Longest wait for `BlueZ`'s answer to `Device1.Connect`: past it the attempt counts as failed.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(40);

/// Run `f` on its own thread; `None` when it did not return within `limit`.
pub fn with_timeout<T: Send + 'static>(
    limit: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("kb-link-call".into())
        .spawn(move || {
            let _ = tx.send(f());
        })
        .ok()?;
    rx.recv_timeout(limit).ok()
}

/// A `Device1.Connect` is waiting for `BlueZ` for one of the keyboards.
pub fn connect_in_flight(shared: &SharedStatus) -> bool {
    shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|s| s.connecting)
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
                // zbus 4 has no per-call timeout.
                let r = with_timeout(CONNECT_TIMEOUT, move || {
                    conn.call_method(
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
                    })
                })
                .unwrap_or_else(|| {
                    tracing::warn!(
                        "link: BlueZ did not answer Connect for {mac} within {CONNECT_TIMEOUT:?}"
                    );
                    Err(ConnectError::Other(format!(
                        "BlueZ did not answer Device1.Connect within {} s",
                        CONNECT_TIMEOUT.as_secs()
                    )))
                });
                let _ = tx.send(KMsg::ConnectDone(mac, r));
            });
    }

    fn disconnect(&mut self, path: &str, mac: &str) {
        let (path, mac) = (path.to_string(), mac.to_string());
        let Some(conn) = self.calls().cloned() else {
            tracing::warn!("link: no system bus, {mac} not disconnected");
            return;
        };
        let _ = std::thread::Builder::new()
            .name("kb-link-disconnect".into())
            .spawn(move || {
                let r = conn.call_method(
                    Some("org.bluez"),
                    path.as_str(),
                    Some("org.bluez.Device1"),
                    "Disconnect",
                    &(),
                );
                match r {
                    Ok(_) => tracing::info!("link: {mac} disconnected on request"),
                    Err(e) => tracing::warn!("link: disconnection of {mac} failed: {e}"),
                }
            });
    }

    fn forget(&mut self, path: &str, mac: &str) {
        let (path, mac) = (path.to_string(), mac.to_string());
        let Some(adapter) = adapter_of(&path) else {
            tracing::warn!("link: {path} has no adapter, {mac} not removed");
            return;
        };
        let Some(conn) = self.calls().cloned() else {
            tracing::warn!("link: no system bus, {mac} not removed");
            return;
        };
        let _ = std::thread::Builder::new()
            .name("kb-link-forget".into())
            .spawn(move || {
                let Ok(dev) = zbus::zvariant::ObjectPath::try_from(path.as_str()) else {
                    return;
                };
                let r = conn.call_method(
                    Some("org.bluez"),
                    adapter.as_str(),
                    Some("org.bluez.Adapter1"),
                    FORGET_CALL,
                    &(dev,),
                );
                match r {
                    Ok(_) => tracing::warn!("link: {mac} removed from this computer"),
                    Err(e) => tracing::warn!("link: removal of {mac} failed: {e}"),
                }
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

    fn removed(&mut self, name: &str) {
        crate::notify::keyboard_removed(name);
    }

    fn forgotten(&mut self, mac: &str) {
        self.mailbox.send(Msg::Forgotten(mac.to_string()));
    }

    fn notify_unstable(&mut self, name: &str, count: usize) {
        crate::notify::link_unstable(name, count);
    }

    fn enumerate(&mut self) -> Option<Vec<DevInfo>> {
        let r = self.calls().map(enumerate);
        match r {
            Some(Ok(mut l)) => {
                // Keyboards without a BlueZ battery: what UPower has cached.
                if l.iter().any(|d| d.battery.is_none()) {
                    if let Some(conn) = self.calls.as_ref() {
                        let cached = upower_batteries(conn);
                        for d in l.iter_mut().filter(|d| d.battery.is_none()) {
                            d.battery = cached.get(&d.mac).copied();
                        }
                    }
                }
                Some(l)
            }
            Some(Err(e)) => {
                tracing::debug!("link: enumeration failed: {e}");
                self.calls = None;
                None
            }
            None => None,
        }
    }
}

/// Battery percentages `UPower` holds in its cache for keyboards, by address (upper case).
#[must_use]
pub fn upower_batteries(sys: &Connection) -> HashMap<String, u8> {
    let mut out = HashMap::new();
    let paths: Vec<zbus::zvariant::OwnedObjectPath> = match sys
        .call_method(
            Some("org.freedesktop.UPower"),
            "/org/freedesktop/UPower",
            Some("org.freedesktop.UPower"),
            "EnumerateDevices",
            &(),
        )
        .and_then(|r| r.body().deserialize())
    {
        Ok(p) => p,
        Err(_) => return out,
    };
    for p in paths {
        if !akm_core::model::is_keyboard_upower_path(p.as_str()) {
            continue;
        }
        let props: Props = match sys
            .call_method(
                Some("org.freedesktop.UPower"),
                p.as_str(),
                Some("org.freedesktop.DBus.Properties"),
                "GetAll",
                &("org.freedesktop.UPower.Device",),
            )
            .and_then(|r| r.body().deserialize())
        {
            Ok(p) => p,
            Err(_) => continue,
        };
        let mac = prop_str(&props, "Serial")
            .map(|s| s.to_ascii_uppercase())
            .filter(|s| crate::devices::device_path(s).is_some())
            .or_else(|| mac_in(&prop_str(&props, "NativePath").unwrap_or_default()));
        let pct = props
            .get("Percentage")
            .and_then(|v| f64::try_from(v).ok())
            .filter(|p| p.is_finite() && (0.0..=100.0).contains(p));
        if let (Some(mac), Some(pct)) = (mac, pct) {
            out.insert(mac, akm_core::conv::pct_u8(pct));
        }
    }
    out
}

/// The first `XX:XX:XX:XX:XX:XX` (or `XX_XX_...`) found in `s`, upper case with colons.
#[must_use]
pub fn mac_in(s: &str) -> Option<String> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(16)).find_map(|i| {
        let w = &b[i..i + 17];
        let ok = w.iter().enumerate().all(|(j, c)| {
            if j % 3 == 2 {
                *c == b':' || *c == b'_'
            } else {
                c.is_ascii_hexdigit()
            }
        });
        ok.then(|| {
            String::from_utf8_lossy(w)
                .replace('_', ":")
                .to_ascii_uppercase()
        })
    })
}

/// The `BlueZ` method that removes a pairing (`org.bluez.Adapter1`).
pub const FORGET_CALL: &str = "RemoveDevice";

/// Adapter object of a device object: `/org/bluez/hci0/dev_XX` -> `/org/bluez/hci0`.
#[must_use]
pub fn adapter_of(device_path: &str) -> Option<String> {
    let (adapter, dev) = device_path.rsplit_once('/')?;
    (dev.starts_with("dev_") && adapter.starts_with("/org/bluez/")).then(|| adapter.to_string())
}

static CONTROL: std::sync::OnceLock<Mutex<Option<Sender<KMsg>>>> = std::sync::OnceLock::new();

static STATUS: std::sync::OnceLock<SharedStatus> = std::sync::OnceLock::new();

/// A reconnection attempt is waiting for `BlueZ`: "Reconnect" would send nothing new.
pub fn reconnect_in_flight() -> bool {
    STATUS.get().is_some_and(connect_in_flight)
}

fn control() -> Option<Sender<KMsg>> {
    CONTROL.get().and_then(|m| {
        m.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    })
}

fn install_control(tx: Sender<KMsg>) {
    *CONTROL
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tx);
}

/// "Reconnect" (tray): one page now, at least 20 s after the previous one, never while asleep or
/// while the pairing is refused.
#[must_use]
pub fn user_reconnect() -> bool {
    control().is_some_and(|tx| tx.send(KMsg::Request).is_ok())
}

/// "Disconnect": `Device1.Disconnect` of `mac`; the daemon then pages nothing until asked.
#[must_use]
pub fn user_disconnect(mac: Option<&str>) -> bool {
    control().is_some_and(|tx| {
        tx.send(KMsg::UserDisconnect(mac.map(str::to_string)))
            .is_ok()
    })
}

pub(crate) fn user_forget(mac: &str) -> bool {
    control().is_some_and(|tx| tx.send(KMsg::UserForget(mac.to_string())).is_ok())
}

/// # Errors
/// The D-Bus error of adding a match rule.
pub fn add_rules(conn: &Connection) -> zbus::Result<()> {
    let dbus = DBusProxy::new(conn)?;
    for iface in [
        "org.bluez.Device1",
        "org.bluez.Adapter1",
        "org.bluez.Battery1",
    ] {
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
    for member in ["InterfacesRemoved", "InterfacesAdded"] {
        dbus.add_match_rule(
            MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender("org.bluez")?
                .interface("org.freedesktop.DBus.ObjectManager")?
                .member(member)?
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
    listen_on(&Connection::system()?, &Connection::system()?, tx)
}

/// The listener loop on explicit connections: the system bus, or a private bus in tests.
///
/// # Errors
/// The D-Bus error of adding the match rules.
#[allow(clippy::too_many_lines)] // one arm per D-Bus signal
pub fn listen_on(conn: &Connection, calls: &Connection, tx: &Sender<KMsg>) -> zbus::Result<()> {
    add_rules(conn)?;
    let it = MessageIterator::from(conn);
    let mut origin = Origin::new(calls, &["org.bluez"]);
    // Subscribed first, then enumerated: nothing falls in the gap.
    match enumerate(calls) {
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
        if !origin.accept(&msg) {
            continue;
        }
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
                    match enumerate(calls) {
                        Ok(l) => vec![KMsg::Sync(l)],
                        Err(_) => vec![KMsg::BluezGone],
                    }
                }
            }
            ("InterfacesRemoved", _) => {
                let Ok((gone, ifaces)) = msg
                    .body()
                    .deserialize::<(zbus::zvariant::OwnedObjectPath, Vec<String>)>()
                else {
                    continue;
                };
                if ifaces.iter().any(|i| i == "org.bluez.Device1") {
                    vec![KMsg::Removed(gone.to_string())]
                } else {
                    Vec::new()
                }
            }
            ("InterfacesAdded", _) => {
                let Ok((added, ifaces)) = msg
                    .body()
                    .deserialize::<(zbus::zvariant::OwnedObjectPath, HashMap<String, Props>)>()
                else {
                    continue;
                };
                if ifaces.contains_key("org.bluez.Device1") {
                    vec![KMsg::Added(added.to_string())]
                } else {
                    Vec::new()
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
                } else if i == "org.bluez.Battery1" {
                    if let Some(p) = changed.get("Percentage").and_then(|v| u8::try_from(v).ok()) {
                        v.push(KMsg::Battery(path.clone(), p));
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

#[derive(Clone)]
pub struct KeeperHandle {
    pub tx: Sender<KMsg>,
    pub shared: SharedStatus,
    /// Link statistics, for `Link.Quality()`.
    pub stats: Option<Arc<crate::linkq::Store>>,
}

/// Start the keeper (listener + logind + decision threads).
pub fn spawn(mailbox: Arc<Mailbox>, notify: bool) -> KeeperHandle {
    spawn_with(mailbox, notify, None, true, None)
}

/// [`spawn`] recording the disconnections in `stats`; `alert`.
pub fn spawn_with(
    mailbox: Arc<Mailbox>,
    notify: bool,
    stats: Option<Arc<crate::linkq::Store>>,
    alert: bool,
    shared: Option<SharedStatus>,
) -> KeeperHandle {
    prepare(stats, shared).start(mailbox, notify, alert)
}

/// A keeper whose handle exists (to export it) before its threads run; messages wait in its inbox.
pub struct Pending {
    handle: KeeperHandle,
    rx: Receiver<KMsg>,
}

/// The handle of a keeper started later by [`Pending::start`].
#[must_use]
pub fn prepare(stats: Option<Arc<crate::linkq::Store>>, shared: Option<SharedStatus>) -> Pending {
    let (tx, rx) = mpsc::channel::<KMsg>();
    // Given by the caller when the actor reads it too (roster).
    let shared: SharedStatus = shared.unwrap_or_default();
    Pending {
        handle: KeeperHandle { tx, shared, stats },
        rx,
    }
}

impl Pending {
    #[must_use]
    pub fn handle(&self) -> KeeperHandle {
        self.handle.clone()
    }

    /// Start the listener, logind and decision threads.
    pub fn start(self, mailbox: Arc<Mailbox>, notify: bool, alert: bool) -> KeeperHandle {
        let Pending { handle, rx } = self;
        let KeeperHandle {
            tx,
            shared,
            stats: kstats,
        } = handle.clone();
        install_control(tx.clone());
        let _ = STATUS.set(shared.clone());
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
            .spawn(move || {
                let mut k = Keeper::new(bus, sh, notify);
                if let Some(s) = kstats {
                    k.set_stats(s, alert);
                }
                run(k, &rx);
            });
        handle
    }
}

fn run<B: LinkBus>(mut k: Keeper<B>, rx: &Receiver<KMsg>) {
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
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        serde_json::Value::Array(s.iter().map(LinkStatus::to_json).collect()).to_string()
    }

    /// Everything kept about the link of each keyboard, as a JSON
    /// object keyed by MAC: `{summary, disconnects: [{ts, reason}],
    /// signal_by_hour: [{start, samples, mean, min, max}]}` (7 days). `{}`
    /// when the statistics are not kept.
    fn quality(&self) -> String {
        self.handle
            .stats
            .as_ref()
            .map_or_else(|| "{}".to_string(), |s| s.json(unix_now()).to_string())
    }

    /// Page the keyboard now (still rate-limited to one attempt per 20 s,
    /// never while sleeping or when the pairing is refused). `false` while an
    /// attempt is already waiting for `BlueZ`: nothing new would be sent.
    fn reconnect(&self) -> bool {
        if connect_in_flight(&self.handle.shared) {
            tracing::info!(
                "link: reconnection asked while an attempt is in flight: nothing new sent"
            );
            return false;
        }
        self.handle.tx.send(KMsg::Request).is_ok()
    }

    /// Disconnect the keyboard `mac` (`""` = every connected keyboard):
    /// `BlueZ` `Device1.Disconnect`. The daemon then does not page it until it
    /// comes back by itself or `Reconnect()` is called. Writes nothing
    /// to the keyboard.
    fn disconnect(&self, mac: &str) -> bool {
        let which = (!mac.is_empty()).then(|| mac.to_string());
        self.handle.tx.send(KMsg::UserDisconnect(which)).is_ok()
    }

    /// Ask to forget the keyboard `mac`: raises a notification "Forget the
    /// keyboard?" whose button, pressed within a minute, removes the pairing
    /// from `BlueZ`. This method never removes anything by itself.
    /// Returns false for an unknown keyboard.
    fn request_forget(&self, mac: &str) -> bool {
        let name = self
            .handle
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|s| s.mac.eq_ignore_ascii_case(mac))
            .map(|s| s.name.clone());
        match name {
            Some(name) => crate::forget::request(mac, &name),
            None => false,
        }
    }
}

/// Export the link object on the daemon's session connection.
///
/// # Errors
/// The D-Bus error of the export.
pub fn export(conn: &Connection, handle: KeeperHandle) -> zbus::Result<()> {
    conn.object_server().at(LINK_PATH, LinkIface { handle })?;
    Ok(())
}

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
        let mut v = vec![(*t).to_string()];
        v.extend(args.iter().map(std::string::ToString::to_string));
        v.push("akmctl".into());
        v.push("repair".into());
        v
    })
}

/// Open a terminal running the guided repair, in its own scope and reaped (tray action).
pub fn launch_repair(activation_token: Option<&str>) {
    if let Some(argv) = repair_command(|b| crate::spawn_ui::which(b).is_some()) {
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        match crate::spawn_ui::launch(&argv, activation_token) {
            Ok(()) => tracing::info!("guided repair started in {}", argv[0]),
            Err(e) => tracing::warn!("cannot start {}: {e}", argv[0]),
        }
    } else {
        tracing::warn!("no terminal found for akmctl repair");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connect_without_answer_ends_and_reconnect_is_honest() {
        let t = Instant::now();
        let r = with_timeout(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(3));
            1
        });
        assert_eq!(r, None);
        assert!(t.elapsed() < Duration::from_secs(1));
        assert_eq!(with_timeout(Duration::from_secs(1), || 2), Some(2));
        let shared = SharedStatus::default();
        assert!(!connect_in_flight(&shared));
        shared.lock().unwrap().push(LinkStatus {
            mac: "AA".into(),
            name: String::new(),
            health: "connecting".into(),
            since: 0,
            attempts: 1,
            failures: 0,
            last_error: String::new(),
            last_reason: String::new(),
            updated: 0,
            quality: None,
            connected: false,
            battery: None,
            paired: true,
            connecting: true,
        });
        assert!(connect_in_flight(&shared));
    }
    use akm_core::machine::{Action as MAction, Machine};
    use akm_core::recovery::{Health, MIN_SPACING};

    const MAC: &str = "AA:BB:CC:DD:EE:F1";
    const PATH: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_F1";

    struct Fake {
        world: Vec<DevInfo>,
        bluez_up: bool,
        connects: Vec<Instant>,
        notes: Vec<(String, Urgency)>,
        removed: Vec<String>,
        forgotten: Vec<String>,
        unstable: Vec<(String, usize)>,
        disconnects: Vec<String>,
        forgets: Vec<String>,
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
        fn removed(&mut self, name: &str) {
            self.removed.push(name.to_string());
        }
        fn forgotten(&mut self, mac: &str) {
            self.forgotten.push(mac.to_string());
        }
        fn notify_unstable(&mut self, name: &str, count: usize) {
            self.unstable.push((name.to_string(), count));
        }
        fn disconnect(&mut self, path: &str, _mac: &str) {
            self.disconnects.push(path.to_string());
        }
        fn forget(&mut self, path: &str, _mac: &str) {
            self.forgets.push(path.to_string());
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
            name: "Alice's keyboard #1".into(),
            connected,
            paired: true,
            battery: None,
        }
    }

    const MAC2: &str = "AA:BB:CC:DD:EE:F2";
    const PATH2: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_F2";

    fn kb2(connected: bool, battery: Option<u8>) -> DevInfo {
        DevInfo {
            path: PATH2.into(),
            mac: MAC2.into(),
            name: "Living room keyboard".into(),
            connected,
            paired: true,
            battery,
        }
    }

    #[test]
    fn two_keyboards_are_followed_and_one_leaving_does_not_affect_the_other() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        let mut first = kb(true);
        first.battery = Some(80);
        k.bus().world = vec![first.clone(), kb2(true, Some(12))];
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        let st = k.status(t0);
        assert_eq!(st.len(), 2);
        let of = |st: &[LinkStatus], mac: &str| st.iter().find(|s| s.mac == mac).unwrap().clone();
        assert_eq!(
            (of(&st, MAC).connected, of(&st, MAC).battery),
            (true, Some(80))
        );
        assert_eq!(
            (of(&st, MAC2).connected, of(&st, MAC2).battery),
            (true, Some(12))
        );
        assert_eq!(of(&st, MAC2).name, "Living room keyboard");
        k.handle(KMsg::Battery(PATH2.into(), 11), t0);
        assert_eq!(of(&k.status(t0), MAC2).battery, Some(11));
        assert_eq!(of(&k.status(t0), MAC).battery, Some(80));
        let t = t0 + Duration::from_mins(1);
        k.bus().world = vec![first.clone(), kb2(false, Some(11))];
        k.handle(
            KMsg::Disconnected(PATH2.into(), DisconnectReason::Timeout),
            t,
        );
        k.handle(KMsg::Connected(PATH2.into(), false), t);
        let st = k.status(t);
        assert_eq!(
            (of(&st, MAC2).connected, of(&st, MAC2).health.as_str()),
            (false, "dormant")
        );
        assert_eq!(
            (of(&st, MAC).connected, of(&st, MAC).health.as_str()),
            (true, "connected")
        );
        let mut t2 = t;
        for _ in 0..120 {
            t2 += Duration::from_secs(1);
            k.bus().now = t2;
            k.tick(t2);
        }
        let got = &k.bus().connects;
        assert!(!got.is_empty(), "{got:?}");
        assert_eq!(of(&k.status(t2), MAC).attempts, 0);
        assert!(of(&k.status(t2), MAC2).attempts >= 1);
        k.bus().world = vec![first, kb2(true, Some(11))];
        k.handle(KMsg::Connected(PATH2.into(), true), t2);
        k.bus().world = vec![kb2(true, Some(11))];
        k.handle(KMsg::Removed(PATH.into()), t2);
        let st = k.status(t2);
        assert_eq!(st.len(), 1);
        assert_eq!((st[0].mac.as_str(), st[0].connected), (MAC2, true));
        let j = st[0].to_json();
        assert_eq!(
            (j["connected"].as_bool(), j["battery"].as_u64()),
            (Some(true), Some(11))
        );
        assert_eq!(
            mac_in("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_F2").as_deref(),
            Some(MAC2)
        );
        assert_eq!(
            mac_in("hid-aa:bb:cc:dd:ee:f1-battery").as_deref(),
            Some(MAC)
        );
        assert_eq!(mac_in("keyboard_dev_xx"), None);
    }

    fn keeper(connected: bool, t0: Instant) -> Keeper<Fake> {
        let fake = Fake {
            world: vec![kb(connected)],
            bluez_up: true,
            connects: Vec::new(),
            notes: Vec::new(),
            removed: Vec::new(),
            forgotten: Vec::new(),
            unstable: Vec::new(),
            disconnects: Vec::new(),
            forgets: Vec::new(),
            machine: Machine::new(),
            clears: 0,
            now: t0,
        };
        let mut k = Keeper::new(fake, Arc::new(Mutex::new(Vec::new())), true);
        let base = unix_now();
        k.wall = Box::new(move |now| base + now.saturating_duration_since(t0).as_secs());
        k
    }

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

    #[allow(clippy::many_single_char_names)] // test fixtures with short local names
    #[test]
    fn reboot_scenario_pages_then_alerts_then_recovers() {
        let t0 = Instant::now();
        let mut k = keeper(false, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        assert_eq!(health(&k, t0), "dormant");
        let t = run_for(&mut k, t0, 15 * 60);
        assert_eq!(health(&k, t), "unreachable");
        let n = &k.bus().notes;
        assert_eq!(n.len(), 1, "{n:?}");
        assert_eq!(n[0].0, "Keyboard unreachable");
        let c = &k.bus().connects;
        assert!(c.len() >= 4 && c.len() <= 9, "{} pages in 15 min", c.len());
        for w in c.windows(2) {
            assert!(w[1] - w[0] >= MIN_SPACING);
        }
        k.bus().world = vec![kb(true)];
        k.handle(KMsg::Connected(PATH.into(), true), t);
        k.tick(t);
        assert_eq!(health(&k, t), "connected");
        assert_eq!(k.bus().notes.last().unwrap().0, "Keyboard reconnected");
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
            ("Keyboard: re-pairing needed".into(), Urgency::Critical)
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
        let got = &k.bus().notes;
        assert!(got.is_empty(), "{got:?}");
        assert!(k.bus().connects.len() <= 12 + 44 + 2);
    }

    #[test]
    fn issue_165_bluez_back_without_keyboard_stops_the_probing() {
        let t0 = Instant::now();
        let mut k = keeper(false, t0);
        k.bus().bluez_up = false;
        k.bus().machine.on_event(&Event::NoBluez, t0);
        assert!(k.bus().machine.is_connected());
        k.handle(KMsg::BluezGone, t0);
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
        let t1 = t0 + BOND_GRACE + Duration::from_millis(1);
        k.tick(t1);
        assert_eq!(health(&k, t1), Health::AuthFailed.as_str());
        assert_eq!(k.bus().notes.len(), 1);
        k.handle(KMsg::Sync(vec![]), t1);
        let got = k.status(t1);
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn forgetting_from_plasma_removes_the_keyboard_and_notifies_once() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        k.handle(KMsg::Disconnected(PATH.into(), DisconnectReason::Local), t0);
        k.handle(KMsg::Connected(PATH.into(), false), t0);
        k.handle(KMsg::Paired(PATH.into(), false), t0);
        k.tick(t0);
        let t1 = t0 + Duration::from_millis(40);
        k.bus().world = vec![];
        k.handle(KMsg::Removed(PATH.into()), t1);
        assert!(k.status(t1).is_empty(), "ghost keyboard gone immediately");
        assert_eq!(k.bus().removed, ["Alice's keyboard #1"]);
        assert_eq!(k.bus().forgotten.len(), 1);
        let t = run_for(&mut k, t1, 3600);
        assert!(
            k.bus().connects.is_empty(),
            "no reconnection to a forgotten device"
        );
        assert!(
            k.bus().notes.is_empty(),
            "no re-pairing notice: {:?}",
            k.bus().notes
        );
        assert!(k
            .next_deadline()
            .is_none_or(|d| d > t + Duration::from_secs(30)));
        k.handle(KMsg::Removed(PATH.into()), t);
        assert_eq!(k.bus().removed.len(), 1);
    }

    #[test]
    fn removal_of_a_planned_reconnection_cancels_it() {
        let t0 = Instant::now();
        let mut k = keeper(false, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        assert_eq!(health(&k, t0), "dormant");
        k.bus().world = vec![];
        k.handle(KMsg::Removed(PATH.into()), t0 + Duration::from_secs(1));
        run_for(&mut k, t0, 600);
        let got = &k.bus().connects;
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn a_pairing_that_stays_lost_still_asks_for_repair_after_the_grace() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        k.handle(KMsg::Paired(PATH.into(), false), t0);
        k.tick(t0 + BOND_GRACE / 2);
        assert!(k.bus().notes.is_empty(), "tolerance: nothing yet");
        k.tick(t0 + BOND_GRACE);
        assert_eq!(k.bus().notes.len(), 1);
        let got = &k.bus().removed;
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn paired_again_within_the_grace_cancels_the_bond_loss() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        k.handle(KMsg::Paired(PATH.into(), false), t0);
        k.handle(
            KMsg::Paired(PATH.into(), true),
            t0 + Duration::from_millis(200),
        );
        k.tick(t0 + BOND_GRACE * 2);
        let got = &k.bus().notes;
        assert!(got.is_empty(), "{got:?}");
        assert_ne!(health(&k, t0), Health::AuthFailed.as_str());
    }

    #[test]
    fn a_new_device_object_triggers_an_enumeration() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        k.handle(KMsg::Sync(vec![]), t0);
        let got = k.status(t0);
        assert!(got.is_empty(), "{got:?}");
        k.handle(KMsg::Added(PATH.into()), t0);
        assert_eq!(k.status(t0).len(), 1);
    }

    fn drop_and_return(k: &mut Keeper<Fake>, t: Instant, reason: DisconnectReason) -> Instant {
        k.bus().world = vec![kb(false)];
        k.handle(KMsg::Disconnected(PATH.into(), reason), t);
        k.handle(KMsg::Connected(PATH.into(), false), t);
        k.tick(t);
        let back = t + Duration::from_secs(30);
        k.bus().world = vec![kb(true)];
        k.handle(KMsg::Connected(PATH.into(), true), back);
        k.tick(back);
        back
    }

    fn keeper_with_stats(t0: Instant) -> (Keeper<Fake>, Arc<crate::linkq::Store>) {
        let mut k = keeper(true, t0);
        let stats = crate::linkq::Store::new(None);
        k.set_stats(stats.clone(), true);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        (k, stats)
    }

    #[test]
    fn four_link_losses_within_an_hour_raise_one_unstable_alert_per_episode() {
        let t0 = Instant::now();
        let (mut k, _stats) = keeper_with_stats(t0);
        let mut t = t0;
        for i in 0..3 {
            t = drop_and_return(
                &mut k,
                t + Duration::from_mins(5),
                DisconnectReason::Timeout,
            );
            assert!(k.bus().unstable.is_empty(), "{} is not more than 3", i + 1);
        }
        t = drop_and_return(
            &mut k,
            t + Duration::from_mins(5),
            DisconnectReason::Timeout,
        );
        assert_eq!(k.bus().unstable, [("Alice's keyboard #1".to_string(), 4)]);
        t = drop_and_return(
            &mut k,
            t + Duration::from_mins(2),
            DisconnectReason::Timeout,
        );
        run_for(&mut k, t, 600);
        assert_eq!(k.bus().unstable.len(), 1, "once per episode");
        let q = k.status(t)[0]
            .quality
            .clone()
            .expect("quality in the status");
        assert!(q.unstable);
        assert_eq!((q.disconnects_last_hour, q.unexpected_last_hour), (5, 5));
        let j = k.status(t)[0].to_json();
        assert_eq!(j["quality"]["disconnects_last_hour"], 5);
        let mut t = run_for(&mut k, t, 2 * 3600);
        assert!(!k.status(t)[0].quality.as_ref().unwrap().unstable);
        for _ in 0..4 {
            t = drop_and_return(
                &mut k,
                t + Duration::from_mins(1),
                DisconnectReason::Timeout,
            );
        }
        assert_eq!(k.bus().unstable.len(), 2);
    }

    #[test]
    fn the_two_bluez_signals_of_one_disconnection_are_counted_once() {
        let t0 = Instant::now();
        let (mut k, stats) = keeper_with_stats(t0);
        let t = t0 + Duration::from_mins(1);
        k.bus().world = vec![kb(false)];
        k.handle(KMsg::Connected(PATH.into(), false), t);
        k.handle(KMsg::Disconnected(PATH.into(), DisconnectReason::Remote), t);
        let unix = k.unix(t);
        let j = stats.json(unix);
        assert_eq!(j[MAC]["disconnects"].as_array().unwrap().len(), 1);
        assert_eq!(j[MAC]["disconnects"][0]["reason"], "remote", "refined");
        k.bus().world = vec![kb(true)];
        k.handle(
            KMsg::Connected(PATH.into(), true),
            t + Duration::from_secs(30),
        );
        let t = drop_and_return(
            &mut k,
            t + Duration::from_mins(1),
            DisconnectReason::Timeout,
        );
        let j = stats.json(k.unix(t));
        assert_eq!(j[MAC]["disconnects"].as_array().unwrap().len(), 2);
        assert_eq!(j[MAC]["disconnects"][1]["reason"], "timeout");
    }

    #[test]
    fn disconnections_for_system_sleep_never_raise_the_alert() {
        let t0 = Instant::now();
        let (mut k, stats) = keeper_with_stats(t0);
        let mut t = t0;
        for _ in 0..6 {
            t += Duration::from_mins(2);
            k.handle(KMsg::Sleep(SleepEvent::Sleeping), t);
            k.bus().world = vec![kb(false)];
            k.handle(
                KMsg::Disconnected(PATH.into(), DisconnectReason::Suspend),
                t,
            );
            t += Duration::from_mins(1);
            k.bus().world = vec![kb(true)];
            k.handle(KMsg::Sleep(SleepEvent::Resumed), t);
            k.tick(t);
        }
        let got = &k.bus().unstable;
        assert!(got.is_empty(), "{got:?}");
        let q = stats.quality(MAC, k.unix(t)).unwrap();
        assert_eq!((q.disconnects_last_hour, q.unexpected_last_hour), (6, 0));
        k.handle(KMsg::Sleep(SleepEvent::Sleeping), t);
        k.bus().world = vec![kb(false)];
        k.handle(KMsg::Sleep(SleepEvent::Resumed), t + Duration::from_mins(1));
        let j = stats.json(k.unix(t) + 60);
        assert_eq!(
            j[MAC]["disconnects"].as_array().unwrap().last().unwrap()["reason"],
            "suspend"
        );
    }

    #[test]
    fn user_disconnect_goes_to_bluez_and_is_not_undone_until_reconnect() {
        let t0 = Instant::now();
        let (mut k, stats) = keeper_with_stats(t0);
        let t = t0 + Duration::from_mins(1);
        k.handle(KMsg::UserDisconnect(Some(MAC.to_ascii_lowercase())), t);
        assert_eq!(
            k.bus().disconnects,
            [PATH],
            "Device1.Disconnect on the keyboard"
        );
        k.bus().world = vec![kb(false)];
        k.handle(
            KMsg::Disconnected(PATH.into(), DisconnectReason::Local),
            t + Duration::from_secs(1),
        );
        k.handle(
            KMsg::Connected(PATH.into(), false),
            t + Duration::from_secs(1),
        );
        assert_eq!(health(&k, t), "dormant");
        let j = stats.json(k.unix(t) + 2);
        assert_eq!(j[MAC]["disconnects"][0]["reason"], "user");
        // Six hours: the daemon never pages a keyboard the user disconnected.
        let t = run_for(&mut k, t, 6 * 3600);
        assert!(k.bus().connects.is_empty(), "paged against the user's will");
        assert!(k.bus().unstable.is_empty() && k.bus().notes.is_empty());
        k.handle(KMsg::UserDisconnect(None), t);
        assert_eq!(k.bus().disconnects.len(), 1);
        k.handle(KMsg::Request, t);
        k.bus().now = t;
        k.tick(t);
        assert_eq!(k.bus().connects.len(), 1);
        k.handle(
            KMsg::ConnectDone(MAC.into(), Err(ConnectError::NoAnswer)),
            t + Duration::from_secs(2),
        );
        k.handle(KMsg::Request, t + Duration::from_secs(5));
        k.tick(t + Duration::from_secs(5));
        assert_eq!(k.bus().connects.len(), 1, "5 s after the last page");
        k.tick(t + MIN_SPACING);
        assert_eq!(k.bus().connects.len(), 2);
    }

    #[test]
    fn user_disconnect_targets_connected_keyboards_only() {
        let t0 = Instant::now();
        let mut k = keeper(false, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        k.handle(KMsg::UserDisconnect(None), t0);
        assert!(
            k.bus().disconnects.is_empty(),
            "not connected: nothing to do"
        );
        let mut k = keeper(true, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        k.handle(KMsg::UserDisconnect(Some("AA:BB:CC:DD:EE:02".into())), t0);
        assert!(k.bus().disconnects.is_empty(), "another keyboard");
        k.handle(KMsg::UserDisconnect(None), t0);
        assert_eq!(k.bus().disconnects, [PATH]);
    }

    #[test]
    fn a_confirmed_forget_removes_the_device_through_the_adapter() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        k.handle(KMsg::UserForget("AA:BB:CC:DD:EE:02".into()), t0);
        assert!(k.bus().forgets.is_empty(), "unknown keyboard");
        k.handle(KMsg::UserForget(MAC.to_ascii_lowercase()), t0);
        assert_eq!(k.bus().forgets, [PATH]);
        let t1 = t0 + Duration::from_millis(200);
        k.bus().world = vec![];
        k.handle(KMsg::Disconnected(PATH.into(), DisconnectReason::Local), t1);
        k.handle(KMsg::Connected(PATH.into(), false), t1);
        k.handle(KMsg::Paired(PATH.into(), false), t1);
        k.handle(KMsg::Removed(PATH.into()), t1);
        let got = k.status(t1);
        assert!(got.is_empty(), "{got:?}");
        assert_eq!(
            k.bus().removed,
            ["Alice's keyboard #1"],
            "one notice: what to do next"
        );
        run_for(&mut k, t1, 3600);
        assert!(k.bus().connects.is_empty() && k.bus().notes.is_empty());
        assert_eq!(adapter_of(PATH).as_deref(), Some("/org/bluez/hci0"));
        assert_eq!(adapter_of("/org/bluez/hci0"), None);
        assert_eq!(adapter_of("/some/where/dev_AA"), None);
        assert_eq!(FORGET_CALL, concat!("Remove", "Device"));
        // Architecture guard (one forget path), on production tokens.
        let flat = akm_core::srclint::prod_tokens;
        let code = flat(include_str!("repair.rs"));
        assert_eq!(code.matches("KMsg::UserForget(mac.to_string())").count(), 1);
        assert!(code.contains("pub(crate)fnuser_forget"));
        assert!(flat(include_str!("forget.rs")).contains("crate::repair::user_forget(&p.mac)"));
        assert!(!flat(include_str!("notify.rs")).contains("user_forget"));
    }

    #[test]
    fn the_unstable_alert_can_be_turned_off() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        let stats = crate::linkq::Store::new(None);
        k.set_stats(stats.clone(), false);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        let mut t = t0;
        for _ in 0..5 {
            t = drop_and_return(
                &mut k,
                t + Duration::from_mins(1),
                DisconnectReason::Timeout,
            );
        }
        let got = &k.bus().unstable;
        assert!(got.is_empty(), "{got:?}");
        assert!(stats.quality(MAC, k.unix(t)).unwrap().unstable);
    }

    #[test]
    fn a_disconnection_after_a_long_sleep_is_dated_on_the_wall_clock() {
        use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
        let t0 = Instant::now();
        let (mut k, stats) = keeper_with_stats(t0);
        // `Instant` stops during suspend: after two hours asleep the wall clock is ahead of it.
        let slept = Arc::new(AtomicU64::new(0));
        let (base, s) = (unix_now(), slept.clone());
        k.wall = Box::new(move |now| {
            base + now.saturating_duration_since(t0).as_secs() + s.load(Relaxed)
        });
        slept.store(7200, Relaxed);
        let t = drop_and_return(
            &mut k,
            t0 + Duration::from_mins(1),
            DisconnectReason::Timeout,
        );
        let wall = base + 7200 + t.saturating_duration_since(t0).as_secs();
        let q = stats.quality(MAC, wall).unwrap();
        assert_eq!((q.disconnects_last_hour, q.unexpected_last_hour), (1, 1));
        let st = k.status(t)[0].quality.clone().unwrap();
        assert_eq!(
            st.disconnects_last_hour, 1,
            "Status() agrees with Quality()"
        );
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
        let (s, b, u) = notice_text(&Notice::RepairNeeded { why: "x".into() }, "Kb", t);
        assert!(s.contains("re-pairing") && b.contains("akmctl repair"));
        assert_eq!(u, Urgency::Critical);
        let (_, b, _) = notice_text(
            &Notice::Unreachable { since: t },
            "Kb",
            t + Duration::from_mins(11),
        );
        assert!(b.contains("11 min") && b.contains("akmctl doctor"));
    }
}
