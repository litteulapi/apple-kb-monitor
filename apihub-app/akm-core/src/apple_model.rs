//! The model: what Apple's driver does with this keyboard, as one pure state machine.

use std::io;
use std::time::{Duration, Instant};

/// Every duration, threshold and counter of the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    /// R1. Handshake wait of the `SET_PROTOCOL(Report)` sent by `deviceReady`.
    pub set_protocol_timeout: Duration,
    /// R1. First battery read after the device is ready.
    pub first_battery_read: Duration,
    /// R1. Battery period when both reads succeeded.
    pub battery_period: Duration,
    /// R1. Battery period after a failed read (`0x47` or `0x30`).
    pub battery_retry: Duration,
    /// R2. Wait of a GET (data) or SET (HANDSHAKE).
    pub report_timeout: Duration,
    /// R2. Watchdog margin added to the wait.
    pub guard_margin: Duration,
    /// R4. Wait of the first request after a sleep.
    pub sleep_report_timeout: Duration,
    /// R4. Watchdog of that request.
    pub sleep_guard: Duration,
    /// R2. Minimum spacing of two requests to the keyboard.
    pub min_gap: Duration,
    /// R3. Consecutive timeouts that open the breaker.
    pub trip_after: u32,
    pub counter_after_sleep: u32,
    /// R3, Linux side. A failed request that came back faster than this is a HANDSHAKE refusal (an
    /// answer), slower is a silence.
    pub refusal_max: Duration,
    /// R3, Linux side. Bound of the `org.bluez.Device1.Disconnect` call that replaces
    /// `SetHIDDriverReady(false)`.
    pub disconnect_call_timeout: Duration,
    /// R5. Raw percentages from here on display 100.
    pub display_full_from: u8,
    /// R5. Start of the stretched part of the display curve.
    pub display_curve_from: u8,
    /// R5. Slope of the stretched part.
    pub display_slope: f64,
    /// R5. Ceiling of the raw percentage.
    pub percent_max: u8,
    /// R5. Highest valid battery state.
    pub battery_state_max: u8,
}

/// The values for the A1314 (PID 598).
pub const APPLE: Timing = Timing {
    set_protocol_timeout: Duration::from_secs(10),
    first_battery_read: Duration::from_mins(1),
    battery_period: Duration::from_hours(4),
    battery_retry: Duration::from_hours(1),
    report_timeout: Duration::from_millis(3_500),
    guard_margin: Duration::from_secs(1),
    sleep_report_timeout: Duration::from_millis(4_500),
    sleep_guard: Duration::from_millis(5_500),
    min_gap: Duration::from_secs(1),
    trip_after: 3,
    counter_after_sleep: 1,
    refusal_max: Duration::from_secs(1),
    disconnect_call_timeout: Duration::from_secs(5),
    display_full_from: 54,
    display_curve_from: 21,
    display_slope: 2.4375,
    percent_max: 100,
    battery_state_max: 3,
};

/// Report ids Apple uses with this keyboard.
pub mod ids {
    /// Feature `BatteryPercent`, read (R2).
    pub const BATTERY_PERCENT: u8 = 0x47;
    /// Input `BatteryState`, read after `0x47` and pushed (R2, R5).
    pub const BATTERY_STATE: u8 = 0x30;
    /// Feature `WillShutdown`, written with the id alone (R7).
    pub const WILL_SHUTDOWN: u8 = 0x40;
    /// Input of `AppleBluetoothHIDKeyboard` carrying the power bit (R6).
    pub const KEYBOARD_STATUS: u8 = 0x13;
    /// Bit of `0x13` that is 0 when the keyboard switches off (R6).
    pub const POWERED_ON_BIT: u8 = 0x02;
}

/// R2: the battery read, in Apple's order: GET Feature `0x47`, then GET Input `0x30`.
pub const BATTERY_READ: [Request; 2] = [
    Request::GetFeature(ids::BATTERY_PERCENT),
    Request::GetInput(ids::BATTERY_STATE),
];

/// What may go out to the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    GetFeature(u8),
    GetInput(u8),
    SetFeature(u8),
    /// `SET_PROTOCOL(Report)` `71`, sent once by `deviceReady` (R1).
    SetProtocol,
    /// `HID_CONTROL` `1n`: sent by `akm-helper hid-control`, which reads the daemon's published
    /// breaker and is blocked like the rest while it is open.
    HidControl(u8),
    /// DATA Output on the interrupt channel (`CapsLock` LED).
    Output,
}

