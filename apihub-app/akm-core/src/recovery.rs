//! Link health and recovery policy of a Bluetooth keyboard (#142, #144, #145).
//!
//! [`Recovery`] is pure (no D-Bus, no clock): the daemon feeds it BlueZ facts
//! (connected, disconnected + reason, result of a `Device1.Connect`), logind
//! facts (sleep / resume) and the current [`Instant`]; [`Recovery::poll`]
//! says what to do. Tests drive it with simulated time.
//!
//! Model (docs/RECONNEXION-PAIRAGE.md §5):
//!
//! * `Connected` — link up. Nothing to do.
//! * `Dormant` — link down with no sign of trouble: the keyboard went to
//!   sleep (it disconnects itself: reason *Remote*), someone disconnected it
//!   (*Local*), or the link was just lost and recovery is in progress. The
//!   keyboard comes back by itself on a key press; the host also pages it at
//!   a slow, bounded cadence in case its own reconnection failed while it is
//!   still awake and page-scanning (HIDNormallyConnectable = true).
//! * `Unreachable` — after a link loss (*Timeout*), a resume, or a daemon
//!   start without the keyboard, it has not answered for
//!   [`UNREACHABLE_AFTER`] despite [`UNREACHABLE_MIN_FAILURES`] attempts.
//!   One notification per episode; attempts continue, slower.
//! * `AuthFailed` — evidence that the stored pairing is no longer accepted
//!   (disconnect reason *Authentication*, `br-connection-key-missing`,
//!   `AuthenticationFailed/Rejected`, bond lost). Attempts STOP (they cannot
//!   succeed and only hammer the keyboard); the user is told a re-pairing is
//!   needed (`akmctl repair`). Never acted upon automatically.
//! * `Suspended` — the system is going to sleep / sleeping: no attempt.
//!
//! **Disconnection asked by the breaker** (#251, Apple's
//! `SetHIDDriverReady(false)` after 3 silences, `crate::apple_model`): the
//! daemon calls `Device1.Disconnect`, BlueZ reports reason *Local*, so the
//! episode is `Quiet`: no page before [`DORMANT_GRACE`], then the slow
//! cadence. Apple's `bluetoothd` does not page the keyboard after that either
//! (its HID auto-connect runs at a power-state change, i.e. our resume); the
//! keyboard comes back by itself on a key press, with a closed breaker.
//!
//! Never a tight loop: two attempts are at least [`MIN_SPACING`] apart, a
//! *busy* answer (BlueZ already connecting) is retried after [`BUSY_RETRY`]
//! without counting as a failure, and the cadence slows down to
//! [`SLOW_PERIOD`].

use std::time::{Duration, Instant};

/// Minimum time between two connection attempts, whatever happens.
pub const MIN_SPACING: Duration = Duration::from_secs(20);
/// After a "busy / in progress" answer.
pub const BUSY_RETRY: Duration = Duration::from_secs(30);
/// Wait after resume before the first attempt (BlueZ `ResumeDelay` is 2 s).
pub const RESUME_GRACE: Duration = Duration::from_secs(4);
/// First attempt after a link loss: leave BlueZ's own reconnection a chance.
pub const LOSS_GRACE: Duration = Duration::from_secs(20);
/// First attempt after the keyboard went to sleep / was disconnected on purpose.
pub const DORMANT_GRACE: Duration = Duration::from_secs(5 * 60);
/// Cadence after the initial schedule, during the first hour of an episode.
pub const MEDIUM_PERIOD: Duration = Duration::from_secs(5 * 60);
/// Cadence after the first hour of an episode.
pub const SLOW_PERIOD: Duration = Duration::from_secs(15 * 60);
/// Episode age after which the slow cadence applies.
pub const SLOW_AFTER: Duration = Duration::from_secs(3600);
/// Unreachable when lost for this long ...
pub const UNREACHABLE_AFTER: Duration = Duration::from_secs(10 * 60);
/// ... with at least this many unanswered pages.
pub const UNREACHABLE_MIN_FAILURES: u32 = 3;
/// Delays of the first attempts after a link loss / resume / start
/// (relative to the previous attempt), then [`MEDIUM_PERIOD`].
const LOSS_SCHEDULE: [Duration; 4] = [
    Duration::from_secs(20),
    Duration::from_secs(40),
    Duration::from_secs(60),
    Duration::from_secs(120),
];

/// Health of the link, as published (D-Bus `Health`, `akmctl doctor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// Not known yet (daemon start, BlueZ absent).
    Unknown,
    Connected,
    Dormant,
    Unreachable,
    AuthFailed,
    Suspended,
}

impl Health {
    pub fn as_str(self) -> &'static str {
        match self {
            Health::Unknown => "unknown",
            Health::Connected => "connected",
            Health::Dormant => "dormant",
            Health::Unreachable => "unreachable",
            Health::AuthFailed => "auth-failed",
            Health::Suspended => "suspended",
        }
    }

    pub fn parse(s: &str) -> Option<Health> {
        [
            Health::Unknown,
            Health::Connected,
            Health::Dormant,
            Health::Unreachable,
            Health::AuthFailed,
            Health::Suspended,
        ]
        .into_iter()
        .find(|h| h.as_str() == s)
    }
}

