//! The acquisition actor: the only code that touches the keyboard (#66).
//!
//! One thread owns every hardware access (hidraw Feature Reports, kernel
//! `power_supply`, RSSI helper) and the BlueZ battery provider. It sleeps
//! until a BlueZ / UPower event, a command or the next scheduled action
//! ([`Machine`]) and publishes an immutable [`Snapshot`] into a [`Watch`].
//! Consumers (D-Bus interface, UI) never block it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use akm_core::alerts::{AlertConfig, AlertState, Urgency};
use akm_core::chemistry::{self, Chemistry};
use akm_core::batteries::{self, Detector};
use akm_core::forecast::{self, Forecast};
use akm_core::history::{
    format_remaining, legacy_path, Clock, History, HistoryEntry, HistoryEvent, SystemClock,
    RETENTION_S,
};
use akm_core::link::LinkTracker;
use akm_core::machine::{Action, Event, Machine, NameReread, RSSI_MAX_AGE};
use akm_core::reminder::NoticeMemory;
use akm_core::rssi::{self, RssiTracker};
use akm_core::{discover, hidraw, led, power, KbReport, Snapshot, Watch};

use crate::alias::{AliasBackend, BluezAlias};
use crate::events::{DeviceEvent, EventHub};
use crate::{bluez, notify, powerdevil, watcher};

/// Messages handled by the actor thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// From the BlueZ / UPower watcher.
    Bus(Event),
    /// Explicit refresh (D-Bus `Refresh()`).
    Refresh,
    /// The name stored in the keyboard was rewritten (D-Bus `RereadName()`);
    /// the answer of the machine goes back through the reply slot.
    RereadName(NameReply),
    /// The BlueZ alias of a keyboard is now this (`None` = unknown).
    Alias(String, Option<String>),
    /// Stop the actor.
    Quit,
}

/// Where the actor answers a `RereadName()` (`none()` = nobody waits). All
/// replies compare equal: the message is what matters to `Msg: PartialEq`.
#[derive(Debug, Clone, Default)]
pub struct NameReply(Option<mpsc::SyncSender<NameReread>>);

impl PartialEq for NameReply {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl NameReply {
    pub fn none() -> Self {
        Self(None)
    }
    /// A reply slot and the end the caller waits on.
    pub fn channel() -> (Self, mpsc::Receiver<NameReread>) {
        let (tx, rx) = mpsc::sync_channel(1);
        (Self(Some(tx)), rx)
    }
    /// Give the answer (never blocks; a caller gone is not an error).
    pub fn answer(&self, r: NameReread) {
        if let Some(tx) = &self.0 {
            let _ = tx.try_send(r);
        }
    }
}

/// Where commands for the running actor go. The actor installs its sender at
/// start (also after a restart by the supervisor).
#[derive(Debug, Default)]
pub struct Mailbox(Mutex<Option<Sender<Msg>>>);

impl Mailbox {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    /// Install the sender of the running actor (done by the actor itself).
    pub fn install(&self, tx: Sender<Msg>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    }
    /// Send to the running actor; false if none is running.
    pub fn send(&self, m: Msg) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|tx| tx.send(m).is_ok())
    }
}

/// What the actor is allowed to do.
#[derive(Debug, Clone)]
pub struct Options {
    /// Register `org.bluez.BatteryProvider1` on the system bus.
    pub bluez_provider: bool,
    /// Desktop notifications (master switch) + CapsLock flash on low battery.
    pub notify: bool,
    /// Write the battery history (single writer).
    pub history: bool,
    /// Low-battery thresholds and hysteresis (#82).
    pub alerts: AlertConfig,
    /// Low-battery alerts on/off (signals and notifications).
    pub alerts_enabled: bool,
    /// Disconnected / reconnected notifications (#84).
    pub notify_connection: bool,
    /// "New batteries" notification (#85).
    pub notify_battery_replaced: bool,
    /// "Batteries changed too often" notification (#108).
    pub notify_battery_advice: bool,
    /// Defer percentage alerts to KDE PowerDevil when it covers this keyboard
    /// (`[notifications] defer_to_powerdevil`, #254).
    pub defer_to_powerdevil: bool,
    /// Does PowerDevil raise its own low-battery notification for this
    /// keyboard? Replaceable in tests.
    pub powerdevil: Arc<dyn crate::powerdevil::Probe>,
    /// Hours during which non-critical notifications are held
    /// (`[notifications] quiet_hours`, #91).
    pub quiet_hours: akm_core::quiet::QuietHours,
    /// Plasma OSD at a change of Fn mode / at a Caps Lock press
    /// (`[osd] fn_mode`, `[osd] caps_lock`; #100, #101).
    pub osd: crate::osd::Enabled,
    /// "Unstable link" notification (`[notifications] link_unstable`, #105).
    pub notify_link_unstable: bool,
    /// Link statistics shared with the link keeper (set by `main`, #105).
    pub link_stats: Option<Arc<crate::linkq::Store>>,
    /// Count the active minutes per day (`[usage] active_time`, #109).
    pub usage_active_time: bool,
    /// The counter, when the statistics are on (set by `main`).
    pub usage: Option<Arc<crate::usage::Tracker>>,
    /// Declared battery chemistry (`[battery] chemistry`, #178).
    pub chemistry: Chemistry,
    /// Publish the "Apple display" percentage (`[display] apple_percent`, #213).
    pub apple_percent: bool,
    /// Send `WillShutdown` at shutdown (`[apple] will_shutdown`, #191).
    pub will_shutdown: bool,
    /// After Apple's breaker trips, ask BlueZ once to disconnect the keyboard
    /// (`[apple] disconnect_on_breaker`, #251).
    pub disconnect_on_breaker: bool,
    /// How that disconnection is asked (BlueZ `Device1.Disconnect`; tests
    /// replace it).
    pub disconnect: fn(&str, Duration) -> Result<(), String>,
    /// Where detected events go (D-Bus device signals, tray...).
    pub events: Arc<EventHub>,
    /// Where the keyboard's alias is read (BlueZ).
    pub alias: Arc<dyn AliasBackend>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            bluez_provider: true,
            notify: true,
            history: true,
            alerts: AlertConfig::default(),
            alerts_enabled: true,
            notify_connection: true,
            notify_battery_replaced: true,
            notify_battery_advice: true,
            defer_to_powerdevil: true,
            powerdevil: Arc::new(crate::powerdevil::SystemProbe::default()),
            quiet_hours: akm_core::quiet::QuietHours::none(),
            osd: crate::osd::Enabled {
                fn_mode: true,
                caps_lock: true,
            },
            notify_link_unstable: true,
            link_stats: None,
            usage_active_time: false,
            usage: None,
            chemistry: Chemistry::default(),
            apple_percent: true,
            will_shutdown: true,
            disconnect_on_breaker: true,
            disconnect: watcher::request_disconnect,
            events: EventHub::new(),
            alias: Arc::new(BluezAlias::default()),
        }
    }
}

impl Options {
    /// Apply `config.toml` settings.
    pub fn apply_config(&mut self, c: &akm_core::config::Config) {
        self.alerts = c.alerts.clone();
        self.alerts_enabled = c.alerts_enabled;
        self.notify_connection = c.notify_connection;
        self.notify_battery_replaced = c.notify_battery_replaced;
        self.notify_battery_advice = c.notify_battery_advice;
        self.defer_to_powerdevil = c.defer_to_powerdevil;
        self.quiet_hours = c.quiet_hours.clone();
        self.usage_active_time = c.usage_active_time;
        self.notify_link_unstable = c.notify_link_unstable;
        self.osd = crate::osd::Enabled {
            fn_mode: c.osd_fn_mode,
            caps_lock: c.osd_caps_lock,
        };
        self.chemistry = c.chemistry;
        self.apple_percent = c.apple_percent;
        self.will_shutdown = c.will_shutdown;
        self.disconnect_on_breaker = c.disconnect_on_breaker;
    }
}

/// The event the passive publisher gets for a battery state read by GET Input
/// `0x30` (R2, #251): the same `BattStat` as the pushed `A1 30 xx`, so the
/// publisher's `battery_state_alert(old, new)` dedupes a repeat and alerts on
/// a rise only.
pub fn battery_state_event(k: &KbReport) -> Option<akm_core::passive::PassiveEvent> {
    k.battery
        .state
        .map(|value| akm_core::passive::PassiveEvent::BattStat { value })
}

/// Journal line for a change of the BlueZ alias from `old` to `new`:
/// `(warn, text)`. A change to the alias this program set last is the echo
/// of its own write (info); any other change was made elsewhere (KDE
/// Bluetooth settings, `bluetoothctl`, a removed pairing) and is a warning
/// naming what was expected, set by whom and when. `None`: no change, or
/// first reading.
pub fn alias_change_log(
    mac: &str,
    old: Option<&str>,
    new: Option<&str>,
    memory: &akm_core::alias::AliasMemory,
) -> Option<(bool, String)> {
    let old = old?;
    if Some(old) == new {
        return None;
    }
    let new_s = new.unwrap_or("(none)");
    Some(match memory.expected(mac) {
        Some(e) if Some(e.alias.as_str()) == new => (
            false,
            format!("BlueZ alias of {mac}: {old:?} -> {new_s:?} (set through this monitor by {})", e.by),
        ),
        Some(e) => (
            true,
            format!(
                "BlueZ alias of {mac} changed OUTSIDE this monitor: {old:?} -> {new_s:?}; expected {:?} (set by {} at {}). \
                 BlueZ does not tell who: KDE Bluetooth settings, bluetoothctl, or a removed and re-made pairing",
                e.alias, e.by, e.set_at
            ),
        ),
        None => (
            true,
            format!("BlueZ alias of {mac} changed outside this monitor: {old:?} -> {new_s:?} (no alias remembered)"),
        ),
    })
}

/// The "batteries changed too often" advice a replacement at `ts` raises:
/// the one of the history, only when this very replacement is what ended the
/// second short set (said once, not at every later replacement check).
pub fn replacement_advice(
    entries: &[HistoryEntry],
    ts: u64,
) -> Option<akm_core::advice::ShortLife> {
    akm_core::advice::short_life(&batteries::battery_sets(entries)).filter(|a| a.since == ts)
}

