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
use akm_core::batteries::{self, Detector};
use akm_core::forecast::{self, Forecast};
use akm_core::history::{
    format_remaining, legacy_path, Clock, History, HistoryEntry, HistoryEvent, SystemClock,
    RETENTION_S,
};
use akm_core::link::LinkTracker;
use akm_core::machine::{Action, Event, Machine, RSSI_MAX_AGE};
use akm_core::rssi::{self, RssiTracker};
use akm_core::{discover, hidraw, led, power, KbReport, Snapshot, Watch};

use crate::alias::{AliasBackend, BluezAlias};
use crate::events::{DeviceEvent, EventHub};
use crate::{bluez, notify, watcher};

/// Messages handled by the actor thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// From the BlueZ / UPower watcher.
    Bus(Event),
    /// Explicit refresh (D-Bus `Refresh()`).
    Refresh,
    /// The BlueZ alias of a keyboard is now this (`None` = unknown).
    Alias(String, Option<String>),
    /// Stop the actor.
    Quit,
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
    }
}

/// Minimum spacing between two history samples.
const HISTORY_SPACING: Duration = Duration::from_secs(300);
/// The published snapshot (LED, RSSI expiry) is refreshed at least this often.
const TICK: Duration = Duration::from_secs(5);

fn unix_now() -> u64 {
    SystemClock.now()
}