/// How a request ended (R3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// DATA of the requested id, or HANDSHAKE SUCCESSFUL.
    Answered,
    /// HANDSHAKE error (`0x03` `UNSUPPORTED_REQUEST...)`: an answer all the same.
    Refused,
    /// Nothing before the watchdog.
    Timeout,
    /// DATA of another id: ignored by Apple, the request then expires.
    WrongId,
}

impl Outcome {
    /// Does it count against the breaker?
    #[must_use]
    pub fn is_silence(self) -> bool {
        matches!(self, Outcome::Timeout | Outcome::WrongId)
    }

    /// Classify the result of a Linux GET Feature of `id` that took `took`, against the watchdog
    /// `guard` of that request.
    #[must_use]
    pub fn classify(r: &io::Result<Vec<u8>>, id: u8, took: Duration, guard: Duration) -> Outcome {
        match r {
            Ok(_) if took > guard => Outcome::Timeout,
            Ok(b) if b.first() == Some(&id) => Outcome::Answered,
            Ok(_) => Outcome::WrongId,
            Err(e) if took >= APPLE.refusal_max || is_link_silence(e) => Outcome::Timeout,
            Err(_) => Outcome::Refused,
        }
    }
}

fn is_link_silence(e: &io::Error) -> bool {
    use io::ErrorKind as K;
    matches!(
        e.kind(),
        K::TimedOut | K::NotConnected | K::BrokenPipe | K::ConnectionAborted | K::ConnectionReset
    ) || matches!(
        e.raw_os_error(),
        Some(
            libc::ENODEV
                | libc::ENXIO
                | libc::EBADF
                | libc::ETIMEDOUT
                | libc::EHOSTDOWN
                | libc::ENOTCONN
                | libc::ESHUTDOWN
        )
    )
}

/// Wait and watchdog of one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    pub wait: Duration,
    pub guard: Duration,
}

/// `mHandshakeTimeoutCounter` (`+0xA1`) and the breaker flag (`+0xA0`) of `IOBluetoothHIDDriver`,
/// plus the one-shot disconnection request.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // reason: mirrors independent flags of the Apple driver
pub struct Breaker {
    counter: u32,
    open: bool,
    sleep_timeout: bool,
    disconnect_pending: bool,
    disconnect_done: bool,
}

impl Breaker {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            counter: 0,
            open: false,
            sleep_timeout: false,
            disconnect_pending: false,
            disconnect_done: false,
        }
    }

    #[cfg(test)]
    /// R3: flag `+0xA0`.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Consecutive silences so far.
    #[must_use]
    pub fn counter(&self) -> u32 {
        self.counter
    }

    /// R3: may a request go out?
    pub fn allow(&mut self) -> bool {
        !self.open
    }

    /// Same as [`Breaker::allow`] without consuming anything.
    #[must_use]
    pub fn would_allow(&self) -> bool {
        !self.open
    }

    /// Timeouts of the next request (R2, R4).
    #[must_use]
    pub fn timeouts(&self) -> Timeouts {
        if self.sleep_timeout {
            Timeouts {
                wait: APPLE.sleep_report_timeout,
                guard: APPLE.sleep_guard,
            }
        } else {
            Timeouts {
                wait: APPLE.report_timeout,
                guard: APPLE.report_timeout + APPLE.guard_margin,
            }
        }
    }

    /// A request goes out now: its timeouts.
    pub fn begin(&mut self) -> Timeouts {
        let t = self.timeouts();
        self.sleep_timeout = false;
        t
    }

    /// Compatibility form: `true` = answered, `false` = silence.
    pub fn record(&mut self, ok: bool) {
        self.record_outcome(if ok {
            Outcome::Answered
        } else {
            Outcome::Timeout
        });
    }

    /// R3: count the outcome.
    pub fn record_outcome(&mut self, o: Outcome) -> bool {
        if !o.is_silence() {
            self.counter = 0;
            self.open = false;
            return false;
        }
        self.counter = self.counter.saturating_add(1);
        if self.open || self.counter < APPLE.trip_after {
            return false;
        }
        self.open = true;
        if !self.disconnect_done {
            self.disconnect_done = true;
            self.disconnect_pending = true;
        }
        true
    }

    /// R3: the disconnection asked by the breaker, once (then false until the next connection).
    pub fn take_disconnect_request(&mut self) -> bool {
        std::mem::take(&mut self.disconnect_pending)
    }

    /// R4: system sleep: flag 0, counter 1, longer timeouts for the first request.
    pub fn after_sleep(&mut self) {
        self.counter = APPLE.counter_after_sleep;
        self.open = false;
        self.sleep_timeout = true;
    }

    /// New connection: a new driver object, everything from zero.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// No impossible state (property tests).
    ///
    /// # Errors
    ///
    /// The invariant that does not hold.
    pub fn check(&self) -> Result<(), &'static str> {
        if self.open && self.counter < APPLE.trip_after {
            return Err("breaker open below the threshold");
        }
        if self.disconnect_pending && !self.disconnect_done {
            return Err("pending disconnection not latched");
        }
        Ok(())
    }
}

