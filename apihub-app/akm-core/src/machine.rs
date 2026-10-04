//! Connection / scheduling state machine of the keyboard actor.

use std::time::{Duration, Instant};

use crate::apple_model::{LinkModel, APPLE};

/// Period of the battery read when it succeeds: Apple's 4 h.
pub const SLOW_READ_PERIOD: Duration = APPLE.battery_period;
/// Floor between two reads forced by `Refresh()` (D-Bus, menu).
pub const FORCE_REFRESH_FLOOR: Duration = Duration::from_mins(5);

/// Answer to an explicit refresh request (D-Bus `Refresh()`, menu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// A read is due now.
    Accepted,
    /// No keyboard connected: nothing is read.
    Disconnected,
    /// A read was made less than [`FORCE_REFRESH_FLOOR`] ago; the next request is taken in `wait`.
    TooSoon { wait: Duration },
}

/// Floor between two re-reads of the name asked by `RereadName()`.
pub const NAME_REREAD_FLOOR: Duration = Duration::from_secs(30);

/// Answer to a `RereadName()` request ([`Machine::request_name_reread`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameReread {
    /// No keyboard connected: nothing will be read.
    NotConnected,
    /// The name is read at the next pass of the actor.
    Now,
    /// The name is read after this delay (floor, or keyboard not acquired yet).
    Deferred(Duration),
}

impl NameReread {
    /// Will the name be read (now or later)?
    #[must_use]
    pub fn accepted(self) -> bool {
        self != NameReread::NotConnected
    }
}

pub const RSSI_PERIOD: Duration = Duration::from_secs(45);
/// A measurement older than this is shown as absent.
pub const RSSI_MAX_AGE: Duration = Duration::from_secs(100);
/// Minimum spacing between two kernel `power_supply` reads triggered by signals.
pub const KERNEL_MIN_SPACING: Duration = Duration::from_secs(20);
const RETRY_CAP: Duration = Duration::from_mins(1);
/// `NoBluez` probing gives up after this many failed acquisitions: ~8 min of backoff.
pub const NOBLUEZ_MAX_PROBES: u32 = 12;
/// A `Reconcile` never drops a keyboard whose `Connected` arrived this recently.
pub const RECONCILE_GRACE: Duration = Duration::from_secs(3);

/// What happened on the buses.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A `BlueZ` keyboard became connected (MAC upper-case, colon separated).
    Connected(String),
    /// A `BlueZ` keyboard disconnected.
    Disconnected(String),
    /// `UPower` / `power_supply` reported a change for a keyboard battery.
    BatterySignal,
    /// `BlueZ` is unreachable: assume present and probe sysfs with backoff.
    NoBluez,
    /// Authoritative set of keyboards `BlueZ` reports connected, from a fresh enumeration.
    Reconcile(Vec<String>),
}

/// What the actor must do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Full HID read (opens the current hidraw, possibly a new one).
    Acquire,
    /// Read the name stored in the keyboard (`0x51`-`0x54`) and nothing else.
    RereadName,
    /// Read the kernel `power_supply` capacity only.
    KernelBattery,
    /// Refresh RSSI through the helper.
    Rssi,
    /// Keyboard gone: publish "absent" and release the fd.
    Clear,
}

#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // reason: independent connection facts, not a state enum
pub struct Machine {
    connected: bool,
    mac: Option<String>,
    standby: Vec<String>,
    connected_at: Option<Instant>,
    acquired: bool,
    attempt: u32,
    next_acquire: Option<Instant>,
    link: LinkModel,
    forced: Option<Instant>,
    battery_cycle: bool,
    next_rssi: Option<Instant>,
    next_kernel: Option<Instant>,
    last_kernel: Option<Instant>,
    last_read: Option<Instant>,
    last_forced: Option<Instant>,
    last_name_reread: Option<Instant>,
    name_reread: Option<Instant>,
    vendor_hold: bool,
}

