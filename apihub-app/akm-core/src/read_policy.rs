//! Safe HID read policy of the daemon (#142, #177; docs/RECONNEXION-PAIRAGE.md §3.6).
//!
//! Three link losses were captured or logged in the middle of GET_REPORT
//! bursts (29/09 15:09, 01/10 04:00:13, 01/10 12:13:08): the keyboard went
//! radio-silent right after a vendor Feature request and stopped page-scanning
//! (supervision timeout, then `Host is down`). Until the BCM2042 firmware is
//! understood, the daemon reads as little as possible:
//!
//! * **allow-list generated from the register map** ([`crate::registry`],
//!   #219): routine reads are the `SafeRead` class (`0x47` declared Battery
//!   Strength, `0x46` battery voltage mV LE, `0x49` latched voltage), and the
//!   `OncePerConnection` class (`0x4F` firmware version, `0x60` battery
//!   thresholds) is requested once after each connection. Never `0xFE` /
//!   `0x4C`, never an undeclared or unknown id, never a scan; no `0xEA` probe
//!   (the hidraw node + BlueZ `Connected` already say the link is up);
//! * **only while the keyboard is in use**: a key press (any input report)
//!   within [`ACTIVE_WINDOW`]. An idle keyboard sits in sniff mode and may be
//!   falling asleep; it is left alone (battery % then comes from the kernel);
//! * **one reader**: a process-wide mutex plus an advisory `flock` on
//!   `$XDG_RUNTIME_DIR/apple-kb-monitor/hid.lock`, shared with the CLI and the
//!   reverse-engineering tools — a busy lock skips the read;
//! * **short**: at least [`MIN_GAP`] (1 s, like macOS 26.5's IOBluetooth
//!   driver, #214) between two requests, [`BUDGET`] per read, stop at the first
//!   failure of any kind (a HIDP timeout costs ~3.5 s);
//! * **Apple's circuit breaker** ([`crate::apple_model::Breaker`], #243,
//!   #251): every outcome is classified like the macOS driver does
//!   ([`crate::apple_model::Outcome::classify`]): a HANDSHAKE refusal is an
//!   answer and resets the count, a silence or an answer of another id counts.
//!   After [`TRIP_AFTER`] silences in a row nothing more is sent (reads and the
//!   `WillShutdown` write share it) and ONE disconnection request is raised
//!   ([`take_disconnect_request`]). Only a new connection
//!   ([`note_connection`]) or a system sleep ([`note_sleep`]: counter 1, so 2
//!   silences suffice after a wake) lifts it; key presses no longer do;
//! * **schedule** (#251): in the daemon the Apple model decides when the
//!   battery is read ([`set_schedule`]: 60 s after the connection, then 4 h,
//!   1 h after a failure); without a schedule (CLI, tests) the read happens
//!   only while the keyboard is in use;
//! * **private lock** (#208): without `XDG_RUNTIME_DIR` the lock lives in a
//!   `0700` directory named after the uid, ownership checked, never followed
//!   through a symlink.
//!
//! The timestamp of the last input report is all that is kept (no content,
//! no keylogging): [`note_input`] is called by the hidraw monitor.

use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::apple_model::Outcome;
use crate::decode::{report_from_uevent, HidSource};
use crate::model::{family_from_uevent, Family};
use crate::power::BatteryReading;
use crate::report::{KbReport, KbWake};

/// The Feature Reports requested in routine: the `SafeRead` class of the
/// register map, in the order they are requested.
pub const ALLOWED: [u8; crate::registry::SAFE_READ_IDS.len()] = crate::registry::SAFE_READ_IDS;
/// Time budget of the once-per-connection phase.
pub const ONCE_BUDGET: Duration = Duration::from_secs(4);
/// Reads happen only if a key was pressed this recently.
pub const ACTIVE_WINDOW: Duration = Duration::from_secs(60);
/// Minimum spacing between two requests (the model's table).
pub const MIN_GAP: Duration = crate::apple_model::APPLE.min_gap;
/// Consecutive silences that open the circuit breaker (the model's table).
pub const TRIP_AFTER: u32 = crate::apple_model::APPLE.trip_after;
/// Time budget of one routine read; no request starts after it. Four
/// requests 1 s apart (`0x47`, Input `0x30`, `0x46`, `0x49`: #251) need 3 s.
pub const BUDGET: Duration = Duration::from_secs(3);
/// Longest wait for the cross-process lock.
pub const LOCK_WAIT: Duration = Duration::from_millis(500);

/// May the daemon request this Feature id at all (routine or once per
/// connection)? Answered by the register map, nowhere else.
pub fn is_allowed(id: u8) -> bool {
    crate::registry::check_read(id).is_ok()
}

// ── once per connection (#219) ─────────────────────────────────────────────

/// What was read once in the current connection.
#[derive(Debug, Default)]
pub struct ConnState {
    /// Ids already requested (successfully or not) since the connection.
    done: std::collections::BTreeSet<u8>,
    /// Frames (id first) answered since the connection.
    values: std::collections::BTreeMap<u8, Vec<u8>>,
}

impl ConnState {
    pub const fn new() -> Self {
        Self {
            done: std::collections::BTreeSet::new(),
            values: std::collections::BTreeMap::new(),
        }
    }
    /// New connection: everything may be read once more, old values are stale.
    pub fn reset(&mut self) {
        self.done.clear();
        self.values.clear();
    }
    /// Claim the single request of `id`; false if it was already made.
    pub fn claim(&mut self, id: u8) -> bool {
        self.done.insert(id)
    }
    /// Was `id` already requested in this connection?
    pub fn requested(&self, id: u8) -> bool {
        self.done.contains(&id)
    }
    pub fn store(&mut self, id: u8, frame: Vec<u8>) {
        self.values.insert(id, frame);
    }
    pub fn frame(&self, id: u8) -> Option<&[u8]> {
        self.values.get(&id).map(Vec::as_slice)
    }
}

static CONN: Mutex<ConnState> = Mutex::new(ConnState::new());

fn global_conn() -> std::sync::MutexGuard<'static, ConnState> {
    CONN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Frame (id first) of a once-per-connection report answered in this
/// connection, if any.
pub fn cached_frame(id: u8) -> Option<Vec<u8>> {
    global_conn().frame(id).map(<[u8]>::to_vec)
}

// ── input activity (timestamp only) ────────────────────────────────────────

static EPOCH: OnceLock<Instant> = OnceLock::new();
/// Milliseconds since EPOCH of the last input report, +1 (0 = never).
static LAST_INPUT: AtomicU64 = AtomicU64::new(0);

fn epoch() -> Instant {
    *EPOCH.get_or_init(Instant::now)
}

/// An input report arrived (key press, wake event). Content is never kept.
pub fn note_input() {
    note_input_at(Instant::now());
}

