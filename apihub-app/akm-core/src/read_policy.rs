//! Safe HID read policy of the daemon (docs/RECONNECTION-PAIRING.md §3.6).

use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::apple_model::Outcome;
use crate::conv::hex_compact as hex;
use crate::decode::{report_from_uevent, HidSource};
use crate::model::{family_from_uevent, Family};
use crate::power::BatteryReading;
use crate::report::{KbReport, KbWake};

/// The Feature Reports requested in routine.
pub const ALLOWED: [u8; crate::registry::SAFE_READ_IDS.len()] = crate::registry::SAFE_READ_IDS;
/// Longest answer that is still normal.
pub const SLOW_ANSWER: Duration = Duration::from_millis(1_500);
const fn slot_ms() -> u64 {
    let d = MIN_GAP.saturating_add(SLOW_ANSWER);
    d.as_secs() * 1000 + d.subsec_millis() as u64
}
/// Time budget of the once-per-connection phase; no request starts after it.
pub const ONCE_BUDGET: Duration = Duration::from_millis(
    (crate::registry::DAEMON_ONCE_IDS.len() + crate::registry::DAEMON_DEFERRED_ONCE_IDS.len())
        as u64
        * slot_ms(),
);
/// Reads happen only if a key was pressed this recently.
pub const ACTIVE_WINDOW: Duration = Duration::from_mins(1);
/// Minimum spacing between two requests (the model's table).
pub const MIN_GAP: Duration = crate::apple_model::APPLE.min_gap;
/// Consecutive silences that open the circuit breaker (the model's table).
pub const TRIP_AFTER: u32 = crate::apple_model::APPLE.trip_after;
/// Time budget of one routine read; no request starts after it.
pub const BUDGET: Duration = Duration::from_millis(
    (crate::registry::SAFE_READ_IDS.len() + crate::registry::SAFE_READ_INPUT_IDS.len()) as u64
        * slot_ms(),
);
/// Longest wait for the cross-process lock.
pub const LOCK_WAIT: Duration = Duration::from_millis(500);

/// May the daemon request this Feature id at all (routine or once per connection)?
#[cfg(any(test, feature = "testseam"))]
#[must_use]
pub fn is_allowed(id: u8) -> bool {
    crate::registry::check_read(id).is_ok()
}

/// What was read once in the current connection.
#[derive(Debug, Default)]
pub struct ConnState {
    done: std::collections::BTreeSet<u8>,
    values: std::collections::BTreeMap<u8, Vec<u8>>,
}

impl ConnState {
    #[must_use]
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
    #[must_use]
    pub fn requested(&self, id: u8) -> bool {
        self.done.contains(&id)
    }
    pub fn store(&mut self, id: u8, frame: Vec<u8>) {
        self.values.insert(id, frame);
    }
    pub fn frame(&self, id: u8) -> Option<&[u8]> {
        self.values.get(&id).map(Vec::as_slice)
    }
    /// Forget these ids only: each may be requested once more in this connection.
    pub fn forget(&mut self, ids: &[u8]) {
        for id in ids {
            self.done.remove(id);
            self.values.remove(id);
        }
    }
}

static CONN: Mutex<ConnState> = Mutex::new(ConnState::new());

fn global_conn() -> std::sync::MutexGuard<'static, ConnState> {
    CONN.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The name stored in the keyboard was just rewritten.
pub fn forget_name_fragments() {
    global_conn().forget(&crate::devname::FRAGMENT_IDS);
}

static EPOCH: OnceLock<Instant> = OnceLock::new();
/// Milliseconds since EPOCH of the last input report, +1 (0 = never).
static LAST_INPUT: AtomicU64 = AtomicU64::new(0);

fn epoch() -> Instant {
    *EPOCH.get_or_init(Instant::now)
}

/// An input report arrived (key press, wake event).
pub fn note_input() {
    note_input_at(Instant::now());
}

fn note_input_at(t: Instant) {
    let ms = crate::conv::millis_u64(t.saturating_duration_since(epoch()));
    LAST_INPUT.store(ms + 1, Ordering::Relaxed);
    // no probe any more on a key press.
}

pub fn last_input_age(now: Instant) -> Option<Duration> {
    let v = LAST_INPUT.load(Ordering::Relaxed);
    (v != 0).then(|| now.saturating_duration_since(epoch() + Duration::from_millis(v - 1)))
}

static LAST_HW: AtomicU64 = AtomicU64::new(0);

static SHARE_HW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Name of the shared stamp, next to `hid.lock`.
pub const HW_STAMP_FILE: &str = "hid.last";

/// Share the instant of the last hardware access with the other processes.
pub fn share_hw_access(on: bool) {
    SHARE_HW.store(on, Ordering::Relaxed);
}

/// Path of the shared stamp (`$XDG_RUNTIME_DIR/apple-kb-monitor/hid.last`).
#[must_use]
pub fn hw_stamp_path() -> PathBuf {
    lock_path().with_file_name(HW_STAMP_FILE)
}

