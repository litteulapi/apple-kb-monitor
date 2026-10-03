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
//!   (`Status()` JSON, `Quality()` JSON, `Reconnect()`, `Disconnect(s mac)`,
//!   `RequestForget(s mac)`), read by `akmctl doctor` and the tray.
//!
//! The keeper never removes a pairing by itself: it connects, reports and
//! notifies. A removal is always the user's: `akmctl repair` after a typed
//! confirmation, or the tray's "Forget this keyboard" after the confirmation
//! of [`crate::forget`] (#104), which is the only sender of
//! [`KMsg::UserForget`]. "Disconnect" and "Reconnect" of the tray go through
//! the same keeper: `Device1.Disconnect`, and a page that keeps the spacing
//! of [`Recovery`].

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
/// BlueZ clears `Paired` just before it drops the object when a device is
/// forgotten. A `Paired=false` is believed (and the re-pairing notice
/// raised) only if the object is still there after this delay (#252).
pub const BOND_GRACE: Duration = Duration::from_millis(1500);
/// A disconnection within this delay of the user's request is the user's.
pub const USER_DOWN_WINDOW: Duration = Duration::from_secs(15);
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
    /// Battery percentage BlueZ (`Battery1`) or UPower already holds for it;
    /// never asked from the keyboard (#94).
    pub battery: Option<u8>,
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
    /// `org.bluez.Battery1.Percentage` of a device object changed (#94).
    Battery(String, u8),
    /// BlueZ removed the device object (Plasma "Forget", `bluetoothctl
    /// remove`): `ObjectManager.InterfacesRemoved` carrying `Device1` (#252).
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
    /// User asked to disconnect a keyboard (by MAC; `None` = every connected
    /// one): `Device1.Disconnect`, and no page until asked (#104).
    UserDisconnect(Option<String>),
    /// User CONFIRMED the removal of this keyboard (MAC), see
    /// [`crate::forget`]: `Adapter1.RemoveDevice` (#104).
    UserForget(String),
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
    /// The keyboard `name` was removed from the computer from outside; tell
    /// the user once what to do next (#252).
    fn removed(&mut self, _name: &str) {}
    /// The pairing of `mac` is gone from BlueZ: the rest of the daemon stops
    /// presenting it (C12). Must not block.
    fn forgotten(&mut self, _mac: &str) {}
    /// Start `Device1.Disconnect` on `path`. Must not block.
    fn disconnect(&mut self, _path: &str, _mac: &str) {}
    /// Start `Adapter1.RemoveDevice(path)`: the pairing is removed from this
    /// computer. Only called after the user's confirmation. Must not block.
    fn forget(&mut self, _path: &str, _mac: &str) {}
    /// The link of keyboard `name` keeps dropping: `count` disconnections
    /// within the last hour (#105). Told once per episode.
    fn notify_unstable(&mut self, _name: &str, _count: usize) {}
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
    /// Disconnection counts and signal of the last 7 days (#105).
    pub quality: Option<akm_core::linkstats::LinkQuality>,
    /// BlueZ says the keyboard is connected (#94).
    pub connected: bool,
    /// Battery percentage known to BlueZ / UPower, if any (#94).
    pub battery: Option<u8>,
    /// BlueZ `Device1.Paired` (or `Bonded`), as last seen (C6).
    pub paired: bool,
}