/// Minimum spacing between two history samples.
const HISTORY_SPACING: Duration = Duration::from_secs(300);
/// The published snapshot (LED, RSSI expiry) is refreshed at least this often.
const TICK: Duration = Duration::from_secs(5);

/// D-Bus `RereadName()` in the actor: while a keyboard is connected, `forget`
/// drops the four fragments `0x51`-`0x54` (claims and cached values) at once,
/// so the daemon never shows a name it knows to be stale, and the machine
/// schedules ONE read of these four reports: now, or at the end of the 30 s
/// floor (deferred, never dropped). Disconnected: nothing happens.
pub(crate) fn reread_name(
    machine: &mut Machine,
    now: Instant,
    forget: &mut dyn FnMut(),
) -> NameReread {
    let r = machine.request_name_reread(now);
    if r.accepted() {
        forget();
        tracing::info!("RereadName: 0x51-0x54 forgotten, {}", name_reread_text(r));
    }
    r
}

/// What `RereadName()` answers on D-Bus (second value of `(bs)`).
pub fn name_reread_text(r: NameReread) -> String {
    match r {
        NameReread::NotConnected => "no keyboard connected: nothing to read again".into(),
        NameReread::Now => "name read again now (0x51-0x54 only)".into(),
        NameReread::Deferred(d) if d.is_zero() => {
            "name read again as soon as the keyboard can be read (0x51-0x54 only)".into()
        }
        NameReread::Deferred(d) => format!(
            "name read again in {} s (0x51-0x54 only, at most once per {} s)",
            d.as_secs().max(1),
            akm_core::machine::NAME_REREAD_FLOOR.as_secs()
        ),
    }
}

fn unix_now() -> u64 {
    SystemClock.now()
}

struct Actor {
    opts: Options,
    /// Last report acquired. Kept (stale) after a failed read or a
    /// disconnection: `linked` tells whether it is current (#168).
    kb: Option<KbReport>,
    /// A full acquisition succeeded since the last (re)connection.
    linked: bool,
    rssi: RssiTracker,
    rssi_at: Option<u64>,
    provider: Option<bluez::BatteryProvider>,
    provider_mac: Option<String>,
    alerts: AlertState,
    detector: Detector,
    link: LinkTracker,
    forecast: Option<Forecast>,
    /// "Batteries changed too often" (#108), from the sets of the history.
    advice: Option<akm_core::advice::ShortLife>,
    installed_at: Option<u64>,
    history: Option<History>,
    last_history: Option<Instant>,
    last_rotation: Option<Instant>,
    remaining: Option<String>,
    remaining_at: Option<Instant>,
    last_update: u64,
    last_error: Option<String>,
    /// The keyboard reconnected and no reading was judged since: the next one
    /// may carry a firmware step that is not a discharge (#179).
    reconnected: bool,
    /// The single "estimate" reminder was sent for this set of batteries
    /// (PowerDevil covers the keyboard, #254).
    estimate_reminded: bool,
    /// "Change the batteries" / "firmware update" notices already shown,
    /// once per crossing (docs/NOTIFICATIONS.md).
    notices: NoticeMemory,
    /// Where `notices` is kept across restarts (single writer: only when
    /// the history is written too).
    notices_path: Option<std::path::PathBuf>,
    /// `alias.json` (alias last set through this program), read to journal
    /// an alias changed from outside; `None` in tests.
    alias_memory_path: Option<std::path::PathBuf>,
}

impl Actor {
    fn new(opts: Options) -> Self {
        let history = opts.history.then(|| {
            let h = History::open_default();
            match h.migrate_from(&legacy_path()) {
                Ok(true) => tracing::info!("history migrated to {}", h.path().display()),
                Ok(false) => {}
                Err(e) => tracing::warn!("history migration failed: {e}"),
            }
            h
        });
        // #180: the former `voltage` field was a constant; mark it unreliable.
        if let Some(h) = history.as_ref() {
            match h.mark_legacy_voltages() {
                Ok(0) => {}
                Ok(n) => tracing::info!("history: {n} legacy voltage(s) marked unreliable"),
                Err(e) => tracing::warn!("history voltage migration failed: {e}"),
            }
        }
        let past = history.as_ref().map(History::read).unwrap_or_default();
        let notices_path = opts.history.then(NoticeMemory::default_path);
        Self {
            notices: notices_path
                .as_deref()
                .map(NoticeMemory::load)
                .unwrap_or_default(),
            notices_path,
            alias_memory_path: opts
                .history
                .then(akm_core::alias::AliasMemory::default_path),
            alerts: AlertState::new(opts.alerts.clone()),
            detector: Detector::primed(&past),
            link: LinkTracker::new(),
            forecast: None,
            advice: akm_core::advice::short_life(&batteries::battery_sets(&past)),
            installed_at: batteries::current_set_start(&past),
            opts,
            kb: None,
            linked: false,
            rssi: RssiTracker::new(RSSI_MAX_AGE),
            rssi_at: None,
            provider: None,
            provider_mac: None,
            history,
            last_history: None,
            last_rotation: None,
            remaining: None,
            remaining_at: None,
            last_update: 0,
            last_error: None,
            reconnected: false,
            estimate_reminded: false,
        }
    }

    /// Charge estimate of a report for the declared chemistry (#178). The age
    /// of the set comes from the detected installation (#85).
    fn assess(&self, k: &KbReport, now_s: u64) -> chemistry::Assessment {
        chemistry::assess(
            k.battery.voltage_filtered_mv,
            k.battery.voltage_mv,
            self.opts.chemistry,
            self.installed_at.map(|t| now_s.saturating_sub(t)),
        )
    }

    /// Full read: HID Feature Reports, or kernel-only when hidraw is not
    /// readable. Returns true when a connected keyboard was acquired.
    fn acquire(&mut self, mac: Option<&str>) -> bool {
        let hid = hidraw::read_keyboard().filter(|k| {
            mac.is_none_or(|m| {
                k.device
                    .mac
                    .as_deref()
                    .is_some_and(|km| km.eq_ignore_ascii_case(m))
            })
        });
        let (report, err) = match hid {
            Some(k) => (Some(k), None),
            None => {
                let m = mac
                    .map(str::to_string)
                    .or_else(|| hidraw::find_apple_keyboard_mac_in(std::path::Path::new("/sys")));
                match m.as_deref().and_then(hidraw::report_from_sysfs) {
                    Some(k) if k.battery_pct().is_some() => {
                        (Some(k), Some("HID diagnostics unavailable (hidraw not readable or keyboard asleep): kernel battery only".to_string()))
                    }
                    _ => (None, Some("keyboard not reachable (no hidraw answer, no kernel battery)".to_string())),
                }
            }
        };
        gate_wake_monitor(report.as_ref(), mac);
        // Was a battery read really sent and left unanswered? (Not due, lock
        // busy, breaker open: nothing was asked.)
        let read_failed = akm_core::read_policy::peek_last_outcome()
            .is_some_and(akm_core::read_policy::SafeRead::attempted_and_failed);
        self.integrate(report, err, mac, read_failed)
    }

    /// Take the result of an acquisition (no hardware access here).
    /// `read_failed`: vendor reports were requested and not all answered.
    ///
    /// A report without any battery value keeps the last known level instead
    /// of showing "n/a" (#264), marked `battery.kept`: it is not a new
    /// measure, so no history sample, no alert pass and `last_update`
    /// unchanged (the age shown stays the one of the real read).
    fn integrate(
        &mut self,
        report: Option<KbReport>,
        err: Option<String>,
        mac: Option<&str>,
        read_failed: bool,
    ) -> bool {
        self.last_error = err;
        match report {
            Some(mut k) => {
                let mac = k.device.mac.clone();
                // The firmware version and thresholds are read once per
                // connection: a kernel-only fallback keeps what was learned.
                if k.firmware.version.is_none() && k.battery.thresholds.is_none() {
                    if let Some(prev) = self.kb.as_ref().filter(|p| p.device.mac == mac) {
                        k.firmware = prev.firmware.clone();
                        k.battery.thresholds = prev.battery.thresholds;
                        k.battery.threshold_level = prev.battery.threshold_level.clone();
                        k.battery.threshold_margins_mv = prev.battery.threshold_margins_mv;
                        k.battery.apple_display_pct =
                            prev.battery.apple_display_pct.and(k.battery.percentage).map(|p| {
                                akm_core::registry::apple_display_percent(p.round().clamp(0.0, 100.0) as u8)
                            });
                    }
                }
                let mut kept = false;
                if k.battery_pct().is_none() {
                    if let Some(prev) = self.kb.as_ref().filter(|p| p.device.mac == mac) {
                        if prev.battery_pct().is_some() {
                            k.battery = prev.battery.clone();
                            k.battery.kept = true;
                            // With the raw reports they were decoded from.
                            for (id, hex) in &prev.raw {
                                k.raw.entry(id.clone()).or_insert_with(|| hex.clone());
                            }
                            kept = true;
                            if read_failed {
                                self.last_error.get_or_insert_with(|| {
                                    "keyboard silent: battery level kept from the last read".into()
                                });
                            }
                        }
                    }
                }
                if !self.opts.apple_percent {
                    k.battery.apple_display_pct = None;
                }
                let pct = k.battery_pct();
                k.device.alias = mac.as_deref().and_then(|m| self.opts.alias.get(m));
                // The battery state READ (GET Input 0x30 after 0x47, Apple's
                // R2, #251) joins the pushed `A1 30 xx` in the passive
                // publisher: one state, one dedupe, the alerts of #189.
                // A kept state was already told when it was read.
                if let Some(ev) = battery_state_event(&k).filter(|_| !kept) {
                    if !crate::passive::inject(ev) {
                        tracing::debug!(
                            "battery state {:?} read, no passive publisher to tell",
                            k.battery.state
                        );
                    }
                }
                self.kb = Some(k);
                self.linked = true;
                if !kept {
                    self.last_update = unix_now();
                    self.after_battery_update(true);
                }
                if let Some(ev) = mac.and_then(|m| self.link.acquired(&m, pct)) {
                    self.link_event(ev);
                }
                true
            }
            // A failed read keeps the last value (and `linked`): `last_error`
            // says why, `last_update` how old the value is.
            None => {
                // Asked for another keyboard than the one kept: its value is
                // not current for the one now followed.
                if let (Some(want), Some(have)) =
                    (mac, self.kb.as_ref().and_then(|k| k.device.mac.as_deref()))
                {
                    if !want.eq_ignore_ascii_case(have) {
                        self.linked = false;
                    }
                }
                false
            }
        }
    }