/// `CLOCK_MONOTONIC` in milliseconds: the same clock in every process.
#[must_use]
pub fn monotonic_ms() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec; CLOCK_MONOTONIC always exists.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut ts) };
    u64::try_from(ts.tv_sec)
        .unwrap_or(0)
        .saturating_mul(1000)
        .saturating_add(u64::try_from(ts.tv_nsec).unwrap_or(0) / 1_000_000)
}

/// Write the shared stamp (private directory, no symlink followed).
///
/// # Errors
///
/// Any I/O error while creating or writing the file.
pub fn write_hw_stamp(path: &std::path::Path, mono_ms: u64) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    ensure_private_dir(path.parent().ok_or_else(|| io::Error::other("no parent"))?)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    writeln!(f, "{mono_ms}")
}

/// The shared stamp, if the file holds one.
#[must_use]
pub fn read_hw_stamp(path: &std::path::Path) -> Option<u64> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let mut text = String::new();
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .ok()?
        .take(32)
        .read_to_string(&mut text)
        .ok()?;
    text.trim().parse().ok()
}

/// How long ago was a stamp written, if that is recent enough to matter?
#[must_use]
pub fn stamp_age(stamp_ms: u64, now_ms: u64) -> Option<Duration> {
    let age = Duration::from_millis(now_ms.checked_sub(stamp_ms)?);
    (age < MIN_GAP).then_some(age)
}

fn shared_hw_access() -> Option<Instant> {
    if !SHARE_HW.load(Ordering::Relaxed) {
        return None;
    }
    let age = stamp_age(read_hw_stamp(&hw_stamp_path())?, monotonic_ms())?;
    Instant::now().checked_sub(age)
}

/// A request was just sent to the keyboard (read or the `WillShutdown` write).
pub fn note_hw_access() {
    let ms = crate::conv::millis_u64(Instant::now().saturating_duration_since(epoch()));
    LAST_HW.store(ms + 1, Ordering::Relaxed);
    if SHARE_HW.load(Ordering::Relaxed) {
        // Best effort: without the stamp the spacing holds in-process only.
        let _ = write_hw_stamp(&hw_stamp_path(), monotonic_ms());
    }
}

/// Instant of the last request sent to the keyboard, if any.
pub fn last_hw_access() -> Option<Instant> {
    let v = LAST_HW.load(Ordering::Relaxed);
    let own = (v != 0).then(|| epoch() + Duration::from_millis(v - 1));
    own.max(shared_hw_access())
}

/// Why a read was or was not done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Allowed,
    /// The Apple model says no battery read is due now.
    NotDue,
    /// No key press within `ACTIVE_WINDOW`: the keyboard is idle / in sniff.
    Idle,
    /// Another reader holds the lock.
    Busy,
    /// Circuit breaker open: the keyboard stopped answering.
    Tripped,
}

#[must_use]
pub fn gate(age: Option<Duration>) -> Gate {
    match age {
        Some(a) if a < ACTIVE_WINDOW => Gate::Allowed,
        _ => Gate::Idle,
    }
}

static SCHEDULE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The daemon tells, before each acquisition, whether the Apple model has a battery read due
/// (`Some(true)`), none (`Some(false)`), or leaves the decision to the activity gate (`None`).
pub fn set_schedule(due: Option<bool>) {
    SCHEDULE.store(due.map_or(0, |d| if d { 1 } else { 2 }), Ordering::Relaxed);
}

/// The daemon tells that the next acquisition reads the name stored in the keyboard (`0x51`-`0x54`,
/// D-Bus `RereadName`) and nothing else: no routine report, so no battery cycle.
pub fn set_name_only_schedule() {
    SCHEDULE.store(3, Ordering::Relaxed);
}

fn schedule() -> Option<bool> {
    match SCHEDULE.load(Ordering::Relaxed) {
        1 => Some(true),
        2 | 3 => Some(false),
        _ => None,
    }
}

fn name_only_scheduled() -> bool {
    SCHEDULE.load(Ordering::Relaxed) == 3
}

/// Gate of a read: the model's schedule when there is one, else the activity of the keyboard.
#[must_use]
pub fn gate_for(schedule: Option<bool>, age: Option<Duration>) -> Gate {
    match schedule {
        Some(true) => Gate::Allowed,
        Some(false) => Gate::NotDue,
        None => gate(age),
    }
}

static HOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ask the running burst to stop before its next request (`true`), or lift that (`false`).
pub fn hold(on: bool) {
    HOLD.store(on, Ordering::SeqCst);
}

/// Whether [`hold`] is on.
#[must_use]
pub fn held() -> bool {
    HOLD.load(Ordering::SeqCst)
}

static LAST_OUTCOME: Mutex<Option<SafeRead>> = Mutex::new(None);

