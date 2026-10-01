//! Event sources and connection state machine for the keyboard actor (#66).
//!
//! * `spawn_signal_watcher` subscribes to the system bus (BlueZ
//!   `Device1.PropertiesChanged`, `NameOwnerChanged` of `org.bluez`, UPower
//!   `Device.PropertiesChanged`) and forwards [`Event`]s on a channel. It never
//!   touches the keyboard itself.
//! * [`Machine`] is the pure scheduling logic: given events and the clock it
//!   says what to do ([`Action`]). While the keyboard is disconnected it
//!   returns no action at all, so nothing is read and the radio is left alone.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use zbus::blocking::{fdo::DBusProxy, Connection, MessageIterator};
use zbus::zvariant::OwnedValue;
use zbus::MatchRule;

/// Raw HID diagnostic reads (battery voltage, ...) are slow: piles last months.
pub const SLOW_READ_PERIOD: Duration = Duration::from_secs(15 * 60);
/// RSSI refresh while connected (the tracker expires a value after ~2 periods).
pub const RSSI_PERIOD: Duration = Duration::from_secs(45);
/// A measurement older than this is shown as absent.
pub const RSSI_MAX_AGE: Duration = Duration::from_secs(100);
/// Minimum spacing between two kernel power_supply reads triggered by signals.
pub const KERNEL_MIN_SPACING: Duration = Duration::from_secs(20);
/// Upper bound of the acquisition retry backoff (hidraw not there yet).
const RETRY_CAP: Duration = Duration::from_secs(60);

// ── Pure state machine ──────────────────────────────────────────────────────

/// What happened on the buses.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A BlueZ keyboard became connected (MAC upper-case, colon separated).
    Connected(String),
    /// A BlueZ keyboard disconnected.
    Disconnected(String),
    /// UPower / power_supply reported a change for a keyboard battery.
    BatterySignal,
    /// BlueZ is unreachable: assume present and probe sysfs with backoff.
    NoBluez,
}

/// What the actor must do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Full HID read (opens the current hidraw, possibly a new one).
    Acquire,
    /// Read the kernel power_supply capacity only.
    KernelBattery,
    /// Refresh RSSI through the helper.
    Rssi,
    /// Keyboard gone: publish "absent" and release the fd.
    Clear,
}

#[derive(Debug, Clone)]
pub struct Machine {
    connected: bool,
    mac: Option<String>,
    acquired: bool,
    attempt: u32,
    next_acquire: Option<Instant>,
    next_slow: Option<Instant>,
    next_rssi: Option<Instant>,
    next_kernel: Option<Instant>,
    last_kernel: Option<Instant>,
}