fn note_input_at(t: Instant) {
    let ms = t.saturating_duration_since(epoch()).as_millis() as u64;
    LAST_INPUT.store(ms + 1, Ordering::Relaxed);
    // #251: no probe any more on a key press (Apple: only a connection or a
    // sleep lifts the breaker).
}

/// Age of the last input report, if any.
pub fn last_input_age(now: Instant) -> Option<Duration> {
    let v = LAST_INPUT.load(Ordering::Relaxed);
    (v != 0).then(|| now.saturating_duration_since(epoch() + Duration::from_millis(v - 1)))
}

// ── last hardware access (shared by reads and the one write) ───────────────

/// Milliseconds since EPOCH of the last request sent to the keyboard, +1.
static LAST_HW: AtomicU64 = AtomicU64::new(0);

/// A request was just sent to the keyboard (read or the `WillShutdown` write):
/// the next one waits [`MIN_GAP`] after it.
pub fn note_hw_access() {
    let ms = Instant::now().saturating_duration_since(epoch()).as_millis() as u64;
    LAST_HW.store(ms + 1, Ordering::Relaxed);
}

/// Instant of the last request sent to the keyboard, if any.
pub fn last_hw_access() -> Option<Instant> {
    let v = LAST_HW.load(Ordering::Relaxed);
    (v != 0).then(|| epoch() + Duration::from_millis(v - 1))
}

/// Why a read was or was not done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Allowed,
    /// The Apple model says no battery read is due now (#251).
    NotDue,
    /// No key press within ACTIVE_WINDOW: the keyboard is idle / in sniff.
    Idle,
    /// Another reader holds the lock.
    Busy,
    /// Circuit breaker open: the keyboard stopped answering (#214).
    Tripped,
}

pub fn gate(age: Option<Duration>) -> Gate {
    match age {
        Some(a) if a < ACTIVE_WINDOW => Gate::Allowed,
        _ => Gate::Idle,
    }
}

// ── schedule given by the Apple model (#251) ───────────────────────────────

/// 0 = none (CLI, tests: activity gate), 1 = a battery read is due, 2 = not due.
static SCHEDULE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The daemon tells, before each acquisition, whether the Apple model has a
/// battery read due (`Some(true)`), none (`Some(false)`), or leaves the
/// decision to the activity gate (`None`).
pub fn set_schedule(due: Option<bool>) {
    SCHEDULE.store(due.map_or(0, |d| if d { 1 } else { 2 }), Ordering::Relaxed);
}

fn schedule() -> Option<bool> {
    match SCHEDULE.load(Ordering::Relaxed) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// Gate of a read: the model's schedule when there is one (a due read does
/// not wait for a key press: Apple reads every 4 h, in use or not), else the
/// activity of the keyboard.
pub fn gate_for(schedule: Option<bool>, age: Option<Duration>) -> Gate {
    match schedule {
        Some(true) => Gate::Allowed,
        Some(false) => Gate::NotDue,
        None => gate(age),
    }
}

static LAST_OUTCOME: Mutex<Option<SafeRead>> = Mutex::new(None);

/// Outcome of the last [`build_report_safe`], taken once (daemon: was the
/// battery read of the model a success?).
pub fn take_last_outcome() -> Option<SafeRead> {
    LAST_OUTCOME.lock().unwrap_or_else(|e| e.into_inner()).take()
}

// ── circuit breaker (#214, #243, #251) ─────────────────────────────────────

/// Apple's breaker: counter common to every request, 3rd silence = nothing
/// more goes out + one disconnection request. Defined by the model.
pub use crate::apple_model::Breaker;

static BREAKER: Mutex<Breaker> = Mutex::new(Breaker::new());

fn global_breaker() -> std::sync::MutexGuard<'static, Breaker> {
    BREAKER.lock().unwrap_or_else(|e| e.into_inner())
}

/// The process-wide breaker, for the one write ([`crate::parity`]).
pub fn breaker() -> &'static Mutex<Breaker> {
    &BREAKER
}

/// A new connection of the keyboard was announced: closes the breaker.
pub fn note_connection() {
    global_breaker().reset();
    global_conn().reset();
}

/// The system goes to sleep (Apple's `handleSleep`): breaker closed,
/// counter 1, longer timeouts for the first request after the wake.
pub fn note_sleep() {
    global_breaker().after_sleep();
}

/// The disconnection request raised by the breaker, once per connection.
pub fn take_disconnect_request() -> bool {
    global_breaker().take_disconnect_request()
}

/// Is the global breaker open without a pending probe (reads are suspended)?
pub fn tripped() -> bool {
    !global_breaker().would_allow()
}

/// Consecutive silences counted by the global breaker.
pub fn breaker_counter() -> u32 {
    global_breaker().counter()
}

// ── the breaker, published for the other emitters (#244, #251) ─────────────

static BREAKER_PUBLISHER: Mutex<crate::breaker_state::Publisher> =
    Mutex::new(crate::breaker_state::Publisher::new());

/// Path of the published state: next to the HID lock
/// (`$XDG_RUNTIME_DIR/apple-kb-monitor/breaker.state`).
pub fn breaker_state_path() -> PathBuf {
    lock_path().with_file_name(crate::breaker_state::FILE_NAME)
}

/// The daemon publishes its breaker for `akm-hid-control` (root) and
/// `akmctl`: called after every pass of the actor loop, it writes only on a
/// change or as a heartbeat ([`crate::breaker_state::REFRESH`]). `mac` is the
/// keyboard followed. Errors are logged by the caller.
pub fn publish_breaker_state(mac: Option<&str>) -> io::Result<bool> {
    let st = crate::breaker_state::BreakerState {
        mac: mac.map(str::to_ascii_uppercase),
        open: tripped(),
        counter: breaker_counter(),
        written_unix: crate::breaker_state::now_unix(),
        pid: std::process::id(),
    };
    let path = breaker_state_path();
    ensure_private_dir(path.parent().ok_or_else(|| io::Error::other("no parent"))?)?;
    BREAKER_PUBLISHER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .publish(&path, &st, Instant::now())
}

/// The daemon stops: no published state = the readers' former behaviour.
pub fn withdraw_breaker_state() {
    crate::breaker_state::remove(&breaker_state_path());
}

// ── single reader ──────────────────────────────────────────────────────────

static IN_PROCESS: Mutex<()> = Mutex::new(());

/// Path of the cross-process lock (also used by the CLI / RE tools):
/// `$XDG_RUNTIME_DIR/apple-kb-monitor/hid.lock`, else (sudo, ssh without
/// pam_systemd, cron) `<tmp>/apple-kb-monitor-<uid>/hid.lock`, a per-uid name
/// whose directory [`try_lock`] only accepts if it is a private `0700`
/// directory owned by us (#208).
pub fn lock_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(x) => PathBuf::from(x).join("apple-kb-monitor").join("hid.lock"),
        None => {
            // SAFETY: getuid(2) has no preconditions.
            let uid = unsafe { libc::getuid() };
            std::env::temp_dir()
                .join(format!("apple-kb-monitor-{uid}"))
                .join("hid.lock")
        }
    }
}