/// Outcome of the last [`build_report_safe`], taken once.
pub fn take_last_outcome() -> Option<SafeRead> {
    LAST_OUTCOME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

/// Same outcome without consuming it.
pub fn peek_last_outcome() -> Option<SafeRead> {
    *LAST_OUTCOME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Apple's breaker: counter common to every request, 3rd silence = nothing more goes out + one
/// disconnection request.
pub use crate::apple_model::Breaker;

static BREAKER: Mutex<Breaker> = Mutex::new(Breaker::new());

fn global_breaker() -> std::sync::MutexGuard<'static, Breaker> {
    BREAKER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The process-wide breaker, for the one write ([`crate::parity`]).
#[must_use]
pub fn breaker() -> &'static Mutex<Breaker> {
    &BREAKER
}

/// A new connection of the keyboard was announced: closes the breaker.
pub fn note_connection() {
    global_breaker().reset();
    global_conn().reset();
}

/// The system goes to sleep (Apple's `handleSleep`).
pub fn note_sleep() {
    global_breaker().after_sleep();
}

/// The disconnection request raised by the breaker, once per connection.
#[must_use]
pub fn take_disconnect_request() -> bool {
    global_breaker().take_disconnect_request()
}

/// Is the global breaker open (reads are suspended)?
#[must_use]
pub fn tripped() -> bool {
    !global_breaker().would_allow()
}

/// Consecutive silences counted by the global breaker.
#[must_use]
pub fn breaker_counter() -> u32 {
    global_breaker().counter()
}

static BREAKER_PUBLISHER: Mutex<crate::breaker_state::Publisher> =
    Mutex::new(crate::breaker_state::Publisher::new());

/// Path of the published state: next to the HID lock.
#[must_use]
pub fn breaker_state_path() -> PathBuf {
    lock_path().with_file_name(crate::breaker_state::FILE_NAME)
}

/// The daemon publishes its breaker for `akm-helper hid-control` and `akmctl`.
///
/// # Errors
///
/// Any I/O error from [`crate::breaker_state::write`].
pub fn publish_breaker_state(mac: Option<&str>) -> io::Result<bool> {
    let st = crate::breaker_state::BreakerState {
        mac: mac.map(str::to_ascii_uppercase),
        open: tripped(),
        counter: breaker_counter(),
        written_unix: crate::breaker_state::now_unix(),
        pid: std::process::id(),
        starttime: crate::breaker_state::proc_starttime(
            std::path::Path::new("/proc"),
            std::process::id(),
        ),
    };
    let path = breaker_state_path();
    ensure_private_dir(path.parent().ok_or_else(|| io::Error::other("no parent"))?)?;
    BREAKER_PUBLISHER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .publish(&path, &st, Instant::now())
}

/// The daemon stops: no published state = the readers' former behaviour.
pub fn withdraw_breaker_state() {
    crate::breaker_state::remove(&breaker_state_path());
}

static IN_PROCESS: Mutex<()> = Mutex::new(());

/// Path of the cross-process lock.
#[must_use]
pub fn lock_path() -> PathBuf {
    crate::paths::runtime_dir().join("hid.lock")
}

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
            format!(
                "{} is not a private directory owned by uid {me}",
                dir.display()
            ),
        ));
    }
    Ok(())
}

/// Held for the duration of a read.
pub struct ReadLock {
    _file: std::fs::File,
    _guard: std::sync::MutexGuard<'static, ()>,
}

/// Take both locks, waiting at most `wait`.
pub fn try_lock(wait: Duration) -> Option<ReadLock> {
    use std::os::unix::fs::OpenOptionsExt;
    let end = Instant::now() + wait;
    // In-process lock: honour `wait` like the flock, and survive poisoning.
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

/// A [`HidSource`] that refuses (without any I/O) every id the register map does not allow, serves
/// a once-per-connection id once, and spaces the requests by [`MIN_GAP`].
pub struct SafeSource<'a> {
    inner: &'a dyn HidSource,
    last: std::cell::Cell<Option<Instant>>,
    sent: std::cell::Cell<u32>,
    breaker: Option<&'a Mutex<Breaker>>,
    conn: Option<&'a Mutex<ConnState>>,
}

/// Time to wait before a request, given the previous one (pure, testable).
#[must_use]
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
    #[cfg(test)]
    /// Same with a private breaker (tests).
    pub fn with_breaker(inner: &'a dyn HidSource, breaker: &'a Mutex<Breaker>) -> Self {
        Self {
            breaker: Some(breaker),
            ..Self::new(inner)
        }
    }
    #[cfg(any(test, feature = "testseam"))]
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
            Some(c) => c.lock().unwrap_or_else(std::sync::PoisonError::into_inner),
            None => global_conn(),
        }
    }
    fn breaker(&self) -> std::sync::MutexGuard<'_, Breaker> {
        match self.breaker {
            Some(b) => b.lock().unwrap_or_else(std::sync::PoisonError::into_inner),
            None => global_breaker(),
        }
    }
    /// Requests actually sent to the device.
    pub fn sent(&self) -> u32 {
        self.sent.get()
    }
}