impl LinkStatus {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "mac": self.mac,
            "name": self.name,
            "health": self.health,
            "paired": self.paired,
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
    battery: Option<u8>,
    rec: Recovery,
    /// `Paired=false` seen at this instant: the bond loss is declared once
    /// the grace delay passes without an `InterfacesRemoved` (#252).
    unpair_at: Option<Instant>,
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
    /// Link statistics (#105) and whether the "unstable link" alert is told.
    stats: Option<Arc<crate::linkq::Store>>,
    notify_unstable: bool,
    /// Disconnections the user just asked for (MAC -> when): their reason.
    user_down: HashMap<String, Instant>,
    /// An instant and the unix time it was: unix time of any later instant.
    epoch: (Instant, u64),
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
            stats: None,
            notify_unstable: true,
            user_down: HashMap::new(),
            epoch: (Instant::now(), unix_now()),
        }
    }

    /// Record the disconnections in `stats` and raise the "unstable link"
    /// alert (more than 3 within an hour, once per episode) if `alert`.
    pub fn set_stats(&mut self, stats: Arc<crate::linkq::Store>, alert: bool) {
        self.stats = Some(stats);
        self.notify_unstable = alert;
    }

    /// Unix time of `now`.
    fn unix(&self, now: Instant) -> u64 {
        self.epoch.1 + now.saturating_duration_since(self.epoch.0).as_secs()
    }

    /// The reason recorded for a disconnection BlueZ explains by `reason`:
    /// what the daemon knows better comes first (system sleep, a forget in
    /// progress, the keyboard switched off by its button).
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

    /// The keyboard `mac` went from connected to disconnected.
    fn note_down(&self, mac: &str, reason: DisconnectReason, now: Instant) {
        if let Some(s) = self.stats.as_ref() {
            s.disconnect(mac, self.unix(now), self.down_reason(mac, reason, now));
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
        // Disconnections only this enumeration reveals (no signal seen).
        let mut downs: Vec<String> = Vec::new();
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
                            battery: info.battery,
                            rec,
                            unpair_at: None,
                        },
                    );
                }
            }
        }
        for mac in downs {
            self.note_down(&mac, DisconnectReason::Unknown, now);
        }
        // Removed from BlueZ (forgotten by the user): stop following, silently.
        let gone: Vec<String> = self.devs.keys().filter(|m| !seen.contains(m)).cloned().collect();
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
                    let was = std::mem::replace(&mut d.connected, c);
                    if c {
                        d.rec.on_connected(now);
                    } else {
                        d.rec.on_disconnected(DisconnectReason::Unknown, now);
                        if was {
                            self.note_down(&mac, DisconnectReason::Unknown, now);
                        }
                    }
                }
                None if c => self.resync(now), // a keyboard paired meanwhile
                None => {}
            },
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
                match self.devs.get_mut(&mac) {
                    Some(d) => {
                        tracing::warn!("link: removing {mac} from BlueZ (confirmed by the user)");
                        // No page while BlueZ removes it.
                        d.rec.on_user_disconnect(now);
                        let path = d.path.clone();
                        self.user_down.insert(mac.clone(), now);
                        self.bus.forget(&path, &mac);
                    }
                    None => tracing::warn!("link: {mac} is unknown to BlueZ, nothing to forget"),
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
        for (mac, d) in self.devs.iter_mut() {
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
        // Unstable link (#105): more than 3 disconnections within an hour,
        // told once per episode.
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
        // What BlueZ already holds (our own provider, or a GATT battery
        // service): read with the device, nothing is asked from the keyboard.
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

/// Battery percentages UPower holds in its cache for keyboards, by address
/// (upper case). Read-only properties on the system bus: UPower is not asked
/// to refresh and the keyboard is not asked anything (#94). Empty on error.
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
            out.insert(mac, pct.round() as u8);
        }
    }
    out
}

/// The first `XX:XX:XX:XX:XX:XX` (or `XX_XX_...`) found in `s`, upper case
/// with colons.
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

/// The BlueZ method that removes a pairing (`org.bluez.Adapter1`).
pub const FORGET_CALL: &str = "RemoveDevice";

/// Adapter object of a device object: `/org/bluez/hci0/dev_XX` -> `/org/bluez/hci0`.
pub fn adapter_of(device_path: &str) -> Option<String> {
    let (adapter, dev) = device_path.rsplit_once('/')?;
    (dev.starts_with("dev_") && adapter.starts_with("/org/bluez/")).then(|| adapter.to_string())
}