/// Retry delay after `attempt` failed acquisitions: 0.5, 1, 2, 4 ... capped.
pub fn backoff(attempt: u32) -> Duration {
    let ms = 500u64.saturating_mul(1u64 << attempt.min(10));
    Duration::from_millis(ms).min(RETRY_CAP)
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    pub fn new() -> Self {
        Self {
            connected: false,
            mac: None,
            acquired: false,
            attempt: 0,
            next_acquire: None,
            next_slow: None,
            next_rssi: None,
            next_kernel: None,
            last_kernel: None,
        }
    }

    #[cfg(test)]
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    pub fn mac(&self) -> Option<&str> {
        self.mac.as_deref()
    }

    fn reset_timers(&mut self) {
        self.acquired = false;
        self.attempt = 0;
        self.next_acquire = None;
        self.next_slow = None;
        self.next_rssi = None;
        self.next_kernel = None;
    }

    /// Feed an event; may return an immediate action (`Clear`).
    pub fn on_event(&mut self, ev: &Event, now: Instant) -> Option<Action> {
        match ev {
            Event::Connected(mac) => {
                let same = self.connected && self.mac.as_deref() == Some(mac.as_str());
                if !same {
                    self.reset_timers();
                    self.connected = true;
                    self.mac = Some(mac.clone());
                    self.next_acquire = Some(now);
                }
                None
            }
            Event::Disconnected(mac) => {
                if self.connected && (self.mac.is_none() || self.mac.as_deref() == Some(mac)) {
                    self.connected = false;
                    self.mac = None;
                    self.reset_timers();
                    Some(Action::Clear)
                } else {
                    None
                }
            }
            Event::NoBluez => {
                if !self.connected {
                    self.reset_timers();
                    self.connected = true;
                    self.next_acquire = Some(now);
                }
                None
            }
            Event::BatterySignal => {
                if self.connected && self.acquired {
                    let due = self.last_kernel.map_or(now, |t| t + KERNEL_MIN_SPACING);
                    self.next_kernel = Some(due.max(now));
                }
                None
            }
        }
    }

    /// Actions due at `now`. Empty whenever the keyboard is disconnected.
    pub fn due(&mut self, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        if !self.connected {
            return out;
        }
        if !self.acquired {
            if self.next_acquire.is_some_and(|t| t <= now) {
                out.push(Action::Acquire);
            }
            return out;
        }
        if self.next_slow.is_some_and(|t| t <= now) {
            out.push(Action::Acquire);
        }
        if self.next_kernel.is_some_and(|t| t <= now) {
            self.next_kernel = None;
            self.last_kernel = Some(now);
            out.push(Action::KernelBattery);
        }
        if self.next_rssi.is_some_and(|t| t <= now) {
            self.next_rssi = Some(now + RSSI_PERIOD);
            out.push(Action::Rssi);
        }
        out
    }

    /// Report the outcome of an `Acquire`.
    pub fn acquire_done(&mut self, ok: bool, now: Instant) {
        if !self.connected {
            return;
        }
        if ok {
            let first = !self.acquired;
            self.acquired = true;
            self.attempt = 0;
            self.next_acquire = None;
            self.next_slow = Some(now + SLOW_READ_PERIOD);
            self.last_kernel = Some(now);
            if first {
                self.next_rssi = Some(now);
            }
        } else {
            self.acquired = false;
            self.next_slow = None;
            self.next_rssi = None;
            self.next_acquire = Some(now + backoff(self.attempt));
            self.attempt = self.attempt.saturating_add(1);
        }
    }

    /// Earliest instant something is due; `None` = sleep until an event.
    pub fn next_deadline(&self) -> Option<Instant> {
        if !self.connected {
            return None;
        }
        if !self.acquired {
            return self.next_acquire;
        }
        [self.next_slow, self.next_kernel, self.next_rssi].into_iter().flatten().min()
    }
}

// ── D-Bus signal watcher ────────────────────────────────────────────────────

/// Is this BlueZ Modalias an Apple device (USB 05AC or Bluetooth SIG 004C)?
pub fn is_apple_modalias(m: &str) -> bool {
    let m = m.to_ascii_lowercase();
    m.starts_with("usb:v05ac") || m.starts_with("bluetooth:v004c")
}

/// UPower object paths embed the MAC with underscores or `o`: match a keyboard battery.
pub fn is_keyboard_upower_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains("/devices/keyboard_") || p.contains("/devices/battery_hid") || p.contains("hid_")
}

type Props = HashMap<String, OwnedValue>;

fn prop_str(p: &Props, k: &str) -> Option<String> {
    p.get(k).and_then(|v| <&str>::try_from(v).ok().map(str::to_string))
}

fn prop_bool(p: &Props, k: &str) -> Option<bool> {
    p.get(k).and_then(|v| bool::try_from(v).ok())
}

/// Spawn the watcher thread. It reconnects to the bus on failure (10 s) and
/// emits `NoBluez` once if BlueZ cannot be reached at all.
pub fn spawn_signal_watcher(tx: Sender<Event>) {
    let _ = std::thread::Builder::new().name("kb-watch".into()).spawn(move || {
        let mut told_no_bluez = false;
        loop {
            match watch_once(&tx) {
                Ok(()) => return, // receiver gone
                Err(e) => {
                    eprintln!("[watch] D-Bus error: {e} — retry in 10s");
                    if !told_no_bluez {
                        told_no_bluez = true;
                        if tx.send(Event::NoBluez).is_err() {
                            return;
                        }
                    }
                    std::thread::sleep(Duration::from_secs(10));
                }
            }
        }
    });
}

fn add_rules(conn: &Connection) -> zbus::Result<()> {
    let dbus = DBusProxy::new(conn)?;
    let props = |sender: &'static str, arg0: Option<&'static str>| -> zbus::Result<MatchRule<'static>> {
        let mut b = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(sender)?
            .interface("org.freedesktop.DBus.Properties")?
            .member("PropertiesChanged")?;
        if let Some(a) = arg0 {
            b = b.arg(0, a)?;
        }
        Ok(b.build())
    };
    dbus.add_match_rule(props("org.bluez", Some("org.bluez.Device1"))?)?;
    dbus.add_match_rule(props("org.freedesktop.UPower", None)?)?;
    let owner = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, "org.bluez")?
        .build();
    dbus.add_match_rule(owner)?;
    Ok(())
}