impl SafeSource<'_> {
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
        // First request of this source.
        let last = self.last.get().or_else(shared_hw_access);
        let w = wait_before(last, Instant::now());
        if !w.is_zero() {
            std::thread::sleep(w);
        }
        self.sent.set(self.sent.get() + 1);
        let t0 = Instant::now();
        let r = send();
        let took = t0.elapsed();
        self.last.set(Some(Instant::now()));
        note_hw_access();
        // Apple's verdict on this exchange (refusal = answer, other id or late answer = silence).
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

    fn input(&self, report_id: u8) -> io::Result<Vec<u8>> {
        crate::registry::check_read_input(report_id)?;
        self.request(report_id, false, || self.inner.input(report_id))
    }
}

/// The routine battery read, in Apple's order (R2).
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
    /// Did the battery read of the model succeed?
    #[must_use]
    pub fn is_success(self) -> bool {
        matches!(self, SafeRead::Complete | SafeRead::Skipped(Gate::Allowed))
    }

    /// Were requests really sent to the keyboard without all of them being answered?
    #[must_use]
    pub fn attempted_and_failed(self) -> bool {
        matches!(self, SafeRead::Partial)
    }
}

pub fn read_safe(src: &dyn HidSource, report: &mut KbReport) -> SafeRead {
    read_with(&SafeSource::new(src), report, false)
}

fn read_with(safe: &SafeSource<'_>, report: &mut KbReport, with_once: bool) -> SafeRead {
    read_with_hold(safe, report, with_once, &HOLD)
}

fn read_with_hold(
    safe: &SafeSource<'_>,
    report: &mut KbReport,
    with_once: bool,
    hold: &std::sync::atomic::AtomicBool,
) -> SafeRead {
    use crate::apple_model::Request;
    let is_held = || hold.load(Ordering::SeqCst);
    let start = Instant::now();
    let mut complete = true;
    let mut volt_frames: std::collections::HashMap<u8, Vec<u8>> = std::collections::HashMap::new();
    // Apple's order (R2): 0x47, then GET Input 0x30 once, then 0x46 / 0x49.
    for req in routine_reads() {
        if is_held() || (safe.sent() > 0 && start.elapsed() >= BUDGET) {
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
            Request::GetFeature(id @ (0x46 | 0x49)) => {
                report.raw.insert(format!("{id:#04x}"), hex(&b[1..]));
                // Decoded below by the same decoder as `akmctl info`.
                volt_frames.insert(id, b);
            }
            Request::GetFeature(id) => {
                report.raw.insert(format!("{id:#04x}"), hex(&b[1..]));
            }
            _ => {}
        }
    }
    if !volt_frames.is_empty() {
        // 0x46 = cell voltage, 0x49 = filtered voltage, both u16 LE in mV.
        crate::decode::decode_voltage(&volt_frames, &mut report.battery);
        // The daemon keeps its own stricter bound on the cell voltage.
        if report
            .battery
            .voltage_mv
            .is_some_and(|mv| !(1500..=3700).contains(&mv))
        {
            report.battery.voltage_mv = None;
            report.battery.voltage = None;
        }
    }
    if complete && with_once {
        complete = read_once_ids(safe, &is_held);
    }
    report.incomplete = !complete;
    if complete {
        SafeRead::Complete
    } else {
        SafeRead::Partial
    }
}

/// Once-per-connection ids, then the deferred ones; `false` when the mandatory ones are not all read.
fn read_once_ids(safe: &SafeSource<'_>, is_held: &dyn Fn() -> bool) -> bool {
    let phase = Instant::now();
    for id in crate::registry::DAEMON_ONCE_IDS {
        if safe.conn().requested(id) {
            continue;
        }
        if is_held() || phase.elapsed() >= ONCE_BUDGET {
            return false;
        }
        let Ok(b) = safe.feature(id) else {
            return false;
        };
        safe.conn().store(id, b);
    }
    // Lower priority: out of time = left for the next burst, not a failure.
    for id in crate::registry::DAEMON_DEFERRED_ONCE_IDS {
        if safe.conn().requested(id) {
            continue;
        }
        if is_held() || phase.elapsed() >= ONCE_BUDGET {
            break;
        }
        let Ok(b) = safe.feature(id) else {
            return false;
        };
        safe.conn().store(id, b);
    }
    true
}

/// Read the uncached name fragments `0x51`-`0x54` through the same [`SafeSource`].
fn read_name_fragments(safe: &SafeSource<'_>, hold: &std::sync::atomic::AtomicBool) -> SafeRead {
    for id in crate::devname::FRAGMENT_IDS {
        if safe.conn().requested(id) {
            continue;
        }
        if hold.load(Ordering::SeqCst) {
            return SafeRead::Partial;
        }
        match safe.feature(id) {
            Ok(b) => safe.conn().store(id, b),
            Err(_) => return SafeRead::Partial,
        }
    }
    SafeRead::Complete
}