/// Driver state of the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Disconnected,
    /// Connected, `SET_PROTOCOL` not answered yet.
    AwaitingProtocol,
    /// `deviceReady` done: generic commands and battery reads allowed.
    Ready,
    /// `SET_PROTOCOL` failed: never ready on this connection (R1).
    NotReady,
}

/// Connection, readiness and battery timer of `IOAppleBluetoothHIDDriver`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkModel {
    state: LinkState,
    sleeping: bool,
    next_battery: Option<Instant>,
    cycle: bool,
}

impl Default for LinkModel {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkModel {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: LinkState::Disconnected,
            sleeping: false,
            next_battery: None,
            cycle: false,
        }
    }

    #[must_use]
    pub fn state(&self) -> LinkState {
        self.state
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.state == LinkState::Ready
    }

    #[must_use]
    pub fn is_sleeping(&self) -> bool {
        self.sleeping
    }

    /// When the next battery read is due, if one is scheduled.
    #[must_use]
    pub fn next_battery(&self) -> Option<Instant> {
        self.next_battery
    }

    #[cfg(test)]
    /// A battery read started by [`LinkModel::begin_cycle`] is running.
    #[must_use]
    pub fn cycle_running(&self) -> bool {
        self.cycle
    }

    /// The keyboard connected: `deviceReady` will send `SET_PROTOCOL`.
    pub fn connected(&mut self) {
        *self = Self::new();
        self.state = LinkState::AwaitingProtocol;
    }

    /// R1: outcome of `SET_PROTOCOL`.
    pub fn protocol_result(&mut self, ok: bool, now: Instant) {
        if self.state != LinkState::AwaitingProtocol {
            return;
        }
        if ok {
            self.state = LinkState::Ready;
            self.next_battery = (!self.sleeping).then(|| now + APPLE.first_battery_read);
        } else {
            self.state = LinkState::NotReady;
        }
    }

    pub fn disconnected(&mut self) {
        *self = Self::new();
    }

    /// R4: `stopBatteryUpdate` (\[decompiled\] `IOAppleBluetoothHIDDriver::handleSleep`).
    pub fn sleep(&mut self) {
        self.sleeping = true;
        self.next_battery = None;
        self.cycle = false;
    }

    /// R4: `startBatteryUpdate`: a battery read 60 s after the wake.
    pub fn wake(&mut self, now: Instant) {
        self.sleeping = false;
        if self.is_ready() {
            self.next_battery = Some(now + APPLE.first_battery_read);
        }
    }

    /// R1: is the battery timer due?
    #[must_use]
    pub fn battery_due(&self, now: Instant) -> bool {
        !self.cycle && self.next_battery.is_some_and(|t| t <= now)
    }

    /// The timer fired: the read starts (true), or nothing was due (false).
    pub fn begin_cycle(&mut self, now: Instant) -> bool {
        let due = self.battery_due(now);
        self.cycle |= due;
        due
    }

    /// R1: end of the read: next one after [`Timing::battery_period`] if both requests succeeded,
    /// else after [`Timing::battery_retry`].
    pub fn end_cycle(&mut self, level_ok: bool, state_ok: bool, now: Instant) {
        if !std::mem::take(&mut self.cycle) {
            return;
        }
        let period = if level_ok && state_ok {
            APPLE.battery_period
        } else {
            APPLE.battery_retry
        };
        self.next_battery = Some(now + period);
    }

    /// Next instant something is due here (`None` while a read runs).
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.next_battery.filter(|_| !self.cycle)
    }

    /// No impossible state (property tests).
    ///
    /// # Errors
    ///
    /// The invariant that does not hold.
    pub fn check(&self) -> Result<(), &'static str> {
        if self.next_battery.is_some() && (self.state != LinkState::Ready || self.sleeping) {
            return Err("battery timer without a ready, awake device");
        }
        if self.cycle && self.state != LinkState::Ready {
            return Err("battery read on a device that is not ready");
        }
        Ok(())
    }
}