/// Known BlueZ keyboards: object path -> MAC, plus last connected flag.
#[derive(Default)]
struct Known(HashMap<String, (String, bool)>);

fn device_props(conn: &Connection, path: &str) -> zbus::Result<Props> {
    conn.call_method(
        Some("org.bluez"),
        path,
        Some("org.freedesktop.DBus.Properties"),
        "GetAll",
        &("org.bluez.Device1",),
    )?
    .body()
    .deserialize()
}

/// Enumerate BlueZ devices and emit the current state of every Apple keyboard.
fn initial_sync(conn: &Connection, known: &mut Known, tx: &Sender<Event>) -> zbus::Result<bool> {
    let om = zbus::blocking::fdo::ObjectManagerProxy::builder(conn)
        .destination("org.bluez")?
        .path("/")?
        .build()?;
    let mut sent = false;
    for (path, ifaces) in om.get_managed_objects()? {
        let Some(d) = ifaces.iter().find(|(k, _)| k.as_str() == "org.bluez.Device1").map(|(_, v)| v) else {
            continue;
        };
        if !prop_str(d, "Modalias").is_some_and(|m| is_apple_modalias(&m)) {
            continue;
        }
        let Some(mac) = prop_str(d, "Address").map(|a| a.to_ascii_uppercase()) else { continue };
        let connected = prop_bool(d, "Connected").unwrap_or(false);
        known.0.insert(path.to_string(), (mac.clone(), connected));
        if connected {
            sent = true;
            if tx.send(Event::Connected(mac)).is_err() {
                return Ok(sent);
            }
        }
    }
    Ok(sent)
}