/// Create (0700) or verify the directory of the lock: a real directory (no
/// symlink), owned by the current uid, no group/other access.
fn ensure_private_dir(dir: &std::path::Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let md = std::fs::symlink_metadata(dir)?;
    // SAFETY: geteuid(2) has no preconditions.
    let me = unsafe { libc::geteuid() };
    if !md.file_type().is_dir() || md.uid() != me || md.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a private directory owned by uid {me}", dir.display()),
        ));
    }
    Ok(())
}

/// Held for the duration of a read.
pub struct ReadLock {
    _file: std::fs::File,
    _guard: std::sync::MutexGuard<'static, ()>,
}

/// Take both locks, waiting at most `wait`; `None` if another reader is
/// active or the lock directory is not trustworthy.
pub fn try_lock(wait: Duration) -> Option<ReadLock> {
    use std::os::unix::fs::OpenOptionsExt;
    let end = Instant::now() + wait;
    // In-process lock: honour `wait` like the flock, and survive poisoning (a
    // panic in a reader thread must not refuse every later read for good).
    let guard = loop {
        match IN_PROCESS.try_lock() {
            Ok(g) => break g,
            Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                if Instant::now() >= end {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let p = lock_path();
    ensure_private_dir(p.parent()?).ok()?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&p)
        .ok()?;
    loop {
        // SAFETY: valid fd owned by `file`.
        let r = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if r == 0 {
            return Some(ReadLock {
                _file: file,
                _guard: guard,
            });
        }
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

// ── the safe source ────────────────────────────────────────────────────────

/// A [`HidSource`] that refuses (without any I/O) every id the register map
/// does not allow, serves a once-per-connection id once, and spaces the
/// requests by [`MIN_GAP`].
pub struct SafeSource<'a> {
    inner: &'a dyn HidSource,
    last: std::cell::Cell<Option<Instant>>,
    sent: std::cell::Cell<u32>,
    /// `None` = the process-wide breaker.
    breaker: Option<&'a Mutex<Breaker>>,
    /// `None` = the process-wide once-per-connection state.
    conn: Option<&'a Mutex<ConnState>>,
}

/// Time to wait before a request, given the previous one (pure, testable).
pub fn wait_before(last: Option<Instant>, now: Instant) -> Duration {
    last.map_or(Duration::ZERO, |l| {
        MIN_GAP.saturating_sub(now.saturating_duration_since(l))
    })
}

impl<'a> SafeSource<'a> {
    pub fn new(inner: &'a dyn HidSource) -> Self {
        Self {
            inner,
            last: std::cell::Cell::new(None),
            sent: std::cell::Cell::new(0),
            breaker: None,
            conn: None,
        }
    }
    /// Same with a private breaker (tests).
    pub fn with_breaker(inner: &'a dyn HidSource, breaker: &'a Mutex<Breaker>) -> Self {
        Self {
            breaker: Some(breaker),
            ..Self::new(inner)
        }
    }
    /// Same with private breaker and once-per-connection state (tests).
    pub fn with_parts(
        inner: &'a dyn HidSource,
        breaker: &'a Mutex<Breaker>,
        conn: &'a Mutex<ConnState>,
    ) -> Self {
        Self {
            breaker: Some(breaker),
            conn: Some(conn),
            ..Self::new(inner)
        }
    }
    fn conn(&self) -> std::sync::MutexGuard<'_, ConnState> {
        match self.conn {
            Some(c) => c.lock().unwrap_or_else(|e| e.into_inner()),
            None => global_conn(),
        }
    }
    fn breaker(&self) -> std::sync::MutexGuard<'_, Breaker> {
        match self.breaker {
            Some(b) => b.lock().unwrap_or_else(|e| e.into_inner()),
            None => global_breaker(),
        }
    }
    /// Requests actually sent to the device.
    pub fn sent(&self) -> u32 {
        self.sent.get()
    }
}

impl SafeSource<'_> {
    /// The one request path of the policy, Feature or Input: breaker (allow,
    /// timeouts, outcome), 1 s spacing, hardware-access note. The register
    /// map was consulted by the caller.
    fn request(
        &self,
        report_id: u8,
        once: bool,
        send: impl FnOnce() -> io::Result<Vec<u8>>,
    ) -> io::Result<Vec<u8>> {
        let timeouts = {
            let mut b = self.breaker();
            if !b.allow() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "circuit breaker open: the keyboard stopped answering",
                ));
            }
            b.begin()
        };
        if once {
            self.conn().claim(report_id);
        }
        let w = wait_before(self.last.get(), Instant::now());
        if !w.is_zero() {
            std::thread::sleep(w);
        }
        self.sent.set(self.sent.get() + 1);
        let t0 = Instant::now();
        let r = send();
        let took = t0.elapsed();
        self.last.set(Some(Instant::now()));
        note_hw_access();
        // Apple's verdict on this exchange (refusal = answer, other id or
        // late answer = silence).
        let outcome = Outcome::classify(&r, report_id, took, timeouts.guard);
        self.breaker().record_outcome(outcome);
        match outcome {
            Outcome::WrongId | Outcome::Timeout if r.is_ok() => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("report {report_id:#04x}: no answer of this id before the watchdog"),
            )),
            _ => r,
        }
    }
}

impl HidSource for SafeSource<'_> {
    fn feature(&self, report_id: u8) -> io::Result<Vec<u8>> {
        // The register map decides: class SafeRead or OncePerConnection.
        let class = crate::registry::check_read(report_id)?;
        let once = class == crate::registry::Safety::OncePerConnection;
        if once && self.conn().requested(report_id) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "report {report_id:#04x} is read once per connection and was already requested"
                ),
            ));
        }
        self.request(report_id, once, || self.inner.feature(report_id))
    }

    /// GET Input: the register map lets `0x30` through and nothing else
    /// ([`crate::registry::check_read_input`]); same breaker, same spacing,
    /// same verdict as a Feature read (a silence on `0x30` counts like one on
    /// `0x47`, Apple's common counter).
    fn input(&self, report_id: u8) -> io::Result<Vec<u8>> {
        crate::registry::check_read_input(report_id)?;
        self.request(report_id, false, || self.inner.input(report_id))
    }
}

/// The routine battery read, in Apple's order (R2, #251): GET Feature `0x47`,
/// then GET Input `0x30` once, then the project's voltage reports. Generated
/// from the register map (`SAFE_READ_IDS` keeps its order, the Input ids
/// follow the first Feature id).
pub fn routine_reads() -> Vec<crate::apple_model::Request> {
    use crate::apple_model::Request;
    let mut out = Vec::new();
    for (i, id) in crate::registry::SAFE_READ_IDS.into_iter().enumerate() {
        out.push(Request::GetFeature(id));
        if i == 0 {
            out.extend(
                crate::registry::SAFE_READ_INPUT_IDS
                    .into_iter()
                    .map(Request::GetInput),
            );
        }
    }
    out
}