#[cfg(test)]
/// R5: the raw percentage of a `0x47` byte, as stored in `BatteryPercent`.
#[must_use]
pub fn raw_percent(byte: u8) -> u8 {
    byte.min(APPLE.percent_max)
}

/// R5: the percentage "as macOS shows it".
#[must_use]
pub fn display_percent(raw: u8) -> f64 {
    let (full, from) = (APPLE.display_full_from, APPLE.display_curve_from);
    if (full..=APPLE.percent_max).contains(&raw) {
        f64::from(APPLE.percent_max)
    } else if (from..full).contains(&raw) {
        f64::from(from) + f64::from(raw - from) * APPLE.display_slope
    } else {
        f64::from(raw)
    }
}

#[cfg(test)]
/// R5: notification of a battery state change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryNotice {
    /// `'rbsn'` (state 0): back to normal, no alert.
    Normal,
    /// `'btlw'` (state 1): `BluetoothHIDDeviceBatteryLow`.
    Low,
    /// `'btpn'` (states 2, 3): `BluetoothHIDDeviceBatteryDangerouslyLow`.
    DangerouslyLow,
}

#[cfg(test)]
impl BatteryNotice {
    /// None for a state above 3 (`0xE00002BC`).
    #[must_use]
    pub fn of_state(n: u8) -> Option<Self> {
        match n {
            0 => Some(Self::Normal),
            1 => Some(Self::Low),
            2..=3 => Some(Self::DangerouslyLow),
            _ => None,
        }
    }
}

#[cfg(test)]
/// An input report the driver itself decodes (R5, R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    BatteryState(u8),
    KeyboardOff,
}

#[cfg(test)]
#[must_use]
pub fn decode_input(report: &[u8]) -> Option<InputEvent> {
    match *report {
        [ids::BATTERY_STATE, n] => Some(InputEvent::BatteryState(n)),
        [ids::KEYBOARD_STATUS, b] if b & ids::POWERED_ON_BIT == 0 => Some(InputEvent::KeyboardOff),
        _ => None,
    }
}

/// Why a request may not go out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotConnected,
    /// R1: `SET_PROTOCOL` not (yet) answered: `kIOReturnNotReady`.
    NotReady,
    /// R3: `kIOReturnDeviceError`, nothing emitted.
    BreakerOpen,
    /// One request at a time (`IOCommandGate`).
    InFlight,
    /// R2: wait this long first.
    TooSoon(Duration),
    /// \[project\] the system is suspending / asleep: no hardware access.
    Sleeping,
    /// R7: `WillShutdown` was already sent in this run.
    AlreadySent,
}

#[cfg(test)]
/// What the daemon must do or tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// R3: ask the stack to disconnect the keyboard.
    RequestDisconnect,
    /// R5.
    Battery(BatteryNotice),
    /// R6.
    KeyboardOff,
}

/// How a disconnection must be shown (R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disconnection {
    Announced,
    /// After `KeyboardOff`: the disconnect notification is suppressed.
    Silent,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InFlight {
    req: Request,
    deadline: Instant,
}

#[cfg(test)]
/// Apple's driver for one keyboard, whole.
#[derive(Debug, Clone)]
pub struct AppleModel {
    pub breaker: Breaker,
    pub link: LinkModel,
    disconnect_on_breaker: bool,
    last_request: Option<Instant>,
    in_flight: Option<InFlight>,
    battery_state: u8,
    percent: Option<u8>,
    silent_disconnect: bool,
    shutdown_sent: bool,
    effects: Vec<Effect>,
}

#[cfg(test)]
impl AppleModel {
    #[must_use]
    pub fn new(disconnect_on_breaker: bool) -> Self {
        Self {
            breaker: Breaker::new(),
            link: LinkModel::new(),
            disconnect_on_breaker,
            last_request: None,
            in_flight: None,
            battery_state: 0,
            percent: None,
            silent_disconnect: false,
            shutdown_sent: false,
            effects: Vec::new(),
        }
    }

    /// New connection: new driver object (breaker closed, state 0).
    pub fn connected(&mut self) {
        self.link.connected();
        self.breaker.reset();
        self.in_flight = None;
        self.battery_state = 0;
        self.percent = None;
        self.silent_disconnect = false;
    }