/// Fill the report from the frames read once in this connection.
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
            .or_else(|| {
                report.battery.voltage.map(|v| {
                    u32::try_from(crate::conv::u64_from_f64_round(v * 1000.0)).unwrap_or(u32::MAX)
                })
            }),
    ) {
        report.battery.threshold_level = Some(t.level(mv).as_str().to_string());
        report.battery.threshold_margins_mv = Some(t.margins(mv));
    }
    // "Apple display" percentage: only for the PIDs IOBluetooth remaps.
    report.battery.apple_display_pct = pid
        .filter(|&p| crate::registry::apple_display_applies(p))
        .and(report.battery.percentage)
        .filter(|p| p.is_finite() && (0.0..=100.0).contains(p))
        .map(|p| {
            crate::registry::apple_display_percent(
                u8::try_from(crate::conv::u64_from_f64_round(p)).expect("filtered to 0..=100"),
            )
        });
    crate::firmware::assess_report(pid, &mut report.firmware);
    // Name stored in the keyboard: only when the 4 fragments are cached.
    let frags: Option<Vec<Vec<u8>>> = crate::devname::FRAGMENT_IDS
        .iter()
        .map(|&id| st.frame(id).map(<[u8]>::to_vec))
        .collect();
    if let Some(raw) = frags.as_deref().and_then(crate::devname::raw_from_frames) {
        report.device.name_on_keyboard = crate::devname::name_from_raw(&raw);
        report.device.name_on_keyboard_hex = Some(hex(&raw));
    }
}