/// Outcome of a safe read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeRead {
    /// All allowed reports answered.
    Complete,
    /// Stopped at the first failure or at the budget.
    Partial,
    /// Nothing requested (idle keyboard, lock busy).
    Skipped(Gate),
}

impl SafeRead {
    /// Did the battery read of the model succeed? A family without vendor
    /// reports (`Skipped(Allowed)`) has nothing to read: a success.
    pub fn is_success(self) -> bool {
        matches!(self, SafeRead::Complete | SafeRead::Skipped(Gate::Allowed))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Read the routine reports into `report`. Stops at the first failure.
pub fn read_safe(src: &dyn HidSource, report: &mut KbReport) -> SafeRead {
    read_with(SafeSource::new(src), report, false)
}

/// Routine reads, then (daemon only, `with_once`) the once-per-connection
/// reads the daemon wants ([`crate::registry::DAEMON_ONCE_IDS`]) that were not
/// yet made in this connection. Every request goes through the same
/// [`SafeSource`]: same spacing, same breaker, stop at the first failure.
fn read_with(safe: SafeSource<'_>, report: &mut KbReport, with_once: bool) -> SafeRead {
    use crate::apple_model::Request;
    let start = Instant::now();
    let mut complete = true;
    // Apple's order (R2): 0x47, then GET Input 0x30 once, then 0x46 / 0x49.
    for req in routine_reads() {
        if safe.sent() > 0 && start.elapsed() >= BUDGET {
            complete = false;
            break;
        }
        let r = match req {
            Request::GetFeature(id) => safe.feature(id),
            Request::GetInput(id) => safe.input(id),
            _ => continue,
        };
        let Ok(b) = r else {
            complete = false;
            break;
        };
        if b.len() < 2 {
            continue;
        }
        match req {
            Request::GetInput(_) => {
                // Decoded by the passive decoder, like the pushed `A1 30 xx`.
                if let Some(crate::passive::PassiveEvent::BattStat { value }) =
                    crate::passive::decode(&b)
                {
                    report.battery.state = Some(value);
                    report
                        .raw
                        .insert(format!("input {:#04x}", b[0]), hex(&b[1..]));
                }
            }
            Request::GetFeature(0x47) => {
                report.raw.insert("0x47".into(), hex(&b[1..]));
                if let Some(&pct) = b.get(1).filter(|&&p| p <= 100) {
                    if report.battery.percentage.is_none() {
                        report.battery.percentage = Some(f64::from(pct));
                    }
                }
            }
            Request::GetFeature(0x46) => {
                report.raw.insert("0x46".into(), hex(&b[1..]));
                if b.len() >= 3 {
                    // [mesuré] battery voltage in mV, little-endian (= 0xFF BE).
                    let mv = u16::from_le_bytes([b[1], b[2]]);
                    if (1500..=3700).contains(&mv) {
                        report.battery.voltage = Some(f64::from(mv) / 1000.0);
                    }
                }
            }
            Request::GetFeature(id) => {
                report.raw.insert(format!("{id:#04x}"), hex(&b[1..]));
            }
            _ => {}
        }
    }
    if complete && with_once {
        let phase = Instant::now();
        for id in crate::registry::DAEMON_ONCE_IDS {
            if safe.conn().requested(id) {
                continue;
            }
            if phase.elapsed() >= ONCE_BUDGET {
                complete = false;
                break;
            }
            match safe.feature(id) {
                Ok(b) => safe.conn().store(id, b),
                Err(_) => {
                    complete = false;
                    break;
                }
            }
        }
        // Lower priority: out of time = left for the next burst, not a failure.
        if complete {
            for id in crate::registry::DAEMON_DEFERRED_ONCE_IDS {
                if safe.conn().requested(id) {
                    continue;
                }
                if phase.elapsed() >= ONCE_BUDGET {
                    break;
                }
                match safe.feature(id) {
                    Ok(b) => safe.conn().store(id, b),
                    Err(_) => {
                        complete = false;
                        break;
                    }
                }
            }
        }
    }
    report.incomplete = !complete;
    if complete {
        SafeRead::Complete
    } else {
        SafeRead::Partial
    }
}

/// Fill the report from the frames read once in this connection: firmware
/// version (`0x4F`), battery thresholds (`0x60`), then the firmware check
/// against the embedded table for the model `pid`. Nothing is read here.
pub fn apply_cached(report: &mut KbReport, pid: Option<u32>) {
    apply_frames(report, pid, &global_conn());
}

fn apply_frames(report: &mut KbReport, pid: Option<u32>, st: &ConnState) {
    use crate::registry::{u16_le, Thresholds};
    if let Some(f) = st.frame(0x4F) {
        if let Some(v) = f.get(1..).and_then(u16_le) {
            report.firmware.version = Some(crate::firmware::hex(v));
            report.raw.insert("0x4f".to_string(), hex(&f[1..]));
        }
    }
    if let Some(f) = st.frame(0x60) {
        if let Some(t) = f.get(1..).and_then(Thresholds::parse) {
            report.battery.thresholds = Some(t);
            report.raw.insert("0x60".to_string(), hex(&f[1..]));
        }
    }
    if let (Some(t), Some(mv)) = (
        report.battery.thresholds,
        report
            .battery
            .voltage_filtered_mv
            .or(report.battery.voltage_mv)
            .or_else(|| report.battery.voltage.map(|v| (v * 1000.0).round() as u32)),
    ) {
        report.battery.threshold_level = Some(t.level(mv).as_str().to_string());
        report.battery.threshold_margins_mv = Some(t.margins(mv));
    }
    // "Apple display" percentage (#213): only for the PIDs IOBluetooth remaps.
    report.battery.apple_display_pct = pid
        .filter(|&p| crate::registry::apple_display_applies(p))
        .and(report.battery.percentage)
        .filter(|p| p.is_finite() && (0.0..=100.0).contains(p))
        .map(|p| crate::registry::apple_display_percent(p.round() as u8));
    crate::firmware::assess_report(pid, &mut report.firmware);
    // Name stored in the keyboard (#248): only when the 4 fragments are cached.
    let frags: Option<Vec<Vec<u8>>> = crate::devname::FRAGMENT_IDS
        .iter()
        .map(|&id| st.frame(id).map(<[u8]>::to_vec))
        .collect();
    if let Some(raw) = frags.as_deref().and_then(crate::devname::raw_from_frames) {
        report.device.name_on_keyboard = crate::devname::name_from_raw(&raw);
        report.device.name_on_keyboard_hex = Some(hex(&raw));
    }
}

/// Full report for the daemon under the safe policy. The keyboard is present
/// (its hidraw node exists): this never returns `None` for a BCM2042 that is
/// idle — it only refrains from talking to it.
pub fn build_report_safe(
    uevent: &str,
    kernel: Option<BatteryReading>,
    src: &dyn HidSource,
    wake: KbWake,
    now: Instant,
) -> (KbReport, SafeRead) {
    let mut report = report_from_uevent(uevent, kernel);
    report.wake = wake;
    report.bluetooth.connected = true;
    if family_from_uevent(uevent) != Family::Bcm2042 {
        *LAST_OUTCOME.lock().unwrap_or_else(|e| e.into_inner()) = Some(SafeRead::Skipped(Gate::Allowed));
        return (report, SafeRead::Skipped(Gate::Allowed));
    }
    let outcome = match gate_for(schedule(), last_input_age(now)) {
        Gate::Allowed if tripped() => SafeRead::Skipped(Gate::Tripped),
        Gate::Allowed => match try_lock(LOCK_WAIT) {
            Some(_lock) => read_with(SafeSource::new(src), &mut report, true),
            None => SafeRead::Skipped(Gate::Busy),
        },
        g => SafeRead::Skipped(g),
    };
    *LAST_OUTCOME.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
    report.battery.percentage_fine = report.battery.percentage;
    report.breaker_open = tripped();
    // Values read once in this connection survive the following reads.
    apply_cached(&mut report, crate::model::parse_hid_id(uevent).map(|(_, p)| p));
    (report, outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::Fixture;
    use std::cell::RefCell;

    /// Records every id actually sent.
    struct Spy<'a> {
        inner: &'a dyn HidSource,
        log: RefCell<Vec<u8>>,
    }
    impl HidSource for Spy<'_> {
        fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
            self.log.borrow_mut().push(id);
            self.inner.feature(id)
        }
        fn input(&self, id: u8) -> io::Result<Vec<u8>> {
            self.log.borrow_mut().push(id);
            self.inner.input(id)
        }
    }