    /// R6: how the disconnection is announced.
    pub fn disconnected(&mut self) -> Disconnection {
        self.link.disconnected();
        self.in_flight = None;
        if std::mem::take(&mut self.silent_disconnect) {
            Disconnection::Silent
        } else {
            Disconnection::Announced
        }
    }

    /// R4.
    pub fn sleep(&mut self) {
        self.link.sleep();
        self.breaker.after_sleep();
        self.in_flight = None;
    }

    /// R4.
    pub fn wake(&mut self, now: Instant) {
        self.link.wake(now);
    }

    /// Would `req` be allowed now (nothing changes)?
    ///
    /// # Errors
    ///
    /// The [`Refusal`] that [`Self::send`] would return.
    pub fn check_send(&self, req: Request, now: Instant) -> Result<(), Refusal> {
        let ready = match self.link.state() {
            LinkState::Disconnected => return Err(Refusal::NotConnected),
            LinkState::AwaitingProtocol => req == Request::SetProtocol,
            LinkState::Ready => req != Request::SetProtocol,
            LinkState::NotReady => false,
        };
        if self.link.is_sleeping() {
            return Err(Refusal::Sleeping);
        }
        if !self.breaker.would_allow() {
            return Err(Refusal::BreakerOpen);
        }
        if !ready {
            return Err(Refusal::NotReady);
        }
        if self.in_flight.is_some() {
            return Err(Refusal::InFlight);
        }
        let since = self.last_request.map(|t| now.saturating_duration_since(t));
        match since {
            Some(s) if s < APPLE.min_gap => Err(Refusal::TooSoon(APPLE.min_gap.saturating_sub(s))),
            _ => Ok(()),
        }
    }

    /// Record `req` as sent now and return the timeouts to arm.
    ///
    /// # Errors
    ///
    /// [`Refusal`] when the protocol forbids `req` in the current state.
    pub fn send(&mut self, req: Request, now: Instant) -> Result<Timeouts, Refusal> {
        self.check_send(req, now)?;
        self.breaker.allow();
        let mut t = self.breaker.begin();
        if req == Request::SetProtocol {
            t = Timeouts {
                wait: APPLE.set_protocol_timeout,
                guard: APPLE.set_protocol_timeout + APPLE.guard_margin,
            };
        }
        self.last_request = Some(now);
        self.in_flight = Some(InFlight {
            req,
            deadline: now + t.guard,
        });
        Ok(t)
    }

    /// The request in flight ended with `o`.
    pub fn complete(&mut self, o: Outcome, now: Instant) {
        let Some(f) = self.in_flight.take() else {
            return;
        };
        if self.breaker.record_outcome(o)
            && self.disconnect_on_breaker
            && self.breaker.take_disconnect_request()
        {
            self.effects.push(Effect::RequestDisconnect);
        }
        if f.req == Request::SetProtocol {
            self.link.protocol_result(o == Outcome::Answered, now);
        }
    }

    /// Watchdog: the request in flight expires at its guard.
    pub fn poll(&mut self, now: Instant) {
        if self.in_flight.is_some_and(|f| now >= f.deadline) {
            self.complete(Outcome::Timeout, now);
        }
    }

    /// R5: a `0x47` byte was read.
    pub fn battery_percent(&mut self, byte: u8) -> u8 {
        let p = raw_percent(byte);
        self.percent = Some(p);
        p
    }

    #[must_use]
    pub fn percent(&self) -> Option<u8> {
        self.percent
    }

    /// R5: a battery state: a notice on a change only; an invalid state is refused.
    pub fn battery_state(&mut self, n: u8) -> Option<BatteryNotice> {
        let notice = BatteryNotice::of_state(n)?;
        if n == self.battery_state {
            return None;
        }
        self.battery_state = n;
        self.effects.push(Effect::Battery(notice));
        Some(notice)
    }

    /// R5, R6: an input report from the interrupt channel.
    pub fn input(&mut self, report: &[u8]) {
        match decode_input(report) {
            Some(InputEvent::BatteryState(n)) => {
                self.battery_state(n);
            }
            Some(InputEvent::KeyboardOff) => {
                self.silent_disconnect = true;
                self.effects.push(Effect::KeyboardOff);
            }
            None => {}
        }
    }