    /// Apple's breaker tripped (3rd unanswered request in a row): nothing more
    /// goes to the keyboard, and, like macOS asking bluetoothd
    /// (`SetHIDDriverReady(false)`), BlueZ is asked once to disconnect it
    /// (#251). Returns whether the disconnection was asked.
    fn after_breaker(&mut self, mac: Option<&str>) -> bool {
        if !akm_core::read_policy::take_disconnect_request() {
            return false;
        }
        let kb_mac = self.kb.as_ref().and_then(|k| k.device.mac.clone());
        let Some(mac) = mac.map(str::to_string).or(kb_mac) else {
            tracing::warn!("circuit breaker open: keyboard address unknown, no disconnection asked");
            return false;
        };
        if !self.opts.disconnect_on_breaker {
            tracing::warn!(
                "circuit breaker open: 3 unanswered requests, nothing more is sent to {mac}; left connected ([apple] disconnect_on_breaker = false)"
            );
            return false;
        }
        tracing::warn!(
            "circuit breaker open: 3 unanswered requests, asking BlueZ to disconnect {mac} once (as macOS: SetHIDDriverReady(false))"
        );
        match (self.opts.disconnect)(&mac, akm_core::apple_model::APPLE.disconnect_call_timeout) {
            Ok(()) => tracing::info!("disconnection of {mac} requested"),
            Err(e) => tracing::warn!("disconnection of {mac} not done: {e} (not retried)"),
        }
        true
    }