struct Actor {
    opts: Options,
    kb: Option<KbReport>,
    rssi: RssiTracker,
    rssi_at: Option<u64>,
    provider: Option<bluez::BatteryProvider>,
    provider_mac: Option<String>,
    alerts: AlertState,
    detector: Detector,
    link: LinkTracker,
    forecast: Option<Forecast>,
    installed_at: Option<u64>,
    history: Option<History>,
    last_history: Option<Instant>,
    last_rotation: Option<Instant>,
    remaining: Option<String>,
    remaining_at: Option<Instant>,
    last_update: u64,
    last_error: Option<String>,
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
        let past = history.as_ref().map(History::read).unwrap_or_default();
        Self {
            alerts: AlertState::new(opts.alerts.clone()),
            detector: Detector::primed(&past),
            link: LinkTracker::new(),
            forecast: None,
            installed_at: batteries::current_set_start(&past),
            opts,
            kb: None,
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
        }
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
        self.last_error = err;
        gate_wake_monitor(report.as_ref(), mac);
        match report {
            Some(mut k) => {
                let mac = k.device.mac.clone();
                let pct = k.battery_pct();
                k.device.alias = mac.as_deref().and_then(|m| self.opts.alias.get(m));
                self.kb = Some(k);
                self.last_update = unix_now();
                self.after_battery_update(true);
                if let Some(ev) = mac.and_then(|m| self.link.acquired(&m, pct)) {
                    self.link_event(ev);
                }
                true
            }
            None => {
                self.kb = None;
                false
            }
        }
    }

    /// The BlueZ alias changed (rename, `bluetoothctl`, system settings).
    fn set_alias(&mut self, mac: &str, alias: Option<String>) {
        if let Some(k) = self.kb.as_mut() {
            if k.device.mac.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(mac)) {
                k.device.alias = alias;
            }
        }
    }

    /// Kernel power_supply capacity only (UPower signal).
    fn kernel_battery(&mut self) {
        let Some(k) = self.kb.as_mut() else { return };
        if let Some(r) = k.device.mac.as_deref().and_then(power::kernel_battery) {
            k.battery.percentage_fine = Some(f64::from(r.percent));
            k.battery.percentage = Some(f64::from(r.percent));
            self.last_update = unix_now();
            self.after_battery_update(false);
        }
    }

    /// BlueZ reported the keyboard gone.
    fn disconnected(&mut self) {
        if let Some(ev) = self.link.disconnected() {
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

    fn clear(&mut self) {
        self.kb = None;
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
        let voltage = k.battery.voltage;
        let now = Instant::now();

        let due = self
            .last_history
            .is_none_or(|t| now.duration_since(t) >= HISTORY_SPACING);
        if due || (full_read && voltage.is_some()) {
            let ts = self.history.as_ref().map_or_else(unix_now, History::now);
            let mut entry = HistoryEntry::sample(ts, pct, voltage);
            if let Some(r) = self.detector.observe(&entry) {
                entry.event = Some(HistoryEvent::BatteryReplaced);
                self.on_replaced(mac.as_deref(), r);
            }
            match self.history.as_ref().map(|h| h.append_entry(&entry)) {
                Some(Ok(true)) | None => {
                    self.last_history = Some(now);
                    self.remaining_at = None; // new data: recompute the forecast
                }
                Some(Ok(false)) => {}
                Some(Err(e)) => tracing::warn!("history append failed: {e}"),
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

        if self.opts.alerts_enabled {
            if let Some(c) = self.alerts.update(pct) {
                tracing::warn!("low battery: {pct:.0}% (threshold {}%)", c.threshold);
                if self.opts.notify {
                    notify::battery_crossing(&c);
                    if c.urgency == Urgency::Critical {
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
        self.installed_at = Some(r.ts);
        if self.opts.notify && self.opts.notify_battery_replaced {
            notify::battery_replaced(&r);
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
        if r.is_some() {
            self.rssi_at = Some(unix_now());
        }
        self.rssi.record(&mac, r, Instant::now());
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
        if let Some(t) = batteries::current_set_start(&entries) {
            self.installed_at = Some(t);
        }
    }

    fn snapshot(&mut self) -> Snapshot {
        let now = Instant::now();
        self.refresh_remaining(now);
        let mut kb = self.kb.clone();
        let mut rssi_at = None;
        if let Some(k) = kb.as_mut() {
            // RSSI is exposed only while fresh and taken from this very MAC.
            let cur = k
                .device
                .mac
                .as_deref()
                .and_then(|m| self.rssi.current(m, now));
            k.radio.rssi_dbm = cur.map(|c| c.0);
            k.radio.tx_power_dbm = cur.and_then(|c| c.1);
            k.bluetooth.rssi_dbus = None;
            k.bluetooth.tx_power_dbus = None;
            rssi_at = cur.and(self.rssi_at);
        }
        let (caps, num) = if kb.is_some() {
            led::read_led_state()
        } else {
            (false, false)
        };
        Snapshot {
            connected: kb.is_some(),
            kb_error: kb.is_none().then(|| "Keyboard: not found".to_string()),
            keyboard: kb,
            caps_lock: caps,
            num_lock: num,
            remaining_display: self.remaining.clone(),
            rssi_at,
            last_update: self.last_update,
            last_error: self.last_error.clone(),
            forecast: self.forecast.clone(),
            batteries_installed_at: self.installed_at,
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
        })
        .expect("spawn kb-supervisor");
    ActorHandle {
        quit,
        mailbox,
        thread: Some(thread),
    }
}

/// Event loop. While the keyboard is disconnected nothing keyboard-related runs.
fn run(watch: Arc<Watch>, mailbox: Arc<Mailbox>, quit: Arc<AtomicBool>, opts: Options) {
    let (tx, rx) = mpsc::channel::<Msg>();
    mailbox.install(tx.clone());
    watcher::spawn_signal_watcher(tx);
    let mut machine = Machine::new();
    let mut actor = Actor::new(opts);
    watch.publish(actor.snapshot());

    loop {
        if quit.load(Ordering::Relaxed) {
            break;
        }
        let now = Instant::now();
        let wait = machine
            .next_deadline()
            .map_or(TICK, |d| d.saturating_duration_since(now).min(TICK));
        match rx.recv_timeout(wait) {
            Ok(Msg::Bus(ev)) => {
                if machine.on_event(&ev, Instant::now()) == Some(Action::Clear) {
                    tracing::info!("keyboard disconnected");
                    actor.disconnected();
                }
            }
            Ok(Msg::Refresh) => machine.force_refresh(Instant::now()),
            Ok(Msg::Alias(mac, alias)) => actor.set_alias(&mac, alias),
            Ok(Msg::Quit) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        for action in machine.due(Instant::now()) {
            match action {
                Action::Acquire => {
                    let mac = machine.mac().map(str::to_string);
                    let ok = actor.acquire(mac.as_deref());
                    machine.acquire_done(ok, Instant::now());
                    if ok {
                        let pct = actor.kb.as_ref().and_then(KbReport::battery_pct);
                        tracing::info!(
                            "acquired {} battery={}",
                            mac.as_deref().unwrap_or("?"),
                            pct.map_or("n/a".into(), |p| format!("{p:.0}%"))
                        );
                    }
                }
                Action::KernelBattery => actor.kernel_battery(),
                Action::Rssi => actor.refresh_rssi(),
                Action::Clear => actor.clear(),
            }
        }
        watch.publish(actor.snapshot());
    }
    // Dropping the actor drops the BlueZ provider: unregistered cleanly.
    drop(actor);
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