    /// The keyboard as measured: Feature 0x47/0x46/0x49 and GET Input 0x30
    /// answering `30 00` (RE-HID-EXHAUSTIF §2.2).
    fn fixture() -> Fixture {
        Fixture::new()
            .with_input(&[0x30, 0])
            .with(&[0x47, 99])
            .with(&[0x46, 0xBA, 0x0B]) // 0x0BBA = 3002 mV
            .with(&[0x49, 0x89, 0x0B])
            .with(&[0xEA, 98])
            .with(&[0xFE, 0, 0, 0, 0, 0, 0, 0, 0])
    }

    #[test]
    fn only_the_allow_list_reaches_the_device() {
        let f = fixture();
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let safe = SafeSource::new(&spy);
        for id in [0xFE, 0xEA, 0x4C, 0x09, 0xF5, 0x00, 0x13] {
            let e = safe.feature(id).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        }
        assert!(spy.log.borrow().is_empty(), "refused ids never sent");
        assert!(safe.feature(0x47).is_ok());
        assert_eq!(*spy.log.borrow(), vec![0x47]);
        for id in 0..=255u8 {
            let want = [0x46, 0x47, 0x49, 0x4F, 0x60, 0x51, 0x52, 0x53, 0x54].contains(&id);
            assert_eq!(is_allowed(id), want, "{id:#04x}");
        }
        // The routine list is generated from the register map.
        assert_eq!(ALLOWED, [0x47, 0x46, 0x49]);
    }

    /// Never-read and never-write classes are refused without any I/O, for
    /// every one of the 256 ids and through every entry of the policy.
    #[test]
    fn no_forbidden_class_is_reachable_through_the_policy() {
        use crate::registry::{classify_feature, Safety};
        let full = Fixture::from_hex_dump(&(0..=255u32).map(|i| format!("{i:02x} 00 00 00 00 00 00 00 00\n")).collect::<String>()).unwrap();
        let spy = Spy {
            inner: &full,
            log: RefCell::new(Vec::new()),
        };
        let breaker = Mutex::new(Breaker::new());
        let conn = Mutex::new(ConnState::new());
        for id in 0..=255u8 {
            let safe = SafeSource::with_parts(&spy, &breaker, &conn);
            let class = classify_feature(id);
            let r = safe.feature(id);
            if matches!(class, Safety::NeverRead | Safety::NeverWrite | Safety::Unknown | Safety::ManualOnly | Safety::PassiveInput) {
                assert_eq!(r.unwrap_err().kind(), io::ErrorKind::PermissionDenied, "{id:#04x} {class:?}");
            }
        }
        let sent = spy.log.borrow().clone();
        for id in sent {
            assert!(
                matches!(classify_feature(id), Safety::SafeRead | Safety::OncePerConnection),
                "{id:#04x} reached the device"
            );
        }
        // NeverRead ids in particular: 0xFE and 0x4C.
        assert!(!spy.log.borrow().contains(&0xFE) && !spy.log.borrow().contains(&0x4C));
    }