/// Retry delay after `attempt` failed acquisitions: 0.5, 1, 2, 4 ... capped.
#[must_use]
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
    #[must_use]
    pub fn new() -> Self {
        Self {
            connected: false,
            mac: None,
            standby: Vec::new(),
            connected_at: None,
            acquired: false,
            attempt: 0,
            next_acquire: None,
            link: LinkModel::new(),
            forced: None,
            battery_cycle: false,
            next_rssi: None,
            next_kernel: None,
            last_kernel: None,
            last_read: None,
            last_forced: None,
            last_name_reread: None,
            name_reread: None,
            vendor_hold: false,
        }
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    #[cfg(any(test, feature = "testseam"))]
    /// Has a full acquisition succeeded since the last (re)connection?
    #[must_use]
    pub fn is_acquired(&self) -> bool {
        self.acquired
    }

    /// Explicit refresh request (D-Bus `Refresh()`): a full read is due now if connected.
    pub fn request_refresh(&mut self, now: Instant) -> RefreshOutcome {
        if !self.connected {
            return RefreshOutcome::Disconnected;
        }
        let left = |t: Option<Instant>| {
            t.and_then(|t| FORCE_REFRESH_FLOOR.checked_sub(now.saturating_duration_since(t)))
                .filter(|d| !d.is_zero())
        };
        let wait = left(self.last_forced)
            .into_iter()
            .chain(left(self.last_read).filter(|_| self.acquired))
            .max();
        if let Some(wait) = wait {
            return RefreshOutcome::TooSoon { wait };
        }
        self.last_forced = Some(now);
        if self.acquired {
            // Apple's `UpdateBatteryLevel` command: a read now, timer unchanged.
            self.forced = Some(now);
        } else {
            self.attempt = 0;
            self.next_acquire = Some(now);
        }
        RefreshOutcome::Accepted
    }

    /// D-Bus `RereadName()`: the name stored in the keyboard was just rewritten.
    pub fn request_name_reread(&mut self, now: Instant) -> NameReread {
        if !self.connected {
            return NameReread::NotConnected;
        }
        let floor = self.last_name_reread.map_or(now, |t| t + NAME_REREAD_FLOOR);
        // A request already pending keeps its (earlier) deadline.
        let due = self.name_reread.unwrap_or_else(|| floor.max(now));
        self.name_reread = Some(due);
        match due.saturating_duration_since(now) {
            d if d.is_zero() && self.acquired && !self.vendor_hold => NameReread::Now,
            d => NameReread::Deferred(d),
        }
    }

    #[cfg(any(test, feature = "testseam"))]
    /// Is a `RereadName()` waiting to be served?
    #[must_use]
    pub fn name_reread_pending(&self) -> bool {
        self.name_reread.is_some()
    }

    /// The keyboard has been silent since a system wake.
    pub fn set_vendor_hold(&mut self, on: bool) {
        self.vendor_hold = on;
    }

    #[must_use]
    pub fn mac(&self) -> Option<&str> {
        self.mac.as_deref()
    }

    /// Connected keyboards waiting behind the followed one.
    #[cfg(test)]
    #[must_use]
    pub fn standby(&self) -> &[String] {
        &self.standby
    }

    /// Apple's connection model (readiness, battery timer).
    #[must_use]
    pub fn link(&self) -> &LinkModel {
        &self.link
    }

    /// Does the `Acquire` just handed out by [`Machine::due`] carry the battery read of the model?
    #[must_use]
    pub fn vendor_reads_due(&self) -> bool {
        self.battery_cycle
    }

    /// System sleep (Apple's `handleSleep`: battery timer stopped).
    pub fn on_sleep(&mut self) {
        self.link.sleep();
        self.battery_cycle = false;
    }

    /// Wake (Apple's `handleWake`: battery read 60 s later if ready).
    pub fn on_wake(&mut self, now: Instant) {
        self.link.wake(now);
    }

    fn follow(&mut self, now: Instant) {
        self.link.connected();
        self.next_acquire = Some(now);
    }

    fn reset_timers(&mut self) {
        self.acquired = false;
        self.attempt = 0;
        self.next_acquire = None;
        self.link.disconnected();
        self.forced = None;
        self.battery_cycle = false;
        self.next_rssi = None;
        self.next_kernel = None;
        // A new connection reads the name anyway (first battery cycle).
        self.name_reread = None;
        self.vendor_hold = false;
    }

    /// Feed an event; may return an immediate action (`Clear`).
    pub fn on_event(&mut self, ev: &Event, now: Instant) -> Option<Action> {
        match ev {
            Event::Connected(mac) => {
                if self.connected && self.mac.as_deref() == Some(mac.as_str()) {
                    return None;
                }
                if self.connected && self.mac.is_some() {
                    // Another keyboard while one is followed.
                    if !self.standby.contains(mac) {
                        self.standby.push(mac.clone());
                    }
                    return None;
                }
                self.reset_timers();
                self.connected = true;
                self.mac = Some(mac.clone());
                self.connected_at = Some(now);
                self.follow(now);
                None
            }
            Event::Disconnected(mac) => {
                if let Some(i) = self.standby.iter().position(|m| m == mac) {
                    self.standby.remove(i);
                    return None;
                }
                if self.connected && (self.mac.is_none() || self.mac.as_deref() == Some(mac)) {
                    self.reset_timers();
                    if self.standby.is_empty() {
                        self.connected = false;
                        self.mac = None;
                    } else {
                        // Hand over to the next connected keyboard.
                        self.mac = Some(self.standby.remove(0));
                        self.follow(now);
                    }
                    Some(Action::Clear)
                } else {
                    None
                }
            }
            Event::NoBluez => {
                if !self.connected {
                    self.reset_timers();
                    self.connected = true;
                    self.follow(now);
                }
                None
            }
            Event::Reconcile(macs) => self.reconcile(macs, now),
            Event::BatterySignal => {
                if self.connected && self.acquired {
                    let due = self.last_kernel.map_or(now, |t| t + KERNEL_MIN_SPACING);
                    self.next_kernel = Some(due.max(now));
                }
                None
            }
        }
    }

    fn reconcile(&mut self, macs: &[String], now: Instant) -> Option<Action> {
        self.standby.retain(|m| macs.contains(m));
        let followed = self.mac.as_ref().is_some_and(|m| macs.contains(m));
        let fresh = self
            .connected_at
            .is_some_and(|t| now.saturating_duration_since(t) < RECONCILE_GRACE);
        if self.connected && self.mac.is_some() && (followed || fresh) {
            for m in macs {
                if self.mac.as_ref() != Some(m) && !self.standby.contains(m) {
                    self.standby.push(m.clone());
                }
            }
            return None;
        }
        let was_active = self.connected;
        self.reset_timers();
        // Prefer a keyboard already queued, then BlueZ's order.
        let next = self
            .standby
            .first()
            .cloned()
            .or_else(|| macs.first().cloned());
        if let Some(n) = next {
            self.standby.retain(|m| *m != n);
            for m in macs {
                if *m != n && !self.standby.contains(m) {
                    self.standby.push(m.clone());
                }
            }
            self.connected = true;
            self.mac = Some(n);
            self.connected_at = Some(now);
            self.follow(now);
        } else {
            self.connected = false;
            self.mac = None;
            self.connected_at = None;
            self.standby.clear();
        }
        was_active.then_some(Action::Clear)
    }

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
        // While held (silent since a wake) the cycle is not even begun.
        if !self.vendor_hold {
            let scheduled = self.link.begin_cycle(now);
            if scheduled || self.forced.is_some_and(|t| t <= now) {
                self.forced = None;
                self.battery_cycle = true;
                out.push(Action::Acquire);
            }
            if self.name_reread.is_some_and(|t| t <= now) {
                self.name_reread = None;
                self.last_name_reread = Some(now);
                out.push(Action::RereadName);
            }
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

    /// Report the outcome of an `Acquire` (battery read, if any, failed).
    pub fn acquire_done(&mut self, ok: bool, now: Instant) {
        self.acquire_done_with(ok, None, now);
    }

    /// Report the outcome of an `Acquire`.
    pub fn acquire_done_with(&mut self, ok: bool, vendor_ok: Option<bool>, now: Instant) {
        if !self.connected {
            return;
        }
        if std::mem::take(&mut self.battery_cycle) {
            let read_ok = ok && vendor_ok == Some(true);
            // The burst is `0x47` then GET Input `0x30` (Apple's R2), stopping at the first
            // failure: one verdict covers both halves.
            self.link.end_cycle(read_ok, read_ok, now);
        }
        if ok {
            let first = !self.acquired;
            self.acquired = true;
            self.attempt = 0;
            self.next_acquire = None;
            if !self.link.is_ready() {
                self.link.protocol_result(true, now);
            }
            self.last_read = Some(now);
            self.last_kernel = Some(now);
            if first {
                self.next_rssi = Some(now);
            }
        } else {
            self.acquired = false;
            self.forced = None;
            self.next_rssi = None;
            self.next_acquire = Some(now + backoff(self.attempt));
            self.attempt = self.attempt.saturating_add(1);
            if self.mac.is_none() && self.attempt >= NOBLUEZ_MAX_PROBES {
                // NoBluez probing found nothing.
                self.reset_timers();
                self.connected = false;
            }
        }
    }

    /// Earliest instant something is due; `None` = sleep until an event.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        if !self.connected {
            return None;
        }
        if !self.acquired {
            return self.next_acquire;
        }
        // Held: the vendor deadlines are not served, so they must not wake the loop either.
        let vendor = [self.link.next_deadline(), self.forced, self.name_reread]
            .into_iter()
            .flatten()
            .filter(|_| !self.vendor_hold);
        vendor
            .chain([self.next_kernel, self.next_rssi].into_iter().flatten())
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: &str = "AA:BB:CC:DD:EE:F1";

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn disconnected_machine_does_nothing() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        let got = m.due(t0);
        assert!(got.is_empty(), "{got:?}");
        assert_eq!(m.next_deadline(), None);
        assert_eq!(m.on_event(&Event::BatterySignal, t0), None);
        assert!(
            m.due(t0 + s(10_000)).is_empty(),
            "{:?}",
            m.due(t0 + s(10_000))
        );
        assert_eq!(m.next_deadline(), None);
    }

    #[test]
    fn connect_acquires_then_schedules_slowly() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        assert_eq!(m.due(t0), vec![Action::Acquire]);
        assert!(!m.vendor_reads_due(), "readiness only, no vendor report");
        m.acquire_done(true, t0);
        assert_eq!(m.due(t0), vec![Action::Rssi]);
        let got = m.due(t0 + s(10));
        assert!(got.is_empty(), "{got:?}");
        assert_eq!(m.due(t0 + RSSI_PERIOD), vec![Action::Rssi]);
        let first = (1..=3600)
            .map(|i| t0 + s(i))
            .find(|&t| m.due(t).contains(&Action::Acquire))
            .unwrap();
        assert_eq!(first, t0 + s(60));
        assert!(m.vendor_reads_due());
        m.acquire_done_with(true, Some(true), first);
        assert!(!m.vendor_reads_due());
        let acq = (1..=1000)
            .map(|i| first + RSSI_PERIOD * i)
            .map(|t| (t, m.due(t)))
            .find(|(_, a)| a.contains(&Action::Acquire))
            .unwrap();
        assert!(acq.0 >= first + SLOW_READ_PERIOD);
        assert!(acq.0 < first + SLOW_READ_PERIOD + RSSI_PERIOD);
    }

    #[test]
    fn battery_reads_follow_apples_schedule() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.due(t0);
        m.acquire_done(true, t0);
        m.due(t0);
        assert!(m.link().is_ready());
        assert_eq!(m.next_deadline(), Some(t0 + s(45)), "RSSI first");
        m.due(t0 + s(45));
        assert_eq!(m.next_deadline(), Some(t0 + s(60)));
        assert_eq!(m.due(t0 + s(60)), vec![Action::Acquire]);
        m.acquire_done_with(true, Some(false), t0 + s(61));
        assert_eq!(m.link().next_battery(), Some(t0 + s(61 + 3600)));
        assert!(m
            .due(t0 + s(61 + 3599))
            .iter()
            .all(|a| *a != Action::Acquire));
        assert!(m.due(t0 + s(61 + 3600)).contains(&Action::Acquire));
        m.acquire_done(true, t0 + s(61 + 3600));
        assert_eq!(m.link().next_battery(), Some(t0 + s(61 + 7200)));
        assert!(m.due(t0 + s(61 + 7200)).contains(&Action::Acquire));
        m.acquire_done_with(true, Some(true), t0 + s(61 + 7200));
        assert_eq!(
            m.link().next_battery(),
            Some(t0 + s(61 + 7200) + SLOW_READ_PERIOD)
        );
        let t = t0 + s(61 + 7200) + SLOW_READ_PERIOD;
        assert!(m.due(t).contains(&Action::Acquire));
        m.acquire_done_with(false, Some(true), t);
        assert_eq!(m.link().next_battery(), Some(t + s(3600)));
    }

    #[test]
    fn a_keyboard_never_ready_is_never_read() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        let mut t = t0;
        for _ in 0..200 {
            for a in m.due(t) {
                assert_eq!(a, Action::Acquire);
                assert!(!m.vendor_reads_due());
                m.acquire_done(false, t);
            }
            t += s(30);
        }
        assert!(!m.link().is_ready());
        assert_eq!(m.link().next_battery(), None);
    }

    #[test]
    fn sleep_stops_the_timer_and_the_wake_restarts_it() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.due(t0);
        m.acquire_done(true, t0);
        m.due(t0);
        assert!(m.due(t0 + s(60)).contains(&Action::Acquire));
        m.on_sleep();
        assert!(!m.vendor_reads_due(), "the read in flight is dropped");
        assert_eq!(m.link().next_battery(), None);
        assert!(!m.due(t0 + s(5000)).contains(&Action::Acquire));
        m.on_wake(t0 + s(6000));
        assert_eq!(m.link().next_battery(), Some(t0 + s(6060)));
        assert!(m.due(t0 + s(6060)).contains(&Action::Acquire));
    }

    #[test]
    fn refresh_reads_now_without_moving_the_timer() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.due(t0);
        m.acquire_done(true, t0);
        m.due(t0);
        let t1 = t0 + s(60) + FORCE_REFRESH_FLOOR + s(1);
        m.due(t0 + s(60));
        m.acquire_done_with(true, Some(true), t0 + s(60));
        assert_eq!(m.request_refresh(t1), RefreshOutcome::Accepted);
        assert!(m.next_deadline().is_some_and(|d| d <= t1));
        assert!(m.due(t1).contains(&Action::Acquire));
        assert!(m.vendor_reads_due());
        m.acquire_done_with(true, Some(true), t1);
        assert_eq!(m.link().next_battery(), Some(t0 + s(60) + SLOW_READ_PERIOD));
        m.on_event(&Event::Disconnected(MAC.into()), t1 + s(1));
        assert_eq!(
            m.link().state(),
            crate::apple_model::LinkState::Disconnected
        );
        assert!(!m.vendor_reads_due());
        m.on_event(&Event::Connected(MAC.into()), t1 + s(2));
        assert_eq!(
            m.link().state(),
            crate::apple_model::LinkState::AwaitingProtocol
        );
    }

    #[test]
    fn retry_backoff_while_hidraw_missing_then_recover() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        assert_eq!(m.due(t0), vec![Action::Acquire]);
        m.acquire_done(false, t0);
        assert!(
            m.due(t0 + Duration::from_millis(400)).is_empty(),
            "{:?}",
            m.due(t0 + Duration::from_millis(400))
        );
        assert_eq!(
            m.due(t0 + Duration::from_millis(500)),
            vec![Action::Acquire]
        );
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
        assert_eq!(
            m.on_event(&Event::Disconnected(MAC.into()), t0 + s(5)),
            Some(Action::Clear)
        );
        assert!(!m.is_connected());
        assert!(
            m.due(t0 + s(100_000)).is_empty(),
            "{:?}",
            m.due(t0 + s(100_000))
        );
        assert_eq!(m.next_deadline(), None);
        m.on_event(&Event::Connected(MAC.into()), t0 + s(10));
        assert_eq!(
            m.on_event(&Event::Disconnected("AA:BB:CC:DD:EE:FF".into()), t0 + s(11)),
            None
        );
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
        let got = m.due(t0 + s(1));
        assert!(got.is_empty(), "{got:?}");
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
    fn a_refused_refresh_says_why_and_how_long_to_wait() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        assert_eq!(m.request_refresh(t0), RefreshOutcome::Disconnected);
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.acquire_done(true, t0);
        assert_eq!(
            m.request_refresh(t0 + s(30)),
            RefreshOutcome::TooSoon {
                wait: FORCE_REFRESH_FLOOR.checked_sub(s(30)).unwrap()
            }
        );
        let t1 = t0 + FORCE_REFRESH_FLOOR;
        assert_eq!(m.request_refresh(t1), RefreshOutcome::Accepted);
    }

    #[test]
    fn refresh_forces_a_read_only_when_connected() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.request_refresh(t0);
        let got = m.due(t0);
        assert!(got.is_empty(), "{got:?}");
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.acquire_done(true, t0);
        m.due(t0);
        let got = m.due(t0 + s(1));
        assert!(got.is_empty(), "{got:?}");
        assert_ne!(m.request_refresh(t0 + s(1)), RefreshOutcome::Accepted);
        let got = m.due(t0 + s(1));
        assert!(got.is_empty(), "{got:?}");
        let t1 = t0 + FORCE_REFRESH_FLOOR + s(1);
        assert_eq!(m.request_refresh(t1), RefreshOutcome::Accepted);
        assert!(m.due(t1).contains(&Action::Acquire));
        m.acquire_done(false, t1 + s(1));
        m.acquire_done(false, t1 + s(2));
        assert_ne!(
            m.request_refresh(t1 + s(2)),
            RefreshOutcome::Accepted,
            "a recent refresh is ignored"
        );
        let t2 = t1 + FORCE_REFRESH_FLOOR + s(2);
        m.acquire_done(false, t2);
        assert_eq!(m.request_refresh(t2), RefreshOutcome::Accepted);
        assert!(m.due(t2).contains(&Action::Acquire));
        assert!(m.is_connected() && !m.is_acquired());
    }

    fn acquired_machine(t0: Instant) -> Machine {
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.due(t0);
        m.acquire_done(true, t0);
        m.due(t0);
        m
    }

    #[test]
    fn name_reread_is_its_own_action_and_never_a_battery_cycle() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        assert_eq!(m.request_name_reread(t0), NameReread::NotConnected);
        assert!(!NameReread::NotConnected.accepted());
        let got = m.due(t0);
        assert!(got.is_empty(), "{got:?}");
        let mut m = acquired_machine(t0);
        assert_eq!(m.request_name_reread(t0 + s(1)), NameReread::Now);
        assert_eq!(m.next_deadline(), Some(t0 + s(1)));
        let due = m.due(t0 + s(1));
        assert_eq!(due, vec![Action::RereadName]);
        assert!(!due.contains(&Action::Acquire), "no routine read");
        assert!(!m.vendor_reads_due(), "not a battery cycle");
        assert!(!m.name_reread_pending());
        assert_eq!(m.link().next_battery(), Some(t0 + s(60)));
    }

    #[test]
    fn name_reread_inside_the_floor_is_deferred_not_dropped() {
        let t0 = Instant::now();
        let mut m = acquired_machine(t0);
        assert_eq!(m.request_name_reread(t0 + s(1)), NameReread::Now);
        assert_eq!(m.due(t0 + s(1)), vec![Action::RereadName]);
        assert_eq!(
            m.request_name_reread(t0 + s(10)),
            NameReread::Deferred(s(21))
        );
        assert!(NameReread::Deferred(s(21)).accepted());
        assert!(m.name_reread_pending());
        assert_eq!(
            m.request_name_reread(t0 + s(20)),
            NameReread::Deferred(s(11))
        );
        assert!(!m.due(t0 + s(30)).contains(&Action::RereadName));
        let t1 = t0 + s(1) + NAME_REREAD_FLOOR;
        assert!(m.next_deadline().is_some_and(|d| d <= t1));
        assert_eq!(m.due(t1), vec![Action::RereadName], "served at the floor");
        assert!(!m.due(t1 + s(1)).contains(&Action::RereadName), "once");
        assert_eq!(NAME_REREAD_FLOOR, s(30));
        assert_eq!(
            m.request_name_reread(t1 + s(1)),
            NameReread::Deferred(s(29))
        );
        m.on_event(&Event::Disconnected(MAC.into()), t1 + s(2));
        assert!(!m.name_reread_pending());
    }

    #[test]
    fn name_reread_before_the_first_acquisition_waits_for_it() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        assert_eq!(m.request_name_reread(t0), NameReread::Deferred(s(0)));
        assert_eq!(m.due(t0), vec![Action::Acquire]);
        assert!(!m.vendor_reads_due());
        m.acquire_done(true, t0 + s(1));
        assert!(m.due(t0 + s(1)).contains(&Action::RereadName));
    }

    #[test]
    fn a_held_battery_cycle_is_not_consumed_and_goes_out_when_released() {
        let t0 = Instant::now();
        let mut m = acquired_machine(t0);
        m.due(t0 + s(60));
        m.acquire_done_with(true, Some(true), t0 + s(60));
        m.on_sleep();
        let wake = t0 + s(6000);
        m.on_wake(wake);
        assert_eq!(m.link().next_battery(), Some(wake + s(60)));
        m.set_vendor_hold(true);
        let due = m.due(wake + s(60));
        assert!(!due.contains(&Action::Acquire), "{due:?}");
        assert!(!m.vendor_reads_due());
        assert_eq!(m.link().next_battery(), Some(wake + s(60)), "not consumed");
        assert!(
            m.next_deadline().is_none_or(|d| d > wake + s(60)),
            "no spin"
        );
        assert_eq!(
            m.request_name_reread(wake + s(70)),
            NameReread::Deferred(s(0))
        );
        assert!(!m.due(wake + s(80)).contains(&Action::RereadName));
        m.set_vendor_hold(false);
        let due = m.due(wake + s(120));
        assert!(due.contains(&Action::Acquire), "{due:?}");
        assert!(due.contains(&Action::RereadName));
        assert!(m.vendor_reads_due());
        m.acquire_done_with(true, Some(true), wake + s(121));
        assert_eq!(
            m.link().next_battery(),
            Some(wake + s(121) + SLOW_READ_PERIOD),
            "a success: 4 h, never the 1 h retry of a failure"
        );
    }

    #[test]
    fn second_keyboard_never_takes_the_place_of_the_connected_one() {
        const OTHER: &str = "AA:BB:CC:DD:EE:02";
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.acquire_done(true, t0);
        m.due(t0);
        assert_eq!(m.on_event(&Event::Connected(OTHER.into()), t0 + s(1)), None);
        assert_eq!(m.mac(), Some(MAC));
        assert!(m.is_acquired());
        let got = m.due(t0 + s(1));
        assert!(got.is_empty(), "{got:?}");
        assert_eq!(m.standby(), [OTHER.to_string()]);
        m.on_event(&Event::Connected(OTHER.into()), t0 + s(2));
        assert_eq!(m.standby().len(), 1);
        assert_eq!(
            m.on_event(&Event::Disconnected(MAC.into()), t0 + s(3)),
            Some(Action::Clear)
        );
        assert!(m.is_connected());
        assert_eq!(m.mac(), Some(OTHER));
        assert_eq!(m.due(t0 + s(3)), vec![Action::Acquire]);
        let got = m.standby();
        assert!(got.is_empty(), "{got:?}");
        assert_eq!(
            m.on_event(&Event::Disconnected(OTHER.into()), t0 + s(4)),
            Some(Action::Clear)
        );
        assert!(!m.is_connected());
    }

    #[test]
    fn standby_keyboard_disconnect_changes_nothing() {
        const OTHER: &str = "AA:BB:CC:DD:EE:02";
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.acquire_done(true, t0);
        m.on_event(&Event::Connected(OTHER.into()), t0 + s(1));
        assert_eq!(
            m.on_event(&Event::Disconnected(OTHER.into()), t0 + s(2)),
            None
        );
        let got = m.standby();
        assert!(got.is_empty(), "{got:?}");
        assert_eq!(m.mac(), Some(MAC));
        assert!(m.is_acquired());
        assert_eq!(
            m.on_event(&Event::Disconnected(MAC.into()), t0 + s(3)),
            Some(Action::Clear)
        );
        assert!(!m.is_connected());
    }

    #[test]
    fn no_bluez_probing_gives_up_without_a_keyboard() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::NoBluez, t0);
        let mut t = t0;
        let mut acquires = 0;
        for _ in 0..(100 * 60) {
            if m.due(t).contains(&Action::Acquire) {
                acquires += 1;
                m.acquire_done(false, t);
            }
            t += s(1);
        }
        assert_eq!(acquires, NOBLUEZ_MAX_PROBES as usize, "bounded probing");
        assert!(!m.is_connected());
        assert_eq!(m.next_deadline(), None);
        m.on_event(&Event::Reconcile(vec![MAC.into()]), t);
        assert_eq!(m.mac(), Some(MAC));
        assert_eq!(m.due(t), vec![Action::Acquire]);
    }

    #[test]
    fn bluez_back_without_keyboard_ends_no_bluez_mode() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::NoBluez, t0);
        m.acquire_done(false, t0);
        assert_eq!(
            m.on_event(&Event::Reconcile(vec![]), t0 + s(30)),
            Some(Action::Clear)
        );
        assert!(!m.is_connected());
        assert!(
            m.due(t0 + s(100_000)).is_empty(),
            "{:?}",
            m.due(t0 + s(100_000))
        );
    }

    #[test]
    fn resync_reports_a_disconnection_missed_by_the_watcher() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.acquire_done(true, t0);
        assert_eq!(
            m.on_event(&Event::Reconcile(vec![]), t0 + s(60)),
            Some(Action::Clear)
        );
        assert!(!m.is_connected());
        m.on_event(&Event::Connected(MAC.into()), t0 + s(70));
        assert_eq!(m.due(t0 + s(70)), vec![Action::Acquire]);
    }

    #[test]
    fn reconcile_keeps_a_followed_keyboard_and_queues_others() {
        const OTHER: &str = "AA:BB:CC:DD:EE:02";
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        m.acquire_done(true, t0);
        assert_eq!(
            m.on_event(
                &Event::Reconcile(vec![OTHER.into(), MAC.into()]),
                t0 + s(10)
            ),
            None
        );
        assert_eq!(m.mac(), Some(MAC));
        assert!(
            m.is_acquired(),
            "no re-acquisition for a consistent snapshot"
        );
        assert_eq!(m.standby(), [OTHER.to_string()]);
        assert_eq!(
            m.on_event(&Event::Reconcile(vec![OTHER.into()]), t0 + s(20)),
            Some(Action::Clear)
        );
        assert_eq!(m.mac(), Some(OTHER));
        let got = m.standby();
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn stale_snapshot_never_drops_a_fresh_connection() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        m.on_event(&Event::Connected(MAC.into()), t0);
        assert_eq!(m.on_event(&Event::Reconcile(vec![]), t0 + s(1)), None);
        assert!(m.is_connected());
        assert_eq!(
            m.on_event(&Event::Reconcile(vec![]), t0 + RECONCILE_GRACE),
            Some(Action::Clear)
        );
    }

    #[test]
    fn reconcile_while_idle_follows_an_unannounced_keyboard() {
        let t0 = Instant::now();
        let mut m = Machine::new();
        assert_eq!(m.on_event(&Event::Reconcile(vec![MAC.into()]), t0), None);
        assert_eq!(m.due(t0), vec![Action::Acquire]);
        assert_eq!(
            m.on_event(&Event::Reconcile(vec![]), t0 + s(1)),
            None,
            "fresh"
        );
    }
}