    /// The BlueZ alias changed (rename, `bluetoothctl`, system settings).
    /// A change is journalled with what this program remembers having set
    /// (`alias.json`): BlueZ does not say who renamed the device.
    fn set_alias(&mut self, mac: &str, alias: Option<String>) {
        let memory = self
            .alias_memory_path
            .as_deref()
            .map(akm_core::alias::AliasMemory::load)
            .unwrap_or_default();
        if let Some(k) = self.kb.as_mut() {
            if k.device.mac.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(mac)) {
                if let Some((warn, line)) =
                    alias_change_log(mac, k.device.alias.as_deref(), alias.as_deref(), &memory)
                {
                    if warn {
                        tracing::warn!("{line}");
                    } else {
                        tracing::info!("{line}");
                    }
                }
                k.device.alias = alias;
            }
        }
    }

    /// Kernel power_supply capacity only (UPower signal).
    fn kernel_battery(&mut self) {
        let Some(k) = self.kb.as_mut().filter(|_| self.linked) else {
            return;
        };
        if let Some(r) = k.device.mac.as_deref().and_then(power::kernel_battery) {
            k.battery.percentage_fine = Some(f64::from(r.percent));
            k.battery.percentage = Some(f64::from(r.percent));
            k.battery.kept = false;
            if k.battery.apple_display_pct.is_some() {
                k.battery.apple_display_pct = Some(akm_core::registry::apple_display_percent(r.percent));
            }
            self.last_update = unix_now();
            self.after_battery_update(false);
        }
    }

    /// BlueZ reported the keyboard gone.
    fn disconnected(&mut self) {
        // `akmctl repair` forgets the keyboard (RecantConnection 0x41, #217):
        // the disconnection is expected and not notified, like macOS.
        if akm_core::link::take_expected_disconnect() {
            tracing::info!(
                "expected disconnection (akmctl repair, RecantConnection): not notified"
            );
            let _ = self.link.disconnected_as(false);
            self.clear();
            return;
        }
        // The keyboard announced its switch-off just before (`0x13` bit 1 = 0):
        // an Off, not a lost link (#190).
        self.disconnected_with(akm_core::link::take_keyboard_off());
    }

    fn disconnected_with(&mut self, powered_off: bool) {
        if let Some(ev) = self.link.disconnected_as(powered_off) {
            self.link_event(ev);
        }
        self.clear();
    }

    fn link_event(&mut self, ev: akm_core::link::LinkEvent) {
        tracing::info!("{ev:?}");
        if self.opts.notify && self.opts.notify_connection {
            notify::link(&ev);
        }
        self.opts.events.publish(DeviceEvent::Link(ev));
    }

    /// The keyboard is gone: its last report stays as a stale value (offline
    /// state of the tray, v2 forecast), the error text is brought up to date.
    fn clear(&mut self) {
        self.linked = false;
        self.reconnected = true;
        self.last_error = None;
        hidraw::set_wake_monitor_enabled(false);
        self.rssi.clear();
        self.rssi_at = None;
        hidraw::close_hid_fd();
        if let (Some(old), Some(bp)) = (self.provider_mac.take(), self.provider.as_ref()) {
            bp.remove(&old);
        }
    }

    /// History sample, BlueZ Battery1 export, low-battery alert.
    fn after_battery_update(&mut self, full_read: bool) {
        let Some(k) = self.kb.as_ref() else { return };
        let Some(pct) = k.battery_pct() else { return };
        let mac = k.device.mac.clone();
        // A voltage outside the plausible range is a misread ADC, not a sample.
        let voltage = k
            .battery
            .voltage
            .filter(|v| akm_core::history::VOLTAGE_RANGE.contains(v));
        let now = Instant::now();

        let due = self
            .last_history
            .is_none_or(|t| now.duration_since(t) >= HISTORY_SPACING);
        if (due || (full_read && voltage.is_some()))
            && akm_core::history::valid_sample(pct, voltage)
        {
            // Validate BEFORE the detector (#163); the detector state moves only
            // once the sample is stored.
            let ts = self.history.as_ref().map_or_else(unix_now, History::now);
            // The real voltages in mV (0x46 and 0x49), not the legacy constant (#180).
            let mv46 = voltage.and(k.battery.voltage_mv);
            let mv49 = k
                .battery
                .voltage_filtered_mv
                .filter(|mv| akm_core::history::VOLTAGE_RANGE.contains(&(f64::from(*mv) / 1000.0)));
            let mut entry = HistoryEntry::measured(ts, pct, mv46, mv49);
            if self.detector.check(&entry).is_some() {
                entry.event = Some(HistoryEvent::BatteryReplaced);
            }
            let stored = match self.history.as_ref().map(|h| h.append_entry(&entry)) {
                Some(Ok(true)) | None => {
                    self.last_history = Some(now);
                    self.remaining_at = None; // new data: recompute the forecast
                    true
                }
                Some(Ok(false)) => false,
                Some(Err(e)) => {
                    tracing::warn!("history append failed: {e}");
                    false
                }
            };
            if stored {
                if let Some(r) = self.detector.observe(&entry) {
                    self.on_replaced(mac.as_deref(), r);
                }
            }
        }
        let Some(k) = self.kb.as_ref() else { return };
        if let Some(h) = self.history.as_ref() {
            if self
                .last_rotation
                .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(24 * 3600))
            {
                self.last_rotation = Some(now);
                match h.rotate(RETENTION_S) {
                    Ok(0) => {}
                    Ok(n) => tracing::info!("history rotation: {n} old entries removed"),
                    Err(e) => tracing::warn!("history rotation failed: {e}"),
                }
            }
            // Size bound (5 MiB, #96): a `stat` per pass, a rewrite only over it.
            match h.enforce_size() {
                Ok(0) => {}
                Ok(n) => tracing::warn!(
                    "history over {} bytes: {n} oldest entries removed",
                    akm_core::history_limits::MAX_BYTES
                ),
                Err(e) => tracing::warn!("history size rotation failed: {e}"),
            }
        }

        if let (true, Some(mac)) = (self.opts.bluez_provider, k.device.mac.clone()) {
            if self.provider.is_none() {
                self.provider = bluez::BatteryProvider::spawn();
            }
            if let Some(bp) = self.provider.as_ref() {
                if self
                    .provider_mac
                    .as_deref()
                    .is_some_and(|m| !m.eq_ignore_ascii_case(&mac))
                {
                    // Another keyboard: withdraw the old object (MAC tracking).
                    if let Some(old) = self.provider_mac.take() {
                        bp.remove(&old);
                    }
                }
                bp.set_battery(&mac, pct.round().clamp(0.0, 100.0) as u8);
                self.provider_mac = Some(mac);
            }
        }

        // Alerts run on the charge estimated by the declared chemistry when
        // there is one, else on the keyboard's percentage (#178). The first
        // reading after a reconnection may carry a firmware step: it raises no
        // alert (#179).
        let after_reconnect = std::mem::take(&mut self.reconnected);
        if self.opts.alerts_enabled {
            let assessment = self.assess(k, self.history.as_ref().map_or_else(unix_now, History::now));
            let (alert_pct, basis) = chemistry::alert_pct(assessment.estimate.as_ref(), pct);
            if let Some(c) = self.alerts.update_after(alert_pct, after_reconnect) {
                tracing::warn!(
                    "low battery: {alert_pct:.0}% ({basis:?}, keyboard {pct:.0}%, threshold {}%)",
                    c.threshold
                );
                // The keyboard's own 0x30 alert, if it already announced the
                // same level, is the one the user got (#189).
                let fresh = akm_core::alerts::dedupe()
                    .allow_percent(akm_core::alerts::rank_of(c.urgency), unix_now());
                if !fresh {
                    tracing::info!("low battery {alert_pct:.0}%: not shown, the keyboard already announced it");
                }
                if self.opts.notify && fresh {
                    let covered = self.opts.defer_to_powerdevil && self.opts.powerdevil.covers();
                    match powerdevil::plan(
                        self.opts.defer_to_powerdevil,
                        covered,
                        basis == chemistry::AlertBasis::Estimate,
                        self.estimate_reminded,
                    ) {
                        powerdevil::Plan::Normal => notify::battery_crossing(&c, basis),
                        powerdevil::Plan::Reminder => {
                            self.estimate_reminded = true;
                            notify::battery_estimate(alert_pct, self.opts.powerdevil.low_level());
                        }
                        powerdevil::Plan::Skip => tracing::info!(
                            "low battery {alert_pct:.0}%: not shown, KDE PowerDevil already warns about this keyboard"
                        ),
                    }
                    // Apple's breaker blocks every emission, LED included (#251).
                    if c.urgency == Urgency::Critical && !akm_core::read_policy::tripped() {
                        led::flash_capslock_for(mac.clone(), 5);
                    }
                }
                if let Some(mac) = mac {
                    self.opts
                        .events
                        .publish(DeviceEvent::BatteryLevelCrossed { mac, crossing: c });
                }
            }
        }
        if self.opts.notify {
            for n in self.due_notices(notify::Lang::detect()) {
                // "Ignore this reminder" silences the Low reminder only.
                if n.event == notify::Event::BatteryReminder
                    && n.urgency != Urgency::Critical
                    && notify::reminder_suppressed(akm_core::reminder::ReminderLevel::Low)
                {
                    tracing::info!(
                        "battery reminder not shown: ignored by the user until new batteries"
                    );
                    continue;
                }
                notify::deliver(n);
            }
        } else {
            let _ = self.due_notices(notify::Lang::En);
        }
    }

    /// The "change the batteries" and "firmware update" notices due after
    /// this reading, each once per crossing (re-armed when the voltage climbs
    /// back, new batteries, or the firmware is up to date again). The memory
    /// is updated (and saved) even when notifications are off, so turning
    /// them on later does not replay old crossings.
    fn due_notices(&mut self, lang: notify::Lang) -> Vec<notify::Notification> {
        let mut out = Vec::new();
        let Some(k) = self.kb.as_ref().filter(|_| self.linked) else {
            return out;
        };
        let Some(mac) = k.device.mac.clone() else {
            return out;
        };
        let mut changed = false;
        // Battery: the keyboard's own thresholds (0x60 = 0x5A) against the
        // smoothed voltage 0x49, as macOS compares them.
        let mv = k.battery.voltage_filtered_mv.or(k.battery.voltage_mv);
        if let (Some(t), Some(mv), true) = (k.battery.thresholds, mv, self.opts.alerts_enabled) {
            let pct = k.battery_pct();
            let (due, ch) = self.notices.battery(&mac, mv, &t);
            changed |= ch;
            if let Some(r) = due {
                tracing::warn!(
                    "battery reminder: {mv} mV under the keyboard's {:?} threshold ({} mV), indication {:?} %",
                    r.level,
                    r.threshold_mv,
                    pct.map(f64::round)
                );
                out.push(notify::reminder_notification(&r, pct, lang));
            }
        }
        // Firmware: the embedded table says a newer public version exists.
        let fw = &k.firmware;
        let latest = (fw.status == "update_available")
            .then(|| fw.latest_known.clone())
            .flatten();
        let (due, ch) = self
            .notices
            .firmware(&mac, latest.as_deref(), fw.status == "up_to_date");
        changed |= ch;
        if due {
            let current = fw.version.clone().unwrap_or_else(|| "?".into());
            let latest = latest.unwrap_or_default();
            tracing::info!("firmware update known: {current} -> {latest}");
            out.push(notify::firmware_notification(&current, &latest, lang));
        }
        if changed {
            self.save_notices();
        }
        out
    }

    fn save_notices(&self) {
        if let Some(p) = self.notices_path.as_deref() {
            if let Err(e) = self.notices.save(p) {
                tracing::warn!("cannot save {}: {e}", p.display());
            }
        }
    }

    /// New batteries: re-arm the alerts, notify, publish.
    fn on_replaced(&mut self, mac: Option<&str>, r: batteries::Replacement) {
        tracing::info!(
            "battery replacement detected: {:?}% -> {:.0}%, {:?} V -> {:?} V",
            r.pct_before,
            r.pct_after,
            r.voltage_before,
            r.voltage_after
        );
        self.alerts.rearm_all();
        self.estimate_reminded = false;
        if mac.is_some_and(|m| self.notices.rearm_battery(m)) {
            self.save_notices();
        }
        akm_core::alerts::dedupe().reset();
        self.installed_at = Some(r.ts);
        if self.opts.notify && self.opts.notify_battery_replaced {
            notify::battery_replaced(&r);
        }
        // Second set in a row replaced within 30 days: say it once (#108).
        let entries = self.history.as_ref().map(History::read).unwrap_or_default();
        self.advice = replacement_advice(&entries, r.ts);
        if let Some(a) = self.advice.as_ref() {
            tracing::warn!("{}", akm_core::advice::line(a, false));
            if self.opts.notify && self.opts.notify_battery_advice {
                notify::battery_advice(a);
            }
        }
        if let Some(mac) = mac {
            self.opts.events.publish(DeviceEvent::BatteryReplaced {
                mac: mac.to_string(),
                replacement: r,
            });
        }
    }

    fn refresh_rssi(&mut self) {
        let Some(mac) = self.kb.as_ref().and_then(|k| k.device.mac.clone()) else {
            return;
        };
        let r = rssi::read_rssi(&mac);
        if let Some((rel, _)) = r {
            let now = unix_now();
            self.rssi_at = Some(now);
            // 7 days of relative signal, by the hour (#105).
            if let Some(stats) = self.opts.link_stats.as_ref() {
                stats.rssi(&mac, now, i32::from(rel));
            }
        }
        self.rssi.record(&mac, r, Instant::now());
    }

    /// Link quality of the keyboard followed, at publication time (#105).
    fn link_quality(&self) -> Option<akm_core::linkstats::LinkQuality> {
        let mac = self.kb.as_ref()?.device.mac.as_deref()?;
        self.opts.link_stats.as_ref()?.quality(mac, unix_now())
    }

    fn refresh_remaining(&mut self, now: Instant) {
        if self
            .remaining_at
            .is_some_and(|t| now.duration_since(t) < HISTORY_SPACING)
        {
            return;
        }
        self.remaining_at = Some(now);
        let entries = self.history.as_ref().map(History::read).unwrap_or_default();
        self.remaining = akm_core::history::estimate_remaining(&entries)
            .map(|(rate, hours)| format_remaining(rate, hours));
        self.forecast = forecast::estimate(&entries).ok();
        if self.history.is_some() {
            self.advice = akm_core::advice::short_life(&batteries::battery_sets(&entries));
        }
        if let Some(t) = batteries::current_set_start(&entries) {
            self.installed_at = Some(t);
        }
    }

    fn snapshot(&mut self) -> Snapshot {
        let now = Instant::now();
        self.refresh_remaining(now);
        let mut kb = self.kb.clone();
        let mut rssi_at = None;
        if let Some(k) = kb.as_mut().filter(|_| self.linked) {
            // RSSI is exposed only while fresh and taken from this very MAC.
            let cur = k
                .device
                .mac
                .as_deref()
                .and_then(|m| self.rssi.current(m, now));
            // BR/EDR: a gap in dB to the ideal range, not dBm (#174).
            k.radio.set_rssi_rel(cur.map(|c| c.0));
            k.radio.tx_power_dbm = cur.and_then(|c| c.1);
            k.bluetooth.rssi_dbus = None;
            k.bluetooth.tx_power_dbus = None;
            rssi_at = cur.and(self.rssi_at);
        }
        // Charge estimate by declared chemistry (#178), recomputed at every
        // publication so a change of set or of config shows at once.
        if let Some(k) = kb.as_mut() {
            let a = self.assess(k, self.history.as_ref().map_or_else(unix_now, History::now));
            k.battery.charge_estimate = a.estimate;
            k.battery.new_batteries = a.new_batteries;
        }
        let (caps, num) = if self.linked {
            led::read_led_state()
        } else {
            (false, false)
        };
        Snapshot {
            connected: self.linked,
            kb_error: (!self.linked).then(|| {
                if kb.is_some() {
                    "Keyboard disconnected".to_string()
                } else {
                    "Keyboard: not found".to_string()
                }
            }),
            keyboard: kb,
            caps_lock: caps,
            num_lock: num,
            remaining_display: self.remaining.clone(),
            rssi_at,
            last_update: self.last_update,
            last_error: self.last_error.clone(),
            forecast: self.forecast.clone(),
            batteries_installed_at: self.installed_at,
            battery_advice: self.advice.clone(),
            link_quality: self.link_quality(),
            usage: self.opts.usage.as_ref().map(|u| u.summary()),
            ..Default::default()
        }
    }
}

/// The wake monitor reads report 0x13: run it only for an acquired keyboard
/// whose descriptor declares it (BCM2042), never for a Magic Keyboard (#129).
fn gate_wake_monitor(report: Option<&KbReport>, mac: Option<&str>) {
    let mac = report.and_then(|k| k.device.mac.as_deref()).or(mac);
    hidraw::set_wake_monitor_enabled(
        report.is_some() && discover::wake_supported_in(std::path::Path::new("/sys"), mac),
    );
}