    #[test]
    fn a_once_per_connection_id_is_requested_once_until_the_next_connection() {
        let f = Fixture::new().with(&[0x4F, 0x50, 0x00]);
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let breaker = Mutex::new(Breaker::new());
        let conn = Mutex::new(ConnState::new());
        let safe = SafeSource::with_parts(&spy, &breaker, &conn);
        assert!(safe.feature(0x4F).is_ok());
        let e = safe.feature(0x4F).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(spy.log.borrow().len(), 1);
        // a second SafeSource (next read burst) shares the connection state
        let safe2 = SafeSource::with_parts(&spy, &breaker, &conn);
        assert_eq!(safe2.feature(0x4F).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        conn.lock().unwrap().reset(); // new connection
        assert!(SafeSource::with_parts(&spy, &breaker, &conn).feature(0x4F).is_ok());
        assert_eq!(spy.log.borrow().len(), 2);
    }

    #[test]
    fn a_full_burst_reads_the_version_and_thresholds_once_then_serves_the_cache() {
        let f = fixture()
            .with(&[0x4F, 0x50, 0x00])
            .with(&[0x60, 0x0b, 0x8a, 0x09, 0xca, 0x09, 0x64, 0x08, 0x06])
            .with(b"\x51Clavier ")
            .with(b"\x52de maria")
            .with(b"\x53 #1\0\0\0\0\0")
            .with(&[0x54, 0, 0, 0, 0, 0, 0, 0, 0]);
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let breaker = Mutex::new(Breaker::new());
        let conn = Mutex::new(ConnState::new());
        let mut r = KbReport::default();
        let out = read_with(SafeSource::with_parts(&spy, &breaker, &conn), &mut r, true);
        assert_eq!(
            out,
            SafeRead::Complete,
            "running out of time on the name is not a failure"
        );
        assert_eq!(spy.log.borrow()[..6], [0x47, 0x30, 0x46, 0x49, 0x4F, 0x60]);
        // The name fragments follow, in order, over one or more bursts, once each.
        for _ in 0..4 {
            if crate::registry::DAEMON_DEFERRED_ONCE_IDS
                .iter()
                .all(|&id| conn.lock().unwrap().requested(id))
            {
                break;
            }
            let mut rx = KbReport::default();
            assert_eq!(
                read_with(SafeSource::with_parts(&spy, &breaker, &conn), &mut rx, true),
                SafeRead::Complete
            );
        }
        let names: Vec<u8> = spy
            .log
            .borrow()
            .iter()
            .copied()
            .filter(|id| (0x51..=0x54).contains(id))
            .collect();
        assert_eq!(names, vec![0x51, 0x52, 0x53, 0x54]);
        apply_frames(&mut r, Some(0x0256), &conn.lock().unwrap());
        assert_eq!(
            r.device.name_on_keyboard.as_deref(),
            Some("Clavier de maria #1")
        );
        assert_eq!(
            r.device.name_on_keyboard_hex.as_deref().map(str::len),
            Some(64)
        );
        assert_eq!(r.firmware.version.as_deref(), Some("0x0050"));
        assert_eq!(r.firmware.status, "up_to_date");
        assert_eq!(r.battery.thresholds.unwrap().as_array(), [2954, 2506, 2404, 2054]);
        assert_eq!(r.battery.threshold_level.as_deref(), Some("ok"));
        assert_eq!(r.raw.get("0x4f").map(String::as_str), Some("5000"));
        // second burst: routine reads only, once-ids are not requested again
        let mut r2 = KbReport::default();
        spy.log.borrow_mut().clear();
        let out = read_with(SafeSource::with_parts(&spy, &breaker, &conn), &mut r2, true);
        assert_eq!(out, SafeRead::Complete);
        assert_eq!(*spy.log.borrow(), vec![0x47, 0x30, 0x46, 0x49]);
        apply_frames(&mut r2, Some(0x0256), &conn.lock().unwrap());
        assert_eq!(
            r2.firmware.version.as_deref(),
            Some("0x0050"),
            "cache survives"
        );
        // the CLI path (read_safe) never makes the once-per-connection reads
        let spy2 = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let mut r3 = KbReport::default();
        let _ = read_with(
            SafeSource::with_parts(
                &spy2,
                &Mutex::new(Breaker::new()),
                &Mutex::new(ConnState::new()),
            ),
            &mut r3,
            false,
        );
        assert_eq!(*spy2.log.borrow(), vec![0x47, 0x30, 0x46, 0x49]);
    }

    #[test]
    fn unreadable_once_report_stops_the_burst_and_leaves_the_firmware_unknown() {
        let f = fixture(); // no 0x4F: NotFound
        let breaker = Mutex::new(Breaker::new());
        let conn = Mutex::new(ConnState::new());
        let mut r = KbReport::default();
        let out = read_with(SafeSource::with_parts(&f, &breaker, &conn), &mut r, true);
        assert_eq!(out, SafeRead::Partial);
        apply_frames(&mut r, Some(0x0256), &conn.lock().unwrap());
        assert_eq!(r.firmware.version, None);
        assert_eq!(r.firmware.status, "unknown");
        assert_eq!(r.battery.thresholds, None);
        // the failed request counted for the breaker
        assert!(!breaker.lock().unwrap().is_open());
    }

    #[test]
    fn requests_are_spaced() {
        assert_eq!(MIN_GAP, Duration::from_millis(1000), "macOS 26.5 spacing");
        let t = Instant::now();
        assert_eq!(wait_before(None, t), Duration::ZERO);
        assert_eq!(wait_before(Some(t), t), MIN_GAP);
        assert_eq!(
            wait_before(Some(t), t + Duration::from_millis(400)),
            Duration::from_millis(600)
        );
        assert_eq!(wait_before(Some(t), t + MIN_GAP), Duration::ZERO);
        assert_eq!(wait_before(Some(t), t + MIN_GAP * 5), Duration::ZERO);
    }

    /// Always times out (HIDP timeout), counting the requests that reach it.
    struct Dead(std::cell::Cell<u32>);
    impl HidSource for Dead {
        fn feature(&self, _: u8) -> io::Result<Vec<u8>> {
            self.0.set(self.0.get() + 1);
            Err(io::Error::from(io::ErrorKind::TimedOut))
        }
    }

    #[test]
    fn breaker_trips_after_three_failures_and_needs_a_sign_of_life() {
        let mut b = Breaker::new();
        for _ in 0..TRIP_AFTER - 1 {
            assert!(b.allow());
            b.record(false);
        }
        assert!(!b.is_open());
        assert!(b.allow());
        b.record(false);
        assert!(b.is_open());
        // open: nothing goes out, however often it is asked
        assert!((0..100).all(|_| !b.allow()));
        // a keystroke re-arms exactly one probe
        b.alive();
        assert!(b.allow());
        assert!(!b.allow());
        b.record(false); // probe timed out: still open
        assert!(b.is_open() && !b.would_allow());
        b.alive();
        assert!(b.allow());
        b.record(true); // the keyboard answered: closed
        assert!(!b.is_open() && b.allow());
        // new connection closes it too
        for _ in 0..TRIP_AFTER {
            b.record(false);
        }
        assert!(b.is_open());
        b.reset();
        assert!(b.allow());
        // a success in between resets the count
        b.record(false);
        b.record(false);
        b.record(true);
        b.record(false);
        assert!(!b.is_open());
        // input while closed does not arm anything
        b.alive();
        assert!(!b.is_open());
    }

    #[test]
    fn dead_keyboard_gets_three_requests_then_silence() {
        let br = Mutex::new(Breaker::new());
        let dead = Dead(std::cell::Cell::new(0));
        // each read stops at its first failure: 3 reads = 3 requests
        for _ in 0..TRIP_AFTER {
            let mut r = KbReport::default();
            let s = SafeSource::with_breaker(&dead, &br);
            assert_eq!(read_with(s, &mut r, false), SafeRead::Partial);
        }
        assert_eq!(dead.0.get(), TRIP_AFTER);
        // breaker open: further reads send nothing
        for _ in 0..10 {
            let mut r = KbReport::default();
            let s = SafeSource::with_breaker(&dead, &br);
            assert_eq!(read_with(s, &mut r, false), SafeRead::Partial);
        }
        assert_eq!(dead.0.get(), TRIP_AFTER, "no request while open");
        // sign of life -> one probe only
        br.lock().unwrap().alive();
        let mut r = KbReport::default();
        read_with(SafeSource::with_breaker(&dead, &br), &mut r, false);
        assert_eq!(dead.0.get(), TRIP_AFTER + 1);
        read_with(SafeSource::with_breaker(&dead, &br), &mut r, false);
        assert_eq!(dead.0.get(), TRIP_AFTER + 1);
    }

    #[test]
    fn lock_dir_is_private_per_uid_and_refuses_foreign_or_linked_dirs() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let base = std::env::temp_dir().join(format!("akm-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        // fresh: created 0700
        let d = base.join("ok");
        ensure_private_dir(&d).unwrap();
        assert_eq!(std::fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
        ensure_private_dir(&d).unwrap();
        // group/other access: refused
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ensure_private_dir(&d).is_err());
        // symlink to a directory: refused, target untouched
        let target = base.join("target");
        std::fs::create_dir(&target).unwrap();
        let l = base.join("link");
        symlink(&target, &l).unwrap();
        assert!(ensure_private_dir(&l).is_err());
        // symlinked lock file: O_NOFOLLOW
        let sub = base.join("p");
        ensure_private_dir(&sub).unwrap();
        let victim = base.join("victim");
        symlink(&victim, sub.join("hid.lock")).unwrap();
        use std::os::unix::fs::OpenOptionsExt;
        let r = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(sub.join("hid.lock"));
        assert!(r.is_err() && !victim.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn safe_read_decodes_percentage_and_voltage() {
        let f = fixture();
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let mut r = KbReport::default();
        assert_eq!(read_safe(&spy, &mut r), SafeRead::Complete);
        assert_eq!(
            *spy.log.borrow(),
            vec![0x47, 0x30, 0x46, 0x49],
            "4 requests (0x47, Input 0x30, 0x46, 0x49), no probe, no scan"
        );
        assert_eq!(r.battery.percentage, Some(99.0));
        assert_eq!(r.battery.voltage, Some(3.002));
        assert_eq!(
            r.battery.state,
            Some(0),
            "GET Input 0x30 decoded by the passive decoder"
        );
        assert_eq!(r.raw.get("input 0x30").map(String::as_str), Some("00"));
        assert!(!r.incomplete);
    }

    // ── GET Input 0x30 (Apple R2, #251) ────────────────────────────────────

    /// Records Feature and Input requests apart, with their instants.
    struct DirSpy<'a> {
        inner: &'a dyn HidSource,
        log: RefCell<Vec<(&'static str, u8, Instant)>>,
    }
    impl<'a> DirSpy<'a> {
        fn new(inner: &'a dyn HidSource) -> Self {
            Self {
                inner,
                log: RefCell::new(Vec::new()),
            }
        }
        fn seq(&self) -> Vec<(&'static str, u8)> {
            self.log
                .borrow()
                .iter()
                .map(|(d, id, _)| (*d, *id))
                .collect()
        }
    }
    impl HidSource for DirSpy<'_> {
        fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
            self.log.borrow_mut().push(("feature", id, Instant::now()));
            self.inner.feature(id)
        }
        fn input(&self, id: u8) -> io::Result<Vec<u8>> {
            self.log.borrow_mut().push(("input", id, Instant::now()));
            self.inner.input(id)
        }
    }

    /// R2: 0x47, then GET Input 0x30 (as an Input, once), then 0x46 / 0x49,
    /// at least 1 s between each; a second burst reads 0x30 once more, never
    /// twice in one.
    #[test]
    fn the_burst_is_0x47_then_input_0x30_then_the_voltages_1s_apart() {
        let f = fixture().with_input(&[0x30, 1]);
        let spy = DirSpy::new(&f);
        let mut r = KbReport::default();
        assert_eq!(read_safe(&spy, &mut r), SafeRead::Complete);
        assert_eq!(
            spy.seq(),
            vec![
                ("feature", 0x47),
                ("input", 0x30),
                ("feature", 0x46),
                ("feature", 0x49)
            ]
        );
        assert_eq!(r.battery.state, Some(1), "low, from the Input read");
        {
            let log = spy.log.borrow();
            for w in log.windows(2) {
                let gap = w[1].2.duration_since(w[0].2);
                assert!(
                    gap >= MIN_GAP - Duration::from_millis(5),
                    "{:?} -> {:?}: {gap:?}",
                    w[0].1,
                    w[1].1
                );
            }
        }
        assert_eq!(
            routine_reads()[..2],
            crate::apple_model::BATTERY_READ,
            "Apple's battery read first"
        );
        // a second burst: exactly one more Input read
        let mut r2 = KbReport::default();
        assert_eq!(read_safe(&spy, &mut r2), SafeRead::Complete);
        let inputs = spy.seq().iter().filter(|(d, _)| *d == "input").count();
        assert_eq!(inputs, 2, "one GET Input 0x30 per burst");
        assert!(spy.seq().iter().all(|(d, id)| *d != "input" || *id == 0x30));
    }

    /// The Input gate of the policy: of the 256 ids only 0x30 reaches the
    /// device as a GET Input; the Feature path never sends 0x30.
    #[test]
    fn only_input_0x30_passes_the_safe_source() {
        let full = Fixture::from_hex_dump(
            &(0..=255u32)
                .map(|i| format!("{i:02x} 00\n"))
                .collect::<String>(),
        )
        .unwrap();
        let mut f = full.clone();
        for id in 0..=255u8 {
            f = f.with_input(&[id, 0]);
        }
        let spy = DirSpy::new(&f);
        let breaker = Mutex::new(Breaker::new());
        let conn = Mutex::new(ConnState::new());
        for id in 0..=255u8 {
            let safe = SafeSource::with_parts(&spy, &breaker, &conn);
            match safe.input(id) {
                Ok(b) => assert_eq!(b, vec![0x30, 0]),
                Err(e) => assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{id:#04x}"),
            }
        }
        let reached: Vec<u8> = spy
            .seq()
            .iter()
            .filter(|(d, _)| *d == "input")
            .map(|(_, id)| *id)
            .collect();
        assert_eq!(reached, vec![0x30]);
        assert!(SafeSource::with_parts(&spy, &breaker, &conn)
            .feature(0x30)
            .is_err());
        assert!(!spy.seq().contains(&("feature", 0x30)));
        assert!(
            !breaker.lock().unwrap().is_open(),
            "a refused id never counts"
        );
    }

    /// A silent 0x30 counts like a silent 0x47 (Apple's common counter): the
    /// burst stops there, the breaker opens at the third silence in a row.
    #[test]
    fn a_failed_input_0x30_counts_for_the_breaker_like_0x47() {
        // 0x47 answers, 0x30 times out (no Input in the fixture = NotFound,
        // slow it down with a Dead-like source): use a source that answers
        // Feature and refuses Input with a link timeout.
        struct HalfDead<'a> {
            fixture: &'a Fixture,
            inputs: std::cell::Cell<u32>,
            /// Then the keyboard goes silent on everything.
            all_dead: std::cell::Cell<bool>,
        }
        impl HidSource for HalfDead<'_> {
            fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
                if self.all_dead.get() {
                    return Err(io::Error::from(io::ErrorKind::TimedOut));
                }
                self.fixture.feature(id)
            }
            fn input(&self, _: u8) -> io::Result<Vec<u8>> {
                self.inputs.set(self.inputs.get() + 1);
                Err(io::Error::from(io::ErrorKind::TimedOut))
            }
        }
        let f = fixture();
        let src = HalfDead {
            fixture: &f,
            inputs: std::cell::Cell::new(0),
            all_dead: std::cell::Cell::new(false),
        };
        let br = Mutex::new(Breaker::new());
        // burst 1: 0x47 answers (counter 0), 0x30 silent (1), stop there
        let mut r = KbReport::default();
        let out = read_with(SafeSource::with_breaker(&src, &br), &mut r, false);
        assert_eq!(out, SafeRead::Partial, "the burst stops at the silent 0x30");
        assert_eq!(r.battery.percentage, Some(99.0), "0x47 was read");
        assert_eq!(r.battery.state, None);
        assert!(r.incomplete);
        assert_eq!(br.lock().unwrap().counter(), 1, "the silent 0x30 counted");
        assert_eq!(src.inputs.get(), 1);
        // the keyboard goes mute: two more silences (0x47) open the breaker,
        // the silence of 0x30 being the first of the three
        src.all_dead.set(true);
        for i in 2..=TRIP_AFTER {
            let mut r = KbReport::default();
            assert_eq!(
                read_with(SafeSource::with_breaker(&src, &br), &mut r, false),
                SafeRead::Partial
            );
            assert_eq!(br.lock().unwrap().counter(), i);
        }
        assert!(br.lock().unwrap().is_open());
        assert_eq!(src.inputs.get(), 1, "0x47 failed first: 0x30 not asked");
        // open: nothing goes out any more, not even 0x47
        let mut r = KbReport::default();
        read_with(SafeSource::with_breaker(&src, &br), &mut r, false);
        assert_eq!(src.inputs.get(), 1);
        assert_eq!(r.battery.percentage, None);
        // a keyboard that answers 0x47 but never 0x30 never trips: the
        // answer resets the counter each burst (Apple's DecodedHandshake)
        let br2 = Mutex::new(Breaker::new());
        src.all_dead.set(false);
        for _ in 0..5 {
            let mut r = KbReport::default();
            read_with(SafeSource::with_breaker(&src, &br2), &mut r, false);
            assert_eq!(br2.lock().unwrap().counter(), 1);
        }
        assert!(!br2.lock().unwrap().is_open());
    }