fn watch_once(tx: &Sender<Event>) -> zbus::Result<()> {
    let conn = Connection::system()?;
    let calls = Connection::system()?; // separate connection for method calls
    add_rules(&conn)?;
    let mut known = Known::default();
    let mut it = MessageIterator::from(conn.clone());
    // Subscribe first, then enumerate: no event can fall in the gap.
    initial_sync(&calls, &mut known, tx)?;

    for msg in &mut it {
        let msg = msg?;
        let hdr = msg.header();
        let member = hdr.member().map(|m| m.as_str().to_string()).unwrap_or_default();
        let path = hdr.path().map(|p| p.as_str().to_string()).unwrap_or_default();
        match member.as_str() {
            "NameOwnerChanged" => {
                // bluetoothd restarted: resync from scratch.
                let (_, _, new): (String, String, String) = msg.body().deserialize()?;
                let old: Vec<(String, bool)> =
                    known.0.drain().map(|(_, (mac, c))| (mac, c)).collect();
                for (mac, c) in old {
                    if c && tx.send(Event::Disconnected(mac)).is_err() {
                        return Ok(());
                    }
                }
                if !new.is_empty() {
                    std::thread::sleep(Duration::from_millis(500));
                    initial_sync(&calls, &mut known, tx)?;
                }
            }
            "PropertiesChanged" => {
                let sender_bluez = path.starts_with("/org/bluez/");
                let (iface, changed, _inv): (String, Props, Vec<String>) = msg.body().deserialize()?;
                if sender_bluez && iface == "org.bluez.Device1" {
                    let Some(connected) = prop_bool(&changed, "Connected") else { continue };
                    let entry = match known.0.get(&path) {
                        Some((m, _)) => Some(m.clone()),
                        None => device_props(&calls, &path).ok().and_then(|p| {
                            prop_str(&p, "Modalias")
                                .filter(|m| is_apple_modalias(m))
                                .and(prop_str(&p, "Address"))
                                .map(|a| a.to_ascii_uppercase())
                        }),
                    };
                    let Some(mac) = entry else { continue };
                    known.0.insert(path.clone(), (mac.clone(), connected));
                    let ev = if connected { Event::Connected(mac) } else { Event::Disconnected(mac) };
                    if tx.send(ev).is_err() {
                        return Ok(());
                    }
                } else if path.starts_with("/org/freedesktop/UPower/devices/")
                    && is_keyboard_upower_path(&path)
                    && changed.contains_key("Percentage")
                    && tx.send(Event::BatterySignal).is_err()
                {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
    Err(zbus::Error::Failure("D-Bus message stream ended".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: &str = "04:DB:56:CA:42:EE";

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn disconnected_machine_does_nothing() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        assert!(m.due(t0).is_empty());
        assert_eq!(m.next_deadline(), None);
        // Battery / RSSI signals while disconnected change nothing.
        assert_eq!(m.on_event(&Event::BatterySignal, t0), None);
        assert!(m.due(t0 + s(10_000)).is_empty());
        assert_eq!(m.next_deadline(), None);
    }

    #[test]
    fn connect_acquires_then_schedules_slowly() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        assert_eq!(m.due(t0), vec![Action::Acquire]);
        m.acquire_done(true, t0);
        // RSSI immediately, then nothing for RSSI_PERIOD.
        assert_eq!(m.due(t0), vec![Action::Rssi]);
        assert!(m.due(t0 + s(10)).is_empty());
        assert_eq!(m.due(t0 + RSSI_PERIOD), vec![Action::Rssi]);
        // Raw HID diagnostics only every SLOW_READ_PERIOD.
        let acq = (1..)
            .map(|i| t0 + RSSI_PERIOD * i)
            .map(|t| (t, m.due(t)))
            .find(|(_, a)| a.contains(&Action::Acquire))
            .unwrap();
        assert!(acq.0 >= t0 + SLOW_READ_PERIOD);
        assert!(acq.0 < t0 + SLOW_READ_PERIOD + RSSI_PERIOD);
    }

    #[test]
    fn retry_backoff_while_hidraw_missing_then_recover() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        assert_eq!(m.due(t0), vec![Action::Acquire]);
        m.acquire_done(false, t0);
        assert!(m.due(t0 + Duration::from_millis(400)).is_empty());
        assert_eq!(m.due(t0 + Duration::from_millis(500)), vec![Action::Acquire]);
        m.acquire_done(false, t0 + Duration::from_millis(500));
        assert_eq!(m.next_deadline(), Some(t0 + Duration::from_millis(1500)));
        assert_eq!(backoff(0), Duration::from_millis(500));
        assert_eq!(backoff(20), RETRY_CAP);
        m.acquire_done(true, t0 + s(2));
        assert_eq!(m.due(t0 + s(2)), vec![Action::Rssi]);
    }

    #[test]
    fn disconnect_clears_and_silences() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.acquire_done(true, t0);
        assert_eq!(m.on_event(&Event::Disconnected(MAC.into()), t0 + s(5)), Some(Action::Clear));
        assert!(!m.is_connected());
        assert!(m.due(t0 + s(100_000)).is_empty());
        assert_eq!(m.next_deadline(), None);
        // Disconnect of another MAC is ignored.
        m.on_event(&Event::Connected(MAC.into()), t0 + s(10));
        assert_eq!(m.on_event(&Event::Disconnected("AA:BB:CC:DD:EE:FF".into()), t0 + s(11)), None);
        assert!(m.is_connected());
    }

    #[test]
    fn reconnect_reacquires_immediately() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.acquire_done(true, t0);
        m.on_event(&Event::Disconnected(MAC.into()), t0 + s(1));
        m.on_event(&Event::Connected(MAC.into()), t0 + s(2));
        assert_eq!(m.due(t0 + s(2)), vec![Action::Acquire]);
    }

    #[test]
    fn duplicate_connected_does_not_reset() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.acquire_done(true, t0);
        m.due(t0);
        m.on_event(&Event::Connected(MAC.into()), t0 + s(1));
        assert!(m.due(t0 + s(1)).is_empty());
    }

    #[test]
    fn battery_signal_is_rate_limited() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.acquire_done(true, t0);
        m.due(t0);
        m.on_event(&Event::BatterySignal, t0 + s(1));
        assert!(!m.due(t0 + s(1)).contains(&Action::KernelBattery));
        assert!(m.due(t0 + s(21)).contains(&Action::KernelBattery));
        m.on_event(&Event::BatterySignal, t0 + s(22));
        assert!(!m.due(t0 + s(22)).contains(&Action::KernelBattery));
    }

    #[test]
    fn no_bluez_falls_back_to_probing() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::NoBluez, t0);
        assert_eq!(m.due(t0), vec![Action::Acquire]);
        m.acquire_done(false, t0);
        assert!(m.next_deadline().unwrap() > t0);
    }

    #[test]
    fn modalias_and_path_filters() {
        assert!(is_apple_modalias("usb:v05ACp0256d0050"));
        assert!(is_apple_modalias("bluetooth:v004Cp029Cd0001"));
        assert!(!is_apple_modalias("usb:v046Dp0001d0001"));
        assert!(is_keyboard_upower_path("/org/freedesktop/UPower/devices/keyboard_hid_04o_db"));
        assert!(!is_keyboard_upower_path("/org/freedesktop/UPower/devices/battery_BAT0"));
    }
}