/// Why BlueZ says the link went down (`Device1.Disconnected` signal, 5.80+).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    /// Supervision timeout: out of range, batteries pulled, host radio stall.
    Timeout,
    /// Terminated by this host (bluetoothctl disconnect, idle timeout...).
    Local,
    /// Terminated by the keyboard (it goes to sleep / is switched off).
    Remote,
    /// Authentication failure: the pairing is not accepted any more.
    Authentication,
    /// Terminated by this host for system suspend.
    Suspend,
    /// No reason known (only `Connected = false` seen).
    Unknown,
}

impl DisconnectReason {
    /// From the D-Bus error-style name `org.bluez.Reason.*`.
    pub fn from_bluez(name: &str) -> Self {
        match name.rsplit('.').next().unwrap_or("") {
            "Timeout" => Self::Timeout,
            "Local" => Self::Local,
            "Remote" => Self::Remote,
            "Authentication" => Self::Authentication,
            "Suspend" => Self::Suspend,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Authentication => "authentication",
            Self::Suspend => "suspend",
            Self::Unknown => "unknown",
        }
    }
}

/// Outcome class of a failed `Device1.Connect`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// No answer to the page (`Host is down`, `br-connection-page-timeout`,
    /// and `br-connection-create-socket`, which is how BlueZ 5.87 reports a
    /// failed HID profile connection — the journal then says `Host is down`).
    NoAnswer,
    /// BlueZ is already connecting / busy: not a failure.
    Busy,
    /// Already connected: wait for the `Connected` property.
    AlreadyConnected,
    /// Pairing refused: key missing, authentication failed or rejected.
    Auth,
    /// The adapter is off or not ready.
    AdapterOff,
    /// The device object is gone (unpaired / removed).
    DeviceGone,
    /// Anything else (message kept for the diagnosis).
    Other(String),
}

impl ConnectError {
    /// Classify a D-Bus error (`name`, `message`) of `org.bluez.Device1.Connect`.
    pub fn classify(name: &str, message: &str) -> Self {
        let n = name.rsplit('.').next().unwrap_or(name);
        let m = message.to_ascii_lowercase();
        match n {
            "AlreadyConnected" => return Self::AlreadyConnected,
            "InProgress" | "Busy" => return Self::Busy,
            "AuthenticationFailed" | "AuthenticationRejected" | "AuthenticationTimeout" => {
                return Self::Auth
            }
            "NotReady" => return Self::AdapterOff,
            "UnknownObject" | "UnknownMethod" | "DoesNotExist" | "ServiceUnknown" => {
                return Self::DeviceGone
            }
            _ => {}
        }
        if m.contains("key-missing") || m.contains("authentication") {
            Self::Auth
        } else if m.contains("already-connected") || m.contains("already connected") {
            Self::AlreadyConnected
        } else if m.contains("busy") || m.contains("in progress") || m.contains("inprogress") {
            Self::Busy
        } else if m.contains("adapter-not-powered") || m.contains("not ready") {
            Self::AdapterOff
        } else if m.contains("page-timeout")
            || m.contains("create-socket")
            || m.contains("host is down")
            || m.contains("br-connection-timeout")
            || m.contains("aborted-by-remote")
            || m.contains("refused")
        {
            Self::NoAnswer
        } else {
            Self::Other(message.to_string())
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::NoAnswer => "no answer to the page (keyboard asleep or out of range)".into(),
            Self::Busy => "BlueZ busy (connection already in progress)".into(),
            Self::AlreadyConnected => "already connected".into(),
            Self::Auth => "pairing refused (link key missing or rejected)".into(),
            Self::AdapterOff => "adapter off or not ready".into(),
            Self::DeviceGone => "device unknown to BlueZ (unpaired?)".into(),
            Self::Other(m) => m.clone(),
        }
    }
}

/// What the user must be told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Lost for long despite attempts: press a key, then `akmctl doctor`.
    Unreachable { since: Instant },
    /// The pairing is not accepted any more: `akmctl repair`.
    RepairNeeded { why: String },
    /// Back after an `Unreachable` / `RepairNeeded` notice.
    Recovered,
}

/// What the daemon must do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Call `org.bluez.Device1.Connect` (asynchronously), then report with
    /// [`Recovery::on_connect_result`].
    Connect,
    Notify(Notice),
}

/// Why the current episode (link down) started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cause {
    /// Keyboard sleep or deliberate disconnection: slow attempts only.
    Quiet,
    /// Link loss, resume, start without the keyboard: active recovery.
    Lost,
}

#[derive(Debug, Clone)]
pub struct Recovery {
    health: Health,
    cause: Cause,
    episode_start: Option<Instant>,
    next_attempt: Option<Instant>,
    last_attempt: Option<Instant>,
    in_flight: bool,
    attempts: u32,
    failures: u32,
    step: usize,
    notified: bool,
    pending: Vec<Notice>,
    last_error: Option<String>,
    last_reason: Option<DisconnectReason>,
    sleeping: bool,
    adapter_on: bool,
    paired: bool,
    /// The user asked for this disconnection (tray "Disconnect", #104): the
    /// host does not page the keyboard until it comes back by itself or the
    /// user asks to reconnect.
    held: bool,
}