    /// R7: `WillShutdown` (SET Feature `0x40`, id alone), once per run.
    ///
    /// # Errors
    ///
    /// [`Refusal`] when already sent or not allowed in the current state.
    pub fn will_shutdown(&mut self, now: Instant) -> Result<Timeouts, Refusal> {
        if self.shutdown_sent {
            return Err(Refusal::AlreadySent);
        }
        let t = self.send(Request::SetFeature(ids::WILL_SHUTDOWN), now)?;
        self.shutdown_sent = true;
        Ok(t)
    }

    #[cfg(test)]
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Earliest instant something is due: watchdog or battery timer.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        match (
            self.in_flight.map(|f| f.deadline),
            self.link.next_deadline(),
        ) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    #[must_use]
    pub fn in_flight(&self) -> Option<Request> {
        self.in_flight.map(|f| f.req)
    }

    /// No impossible state (property tests).
    ///
    /// # Errors
    ///
    /// The invariant that does not hold.
    pub fn check(&self) -> Result<(), &'static str> {
        self.breaker.check()?;
        self.link.check()?;
        if self.in_flight.is_some()
            && (self.link.state() == LinkState::Disconnected || self.link.is_sleeping())
        {
            return Err("request in flight without a link");
        }
        if self.battery_state > APPLE.battery_state_max {
            return Err("invalid battery state");
        }
        Ok(())
    }
}

#[cfg(test)]
/// One line of the rule table (`docs/APPLE-PARITY.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub id: &'static str,
    pub source: &'static str,
    /// Conformance test(s) of this module.
    pub test: &'static str,
}

#[cfg(test)]
/// Every rule with its source and its conformance test.
pub const RULES: &[Rule] = &[
    Rule { id: "R1 ready after SET_PROTOCOL", source: "[decompiled] IOBluetoothHIDDriver::deviceReady 0xfffffe000a35a588", test: "r1_set_protocol_failure_means_never_ready" },
    Rule { id: "R1 battery 60 s / 4 h / 1 h", source: "[decompiled] startBatteryUpdate 0xfffffe000a356034, IOAppleBluetoothHIDDriver::deviceReady 0xfffffe000a354f2c, batteryLevelTimerFired 0xfffffe000a355034", test: "r1_battery_schedule_60s_then_4h_or_1h" },
    Rule { id: "R2 GET 0x47 then Input 0x30", source: "[decompiled] batteryLevelTimerFired, updateBatteryLevel 0xfffffe000a35611c, getBatteryState", test: "r2_read_is_0x47_then_input_0x30" },
    Rule { id: "R2 3.5 s + 1 s guard, 1 s spacing, one at a time", source: "[plist] GetReportTimeoutMS 3500; [decompiled] waitForData 0xfffffe000a35daa0; IOBluetooth delay 0x3e8", test: "r2_timeouts_spacing_and_single_request" },
    Rule { id: "R3 breaker GET+SET, no emission, disconnect", source: "[decompiled] waitForData/waitForHandshake 0xfffffe000a35e478/waitForOkToSend 0xfffffe000a35ec50, sendData 0xfffffe000a35b9cc", test: "r3_three_silences_block_everything_and_ask_once" },
    Rule { id: "R3 refusal resets, wrong id expires", source: "[decompiled] DecodedHandshake 0xfffffe000a35e324, processControlData 0xfffffe000a35c898", test: "r3_refusal_is_an_answer_wrong_id_is_a_silence" },
    Rule { id: "R4 after sleep: counter 1, 4.5/5.5 s", source: "[decompiled] handleSleep 0xfffffe000a35abe4, handleWake 0xfffffe000a354e40", test: "r4_after_sleep_two_silences_suffice" },
    Rule { id: "R5 percentage and battery state", source: "[decompiled] batteryPercent 0x19641ba94, updateBatteryState 0xfffffe000a356378, serviceInterestOfType 0x19641b318", test: "r5_percent_display_and_state_notices" },
    Rule { id: "R6 0x13 bit 1 = KeyboardOff", source: "[decompiled] AppleBluetoothHIDKeyboard::processInterruptData 0xfffffe00090758a0", test: "r6_keyboard_off_silences_the_disconnection" },
    Rule { id: "R7 WillShutdown once", source: "[decompiled] willShutdown 0xfffffe000a35674c -> setExtendedReport 0xfffffe000a356f84", test: "r7_will_shutdown_once_and_only_when_allowed" },
];

#[cfg(test)]
mod tests;