/// Handle on a running supervised actor.
pub struct ActorHandle {
    quit: Arc<AtomicBool>,
    mailbox: Arc<Mailbox>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ActorHandle {
    /// Ask the actor to stop and wait for it (provider unregistered, fd closed).
    pub fn stop(mut self) {
        self.shutdown();
    }

    pub fn mailbox(&self) -> Arc<Mailbox> {
        self.mailbox.clone()
    }

    fn shutdown(&mut self) {
        self.quit.store(true, Ordering::Relaxed);
        self.mailbox.send(Msg::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ActorHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Start the actor under a supervisor: a panicking worker is restarted after
/// 5 s (it used to die silently, freezing every consumer on stale data).
pub fn spawn(watch: Arc<Watch>, mailbox: Arc<Mailbox>, opts: Options) -> ActorHandle {
    let quit = Arc::new(AtomicBool::new(false));
    let (q, mb) = (quit.clone(), mailbox.clone());
    let thread = thread::Builder::new()
        .name("kb-supervisor".into())
        .spawn(move || {
            while !q.load(Ordering::Relaxed) {
                let (w, m, qf, o) = (watch.clone(), mb.clone(), q.clone(), opts.clone());
                let worker = thread::Builder::new()
                    .name("kb-actor".into())
                    .spawn(move || run(w, m, qf, o));
                match worker.map(|h| h.join()) {
                    Ok(Ok(())) => break,
                    Ok(Err(_)) => {
                        tracing::error!("keyboard actor panicked, restarting in 5 s");
                        let mut s = watch.get();
                        s.last_error = Some("keyboard thread crashed, restarting".into());
                        watch.publish(s);
                        thread::sleep(Duration::from_secs(5));
                    }
                    Err(e) => {
                        tracing::error!("cannot spawn keyboard actor: {e}");
                        thread::sleep(Duration::from_secs(5));
                    }
                }
            }
            hidraw::set_wake_monitor_enabled(false);
            hidraw::close_hid_fd();
            akm_core::read_policy::withdraw_breaker_state();
        })
        .expect("spawn kb-supervisor");
    ActorHandle {
        quit,
        mailbox,
        thread: Some(thread),
    }
}

/// After a system wake, has the keyboard stayed silent (no key press since)?
/// `since_wake` = time since the wake (`None` = no wake pending), `input_age`
/// = age of the last input report. While true no vendor report is requested:
/// a keyboard still in its "host asleep" mode answers nothing, and each
/// silence would count towards the breaker and a forced disconnection (#264).
///
/// `listening`: the passive listener runs, so a key press is known. Without
/// it nobody would ever lift the silence: never quiet, the reads follow the
/// model as they did before #264.
pub fn quiet_since_wake(
    since_wake: Option<Duration>,
    input_age: Option<Duration>,
    listening: bool,
) -> bool {
    if !listening {
        return false;
    }
    match (since_wake, input_age) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(w), Some(a)) => a >= w,
    }
}

/// Before the machine says what is due: hold every vendor read while the
/// keyboard is silent since the wake. The battery cycle is then not begun at
/// all (not consumed, not counted as a failure and retried 1 h later): it
/// goes out at the first pass after a key press. Clears `woke_at` once the
/// keyboard was heard. Returns whether the reads are held.
pub(crate) fn hold_while_quiet(
    machine: &mut Machine,
    woke_at: &mut Option<Instant>,
    input_age: Option<Duration>,
    listening: bool,
    now: Instant,
) -> bool {
    let quiet = quiet_since_wake(
        woke_at.map(|w| now.saturating_duration_since(w)),
        input_age,
        listening,
    );
    if !quiet {
        *woke_at = None;
    }
    machine.set_vendor_hold(quiet);
    quiet
}

/// After the read of an [`Action::RereadName`]. The node is gone: the machine
/// acquires again, as after any failed acquisition. The HID lock was busy
/// (another reader, nothing was sent): the request is put back and served at
/// the end of the floor, not lost. A read that was sent and failed is never
/// retried (the fragments stay unread in this connection).
pub(crate) fn after_name_reread(
    machine: &mut Machine,
    ok: bool,
    outcome: Option<akm_core::read_policy::SafeRead>,
    now: Instant,
) {
    use akm_core::read_policy::{Gate, SafeRead};
    if !ok {
        machine.acquire_done(false, now);
    } else if outcome == Some(SafeRead::Skipped(Gate::Busy)) {
        let _ = machine.request_name_reread(now);
    }
}

/// Event loop. While the keyboard is disconnected nothing keyboard-related runs.
fn run(watch: Arc<Watch>, mailbox: Arc<Mailbox>, quit: Arc<AtomicBool>, opts: Options) {
    let (tx, rx) = mpsc::channel::<Msg>();
    mailbox.install(tx.clone());
    watcher::spawn_signal_watcher(tx);
    let mut machine = Machine::new();
    let mut actor = Actor::new(opts);
    let mut was_paused = false;
    // Set at a system wake, cleared by a new connection or a key press: until
    // then the keyboard may still be in its "host asleep" mode, where it
    // answers no GET_REPORT (measured 2026-10-02, #264).
    let mut woke_at: Option<Instant> = None;
    watch.publish(actor.snapshot());

    loop {
        if quit.load(Ordering::Relaxed) {
            break;
        }
        let now = Instant::now();
        let wait = if crate::sleep::paused() {
            TICK // system sleep (#145): nothing is due, never spin
        } else {
            machine
                .next_deadline()
                .map_or(TICK, |d| d.saturating_duration_since(now).min(TICK))
        };
        match rx.recv_timeout(wait) {
            Ok(Msg::Bus(ev)) => {
                let before = (machine.is_connected(), machine.mac().map(str::to_string));
                if machine.on_event(&ev, Instant::now()) == Some(Action::Clear) {
                    tracing::info!("keyboard disconnected");
                    actor.disconnected();
                }
                // A keyboard newly followed: a new driver in Apple's terms
                // (breaker closed, counter 0; #214, #251).
                if machine.is_connected() && (!before.0 || machine.mac() != before.1.as_deref()) {
                    akm_core::read_policy::note_connection();
                    woke_at = None;
                }
            }
            Ok(Msg::Refresh) => {
                let _ = machine.force_refresh(Instant::now());
            }
            Ok(Msg::RereadName(reply)) => {
                reply.answer(reread_name(&mut machine, Instant::now(), &mut || {
                    akm_core::read_policy::forget_name_fragments()
                }));
            }
            Ok(Msg::Alias(mac, alias)) => actor.set_alias(&mac, alias),
            Ok(Msg::Quit) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        // System sleep (#145): no hardware access; the sleep handler waits
        // for this guard before letting the system go down.
        let paused = crate::sleep::paused();
        if paused != was_paused {
            // Apple's handleSleep / handleWake (#251): timer stopped, breaker
            // counter 1; battery read 60 s after the wake.
            was_paused = paused;
            if paused {
                machine.on_sleep();
                akm_core::read_policy::note_sleep();
            } else {
                machine.on_wake(Instant::now());
                woke_at = Some(Instant::now());
            }
        }
        let _io = (!paused).then(crate::sleep::io_guard);
        let due = if paused {
            Vec::new()
        } else {
            let now = Instant::now();
            hold_while_quiet(
                &mut machine,
                &mut woke_at,
                akm_core::read_policy::last_input_age(now),
                crate::passive::listener_started(),
                now,
            );
            machine.due(now)
        };
        for action in due {
            match action {
                Action::Acquire => {
                    let mac = machine.mac().map(str::to_string);
                    // The Apple model decides whether vendor reports are read
                    // (#251); a keyboard silent since a wake holds the cycle
                    // before it begins (`hold_while_quiet`).
                    akm_core::read_policy::set_schedule(Some(machine.vendor_reads_due()));
                    let ok = actor.acquire(mac.as_deref());
                    let read_ok = akm_core::read_policy::take_last_outcome()
                        .is_some_and(akm_core::read_policy::SafeRead::is_success);
                    machine.acquire_done_with(ok, Some(read_ok), Instant::now());
                    actor.after_breaker(mac.as_deref());
                    if ok {
                        let pct = actor.kb.as_ref().and_then(KbReport::battery_pct);
                        tracing::info!(
                            "acquired {} battery={}",
                            mac.as_deref().unwrap_or("?"),
                            pct.map_or("n/a".into(), |p| format!("{p:.0}%"))
                        );
                    }
                }
                Action::RereadName => {
                    // `0x51`-`0x54` alone: not a battery cycle, the model's
                    // timer and `Refresh()`'s floor are untouched.
                    let mac = machine.mac().map(str::to_string);
                    akm_core::read_policy::set_name_only_schedule();
                    let ok = actor.acquire(mac.as_deref());
                    let outcome = akm_core::read_policy::take_last_outcome();
                    // The name-only order must not outlive this read.
                    akm_core::read_policy::set_schedule(Some(false));
                    actor.after_breaker(mac.as_deref());
                    tracing::info!(
                        "name re-read for {}: {outcome:?} (node {})",
                        mac.as_deref().unwrap_or("?"),
                        if ok { "read" } else { "not reachable" }
                    );
                    after_name_reread(&mut machine, ok, outcome, Instant::now());
                }
                Action::KernelBattery => actor.kernel_battery(),
                Action::Rssi => actor.refresh_rssi(),
                Action::Clear => actor.clear(),
            }
        }
        watch.publish(actor.snapshot());
        // Notifications whose time has come: end of the quiet hours (#91),
        // "Remind me tomorrow" (#110). Not while the system goes to sleep.
        if actor.opts.notify && !paused {
            notify::tick();
        }
        // The breaker, published for akm-hid-control (root) and akmctl: Apple's
        // R3 applies to every emitter, HID_CONTROL and the forget included
        // (#244, #251). Written on a change or as a heartbeat only.
        if let Err(e) = akm_core::read_policy::publish_breaker_state(machine.mac()) {
            tracing::warn!("breaker state not published: {e}");
        }
    }
    akm_core::read_policy::withdraw_breaker_state();
    // Dropping the actor drops the BlueZ provider: unregistered cleanly.
    drop(actor);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet_actor() -> Actor {
        Actor::new(Options {
            bluez_provider: false,
            notify: false,
            history: false,
            ..Options::default()
        })
    }

    fn report(pct: f64, voltage: Option<f64>) -> KbReport {
        let mut k = KbReport::default();
        k.battery.percentage = Some(pct);
        k.battery.voltage = voltage;
        k.device.mac = Some("AA:BB:CC:DD:EE:F1".into());
        k.device.model = Some("Apple Wireless Keyboard (A1314)".into());
        k
    }

    /// #251: the read `0x30` becomes the passive `BattStat` event (same path
    /// as the pushed report); nothing when the burst did not read it.
    #[test]
    fn the_read_battery_state_feeds_the_passive_publisher_once() {
        use akm_core::passive::{battery_state_alert, PassiveEvent, PassiveState};
        let mut k = report(50.0, None);
        assert_eq!(battery_state_event(&k), None);
        k.battery.state = Some(1);
        assert_eq!(
            battery_state_event(&k),
            Some(PassiveEvent::BattStat { value: 1 })
        );
        // without a publisher started (tests) the injection reports false
        assert!(!crate::passive::inject(PassiveEvent::BattStat { value: 1 }));
        // and the publisher's dedupe: the same state read again raises nothing,
        // a rise does (what the pushed report already gets, #189)
        let mut st = PassiveState::default();
        st.apply(PassiveEvent::BattStat { value: 1 }, 1);
        assert!(
            battery_state_alert(st.batt_stat, 1).is_none(),
            "repeat: no second alert"
        );
        assert!(
            battery_state_alert(st.batt_stat, 2).is_some(),
            "rise: alert"
        );
        assert!(
            battery_state_alert(Some(2), 1).is_none(),
            "de-escalation: nothing"
        );
    }

    /// An alias changed behind the monitor's back is journalled as a warning
    /// with what was expected and who set it; the echo of our own write is
    /// info; no change, or the first reading, says nothing.
    #[test]
    fn alias_changes_are_journalled_with_the_expected_alias() {
        use akm_core::alias::AliasMemory;
        const M: &str = "AA:BB:CC:DD:EE:F1";
        let mut mem = AliasMemory::default();
        assert_eq!(alias_change_log(M, None, Some("x"), &mem), None);
        assert_eq!(alias_change_log(M, Some("x"), Some("x"), &mem), None);
        let (warn, l) =
            alias_change_log(M, Some("Bureau"), Some("Clavier de alice"), &mem).unwrap();
        assert!(warn && l.contains("no alias remembered"), "{l}");
        mem.remember(M, "Bureau", ":1.9 pid 4 (kcmshell6)", 100);
        let (warn, l) =
            alias_change_log(M, Some("Clavier de alice"), Some("Bureau"), &mem).unwrap();
        assert!(!warn && l.contains("kcmshell6"), "{l}");
        let (warn, l) =
            alias_change_log(M, Some("Bureau"), Some("Clavier de alice"), &mem).unwrap();
        assert!(
            warn && l.contains("OUTSIDE") && l.contains("\"Bureau\"") && l.contains("at 100"),
            "{l}"
        );
        // The actor of the tests reads no alias.json (history off).
        let mut a = quiet_actor();
        assert!(a.alias_memory_path.is_none());
        a.kb = Some(report(50.0, None));
        a.set_alias(M, Some("Bureau".into()));
        assert_eq!(
            a.kb.as_ref().unwrap().device.alias.as_deref(),
            Some("Bureau")
        );
    }

    #[test]
    fn no_vendor_read_after_a_wake_until_a_key_press() {
        let s = Duration::from_secs;
        assert!(!quiet_since_wake(None, None, true), "no wake pending");
        assert!(!quiet_since_wake(None, Some(s(900)), true));
        assert!(quiet_since_wake(Some(s(60)), None, true), "never typed");
        assert!(
            quiet_since_wake(Some(s(60)), Some(s(600)), true),
            "typed before the sleep"
        );
        assert!(quiet_since_wake(Some(s(60)), Some(s(60)), true));
        assert!(
            !quiet_since_wake(Some(s(60)), Some(s(5)), true),
            "typed since the wake"
        );
        // No passive listener: nobody would tell about a key press, never quiet.
        assert!(!quiet_since_wake(Some(s(60)), None, false));
        assert!(!quiet_since_wake(Some(s(60)), Some(s(600)), false));
    }

    /// M4 of the final review: a wake, the first key 2 min later. The read
    /// due 60 s after the wake is held (not consumed, no 1 h retry) and goes
    /// out at the first pass after the key press.
    #[test]
    fn the_read_after_a_wake_waits_for_the_first_key_press_then_goes_out() {
        let s = Duration::from_secs;
        let t0 = Instant::now();
        let mac = "AA:BB:CC:DD:EE:F1";
        let ready = |t0: Instant| {
            let mut m = Machine::new();
            m.on_event(&Event::Connected(mac.into()), t0);
            m.due(t0);
            m.acquire_done(true, t0);
            m.on_sleep();
            m.on_wake(t0 + s(100));
            m
        };
        let mut m = ready(t0);
        let mut woke_at = Some(t0 + s(100));
        // 60 s after the wake, last key 10 min ago: held, nothing consumed.
        let t = t0 + s(160);
        assert!(hold_while_quiet(
            &mut m,
            &mut woke_at,
            Some(s(600)),
            true,
            t
        ));
        assert!(woke_at.is_some());
        assert!(!m.due(t).contains(&Action::Acquire));
        assert_eq!(m.link().next_battery(), Some(t0 + s(160)), "still due");
        // 2 min after the wake a key was pressed 1 s ago: released, read now.
        let t = t0 + s(220);
        assert!(!hold_while_quiet(&mut m, &mut woke_at, Some(s(1)), true, t));
        assert_eq!(woke_at, None);
        assert!(m.due(t).contains(&Action::Acquire));
        assert!(m.vendor_reads_due(), "the battery read itself");

        // Passive listener not started: nothing is held, the read goes out
        // 60 s after the wake as it did before #264.
        let mut m = ready(t0);
        let mut woke_at = Some(t0 + s(100));
        let t = t0 + s(160);
        assert!(!hold_while_quiet(&mut m, &mut woke_at, None, false, t));
        assert_eq!(woke_at, None);
        assert!(m.due(t).contains(&Action::Acquire));
        assert!(m.vendor_reads_due());
    }

    /// M1 of the final review: a level kept from an earlier read is not a
    /// new measure (no history sample, age unchanged), and the keyboard is
    /// called silent only when a read was really sent and failed.
    #[test]
    fn a_kept_battery_level_is_marked_and_never_recorded_as_a_measure() {
        const M: &str = "AA:BB:CC:DD:EE:F1";
        let mut a = quiet_actor();
        assert!(a.integrate(Some(report_mv(60.0, 2600)), None, Some(M), false));
        assert!(a.last_history.is_some(), "a real read is a sample");
        assert!(a.last_update > 0);
        assert!(!a.kb.as_ref().unwrap().battery.kept);
        a.last_history = None;
        a.last_update = 1234;
        // An acquisition that read nothing (not due, lock busy, breaker open).
        let mut blank = KbReport::default();
        blank.device.mac = Some(M.into());
        assert!(a.integrate(Some(blank.clone()), None, Some(M), false));
        let b = &a.kb.as_ref().unwrap().battery;
        assert_eq!(b.percentage, Some(60.0), "last level kept");
        assert_eq!(b.voltage_filtered_mv, Some(2600));
        assert!(b.kept, "and marked as kept");
        assert!(
            a.last_history.is_none(),
            "no history sample for a kept value"
        );
        assert_eq!(
            a.last_update, 1234,
            "the age stays the one of the real read"
        );
        assert_eq!(a.last_error, None, "nothing was asked: not silent");
        assert!(a.snapshot().keyboard.unwrap().battery.kept);
        // Kept twice in a row: same thing.
        assert!(a.integrate(Some(blank.clone()), None, Some(M), false));
        assert!(a.last_history.is_none());
        assert_eq!(a.last_update, 1234);
        // A read really sent and left unanswered: the message says so.
        assert!(a.integrate(Some(blank), None, Some(M), true));
        assert!(a
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("keyboard silent")));
        assert!(a.last_history.is_none());
        assert_eq!(a.last_update, 1234);
        // The next real read is a measure again.
        assert!(a.integrate(Some(report_mv(59.0, 2590)), None, Some(M), false));
        assert!(!a.kb.as_ref().unwrap().battery.kept);
        assert!(a.last_history.is_some());
        assert!(a.last_update > 1234);
        assert_eq!(a.last_error, None);
        // Another keyboard: nothing of the previous one is kept.
        let mut other = KbReport::default();
        other.device.mac = Some("AA:BB:CC:DD:EE:02".into());
        assert!(a.integrate(Some(other), None, None, true));
        assert_eq!(a.kb.as_ref().unwrap().battery_pct(), None);
        assert!(!a.kb.as_ref().unwrap().battery.kept);
    }

    /// #105: the published state carries the link quality of the keyboard
    /// followed, and nothing while no statistics are kept.
    #[test]
    fn snapshot_carries_the_link_quality_of_the_keyboard_followed() {
        const M: &str = "AA:BB:CC:DD:EE:F1";
        let mut a = quiet_actor();
        a.kb = Some(report(50.0, None));
        a.linked = true;
        assert_eq!(a.snapshot().link_quality, None, "no statistics kept");
        let stats = crate::linkq::Store::new(None);
        a.opts.link_stats = Some(stats.clone());
        assert_eq!(a.snapshot().link_quality, None, "nothing recorded yet");
        let now = unix_now();
        stats.disconnect(M, now - 120, "timeout");
        stats.rssi(M, now - 60, -4);
        let q = a.snapshot().link_quality.expect("link quality");
        assert_eq!((q.disconnects_last_hour, q.disconnects_last_day), (1, 1));
        assert_eq!(
            q.signal_7d.as_ref().map(|s| (s.samples, s.min)),
            Some((1, -4))
        );
        assert!(!q.unstable);
        let json = serde_json::to_value(a.snapshot()).unwrap();
        assert_eq!(
            json["link_quality"]["disconnects_by_hour"]
                .as_array()
                .unwrap()
                .len(),
            24
        );
        assert_eq!(
            json["link_quality"]["disconnects_by_day"]
                .as_array()
                .unwrap()
                .len(),
            7
        );
    }

    /// #108: the advice is raised by the replacement that ends the second
    /// short set, and by that one only.
    #[test]
    fn the_advice_is_raised_once_by_the_replacement_that_ends_the_second_short_set() {
        const DAY: u64 = 86_400;
        let t0 = 1_780_000_000u64;
        let mut h = Vec::new();
        let mut set = |start: u64, days: u64| {
            for i in 0..days * 4 {
                let pct = 100.0 - 90.0 * i as f64 / (days * 4) as f64;
                h.push(HistoryEntry::sample(start + i * DAY / 4, pct, None));
            }
        };
        set(t0, 40);
        set(t0 + 40 * DAY, 12);
        set(t0 + 52 * DAY, 18);
        // First sample on the third set of fresh cells.
        h.push(HistoryEntry::sample(t0 + 70 * DAY, 100.0, None));
        let a = replacement_advice(&h, t0 + 70 * DAY).expect("advice");
        assert_eq!(a.days, [12.0, 18.0]);
        assert_eq!(
            replacement_advice(&h, t0 + 52 * DAY),
            None,
            "an older replacement"
        );
        assert_eq!(
            replacement_advice(&h[..h.len() - 1], t0 + 52 * DAY),
            None,
            "one short set"
        );
        // The published state carries it.
        let mut actor = quiet_actor();
        actor.advice = Some(a.clone());
        assert_eq!(actor.snapshot().battery_advice, Some(a));
    }

    #[test]
    fn disconnection_keeps_the_last_value_and_refreshes_errors() {
        // #168
        let mut a = quiet_actor();
        a.kb = Some(report(42.0, Some(2.8)));
        a.linked = true;
        a.last_update = 1234;
        a.last_error = Some("HID diagnostics unavailable (stale)".into());
        let s = a.snapshot();
        assert!(s.connected && s.kb_error.is_none());
        a.disconnected();
        let s = a.snapshot();
        assert!(!s.connected);
        assert_eq!(s.battery_pct(), Some(42.0), "last value kept");
        assert_eq!(s.last_update, 1234, "its age is known");
        assert_eq!(s.last_error, None);
        assert_eq!(s.kb_error.as_deref(), Some("Keyboard disconnected"));
        assert_eq!(s.mac(), Some("AA:BB:CC:DD:EE:F1"));
        // never seen: the old message
        let mut b = quiet_actor();
        assert_eq!(b.snapshot().kb_error.as_deref(), Some("Keyboard: not found"));
    }

    #[test]
    fn invalid_sample_never_reaches_the_detector() {
        // #163: raw byte 255 and 211 V are not samples.
        let mut a = quiet_actor();
        a.kb = Some(report(40.0, Some(2.8)));
        a.after_battery_update(true);
        a.kb = Some(report(255.0, None));
        a.after_battery_update(true);
        a.kb = Some(report(41.0, Some(211.0)));
        a.after_battery_update(true);
        assert_eq!(a.detector.last_replacement(), None);
        assert_eq!(a.installed_at, None);
    }

    fn report_mv(fw_pct: f64, mv_slow: u32) -> KbReport {
        let mut k = report(fw_pct, Some(f64::from(mv_slow + 37) / 1000.0));
        k.battery.voltage_mv = Some(mv_slow + 37);
        k.battery.voltage_filtered_mv = Some(mv_slow);
        k
    }

    fn crossings(rx: &mpsc::Receiver<DeviceEvent>) -> Vec<u8> {
        rx.try_iter()
            .filter_map(|e| match e {
                DeviceEvent::BatteryLevelCrossed { crossing, .. } => Some(crossing.threshold),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn snapshot_carries_the_estimate_next_to_the_firmware_percentage() {
        // #178: firmware 50 % at 2404 mV is about 25 % of real charge (alkaline).
        let mut a = quiet_actor();
        a.kb = Some(report_mv(50.0, 2404));
        a.linked = true;
        let s = a.snapshot();
        let b = &s.keyboard.as_ref().unwrap().battery;
        assert_eq!(b.percentage, Some(50.0), "the indication is not rewritten");
        let e = b.charge_estimate.as_ref().unwrap();
        assert_eq!((e.pct, e.low, e.high), (25.0, 15.0, 35.0));
        assert_eq!(e.chemistry, Chemistry::Alkaline);
        // Undeclared chemistry: no estimate.
        let mut u = Actor::new(Options {
            bluez_provider: false,
            notify: false,
            history: false,
            chemistry: Chemistry::Unknown,
            ..Options::default()
        });
        u.kb = Some(report_mv(50.0, 2404));
        u.linked = true;
        assert!(u.snapshot().keyboard.unwrap().battery.charge_estimate.is_none());
        // Set installed an hour ago: new batteries, no figure from the voltage.
        let mut n = quiet_actor();
        n.kb = Some(report_mv(98.0, 2950));
        n.linked = true;
        n.installed_at = Some(unix_now() - 3600);
        let b = n.snapshot().keyboard.unwrap().battery;
        assert!(b.new_batteries && b.charge_estimate.is_none());
    }

    /// The "change the batteries" reminder follows the keyboard's own
    /// thresholds (0x60: Low 2506 / Critical 2404 mV), once per crossing; the
    /// firmware notice once per known version (docs/NOTIFICATIONS.md).
    #[test]
    fn reminder_and_firmware_notices_are_triggered_once_per_crossing() {
        let mut a = quiet_actor();
        a.linked = true;
        let t = akm_core::registry::Thresholds {
            full_mv: 2954,
            low_mv: 2506,
            critical_mv: 2404,
            empty_mv: 2054,
        };
        let step = |a: &mut Actor, fw_pct: f64, mv: u32, fw: &str| {
            let mut k = report_mv(fw_pct, mv);
            k.battery.thresholds = Some(t);
            k.firmware.version = Some("0x0040".into());
            k.firmware.status = fw.into();
            k.firmware.latest_known = Some("0x0050".into());
            a.kb = Some(k);
            a.due_notices(notify::Lang::Fr)
                .into_iter()
                .map(|n| (n.event, n.urgency, n.body))
                .collect::<Vec<_>>()
        };
        let n = step(&mut a, 90.0, 2775, "update_available");
        assert_eq!(n.len(), 1, "{n:?}");
        assert_eq!(n[0].0, notify::Event::FirmwareUpdate);
        assert!(n[0].2.contains("0x0040") && n[0].2.contains("0x0050"));
        assert!(
            step(&mut a, 88.0, 2770, "update_available").is_empty(),
            "firmware: once"
        );
        let n = step(&mut a, 75.0, 2506, "update_available");
        assert_eq!(n.len(), 1);
        assert_eq!(
            (n[0].0, n[0].1),
            (notify::Event::BatteryReminder, Urgency::Normal)
        );
        assert!(
            n[0].2.contains("seuil Bas du clavier (2506\u{202f}mV)")
                && n[0].2.contains("75\u{202f}%"),
            "{}",
            n[0].2
        );
        assert!(
            step(&mut a, 74.0, 2503, "update_available").is_empty(),
            "Low: once"
        );
        let n = step(&mut a, 50.0, 2404, "update_available");
        assert_eq!(
            (n[0].0, n[0].1),
            (notify::Event::BatteryReminder, Urgency::Critical)
        );
        assert!(
            step(&mut a, 49.0, 2390, "update_available").is_empty(),
            "Critical: once"
        );
        // New batteries re-arm the reminder; the firmware, up to date, re-arms too.
        a.on_replaced(
            Some("AA:BB:CC:DD:EE:F1"),
            akm_core::batteries::Replacement {
                ts: 1,
                pct_before: Some(49.0),
                pct_after: 99.0,
                voltage_before: None,
                voltage_after: None,
            },
        );
        assert!(step(&mut a, 99.0, 2950, "up_to_date").is_empty());
        assert_eq!(
            step(&mut a, 70.0, 2500, "update_available").len(),
            2,
            "both armed again"
        );
        // Unknown thresholds (kernel-only reading): no voltage reminder.
        let mut b = quiet_actor();
        b.linked = true;
        b.kb = Some(report_mv(10.0, 2200));
        assert!(b.due_notices(notify::Lang::En).is_empty());
        // Disconnected: nothing.
        b.linked = false;
        assert!(b.due_notices(notify::Lang::En).is_empty());
    }

    #[test]
    fn alerts_follow_the_estimate_not_the_firmware_scale() {
        // #178: firmware 64 % (2460 mV) is already 30 % of real charge.
        let mut a = quiet_actor();
        let rx = a.opts.events.subscribe();
        for (fw, mv) in [(90.0, 2775), (75.0, 2506)] {
            a.kb = Some(report_mv(fw, mv));
            a.after_battery_update(true);
        }
        assert!(crossings(&rx).is_empty(), "35 % real: nothing yet");
        a.kb = Some(report_mv(64.0, 2455));
        a.after_battery_update(true);
        assert_eq!(crossings(&rx), vec![30]);
        // Same reading again: once only.
        a.after_battery_update(true);
        assert!(crossings(&rx).is_empty());
        // Without a declared chemistry the firmware scale is used.
        let mut u = Actor::new(Options {
            bluez_provider: false,
            notify: false,
            history: false,
            chemistry: Chemistry::Unknown,
            ..Options::default()
        });
        let rxu = u.opts.events.subscribe();
        u.kb = Some(report_mv(64.0, 2455));
        u.after_battery_update(true);
        assert!(crossings(&rxu).is_empty());
        u.kb = Some(report_mv(29.0, 2455));
        u.after_battery_update(true);
        assert_eq!(crossings(&rxu), vec![30]);
    }

    #[test]
    fn announced_switch_off_is_published_as_off_not_as_a_lost_link() {
        // #190: the event is distinct from a disconnection, and the
        // reconnection that follows is still announced.
        let mut a = quiet_actor();
        let rx = a.opts.events.subscribe();
        let mac = "AA:BB:CC:DD:EE:F1";
        a.link.acquired(mac, Some(60.0));
        a.disconnected_with(true);
        assert_eq!(
            rx.try_recv().unwrap(),
            DeviceEvent::Link(akm_core::link::LinkEvent::PoweredOff { mac: mac.into() })
        );
        assert!(rx.try_recv().is_err());
        assert!(!a.snapshot().connected);
        a.link.acquired(mac, Some(60.0));
        a.link.disconnected();
        // without an announcement: the usual loss
        let mut b = quiet_actor();
        let rx = b.opts.events.subscribe();
        b.link.acquired(mac, None);
        b.disconnected_with(false);
        assert_eq!(
            rx.try_recv().unwrap(),
            DeviceEvent::Link(akm_core::link::LinkEvent::Disconnected { mac: mac.into() })
        );
    }

    #[test]
    fn firmware_step_at_a_reconnection_raises_no_alert() {
        // #179: the kernel percentage steps down 32 -> 28 when the keyboard
        // reconnects, with no consumption behind it.
        let mut a = Actor::new(Options {
            bluez_provider: false,
            notify: false,
            history: false,
            chemistry: Chemistry::Unknown,
            ..Options::default()
        });
        let rx = a.opts.events.subscribe();
        a.kb = Some(report(32.0, None));
        a.linked = true;
        a.after_battery_update(true);
        a.disconnected();
        assert!(a.reconnected);
        a.kb = Some(report(28.0, None));
        a.linked = true;
        a.after_battery_update(true);
        assert!(crossings(&rx).is_empty(), "link artefact, not a discharge");
        assert!(!a.reconnected, "consumed by the first reading");
        // A later drop within the session is judged normally.
        a.kb = Some(report(14.0, None));
        a.after_battery_update(false);
        assert_eq!(crossings(&rx), vec![15]);
    }

    #[test]
    fn history_stores_real_millivolts() {
        // #180 through the actor: needs the history on; an own file.
        let dir = std::env::temp_dir().join(format!("akm-actor-hist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = quiet_actor();
        a.history = Some(History::new(dir.join("h.jsonl"), SystemClock));
        a.kb = Some(report_mv(98.0, 2945));
        a.after_battery_update(true);
        let e = a.history.as_ref().unwrap().read();
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].mv_0x46, e[0].mv_0x49), (Some(2982), Some(2945)));
        assert_eq!(e[0].schema, Some(akm_core::history::SCHEMA));
        assert!(e[0].voltage_reliable());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn options_follow_the_config() {
        let o = Options::default();
        assert_eq!(o.alerts.thresholds(), &[30, 15, 5]);
        assert!(o.alerts_enabled && o.notify_connection && o.notify_battery_replaced);
        let (c, _) = akm_core::config::parse(
            "[alerts]\nthresholds = [20]\nenabled = false\n[notifications]\nconnection = false\n",
        );
        let mut o = Options::default();
        o.apply_config(&c);
        assert_eq!(o.alerts.thresholds(), &[20]);
        assert!(!o.alerts_enabled && !o.notify_connection && o.notify_battery_replaced);
        assert_eq!(o.chemistry, Chemistry::Alkaline);
        let (c, _) = akm_core::config::parse("[battery]\nchemistry = \"lithium\"\n");
        o.apply_config(&c);
        assert_eq!(o.chemistry, Chemistry::Lithium);
    }

    /// M2 + M3 of the final review: a request inside the 30 s floor is
    /// deferred (and the stale fragments forgotten at once), never dropped;
    /// what goes out is the name alone, not a battery cycle.
    #[test]
    fn reread_name_forgets_at_once_and_defers_inside_the_floor() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut m = Machine::new();
        let mut forgotten = 0;
        // Disconnected: no effect, nothing forgotten.
        assert_eq!(
            reread_name(&mut m, t0, &mut || forgotten += 1),
            NameReread::NotConnected
        );
        assert_eq!(forgotten, 0);
        m.on_event(&Event::Connected("AA:BB:CC:DD:EE:F1".into()), t0);
        m.acquire_done(true, t0);
        m.due(t0);
        assert_eq!(
            reread_name(&mut m, t0 + s(1), &mut || forgotten += 1),
            NameReread::Now
        );
        assert_eq!(forgotten, 1);
        let due = m.due(t0 + s(1));
        assert!(due.contains(&Action::RereadName), "{due:?}");
        assert!(!due.contains(&Action::Acquire), "never the routine read");
        // Second rename 9 s later: the cache is dropped again (it is stale)
        // and the read is deferred to the end of the floor.
        assert_eq!(
            reread_name(&mut m, t0 + s(10), &mut || forgotten += 1),
            NameReread::Deferred(s(21))
        );
        assert_eq!(forgotten, 2);
        assert!(!m.due(t0 + s(30)).contains(&Action::RereadName));
        assert!(m.due(t0 + s(31)).contains(&Action::RereadName), "served");
        // What D-Bus answers.
        assert!(name_reread_text(NameReread::Now).contains("now"));
        assert!(name_reread_text(NameReread::Deferred(s(21))).contains("in 21 s"));
        assert!(name_reread_text(NameReread::NotConnected).contains("no keyboard"));
    }

    #[test]
    fn a_name_reread_that_found_the_lock_busy_is_put_back_a_failed_one_is_not() {
        use akm_core::read_policy::{Gate, SafeRead};
        let s = Duration::from_secs;
        let t0 = Instant::now();
        let ready = || {
            let mut m = Machine::new();
            m.on_event(&Event::Connected("AA:BB:CC:DD:EE:F1".into()), t0);
            m.acquire_done(true, t0);
            m.due(t0);
            assert_eq!(m.request_name_reread(t0 + s(1)), NameReread::Now);
            assert!(m.due(t0 + s(1)).contains(&Action::RereadName));
            m
        };
        // Lock busy, nothing sent: served again at the end of the floor.
        let mut m = ready();
        after_name_reread(&mut m, true, Some(SafeRead::Skipped(Gate::Busy)), t0 + s(2));
        assert!(m.name_reread_pending());
        assert!(!m.due(t0 + s(30)).contains(&Action::RereadName));
        assert!(m.due(t0 + s(31)).contains(&Action::RereadName));
        // Sent and failed, complete, breaker open: never asked again.
        for o in [
            SafeRead::Partial,
            SafeRead::Complete,
            SafeRead::Skipped(Gate::Tripped),
        ] {
            let mut m = ready();
            after_name_reread(&mut m, true, Some(o), t0 + s(2));
            assert!(!m.name_reread_pending(), "{o:?}");
            assert!(m.is_acquired());
        }
        // Node gone: back to the acquisition, like any failed acquisition.
        let mut m = ready();
        after_name_reread(&mut m, false, None, t0 + s(2));
        assert!(!m.is_acquired() && m.is_connected());
        assert!(m.due(t0 + s(3)).contains(&Action::Acquire));
    }

    #[test]
    fn the_name_reply_reaches_the_caller_and_never_blocks_the_actor() {
        let (reply, rx) = NameReply::channel();
        assert_eq!(
            Msg::RereadName(reply.clone()),
            Msg::RereadName(NameReply::none())
        );
        reply.answer(NameReread::Now);
        reply.answer(NameReread::NotConnected); // slot full: dropped, no block
        assert_eq!(rx.try_recv(), Ok(NameReread::Now));
        NameReply::none().answer(NameReread::Now);
        drop(rx);
        reply.answer(NameReread::Now); // caller gone: no panic
    }

    #[test]
    fn mailbox_without_actor_reports_failure() {
        let mb = Mailbox::new();
        assert!(!mb.send(Msg::Refresh));
        let (tx, rx) = mpsc::channel();
        mb.install(tx);
        assert!(mb.send(Msg::Refresh));
        assert_eq!(rx.recv().unwrap(), Msg::Refresh);
        drop(rx);
        assert!(!mb.send(Msg::Refresh));
    }

    static ASKED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    fn fake_disconnect(mac: &str, t: Duration) -> Result<(), String> {
        assert_eq!(t, akm_core::apple_model::APPLE.disconnect_call_timeout);
        ASKED.lock().unwrap().push(mac.to_string());
        Ok(())
    }

    #[test]
    fn a_tripped_breaker_asks_once_for_the_disconnection() {
        // #251 (RE-GHIDRA-KEXT §2.2): 3 silences -> SetHIDDriverReady(false).
        let mut a = Actor::new(Options {
            bluez_provider: false,
            notify: false,
            history: false,
            disconnect: fake_disconnect,
            ..Options::default()
        });
        let b = akm_core::read_policy::breaker();
        b.lock().unwrap().reset();
        assert!(!a.after_breaker(Some("AA:BB:CC:DD:EE:F1")), "closed: nothing");
        for _ in 0..akm_core::read_policy::TRIP_AFTER {
            b.lock().unwrap().record(false);
        }
        assert!(a.after_breaker(Some("AA:BB:CC:DD:EE:F1")));
        assert!(!a.after_breaker(Some("AA:BB:CC:DD:EE:F1")), "once per connection");
        assert_eq!(*ASKED.lock().unwrap(), vec!["AA:BB:CC:DD:EE:F1".to_string()]);
        // disabled: the request is consumed, BlueZ is not called
        b.lock().unwrap().reset();
        a.opts.disconnect_on_breaker = false;
        for _ in 0..akm_core::read_policy::TRIP_AFTER {
            b.lock().unwrap().record(false);
        }
        assert!(!a.after_breaker(Some("AA:BB:CC:DD:EE:F1")));
        assert_eq!(ASKED.lock().unwrap().len(), 1);
        // the MAC of the last report when the machine has none (NoBluez)
        b.lock().unwrap().reset();
        a.opts.disconnect_on_breaker = true;
        a.kb = Some(report(50.0, None));
        for _ in 0..akm_core::read_policy::TRIP_AFTER {
            b.lock().unwrap().record(false);
        }
        assert!(a.after_breaker(None));
        assert_eq!(ASKED.lock().unwrap().len(), 2);
        b.lock().unwrap().reset();
        // config
        let (c, _) = akm_core::config::parse("[apple]\ndisconnect_on_breaker = false\n");
        let mut o = Options::default();
        assert!(o.disconnect_on_breaker);
        o.apply_config(&c);
        assert!(!o.disconnect_on_breaker);
    }
}