impl Default for Recovery {
    fn default() -> Self {
        Self::new()
    }
}

impl Recovery {
    pub fn new() -> Self {
        Self {
            health: Health::Unknown,
            cause: Cause::Lost,
            episode_start: None,
            next_attempt: None,
            last_attempt: None,
            in_flight: false,
            attempts: 0,
            failures: 0,
            step: 0,
            notified: false,
            pending: Vec::new(),
            last_error: None,
            last_reason: None,
            sleeping: false,
            adapter_on: true,
            paired: true,
            held: false,
        }
    }

    /// The host pages nothing: the user disconnected the keyboard on purpose.
    pub fn held_by_user(&self) -> bool {
        self.held
    }

    // ── read side ──────────────────────────────────────────────────────────

    pub fn health(&self) -> Health {
        self.health
    }
    /// Start of the current episode (link down), if any.
    pub fn since(&self) -> Option<Instant> {
        self.episode_start
    }
    /// Connection attempts made by the daemon in this episode.
    /// BlueZ last said the device is paired (bonded).
    pub fn paired(&self) -> bool {
        self.paired
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }
    /// Unanswered pages in this episode.
    pub fn failures(&self) -> u32 {
        self.failures
    }
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
    pub fn last_reason(&self) -> Option<DisconnectReason> {
        self.last_reason
    }
    /// Next attempt (for the event loop timeout); `None` = wait for events.
    pub fn next_deadline(&self) -> Option<Instant> {
        if self.in_flight || !self.may_attempt() {
            return None;
        }
        self.next_attempt
    }

    fn may_attempt(&self) -> bool {
        !self.sleeping
            && !self.held
            && self.adapter_on
            && self.paired
            && matches!(self.health, Health::Dormant | Health::Unreachable)
    }

    // ── inputs ─────────────────────────────────────────────────────────────

    /// Initial state when the daemon starts (or BlueZ reappears).
    pub fn start(&mut self, connected: bool, paired: bool, now: Instant) {
        self.paired = paired;
        if connected {
            self.on_connected(now);
        } else if !paired {
            self.enter_auth_failed("the keyboard is not paired with this computer", now);
        } else {
            // After a reboot the keyboard is asleep: it must page us. Recover
            // actively (this is the case of the forced re-pairing, #142).
            self.begin_episode(Cause::Lost, now, RESUME_GRACE);
        }
    }

    pub fn on_connected(&mut self, _now: Instant) {
        let was_bad = self.notified;
        self.health = Health::Connected;
        self.episode_start = None;
        self.next_attempt = None;
        self.attempts = 0;
        self.failures = 0;
        self.step = 0;
        self.notified = false;
        self.last_error = None;
        self.paired = true;
        self.held = false;
        if was_bad {
            self.pending.push(Notice::Recovered);
        }
    }

    pub fn on_disconnected(&mut self, reason: DisconnectReason, now: Instant) {
        // `Connected = false` (reason unknown) and the `Disconnected` signal
        // (reason known) arrive in either order: a known reason refines an
        // episode opened without one, as long as nothing was attempted yet.
        let refine = self.health == Health::Dormant
            && self.attempts == 0
            && self.last_reason == Some(DisconnectReason::Unknown)
            && reason != DisconnectReason::Unknown;
        if refine {
            self.health = Health::Connected;
        }
        self.last_reason = Some(reason);
        match reason {
            DisconnectReason::Authentication => {
                self.enter_auth_failed("the keyboard refused the stored pairing", now)
            }
            DisconnectReason::Suspend => {
                self.sleeping = true;
                self.health = Health::Suspended;
                self.episode_start.get_or_insert(now);
                self.next_attempt = None;
            }
            DisconnectReason::Remote | DisconnectReason::Local => {
                if self.health == Health::Connected || self.health == Health::Unknown {
                    self.begin_episode(Cause::Quiet, now, DORMANT_GRACE);
                }
            }
            DisconnectReason::Timeout | DisconnectReason::Unknown => {
                if self.health == Health::Connected || self.health == Health::Unknown {
                    self.begin_episode(Cause::Lost, now, LOSS_GRACE);
                }
            }
        }
    }

    /// Result of the `Connect` started by [`Action::Connect`].
    pub fn on_connect_result(&mut self, r: Result<(), ConnectError>, now: Instant) {
        self.in_flight = false;
        let err = match r {
            Ok(()) => {
                // Connected arrives as a property change; keep waiting for it
                // but do not page again before MIN_SPACING.
                self.next_attempt = Some(now + MIN_SPACING);
                return;
            }
            Err(e) => e,
        };
        self.last_error = Some(err.describe());
        match err {
            ConnectError::Busy | ConnectError::AlreadyConnected => {
                self.next_attempt = Some(now + BUSY_RETRY);
            }
            ConnectError::Auth => {
                self.enter_auth_failed("BlueZ reports the pairing as refused", now)
            }
            ConnectError::DeviceGone => {
                self.paired = false;
                self.enter_auth_failed("the keyboard is no longer known to BlueZ", now)
            }
            ConnectError::AdapterOff => {
                self.adapter_on = false;
                self.next_attempt = None;
            }
            ConnectError::NoAnswer | ConnectError::Other(_) => {
                self.failures = self.failures.saturating_add(1);
                self.schedule_next(now);
                self.check_unreachable(now);
            }
        }
    }