    #[test]
    fn first_failure_stops_the_read() {
        let f = Fixture::new().with(&[0x47, 80]).with_input(&[0x30, 0]); // 0x46 missing -> error
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let mut r = KbReport::default();
        assert_eq!(read_safe(&spy, &mut r), SafeRead::Partial);
        assert_eq!(*spy.log.borrow(), vec![0x47, 0x30, 0x46]);
        assert!(r.incomplete);
        assert_eq!(r.battery.percentage, Some(80.0));
    }

    #[test]
    fn idle_keyboard_is_left_alone() {
        assert_eq!(gate(None), Gate::Idle);
        assert_eq!(gate(Some(ACTIVE_WINDOW)), Gate::Idle);
        assert_eq!(gate(Some(Duration::from_secs(5))), Gate::Allowed);
        let t = Instant::now();
        note_input_at(t);
        assert!(last_input_age(t + Duration::from_secs(10)).unwrap() >= Duration::from_secs(9));
        // build_report_safe on an idle keyboard: no I/O at all
        let f = fixture();
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let uevent = "HID_ID=0005:000005AC:00000256\nHID_NAME=Kb\nHID_UNIQ=04:db:56:ca:42:ee\n";
        let (r, o) =
            build_report_safe(uevent, None, &spy, KbWake::default(), t + ACTIVE_WINDOW * 2);
        assert_eq!(o, SafeRead::Skipped(Gate::Idle));
        assert!(spy.log.borrow().is_empty());
        assert!(r.bluetooth.connected);
    }

