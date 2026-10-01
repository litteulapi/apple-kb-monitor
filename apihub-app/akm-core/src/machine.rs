//! Connection / scheduling state machine of the keyboard actor (#66).
//!
//! [`Machine`] is pure: given bus events ([`Event`], produced by the BlueZ /
//! UPower watcher of the daemon) and the clock, it says what to do
//! ([`Action`]). While the keyboard is disconnected it returns no action at
//! all, so nothing is read and the radio is left alone.

use std::time::{Duration, Instant};

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

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Has a full acquisition succeeded since the last (re)connection?
    pub fn is_acquired(&self) -> bool {
        self.acquired
    }

    /// Explicit refresh request (D-Bus `Refresh()`): a full read is due now if
    /// connected; no effect while disconnected (nothing is ever probed then).
    pub fn force_refresh(&mut self, now: Instant) {
        if !self.connected {
            return;
        }
        if self.acquired {
            self.next_slow = Some(now);
        } else {
            self.attempt = 0;
            self.next_acquire = Some(now);
        }
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
    fn refresh_forces_a_read_only_when_connected() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.force_refresh(t0);
        assert!(m.due(t0).is_empty());
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.acquire_done(true, t0);
        m.due(t0);
        assert!(m.due(t0 + s(1)).is_empty());
        m.force_refresh(t0 + s(1));
        assert_eq!(m.due(t0 + s(1)), vec![Action::Acquire]);
        // while retrying, refresh resets the backoff
        m.acquire_done(false, t0 + s(2));
        m.acquire_done(false, t0 + s(3));
        m.force_refresh(t0 + s(3));
        assert_eq!(m.due(t0 + s(3)), vec![Action::Acquire]);
        assert!(m.is_connected() && !m.is_acquired());
    }
}