/// Sender of the running keeper, for the tray and the confirmation gate.
static CONTROL: std::sync::OnceLock<Mutex<Option<Sender<KMsg>>>> = std::sync::OnceLock::new();

fn control() -> Option<Sender<KMsg>> {
    CONTROL
        .get()
        .and_then(|m| m.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

fn install_control(tx: Sender<KMsg>) {
    *CONTROL
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(tx);
}

/// "Reconnect" (tray): one page now, at least 20 s after the previous one,
/// never while asleep or while the pairing is refused. False: no keeper.
pub fn user_reconnect() -> bool {
    control().is_some_and(|tx| tx.send(KMsg::Request).is_ok())
}

/// "Disconnect" (tray): `Device1.Disconnect` of `mac` (`None`: every
/// connected keyboard); the daemon then pages nothing until asked.
pub fn user_disconnect(mac: Option<&str>) -> bool {
    control().is_some_and(|tx| {
        tx.send(KMsg::UserDisconnect(mac.map(str::to_string)))
            .is_ok()
    })
}

/// Remove `mac` from BlueZ. Only [`crate::forget::confirmed`] calls this.
pub(crate) fn user_forget(mac: &str) -> bool {
    control().is_some_and(|tx| tx.send(KMsg::UserForget(mac.to_string())).is_ok())
}

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
    listen_on(Connection::system()?, Connection::system()?, tx)
}

/// The listener loop on explicit connections (`events` subscribes, `calls`
/// enumerates): the system bus, or a private bus in tests.
pub fn listen_on(conn: Connection, calls: Connection, tx: &Sender<KMsg>) -> zbus::Result<()> {
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
            ("InterfacesRemoved", _) => {
                let Ok((gone, ifaces)) = msg.body().deserialize::<(zbus::zvariant::OwnedObjectPath, Vec<String>)>()
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

/// Handle on the running keeper.
#[derive(Clone)]
pub struct KeeperHandle {
    pub tx: Sender<KMsg>,
    pub shared: SharedStatus,
    /// Link statistics, for `Link.Quality()` (#105).
    pub stats: Option<Arc<crate::linkq::Store>>,
}

/// Start the keeper (listener + logind + decision threads).
pub fn spawn(mailbox: Arc<Mailbox>, notify: bool) -> KeeperHandle {
    spawn_with(mailbox, notify, None, true, None)
}

/// [`spawn`] recording the disconnections in `stats` (#105); `alert`: raise
/// the "unstable link" notification.
pub fn spawn_with(
    mailbox: Arc<Mailbox>,
    notify: bool,
    stats: Option<Arc<crate::linkq::Store>>,
    alert: bool,
    shared: Option<SharedStatus>,
) -> KeeperHandle {
    let (tx, rx) = mpsc::channel::<KMsg>();
    install_control(tx.clone());
    // Given by the caller when the actor reads it too (roster, #94).
    let shared: SharedStatus = shared.unwrap_or_default();
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
    let kstats = stats.clone();
    let _ = std::thread::Builder::new()
        .name("kb-link".into())
        .spawn(move || {
            let mut k = Keeper::new(bus, sh, notify);
            if let Some(s) = kstats {
                k.set_stats(s, alert);
            }
            run(k, rx)
        });
    KeeperHandle { tx, shared, stats }
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

    /// Everything kept about the link of each keyboard (#105), as a JSON
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
    /// never while sleeping or when the pairing is refused).
    fn reconnect(&self) -> bool {
        self.handle.tx.send(KMsg::Request).is_ok()
    }

    /// Disconnect the keyboard `mac` (`""` = every connected keyboard):
    /// BlueZ `Device1.Disconnect`. The daemon then does not page it until it
    /// comes back by itself or `Reconnect()` is called (#104). Writes nothing
    /// to the keyboard.
    fn disconnect(&self, mac: &str) -> bool {
        let which = (!mac.is_empty()).then(|| mac.to_string());
        self.handle.tx.send(KMsg::UserDisconnect(which)).is_ok()
    }

    /// Ask to forget the keyboard `mac`: raises a notification "Forget the
    /// keyboard?" whose button, pressed within a minute, removes the pairing
    /// from BlueZ (#104). This method never removes anything by itself.
    /// Returns false for an unknown keyboard.
    fn request_forget(&self, mac: &str) -> bool {
        let name = self
            .handle
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
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

    const MAC: &str = "AA:BB:CC:DD:EE:F1";
    const PATH: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_F1";

    /// Fake BlueZ + desktop + acquisition machine (the real `Machine`).
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
            name: "Clavier de alice #1".into(),
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
            name: "Clavier du salon".into(),
            connected,
            paired: true,
            battery,
        }
    }

    /// #94 / #119: two keyboards are followed side by side; one leaving
    /// changes nothing for the other.
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
        assert_eq!(of(&st, MAC2).name, "Clavier du salon");
        // BlueZ pushes a new percentage for the second one only.
        k.handle(KMsg::Battery(PATH2.into(), 11), t0);
        assert_eq!(of(&k.status(t0), MAC2).battery, Some(11));
        assert_eq!(of(&k.status(t0), MAC).battery, Some(80));
        // The second keyboard loses its link: the first is untouched.
        let t = t0 + Duration::from_secs(60);
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
        // Only the lost one is paged.
        let mut t2 = t;
        for _ in 0..120 {
            t2 += Duration::from_secs(1);
            k.bus().now = t2;
            k.tick(t2);
        }
        assert!(!k.bus().connects.is_empty());
        assert_eq!(of(&k.status(t2), MAC).attempts, 0);
        assert!(of(&k.status(t2), MAC2).attempts >= 1);
        // It comes back, then the FIRST is removed: the second stays.
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
        // The address is found in what UPower calls the device.
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
        let t1 = t0 + BOND_GRACE + Duration::from_millis(1);
        k.tick(t1);
        assert_eq!(health(&k, t1), Health::AuthFailed.as_str());
        assert_eq!(k.bus().notes.len(), 1);
        k.handle(KMsg::Sync(vec![]), t1);
        assert!(k.status(t1).is_empty());
    }

    /// #252: Plasma "Forget" = Disconnected, Paired=false, then the object
    /// leaves BlueZ. The ghost keyboard disappears at once, nothing is paged,
    /// no "re-pairing needed" is raised, and one clear notice is sent.
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
        assert_eq!(k.bus().removed, ["Clavier de alice #1"]);
        // C12: the rest of the daemon is told, so it stops presenting it.
        assert_eq!(k.bus().forgotten.len(), 1);
        let t = run_for(&mut k, t1, 3600);
        assert!(k.bus().connects.is_empty(), "no reconnection to a forgotten device");
        assert!(k.bus().notes.is_empty(), "no re-pairing notice: {:?}", k.bus().notes);
        assert!(k.next_deadline().is_none_or(|d| d > t + Duration::from_secs(30)));
        // a second signal for the same object says nothing more
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
        assert!(k.bus().connects.is_empty());
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
        assert!(k.bus().removed.is_empty());
    }

    #[test]
    fn paired_again_within_the_grace_cancels_the_bond_loss() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        let w = k.bus().world.clone();
        k.handle(KMsg::Sync(w), t0);
        k.handle(KMsg::Paired(PATH.into(), false), t0);
        k.handle(KMsg::Paired(PATH.into(), true), t0 + Duration::from_millis(200));
        k.tick(t0 + BOND_GRACE * 2);
        assert!(k.bus().notes.is_empty());
        assert_ne!(health(&k, t0), Health::AuthFailed.as_str());
    }

    #[test]
    fn a_new_device_object_triggers_an_enumeration() {
        let t0 = Instant::now();
        let mut k = keeper(true, t0);
        k.handle(KMsg::Sync(vec![]), t0);
        assert!(k.status(t0).is_empty());
        k.handle(KMsg::Added(PATH.into()), t0);
        assert_eq!(k.status(t0).len(), 1);
    }

    /// One link loss then the keyboard back, `secs` later.
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

    /// #105: more than 3 disconnections within an hour raise "unstable link"
    /// once; a later burst is a new episode.
    #[test]
    fn four_link_losses_within_an_hour_raise_one_unstable_alert_per_episode() {
        let t0 = Instant::now();
        let (mut k, _stats) = keeper_with_stats(t0);
        let mut t = t0;
        for i in 0..3 {
            t = drop_and_return(
                &mut k,
                t + Duration::from_secs(300),
                DisconnectReason::Timeout,
            );
            assert!(k.bus().unstable.is_empty(), "{} is not more than 3", i + 1);
        }
        t = drop_and_return(
            &mut k,
            t + Duration::from_secs(300),
            DisconnectReason::Timeout,
        );
        assert_eq!(k.bus().unstable, [("Clavier de alice #1".to_string(), 4)]);
        // More of the same episode: nothing more.
        t = drop_and_return(
            &mut k,
            t + Duration::from_secs(120),
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
        // Two quiet hours, then a new burst: told again.
        let mut t = run_for(&mut k, t, 2 * 3600);
        assert!(!k.status(t)[0].quality.as_ref().unwrap().unstable);
        for _ in 0..4 {
            t = drop_and_return(
                &mut k,
                t + Duration::from_secs(60),
                DisconnectReason::Timeout,
            );
        }
        assert_eq!(k.bus().unstable.len(), 2);
    }

    /// #105: `Connected = false` and `Disconnected(reason)` are one
    /// disconnection, whatever their order.
    #[test]
    fn the_two_bluez_signals_of_one_disconnection_are_counted_once() {
        let t0 = Instant::now();
        let (mut k, stats) = keeper_with_stats(t0);
        let t = t0 + Duration::from_secs(60);
        k.bus().world = vec![kb(false)];
        // Property first, reason second.
        k.handle(KMsg::Connected(PATH.into(), false), t);
        k.handle(KMsg::Disconnected(PATH.into(), DisconnectReason::Remote), t);
        let unix = k.unix(t);
        let j = stats.json(unix);
        assert_eq!(j[MAC]["disconnects"].as_array().unwrap().len(), 1);
        assert_eq!(j[MAC]["disconnects"][0]["reason"], "remote", "refined");
        // Back, then reason first, property second.
        k.bus().world = vec![kb(true)];
        k.handle(
            KMsg::Connected(PATH.into(), true),
            t + Duration::from_secs(30),
        );
        let t = drop_and_return(
            &mut k,
            t + Duration::from_secs(60),
            DisconnectReason::Timeout,
        );
        let j = stats.json(k.unix(t));
        assert_eq!(j[MAC]["disconnects"].as_array().unwrap().len(), 2);
        assert_eq!(j[MAC]["disconnects"][1]["reason"], "timeout");
    }

    /// #105: going to sleep is not an unstable link.
    #[test]
    fn disconnections_for_system_sleep_never_raise_the_alert() {
        let t0 = Instant::now();
        let (mut k, stats) = keeper_with_stats(t0);
        let mut t = t0;
        for _ in 0..6 {
            t += Duration::from_secs(120);
            k.handle(KMsg::Sleep(SleepEvent::Sleeping), t);
            k.bus().world = vec![kb(false)];
            k.handle(
                KMsg::Disconnected(PATH.into(), DisconnectReason::Suspend),
                t,
            );
            t += Duration::from_secs(60);
            k.bus().world = vec![kb(true)];
            k.handle(KMsg::Sleep(SleepEvent::Resumed), t);
            k.tick(t);
        }
        assert!(k.bus().unstable.is_empty());
        let q = stats.quality(MAC, k.unix(t)).unwrap();
        assert_eq!((q.disconnects_last_hour, q.unexpected_last_hour), (6, 0));
        // A link that did not survive a sleep without any signal: same.
        k.handle(KMsg::Sleep(SleepEvent::Sleeping), t);
        k.bus().world = vec![kb(false)];
        k.handle(
            KMsg::Sleep(SleepEvent::Resumed),
            t + Duration::from_secs(60),
        );
        let j = stats.json(k.unix(t) + 60);
        assert_eq!(
            j[MAC]["disconnects"].as_array().unwrap().last().unwrap()["reason"],
            "suspend"
        );
    }

    /// #104: "Disconnect" calls BlueZ once, is recorded as the user's, and
    /// the daemon pages nothing until "Reconnect", which keeps its spacing.
    #[test]
    fn user_disconnect_goes_to_bluez_and_is_not_undone_until_reconnect() {
        let t0 = Instant::now();
        let (mut k, stats) = keeper_with_stats(t0);
        let t = t0 + Duration::from_secs(60);
        k.handle(KMsg::UserDisconnect(Some(MAC.to_ascii_lowercase())), t);
        assert_eq!(
            k.bus().disconnects,
            [PATH],
            "Device1.Disconnect on the keyboard"
        );
        // BlueZ reports it: reason Local, then the property.
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
        // Already disconnected: a second request calls nothing.
        k.handle(KMsg::UserDisconnect(None), t);
        assert_eq!(k.bus().disconnects.len(), 1);
        // "Reconnect": one page now; a second request 5 s later waits for the
        // 20 s spacing of the recovery machine.
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

    /// #104: `UserDisconnect(None)` disconnects every connected keyboard and
    /// only those; an unknown address does nothing.
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

    /// #104: the keeper removes a pairing only on `UserForget` (sent by the
    /// confirmation gate), for a keyboard it follows, and pages nothing while
    /// BlueZ removes it.
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
        // BlueZ: disconnected, unpaired, object removed.
        let t1 = t0 + Duration::from_millis(200);
        k.bus().world = vec![];
        k.handle(KMsg::Disconnected(PATH.into(), DisconnectReason::Local), t1);
        k.handle(KMsg::Connected(PATH.into(), false), t1);
        k.handle(KMsg::Paired(PATH.into(), false), t1);
        k.handle(KMsg::Removed(PATH.into()), t1);
        assert!(k.status(t1).is_empty());
        assert_eq!(
            k.bus().removed,
            ["Clavier de alice #1"],
            "one notice: what to do next"
        );
        run_for(&mut k, t1, 3600);
        assert!(k.bus().connects.is_empty() && k.bus().notes.is_empty());
        assert_eq!(adapter_of(PATH).as_deref(), Some("/org/bluez/hci0"));
        assert_eq!(adapter_of("/org/bluez/hci0"), None);
        assert_eq!(adapter_of("/some/where/dev_AA"), None);
        assert_eq!(FORGET_CALL, concat!("Remove", "Device"));
        // Nothing but the confirmation gate can send `UserForget`.
        let src = include_str!("repair.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert_eq!(code.matches("KMsg::UserForget(mac.to_string())").count(), 1);
        assert!(code.contains("pub(crate) fn user_forget"));
        let tray = [include_str!("forget.rs"), include_str!("notify.rs")].concat();
        assert!(tray.contains("crate::repair::user_forget(&p.mac)"));
        assert!(!include_str!("notify.rs").contains("user_forget"));
    }

    /// #105: with the alert turned off the counts are still kept.
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
                t + Duration::from_secs(60),
                DisconnectReason::Timeout,
            );
        }
        assert!(k.bus().unstable.is_empty());
        assert!(stats.quality(MAC, k.unix(t)).unwrap().unstable);
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