    /// logind `PrepareForSleep(true)`.
    pub fn on_sleep(&mut self, now: Instant) {
        self.sleeping = true;
        self.in_flight = false;
        self.next_attempt = None;
        if self.health != Health::AuthFailed {
            self.health = Health::Suspended;
        }
        self.episode_start.get_or_insert(now);
    }

    /// logind `PrepareForSleep(false)`. `connected`: BlueZ state now.
    pub fn on_resume(&mut self, connected: bool, now: Instant) {
        self.sleeping = false;
        if self.health == Health::AuthFailed {
            return;
        }
        if connected {
            self.on_connected(now);
        } else {
            // Fresh episode: the keyboard lost the link during our sleep and
            // is probably still awake and page-scanning: be quick.
            self.begin_episode(Cause::Lost, now, RESUME_GRACE);
        }
    }

    /// Adapter `Powered` changed.
    pub fn on_adapter(&mut self, powered: bool, now: Instant) {
        self.adapter_on = powered;
        if powered && matches!(self.health, Health::Dormant | Health::Unreachable) {
            self.next_attempt = Some(self.spaced(now + RESUME_GRACE));
        }
    }

    /// `Paired`/`Bonded` became false or the device object vanished.
    pub fn on_bond_lost(&mut self, now: Instant) {
        self.paired = false;
        self.enter_auth_failed("the pairing was removed", now);
    }

    /// The user asked for an immediate attempt (tray / akmctl). Still honours
    /// [`MIN_SPACING`] and never runs while suspended or refused.
    pub fn request_now(&mut self, now: Instant) {
        // "Reconnect" ends a disconnection the user had asked for.
        if self.held {
            self.held = false;
            if self.next_attempt.is_none() {
                self.next_attempt = Some(self.spaced(now));
            }
        }
        if self.may_attempt() && !self.in_flight {
            let t = self.spaced(now);
            self.next_attempt = Some(self.next_attempt.map_or(t, |n| n.min(t)));
        }
    }

    /// The user asks to disconnect the keyboard (tray, #104). The episode
    /// that follows (BlueZ reports reason *Local*) is quiet AND held: no page
    /// at all, until the keyboard reconnects by itself (a key press) or the
    /// user asks to reconnect ([`Recovery::request_now`]).
    pub fn on_user_disconnect(&mut self, _now: Instant) {
        self.held = true;
        self.in_flight = false;
        self.next_attempt = None;
    }

    // ── output ─────────────────────────────────────────────────────────────

    /// Actions due at `now` (notifications first).
    pub fn poll(&mut self, now: Instant) -> Vec<Action> {
        let mut out: Vec<Action> = self.pending.drain(..).map(Action::Notify).collect();
        if self.may_attempt() && !self.in_flight && self.next_attempt.is_some_and(|t| t <= now) {
            self.in_flight = true;
            self.attempts = self.attempts.saturating_add(1);
            self.last_attempt = Some(now);
            self.next_attempt = None;
            out.push(Action::Connect);
        }
        out
    }

    // ── internals ──────────────────────────────────────────────────────────

    fn begin_episode(&mut self, cause: Cause, now: Instant, first: Duration) {
        self.cause = cause;
        self.health = Health::Dormant;
        self.episode_start = Some(now);
        self.attempts = 0;
        self.failures = 0;
        self.step = 0;
        self.in_flight = false;
        self.next_attempt = Some(self.spaced(now + first));
    }

    fn enter_auth_failed(&mut self, why: &str, now: Instant) {
        let first = self.health != Health::AuthFailed;
        self.health = Health::AuthFailed;
        self.episode_start.get_or_insert(now);
        self.next_attempt = None;
        self.in_flight = false;
        self.last_error = Some(why.to_string());
        if first {
            self.notified = true;
            self.pending.push(Notice::RepairNeeded {
                why: why.to_string(),
            });
        }
    }

    /// Never closer than MIN_SPACING to the previous attempt.
    fn spaced(&self, t: Instant) -> Instant {
        match self.last_attempt {
            Some(l) if t < l + MIN_SPACING => l + MIN_SPACING,
            _ => t,
        }
    }

    fn schedule_next(&mut self, now: Instant) {
        let age = self.episode_start.map_or(Duration::ZERO, |s| now - s);
        let delay = if age >= SLOW_AFTER {
            SLOW_PERIOD
        } else if self.cause == Cause::Lost && self.step < LOSS_SCHEDULE.len() {
            let d = LOSS_SCHEDULE[self.step];
            self.step += 1;
            d
        } else {
            MEDIUM_PERIOD
        };
        self.next_attempt = Some(self.spaced(now + delay));
    }