    // ── Apple model (#251) ─────────────────────────────────────────────────

    /// Answers fast with an error (the keyboard's HANDSHAKE refusal under
    /// Linux: EIO in milliseconds).
    struct Refuses;
    impl HidSource for Refuses {
        fn feature(&self, _: u8) -> io::Result<Vec<u8>> {
            Err(io::Error::from_raw_os_error(libc::EIO))
        }
    }

    /// Answers with the frame of another report.
    struct OtherId;
    impl HidSource for OtherId {
        fn feature(&self, _: u8) -> io::Result<Vec<u8>> {
            Ok(vec![0x46, 1, 2])
        }
    }

    #[test]
    fn a_refusal_is_an_answer_and_resets_the_count() {
        // RE-GHIDRA-KEXT §2.5 DecodedHandshake: 2 silences, a refusal, 2
        // silences: never 3 in a row, the breaker stays closed.
        let br = Mutex::new(Breaker::new());
        let dead = Dead(std::cell::Cell::new(0));
        for src in [&dead as &dyn HidSource, &dead, &Refuses, &dead, &dead] {
            let _ = SafeSource::with_breaker(src, &br).feature(0x47);
        }
        assert!(!br.lock().unwrap().is_open());
        assert_eq!(br.lock().unwrap().counter(), 2);
        assert!(!br.lock().unwrap().take_disconnect_request());
    }

    #[test]
    fn an_answer_of_another_id_is_ignored_and_counts_as_a_silence() {
        // processControlData: "Report does not equal the report we asked for".
        let br = Mutex::new(Breaker::new());
        for i in 0..TRIP_AFTER {
            let e = SafeSource::with_breaker(&OtherId, &br).feature(0x47).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::TimedOut);
            assert_eq!(br.lock().unwrap().counter(), i + 1);
        }
        let mut b = br.lock().unwrap();
        assert!(b.is_open());
        assert!(b.take_disconnect_request(), "3rd silence: one disconnection request");
        assert!(!b.take_disconnect_request());
    }

    #[test]
    fn the_schedule_of_the_model_overrides_the_activity_gate() {
        assert_eq!(gate_for(Some(true), None), Gate::Allowed, "due: read even if idle");
        assert_eq!(gate_for(Some(false), Some(Duration::ZERO)), Gate::NotDue);
        assert_eq!(gate_for(None, None), Gate::Idle);
        assert_eq!(gate_for(None, Some(Duration::ZERO)), Gate::Allowed);
        assert!(SafeRead::Complete.is_success() && SafeRead::Skipped(Gate::Allowed).is_success());
        for g in [Gate::NotDue, Gate::Idle, Gate::Busy, Gate::Tripped] {
            assert!(!SafeRead::Skipped(g).is_success());
        }
        assert!(!SafeRead::Partial.is_success());
    }

    #[test]
    fn a_sleep_lowers_the_threshold_to_two_silences() {
        let br = Mutex::new(Breaker::new());
        br.lock().unwrap().after_sleep();
        let dead = Dead(std::cell::Cell::new(0));
        for _ in 0..2 {
            let _ = SafeSource::with_breaker(&dead, &br).feature(0x47);
        }
        assert!(br.lock().unwrap().is_open());
        assert_eq!(dead.0.get(), 2);
    }
}