/// Full report for the daemon under the safe policy.
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
        *LAST_OUTCOME
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(SafeRead::Skipped(Gate::Allowed));
        return (report, SafeRead::Skipped(Gate::Allowed));
    }
    let outcome = match gate_for(schedule(), last_input_age(now)) {
        Gate::NotDue if name_only_scheduled() => {
            if tripped() {
                SafeRead::Skipped(Gate::Tripped)
            } else {
                match try_lock(LOCK_WAIT) {
                    Some(_lock) => read_name_fragments(&SafeSource::new(src), &HOLD),
                    None => SafeRead::Skipped(Gate::Busy),
                }
            }
        }
        Gate::Allowed if tripped() => SafeRead::Skipped(Gate::Tripped),
        Gate::Allowed => match try_lock(LOCK_WAIT) {
            Some(_lock) => read_with(&SafeSource::new(src), &mut report, true),
            None => SafeRead::Skipped(Gate::Busy),
        },
        g => SafeRead::Skipped(g),
    };
    *LAST_OUTCOME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
    report.battery.percentage_fine = report.battery.percentage;
    report.breaker_open = tripped();
    // Values read once in this connection survive the following reads.
    apply_cached(
        &mut report,
        crate::model::parse_hid_id(uevent).map(|(_, p)| p),
    );
    (report, outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::Fixture;
    use std::cell::RefCell;
    use std::fmt::Write as _;

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

    fn fixture() -> Fixture {
        Fixture::new()
            .with_input(&[0x30, 0])
            .with(&[0x47, 99])
            .with(&[0x46, 0xBA, 0x0B]) // 0x0BBA = 3002 mV
            .with(&[0x49, 0x89, 0x0B])
            .with(&[0xEA, 98])
            .with(&[0xFE, 0, 0, 0, 0, 0, 0, 0, 0])
    }

    struct Slow<'a> {
        inner: &'a dyn HidSource,
        delay: Duration,
    }
    impl HidSource for Slow<'_> {
        fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
            std::thread::sleep(self.delay);
            self.inner.feature(id)
        }
        fn input(&self, id: u8) -> io::Result<Vec<u8>> {
            std::thread::sleep(self.delay);
            self.inner.input(id)
        }
    }

    #[test]
    fn a_keyboard_answering_in_one_second_is_read_whole_in_one_burst() {
        let f = fixture()
            .with(&[0x4F, 0x50, 0x00])
            .with(&[0x60, 0x0b, 0x8a, 0x09, 0xca, 0x09, 0x64, 0x08, 0x06])
            .with(b"\x51Alice's ")
            .with(b"\x52keyboard")
            .with(b"\x53 #1\0\0\0\0\0")
            .with(&[0x54, 0, 0, 0, 0, 0, 0, 0, 0]);
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let slow = Slow {
            inner: &spy,
            delay: Duration::from_secs(1),
        };
        let breaker = Mutex::new(Breaker::new());
        let conn = Mutex::new(ConnState::new());
        let mut r = KbReport::default();
        let out = read_with(
            &SafeSource::with_parts(&slow, &breaker, &conn),
            &mut r,
            true,
        );
        assert_eq!(out, SafeRead::Complete);
        assert!(!r.incomplete);
        assert_eq!(
            *spy.log.borrow(),
            vec![0x47, 0x30, 0x46, 0x49, 0x4F, 0x60, 0x51, 0x52, 0x53, 0x54],
            "the routine reads, then every once-per-connection id, in one burst"
        );
        apply_frames(&mut r, Some(0x0256), &conn.lock().unwrap());
        assert_eq!(
            r.device.name_on_keyboard.as_deref(),
            Some("Alice's keyboard #1")
        );
        assert_eq!(r.firmware.version.as_deref(), Some("0x0050"));
        assert!(r.battery.thresholds.is_some());
        assert_eq!(
            breaker.lock().unwrap().counter(),
            0,
            "a slow answer is an answer"
        );
    }

    #[test]
    fn the_budgets_cover_slow_answers_for_every_request() {
        let slot = MIN_GAP + SLOW_ANSWER;
        assert!(SLOW_ANSWER >= Duration::from_secs(1), "measured: 1.0 s");
        assert!(SLOW_ANSWER < crate::apple_model::APPLE.report_timeout);
        assert_eq!(BUDGET, slot * u32::try_from(routine_reads().len()).unwrap());
        let once = crate::registry::DAEMON_ONCE_IDS.len()
            + crate::registry::DAEMON_DEFERRED_ONCE_IDS.len();
        assert_eq!(ONCE_BUDGET, slot * u32::try_from(once).unwrap());
    }

    #[test]
    fn a_held_burst_sends_nothing() {
        let f = fixture();
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let breaker = Mutex::new(Breaker::new());
        let conn = Mutex::new(ConnState::new());
        let hold = std::sync::atomic::AtomicBool::new(true);
        let mut r = KbReport::default();
        let out = read_with_hold(
            &SafeSource::with_parts(&spy, &breaker, &conn),
            &mut r,
            true,
            &hold,
        );
        assert_eq!(out, SafeRead::Partial);
        assert!(spy.log.borrow().is_empty());
        hold.store(false, Ordering::SeqCst);
        let out = read_with_hold(
            &SafeSource::with_parts(&spy, &breaker, &conn),
            &mut r,
            false,
            &hold,
        );
        assert_eq!(out, SafeRead::Complete);
        assert_eq!(*spy.log.borrow(), vec![0x47, 0x30, 0x46, 0x49]);
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
        assert_eq!(ALLOWED, [0x47, 0x46, 0x49]);
    }

    #[test]
    fn no_forbidden_class_is_reachable_through_the_policy() {
        use crate::registry::{classify_feature, Safety};
        let full = Fixture::from_hex_dump(&(0..=255u32).fold(String::new(), |mut s, i| {
            let _ = writeln!(s, "{i:02x} 00 00 00 00 00 00 00 00");
            s
        }))
        .unwrap();
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
            if matches!(
                class,
                Safety::NeverRead
                    | Safety::NeverWrite
                    | Safety::Unknown
                    | Safety::ManualOnly
                    | Safety::PassiveInput
            ) {
                assert_eq!(
                    r.unwrap_err().kind(),
                    io::ErrorKind::PermissionDenied,
                    "{id:#04x} {class:?}"
                );
            }
        }
        let sent = spy.log.borrow().clone();
        for id in sent {
            assert!(
                matches!(
                    classify_feature(id),
                    Safety::SafeRead | Safety::OncePerConnection
                ),
                "{id:#04x} reached the device"
            );
        }
        assert!(!spy.log.borrow().contains(&0xFE) && !spy.log.borrow().contains(&0x4C));
    }

    #[test]
    fn forgetting_the_name_fragments_frees_these_four_ids_only() {
        let mut st = ConnState::new();
        for id in [0x4F, 0x60, 0x51, 0x52, 0x53, 0x54] {
            assert!(st.claim(id));
            st.store(id, vec![id, 1]);
        }
        st.forget(&crate::devname::FRAGMENT_IDS);
        for id in crate::devname::FRAGMENT_IDS {
            assert!(!st.requested(id) && st.frame(id).is_none(), "{id:#04x}");
            assert!(st.claim(id), "{id:#04x} may be requested once more");
        }
        for id in [0x4F, 0x60] {
            assert!(st.requested(id) && st.frame(id).is_some(), "{id:#04x} kept");
        }
        assert_eq!(crate::devname::FRAGMENT_IDS, [0x51, 0x52, 0x53, 0x54]);
        forget_name_fragments();
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
        let safe2 = SafeSource::with_parts(&spy, &breaker, &conn);
        assert_eq!(
            safe2.feature(0x4F).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        conn.lock().unwrap().reset(); // new connection
        assert!(SafeSource::with_parts(&spy, &breaker, &conn)
            .feature(0x4F)
            .is_ok());
        assert_eq!(spy.log.borrow().len(), 2);
    }

    #[test]
    fn a_full_burst_reads_the_version_and_thresholds_once_then_serves_the_cache() {
        let f = fixture()
            .with(&[0x4F, 0x50, 0x00])
            .with(&[0x60, 0x0b, 0x8a, 0x09, 0xca, 0x09, 0x64, 0x08, 0x06])
            .with(b"\x51Alice's ")
            .with(b"\x52keyboard")
            .with(b"\x53 #1\0\0\0\0\0")
            .with(&[0x54, 0, 0, 0, 0, 0, 0, 0, 0]);
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let breaker = Mutex::new(Breaker::new());
        let conn = Mutex::new(ConnState::new());
        let mut r = KbReport::default();
        let out = read_with(&SafeSource::with_parts(&spy, &breaker, &conn), &mut r, true);
        assert_eq!(
            out,
            SafeRead::Complete,
            "running out of time on the name is not a failure"
        );
        assert_eq!(spy.log.borrow()[..6], [0x47, 0x30, 0x46, 0x49, 0x4F, 0x60]);
        for _ in 0..4 {
            if crate::registry::DAEMON_DEFERRED_ONCE_IDS
                .iter()
                .all(|&id| conn.lock().unwrap().requested(id))
            {
                break;
            }
            let mut rx = KbReport::default();
            assert_eq!(
                read_with(
                    &SafeSource::with_parts(&spy, &breaker, &conn),
                    &mut rx,
                    true
                ),
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
            Some("Alice's keyboard #1")
        );
        assert_eq!(
            r.device.name_on_keyboard_hex.as_deref().map(str::len),
            Some(64)
        );
        assert_eq!(r.firmware.version.as_deref(), Some("0x0050"));
        assert_eq!(
            r.firmware.status, "unknown",
            "R4: no public reference for 0x0256"
        );
        assert_eq!(
            r.battery.thresholds.unwrap().as_array(),
            [2954, 2506, 2404, 2054]
        );
        assert_eq!(r.battery.threshold_level.as_deref(), Some("ok"));
        assert_eq!(r.raw.get("0x4f").map(String::as_str), Some("5000"));
        let mut r2 = KbReport::default();
        spy.log.borrow_mut().clear();
        let out = read_with(
            &SafeSource::with_parts(&spy, &breaker, &conn),
            &mut r2,
            true,
        );
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
            &SafeSource::with_parts(
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
        let out = read_with(&SafeSource::with_parts(&f, &breaker, &conn), &mut r, true);
        assert_eq!(out, SafeRead::Partial);
        apply_frames(&mut r, Some(0x0256), &conn.lock().unwrap());
        assert_eq!(r.firmware.version, None);
        assert_eq!(r.firmware.status, "unknown");
        assert_eq!(r.battery.thresholds, None);
        assert!(!breaker.lock().unwrap().is_open());
    }

    #[test]
    fn requests_are_spaced() {
        assert_eq!(MIN_GAP, Duration::from_secs(1), "macOS 26.5 spacing");
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
        assert!((0..100).all(|_| !b.allow()));
        b.record(true); // the keyboard answered: closed
        assert!(!b.is_open() && b.allow());
        for _ in 0..TRIP_AFTER {
            b.record(false);
        }
        assert!(b.is_open());
        b.reset();
        assert!(b.allow());
        b.record(false);
        b.record(false);
        b.record(true);
        b.record(false);
        assert!(!b.is_open());
    }

    #[test]
    fn dead_keyboard_gets_three_requests_then_silence() {
        let br = Mutex::new(Breaker::new());
        let dead = Dead(std::cell::Cell::new(0));
        for _ in 0..TRIP_AFTER {
            let mut r = KbReport::default();
            let s = SafeSource::with_breaker(&dead, &br);
            assert_eq!(read_with(&s, &mut r, false), SafeRead::Partial);
        }
        assert_eq!(dead.0.get(), TRIP_AFTER);
        for _ in 0..10 {
            let mut r = KbReport::default();
            let s = SafeSource::with_breaker(&dead, &br);
            assert_eq!(read_with(&s, &mut r, false), SafeRead::Partial);
        }
        assert_eq!(dead.0.get(), TRIP_AFTER, "no request while open");
        br.lock().unwrap().reset(); // new connection
        let mut r = KbReport::default();
        read_with(&SafeSource::with_breaker(&dead, &br), &mut r, false);
        assert_eq!(dead.0.get(), TRIP_AFTER + 1);
    }

    #[test]
    fn lock_dir_is_private_per_uid_and_refuses_foreign_or_linked_dirs() {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::fs::{symlink, PermissionsExt};
        let base = std::env::temp_dir().join(format!("akm-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let d = base.join("ok");
        ensure_private_dir(&d).unwrap();
        assert_eq!(
            std::fs::metadata(&d).unwrap().permissions().mode() & 0o777,
            0o700
        );
        ensure_private_dir(&d).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ensure_private_dir(&d).is_err());
        let target = base.join("target");
        std::fs::create_dir(&target).unwrap();
        let l = base.join("link");
        symlink(&target, &l).unwrap();
        assert!(ensure_private_dir(&l).is_err());
        let sub = base.join("p");
        ensure_private_dir(&sub).unwrap();
        let victim = base.join("victim");
        symlink(&victim, sub.join("hid.lock")).unwrap();
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

    #[test]
    fn safe_read_fills_millivolts_from_real_frames() {
        let f = Fixture::new()
            .with_input(&[0x30, 0])
            .with(&[0x47, 55])
            .with(&[0x46, 0x99, 0x0B])
            .with(&[0x49, 0x5C, 0x0B]);
        let mut r = KbReport::default();
        assert_eq!(read_safe(&f, &mut r), SafeRead::Complete);
        assert_eq!(r.raw.get("0x46").map(String::as_str), Some("990b"));
        assert_eq!(r.raw.get("0x49").map(String::as_str), Some("5c0b"));
        assert_eq!(r.battery.voltage_mv, Some(2969));
        assert_eq!(r.battery.voltage_filtered_mv, Some(2908));
        assert_eq!(r.battery.voltage, Some(2.969));
        assert!(!r.battery.voltage_doubtful);
    }

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
                    gap >= MIN_GAP.checked_sub(Duration::from_millis(5)).unwrap(),
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
        let mut r2 = KbReport::default();
        assert_eq!(read_safe(&spy, &mut r2), SafeRead::Complete);
        let inputs = spy.seq().iter().filter(|(d, _)| *d == "input").count();
        assert_eq!(inputs, 2, "one GET Input 0x30 per burst");
        assert!(spy.seq().iter().all(|(d, id)| *d != "input" || *id == 0x30));
    }

    #[test]
    fn only_input_0x30_passes_the_safe_source() {
        let full = Fixture::from_hex_dump(&(0..=255u32).fold(String::new(), |mut s, i| {
            let _ = writeln!(s, "{i:02x} 00");
            s
        }))
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

    #[test]
    fn a_failed_input_0x30_counts_for_the_breaker_like_0x47() {
        struct HalfDead<'a> {
            fixture: &'a Fixture,
            inputs: std::cell::Cell<u32>,
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
        let mut r = KbReport::default();
        let out = read_with(&SafeSource::with_breaker(&src, &br), &mut r, false);
        assert_eq!(out, SafeRead::Partial, "the burst stops at the silent 0x30");
        assert_eq!(r.battery.percentage, Some(99.0), "0x47 was read");
        assert_eq!(r.battery.state, None);
        assert!(r.incomplete);
        assert_eq!(br.lock().unwrap().counter(), 1, "the silent 0x30 counted");
        assert_eq!(src.inputs.get(), 1);
        src.all_dead.set(true);
        for i in 2..=TRIP_AFTER {
            let mut r = KbReport::default();
            assert_eq!(
                read_with(&SafeSource::with_breaker(&src, &br), &mut r, false),
                SafeRead::Partial
            );
            assert_eq!(br.lock().unwrap().counter(), i);
        }
        assert!(br.lock().unwrap().is_open());
        assert_eq!(src.inputs.get(), 1, "0x47 failed first: 0x30 not asked");
        let mut r = KbReport::default();
        read_with(&SafeSource::with_breaker(&src, &br), &mut r, false);
        assert_eq!(src.inputs.get(), 1);
        assert_eq!(r.battery.percentage, None);
        // a keyboard that answers 0x47 but never 0x30 never trips.
        let br2 = Mutex::new(Breaker::new());
        src.all_dead.set(false);
        for _ in 0..5 {
            let mut r = KbReport::default();
            read_with(&SafeSource::with_breaker(&src, &br2), &mut r, false);
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
        let f = fixture();
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let uevent = "HID_ID=0005:000005AC:00000256\nHID_NAME=Kb\nHID_UNIQ=aa:bb:cc:dd:ee:f1\n";
        let (r, o) =
            build_report_safe(uevent, None, &spy, KbWake::default(), t + ACTIVE_WINDOW * 2);
        assert_eq!(o, SafeRead::Skipped(Gate::Idle));
        assert!(spy.log.borrow().is_empty());
        assert!(r.bluetooth.connected);
    }

    struct Refuses;
    impl HidSource for Refuses {
        fn feature(&self, _: u8) -> io::Result<Vec<u8>> {
            Err(io::Error::from_raw_os_error(libc::EIO))
        }
    }

    struct OtherId;
    impl HidSource for OtherId {
        fn feature(&self, _: u8) -> io::Result<Vec<u8>> {
            Ok(vec![0x46, 1, 2])
        }
    }

    #[test]
    fn a_refusal_is_an_answer_and_resets_the_count() {
        // Apple driver DecodedHandshake: 2 silences, a refusal, 2 silences.
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
        let br = Mutex::new(Breaker::new());
        for i in 0..TRIP_AFTER {
            let e = SafeSource::with_breaker(&OtherId, &br)
                .feature(0x47)
                .unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::TimedOut);
            assert_eq!(br.lock().unwrap().counter(), i + 1);
        }
        let mut b = br.lock().unwrap();
        assert!(b.is_open());
        assert!(
            b.take_disconnect_request(),
            "3rd silence: one disconnection request"
        );
        assert!(!b.take_disconnect_request());
    }

    #[test]
    fn the_schedule_of_the_model_overrides_the_activity_gate() {
        assert_eq!(
            gate_for(Some(true), None),
            Gate::Allowed,
            "due: read even if idle"
        );
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