    fn check_unreachable(&mut self, now: Instant) {
        if self.cause != Cause::Lost || self.health != Health::Dormant {
            return;
        }
        let Some(start) = self.episode_start else {
            return;
        };
        if now - start >= UNREACHABLE_AFTER && self.failures >= UNREACHABLE_MIN_FAILURES {
            self.health = Health::Unreachable;
            if !self.notified {
                self.notified = true;
                self.pending.push(Notice::Unreachable { since: start });
            }
        }
    }
}

// ── bluetoothd journal classification (akmctl doctor) ─────────────────────

/// Category of a bluetoothd journal line about the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JournalKind {
    /// `connect to …: Host is down (112)` — page timeout.
    PageTimeout,
    /// `HIDP GET_REPORT request timed out`.
    GetReportTimeout,
    /// Authentication / key problems.
    Auth,
    /// `Connection reset by peer` / refused.
    Refused,
    /// `Unknown key … for group …` — main.conf key in the wrong section.
    ConfigIgnored,
    /// Storage write failure (`Unable set contents`, `No space left`).
    StorageError,
}

impl JournalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PageTimeout => "page-timeout",
            Self::GetReportTimeout => "get-report-timeout",
            Self::Auth => "auth",
            Self::Refused => "refused",
            Self::ConfigIgnored => "config-ignored",
            Self::StorageError => "storage-error",
        }
    }
}

/// Classify one bluetoothd journal message; `None` = not relevant.
pub fn classify_journal(msg: &str) -> Option<JournalKind> {
    let m = msg.to_ascii_lowercase();
    if m.contains("host is down") || m.contains("page timeout") {
        Some(JournalKind::PageTimeout)
    } else if m.contains("get_report request timed out") {
        Some(JournalKind::GetReportTimeout)
    } else if m.contains("authentication")
        || m.contains("key missing")
        || m.contains("pin or key")
        || m.contains("bonding failed")
    {
        Some(JournalKind::Auth)
    } else if m.contains("connection reset by peer") || m.contains("connection refused") {
        Some(JournalKind::Refused)
    } else if m.contains("unknown key") && m.contains("for group") {
        Some(JournalKind::ConfigIgnored)
    } else if m.contains("unable set contents") || m.contains("no space left") {
        Some(JournalKind::StorageError)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {

    /// #251: the disconnection asked by the breaker (reason Local) is a quiet
    /// episode: the keyboard that stopped answering is not paged right away.
    #[test]
    fn breaker_disconnection_is_a_quiet_episode() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.on_connected(t0);
        r.on_disconnected(DisconnectReason::Local, t0);
        assert_eq!(r.health(), Health::Dormant);
        assert_eq!(r.next_deadline(), Some(t0 + DORMANT_GRACE));
        assert!(r.poll(t0 + DORMANT_GRACE - Duration::from_secs(1)).is_empty());
        // the keyboard reconnects by itself: nothing left to do
        r.on_connected(t0 + Duration::from_secs(30));
        assert_eq!(r.health(), Health::Connected);
        assert_eq!(r.next_deadline(), None);
        // a resume (Apple: HID auto-connect at the power-state change) pages
        r.on_disconnected(DisconnectReason::Local, t0 + Duration::from_secs(40));
        r.on_sleep(t0 + Duration::from_secs(50));
        r.on_resume(false, t0 + Duration::from_secs(60));
        assert_eq!(r.next_deadline(), Some(t0 + Duration::from_secs(60) + RESUME_GRACE));
    }
    use super::*;

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn connects(a: &[Action]) -> usize {
        a.iter().filter(|x| **x == Action::Connect).count()
    }

    /// Run the machine for `span` with every attempt unanswered; returns the
    /// attempt instants.
    fn run_unanswered(r: &mut Recovery, from: Instant, span: Duration) -> Vec<Instant> {
        let mut t = from;
        let end = from + span;
        let mut at = Vec::new();
        while t <= end {
            for a in r.poll(t) {
                if a == Action::Connect {
                    at.push(t);
                    r.on_connect_result(Err(ConnectError::NoAnswer), t + s(5));
                }
            }
            t += s(1);
        }
        at
    }

    /// #104: "Disconnect" from the tray is never undone by the host; the
    /// keyboard coming back by itself, or "Reconnect", ends it.
    #[test]
    fn a_disconnection_asked_by_the_user_is_never_paged_until_asked() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut r = Recovery::new();
        r.start(true, true, t0);
        r.on_user_disconnect(t0);
        r.on_disconnected(DisconnectReason::Local, t0 + s(1));
        assert_eq!(r.health(), Health::Dormant);
        assert!(r.held_by_user());
        assert_eq!(r.next_deadline(), None, "nothing scheduled");
        // A whole day: not a single page.
        let mut t = t0;
        for _ in 0..(24 * 60) {
            t += s(60);
            assert!(r.poll(t).is_empty());
        }
        assert_eq!(r.attempts(), 0);
        // "Reconnect": one page now, then the usual spacing.
        r.request_now(t);
        assert!(!r.held_by_user());
        assert_eq!(r.poll(t), [Action::Connect]);
        r.on_connect_result(Err(ConnectError::NoAnswer), t + s(5));
        assert!(r.next_deadline().is_some_and(|d| d >= t + MIN_SPACING));
        // The keyboard comes back by itself: nothing is held any more.
        let mut r = Recovery::new();
        r.start(true, true, t0);
        r.on_user_disconnect(t0);
        r.on_disconnected(DisconnectReason::Local, t0);
        r.on_connected(t0 + s(600));
        assert!(!r.held_by_user());
        r.on_disconnected(DisconnectReason::Timeout, t0 + s(700));
        assert!(r.next_deadline().is_some(), "an ordinary loss is recovered");
    }

    /// #104: "Reconnect" twice in a row never pages faster than MIN_SPACING.
    #[test]
    fn reconnect_requests_keep_the_minimum_spacing() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut r = Recovery::new();
        r.start(false, true, t0);
        r.request_now(t0);
        assert_eq!(r.poll(t0), [Action::Connect]);
        r.on_connect_result(Err(ConnectError::NoAnswer), t0 + s(2));
        r.request_now(t0 + s(3));
        assert!(r.poll(t0 + s(3)).is_empty(), "3 s after the last page");
        assert!(r.poll(t0 + MIN_SPACING - s(1)).is_empty());
        assert_eq!(r.poll(t0 + MIN_SPACING), [Action::Connect]);
        // Never while the pairing is refused.
        let mut r = Recovery::new();
        r.start(false, false, t0);
        r.request_now(t0);
        assert!(!r.poll(t0 + s(60)).contains(&Action::Connect));
        assert_eq!(r.attempts(), 0);
    }

    #[test]
    fn classify_bluez_errors() {
        use ConnectError::*;
        let c = ConnectError::classify;
        assert_eq!(
            c("org.bluez.Error.Failed", "br-connection-create-socket"),
            NoAnswer
        );
        assert_eq!(
            c("org.bluez.Error.Failed", "br-connection-page-timeout"),
            NoAnswer
        );
        assert_eq!(c("org.bluez.Error.Failed", "Host is down"), NoAnswer);
        assert_eq!(
            c("org.bluez.Error.Failed", "br-connection-key-missing"),
            Auth
        );
        assert_eq!(c("org.bluez.Error.AuthenticationFailed", ""), Auth);
        assert_eq!(c("org.bluez.Error.AuthenticationRejected", "x"), Auth);
        assert_eq!(c("org.bluez.Error.InProgress", "In Progress"), Busy);
        assert_eq!(c("org.bluez.Error.Failed", "br-connection-busy"), Busy);
        assert_eq!(c("org.bluez.Error.AlreadyConnected", ""), AlreadyConnected);
        assert_eq!(
            c("org.bluez.Error.NotReady", "Resource Not Ready"),
            AdapterOff
        );
        assert_eq!(
            c(
                "org.bluez.Error.Failed",
                "br-connection-adapter-not-powered"
            ),
            AdapterOff
        );
        assert_eq!(
            c("org.freedesktop.DBus.Error.UnknownObject", "no such path"),
            DeviceGone
        );
        assert_eq!(c("org.bluez.Error.Failed", "weird"), Other("weird".into()));
    }

    #[test]
    fn disconnect_reasons_from_bluez() {
        use DisconnectReason::*;
        assert_eq!(
            DisconnectReason::from_bluez("org.bluez.Reason.Timeout"),
            Timeout
        );
        assert_eq!(
            DisconnectReason::from_bluez("org.bluez.Reason.Remote"),
            Remote
        );
        assert_eq!(
            DisconnectReason::from_bluez("org.bluez.Reason.Authentication"),
            Authentication
        );
        assert_eq!(
            DisconnectReason::from_bluez("org.bluez.Reason.Suspend"),
            Suspend
        );
        assert_eq!(
            DisconnectReason::from_bluez("org.bluez.Reason.Local"),
            Local
        );
        assert_eq!(DisconnectReason::from_bluez("garbage"), Unknown);
        for h in [
            "connected",
            "dormant",
            "unreachable",
            "auth-failed",
            "suspended",
            "unknown",
        ] {
            assert_eq!(Health::parse(h).unwrap().as_str(), h);
        }
    }

    #[test]
    fn connected_does_nothing() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(true, true, t0);
        assert_eq!(r.health(), Health::Connected);
        assert!(r.poll(t0 + s(100_000)).is_empty());
        assert_eq!(r.next_deadline(), None);
    }

    #[test]
    fn keyboard_sleep_is_dormant_and_quiet() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(true, true, t0);
        r.on_disconnected(DisconnectReason::Remote, t0 + s(10));
        assert_eq!(r.health(), Health::Dormant);
        // nothing before DORMANT_GRACE
        assert_eq!(connects(&r.poll(t0 + s(10) + DORMANT_GRACE - s(1))), 0);
        // 24 h asleep: slow attempts only, never "unreachable", no notice
        let at = run_unanswered(&mut r, t0 + s(10), s(24 * 3600));
        assert_eq!(r.health(), Health::Dormant);
        assert!(
            at.len() > 10 && at.len() < 24 * 4 + 12 + 2,
            "{} attempts",
            at.len()
        );
        for w in at.windows(2) {
            assert!(w[1] - w[0] >= MEDIUM_PERIOD, "dormant cadence too fast");
        }
        assert!(!r
            .poll(t0 + s(24 * 3600 + 20))
            .iter()
            .any(|a| matches!(a, Action::Notify(_))));
    }

    #[test]
    fn link_loss_recovers_actively_then_slows_down_and_notifies_once() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(true, true, t0);
        r.on_disconnected(DisconnectReason::Timeout, t0);
        assert_eq!(r.next_deadline(), Some(t0 + LOSS_GRACE));
        let mut notices = 0;
        let mut at = Vec::new();
        let mut t = t0;
        while t <= t0 + s(3 * 3600) {
            for a in r.poll(t) {
                match a {
                    Action::Connect => {
                        at.push(t);
                        r.on_connect_result(Err(ConnectError::NoAnswer), t + s(5));
                    }
                    Action::Notify(Notice::Unreachable { since }) => {
                        assert_eq!(since, t0);
                        notices += 1;
                    }
                    Action::Notify(n) => panic!("unexpected {n:?}"),
                }
            }
            t += s(1);
        }
        assert_eq!(notices, 1, "one notification per episode");
        assert_eq!(r.health(), Health::Unreachable);
        // never a tight loop
        for w in at.windows(2) {
            assert!(w[1] - w[0] >= MIN_SPACING);
        }
        // first hour: initial schedule then 5 min; afterwards 15 min
        let late: Vec<_> = at
            .iter()
            .filter(|x| **x >= t0 + SLOW_AFTER + SLOW_PERIOD)
            .collect();
        for w in late.windows(2) {
            assert!(*w[1] - *w[0] >= SLOW_PERIOD);
        }
        assert!(at.len() <= 4 + 12 + 8 + 2, "{} attempts in 3 h", at.len());
        // the keyboard comes back: recovered notice, attempts stop
        r.on_connected(t);
        assert_eq!(r.poll(t), vec![Action::Notify(Notice::Recovered)]);
        assert!(r.poll(t + s(100_000)).is_empty());
    }

    #[test]
    fn reboot_without_keyboard_starts_recovery_quickly() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(false, true, t0);
        assert_eq!(r.health(), Health::Dormant);
        assert_eq!(r.next_deadline(), Some(t0 + RESUME_GRACE));
        assert_eq!(connects(&r.poll(t0 + RESUME_GRACE)), 1);
        // in flight: no second attempt whatever the time
        assert_eq!(connects(&r.poll(t0 + s(600))), 0);
        assert_eq!(r.next_deadline(), None);
    }

    #[test]
    fn busy_is_not_a_failure() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(false, true, t0);
        r.poll(t0 + RESUME_GRACE);
        r.on_connect_result(Err(ConnectError::Busy), t0 + s(5));
        assert_eq!(r.failures(), 0);
        assert_eq!(r.next_deadline(), Some(t0 + s(5) + BUSY_RETRY));
    }

    #[test]
    fn auth_failure_stops_attempts_and_asks_for_repair_once() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(true, true, t0);
        r.on_disconnected(DisconnectReason::Authentication, t0 + s(1));
        assert_eq!(r.health(), Health::AuthFailed);
        let a = r.poll(t0 + s(1));
        assert!(matches!(
            a.as_slice(),
            [Action::Notify(Notice::RepairNeeded { .. })]
        ));
        assert!(
            r.poll(t0 + s(100_000)).is_empty(),
            "no attempt, no repeated notice"
        );
        r.on_disconnected(DisconnectReason::Authentication, t0 + s(2));
        assert!(r.poll(t0 + s(3)).is_empty());
        // resume does not leave auth-failed
        r.on_sleep(t0 + s(10));
        r.on_resume(false, t0 + s(20));
        assert_eq!(r.health(), Health::AuthFailed);
        // the user re-paired: connected again, recovered
        r.on_connected(t0 + s(30));
        assert_eq!(r.poll(t0 + s(30)), vec![Action::Notify(Notice::Recovered)]);
    }

    #[test]
    fn key_missing_during_recovery_switches_to_auth_failed() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(false, true, t0);
        assert_eq!(connects(&r.poll(t0 + RESUME_GRACE)), 1);
        r.on_connect_result(Err(ConnectError::Auth), t0 + s(6));
        assert_eq!(r.health(), Health::AuthFailed);
        assert!(matches!(
            r.poll(t0 + s(6)).as_slice(),
            [Action::Notify(Notice::RepairNeeded { .. })]
        ));
        assert_eq!(r.next_deadline(), None);
    }

    #[test]
    fn unpaired_keyboard_is_never_paged() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(false, false, t0);
        assert_eq!(r.health(), Health::AuthFailed);
        let a = r.poll(t0 + s(10_000));
        assert_eq!(connects(&a), 0);
        let mut r = Recovery::new();
        r.start(true, true, t0);
        r.on_bond_lost(t0 + s(1));
        assert_eq!(r.health(), Health::AuthFailed);
        assert_eq!(connects(&r.poll(t0 + s(10_000))), 0);
    }

    #[test]
    fn suspend_resume_cycle() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(true, true, t0);
        r.on_sleep(t0 + s(1));
        r.on_disconnected(DisconnectReason::Suspend, t0 + s(2));
        assert_eq!(r.health(), Health::Suspended);
        assert!(r.poll(t0 + s(3600)).is_empty(), "nothing while sleeping");
        // resume with the keyboard still connected (rare) → connected
        let mut r2 = r.clone();
        r2.on_resume(true, t0 + s(3600));
        assert_eq!(r2.health(), Health::Connected);
        // usual: link gone → quick recovery after the grace delay
        r.on_resume(false, t0 + s(3600));
        assert_eq!(r.health(), Health::Dormant);
        assert_eq!(connects(&r.poll(t0 + s(3600) + RESUME_GRACE - s(1))), 0);
        assert_eq!(connects(&r.poll(t0 + s(3600) + RESUME_GRACE)), 1);
        r.on_connect_result(Err(ConnectError::NoAnswer), t0 + s(3610));
        // second attempt follows the loss schedule (20 s), not 5 min
        assert_eq!(r.next_deadline(), Some(t0 + s(3610) + LOSS_SCHEDULE[0]));
        // sleeping again in the middle of recovery: attempts stop at once
        r.on_sleep(t0 + s(3615));
        assert_eq!(r.next_deadline(), None);
        assert!(r.poll(t0 + s(9000)).is_empty());
    }

    #[test]
    fn reason_arriving_after_connected_false_refines_the_episode() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(true, true, t0);
        r.on_disconnected(DisconnectReason::Unknown, t0);
        assert_eq!(r.next_deadline(), Some(t0 + LOSS_GRACE));
        r.on_disconnected(DisconnectReason::Remote, t0);
        assert_eq!(
            r.next_deadline(),
            Some(t0 + DORMANT_GRACE),
            "sleep, not loss"
        );
        // a second known reason does not restart anything
        r.on_disconnected(DisconnectReason::Timeout, t0 + s(1));
        assert_eq!(r.next_deadline(), Some(t0 + DORMANT_GRACE));
        // ...but an authentication failure always wins
        r.on_disconnected(DisconnectReason::Authentication, t0 + s(2));
        assert_eq!(r.health(), Health::AuthFailed);
    }

    #[test]
    fn adapter_off_pauses_attempts() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(false, true, t0);
        r.on_adapter(false, t0 + s(1));
        assert!(r.poll(t0 + s(1000)).is_empty());
        r.on_adapter(true, t0 + s(1000));
        assert_eq!(connects(&r.poll(t0 + s(1000) + RESUME_GRACE)), 1);
    }

    #[test]
    fn manual_request_respects_min_spacing() {
        let t0 = Instant::now();
        let mut r = Recovery::new();
        r.start(false, true, t0);
        assert_eq!(connects(&r.poll(t0 + RESUME_GRACE)), 1);
        r.on_connect_result(Err(ConnectError::NoAnswer), t0 + RESUME_GRACE + s(5));
        r.request_now(t0 + RESUME_GRACE + s(6));
        assert_eq!(r.next_deadline(), Some(t0 + RESUME_GRACE + MIN_SPACING));
        // hammering the button does not create a loop
        for i in 0..50 {
            r.request_now(t0 + RESUME_GRACE + s(6 + i));
        }
        assert_eq!(connects(&r.poll(t0 + RESUME_GRACE + s(19))), 0);
        assert_eq!(connects(&r.poll(t0 + RESUME_GRACE + MIN_SPACING)), 1);
    }

    #[test]
    fn journal_lines() {
        use JournalKind::*;
        let c = classify_journal;
        assert_eq!(
            c("profiles/input/device.c:control_connect_cb() connect to AA:BB:CC:DD:EE:F1: Host is down (112)"),
            Some(PageTimeout)
        );
        assert_eq!(
            c("profiles/input/device.c:hidp_report_req_timeout() Device AA:BB:CC:DD:EE:F1 HIDP GET_REPORT request timed out"),
            Some(GetReportTimeout)
        );
        assert_eq!(
            c("src/main.c:check_options() Unknown key ReconnectAttempts for group AdvMon in /etc/bluetooth/main.conf"),
            Some(ConfigIgnored)
        );
        assert_eq!(
            c("src/device.c:store_device_info_cb() Unable set contents for /var/lib/bluetooth/x/info: (No space left on device)"),
            Some(StorageError)
        );
        assert_eq!(
            c("connect to AA:BB:CC:DD:EE:F1: Connection reset by peer (104)"),
            Some(Refused)
        );
        assert_eq!(c("Endpoint registered: sender=:1.100"), None);
    }
}
