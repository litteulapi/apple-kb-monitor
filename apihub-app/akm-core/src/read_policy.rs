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
//! * **circuit breaker** (#214, same as macOS): after [`TRIP_AFTER`] failed
//!   requests in a row nothing more is sent until the keyboard gives a sign of
//!   life: a new connection ([`note_connection`]), or an input report
//!   ([`note_input`]) which re-arms ONE probe request (success closes the
//!   breaker, failure keeps it open);
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
/// Minimum spacing between two requests.
pub const MIN_GAP: Duration = Duration::from_millis(1000);
/// Consecutive failed requests that open the circuit breaker.
pub const TRIP_AFTER: u32 = 3;
/// Time budget of one read; no request starts after it.
pub const BUDGET: Duration = Duration::from_secs(2);
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
    global_breaker().alive();
}

/// Age of the last input report, if any.
pub fn last_input_age(now: Instant) -> Option<Duration> {
    let v = LAST_INPUT.load(Ordering::Relaxed);
    (v != 0).then(|| now.saturating_duration_since(epoch() + Duration::from_millis(v - 1)))
}

/// Why a read was or was not done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Allowed,
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

// ── circuit breaker (#214) ─────────────────────────────────────────────────

/// Consecutive-failure breaker. Pure state, no clock: a keyboard that stops
/// answering is left alone until it shows signs of life.
#[derive(Debug, Default)]
pub struct Breaker {
    fails: u32,
    probe: bool,
}

impl Breaker {
    pub const fn new() -> Self {
        Self {
            fails: 0,
            probe: false,
        }
    }
    pub fn is_open(&self) -> bool {
        self.fails >= TRIP_AFTER
    }
    /// May a request be sent? An open breaker lets exactly one probe through
    /// after a sign of life; asking consumes it.
    pub fn allow(&mut self) -> bool {
        if !self.is_open() {
            return true;
        }
        std::mem::take(&mut self.probe)
    }
    /// Would a request be allowed (without consuming the probe)?
    pub fn would_allow(&self) -> bool {
        !self.is_open() || self.probe
    }
    pub fn record(&mut self, ok: bool) {
        if ok {
            *self = Self::new();
        } else {
            self.fails = self.fails.saturating_add(1);
        }
    }
    /// The keyboard sent an input report: re-arm one probe if tripped.
    pub fn alive(&mut self) {
        if self.is_open() {
            self.probe = true;
        }
    }
    /// New connection: the link was rebuilt, start afresh.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

static BREAKER: Mutex<Breaker> = Mutex::new(Breaker::new());

fn global_breaker() -> std::sync::MutexGuard<'static, Breaker> {
    BREAKER.lock().unwrap_or_else(|e| e.into_inner())
}

/// A new connection of the keyboard was announced: closes the breaker.
pub fn note_connection() {
    global_breaker().reset();
    global_conn().reset();
}

/// Is the global breaker open without a pending probe (reads are suspended)?
pub fn tripped() -> bool {
    !global_breaker().would_allow()
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

impl HidSource for SafeSource<'_> {
    fn feature(&self, report_id: u8) -> io::Result<Vec<u8>> {
        // The register map decides: class SafeRead or OncePerConnection.
        let class = crate::registry::check_read(report_id)?;
        let once = class == crate::registry::Safety::OncePerConnection;
        if once && self.conn().requested(report_id) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("report {report_id:#04x} is read once per connection and was already requested"),
            ));
        }
        if !self.breaker().allow() {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "circuit breaker open: the keyboard stopped answering",
            ));
        }
        if once {
            self.conn().claim(report_id);
        }
        let w = wait_before(self.last.get(), Instant::now());
        if !w.is_zero() {
            std::thread::sleep(w);
        }
        self.sent.set(self.sent.get() + 1);
        let r = self.inner.feature(report_id);
        self.last.set(Some(Instant::now()));
        self.breaker().record(r.is_ok());
        r
    }
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
    let start = Instant::now();
    let mut complete = true;
    for id in crate::registry::SAFE_READ_IDS {
        if safe.sent() > 0 && start.elapsed() >= BUDGET {
            complete = false;
            break;
        }
        let Ok(b) = safe.feature(id) else {
            complete = false;
            break;
        };
        if b.len() < 2 {
            continue;
        }
        report.raw.insert(format!("{id:#04x}"), hex(&b[1..]));
        match id {
            0x47 => {
                if let Some(&pct) = b.get(1).filter(|&&p| p <= 100) {
                    if report.battery.percentage.is_none() {
                        report.battery.percentage = Some(f64::from(pct));
                    }
                }
            }
            0x46 if b.len() >= 3 => {
                // [mesuré] battery voltage in mV, little-endian (= 0xFF BE).
                let mv = u16::from_le_bytes([b[1], b[2]]);
                if (1500..=3700).contains(&mv) {
                    report.battery.voltage = Some(f64::from(mv) / 1000.0);
                }
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
        return (report, SafeRead::Skipped(Gate::Allowed));
    }
    let outcome = match gate(last_input_age(now)) {
        Gate::Allowed if tripped() => SafeRead::Skipped(Gate::Tripped),
        Gate::Allowed => match try_lock(LOCK_WAIT) {
            Some(_lock) => read_with(SafeSource::new(src), &mut report, true),
            None => SafeRead::Skipped(Gate::Busy),
        },
        g => SafeRead::Skipped(g),
    };
    report.battery.percentage_fine = report.battery.percentage;
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
    }

    fn fixture() -> Fixture {
        Fixture::new()
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
            .with(&[0x60, 0x0b, 0x8a, 0x09, 0xca, 0x09, 0x64, 0x08, 0x06]);
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let breaker = Mutex::new(Breaker::new());
        let conn = Mutex::new(ConnState::new());
        let mut r = KbReport::default();
        let out = read_with(SafeSource::with_parts(&spy, &breaker, &conn), &mut r, true);
        assert_eq!(out, SafeRead::Complete);
        assert_eq!(*spy.log.borrow(), vec![0x47, 0x46, 0x49, 0x4F, 0x60]);
        apply_frames(&mut r, Some(0x0256), &conn.lock().unwrap());
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
        assert_eq!(*spy.log.borrow(), vec![0x47, 0x46, 0x49]);
        apply_frames(&mut r2, Some(0x0256), &conn.lock().unwrap());
        assert_eq!(r2.firmware.version.as_deref(), Some("0x0050"), "cache survives");
        // the CLI path (read_safe) never makes the once-per-connection reads
        let spy2 = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let mut r3 = KbReport::default();
        let _ = read_with(SafeSource::with_parts(&spy2, &Mutex::new(Breaker::new()), &Mutex::new(ConnState::new())), &mut r3, false);
        assert_eq!(*spy2.log.borrow(), vec![0x47, 0x46, 0x49]);
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
            vec![0x47, 0x46, 0x49],
            "3 requests, no probe, no scan"
        );
        assert_eq!(r.battery.percentage, Some(99.0));
        assert_eq!(r.battery.voltage, Some(3.002));
        assert!(!r.incomplete);
    }

    #[test]
    fn first_failure_stops_the_read() {
        let f = Fixture::new().with(&[0x47, 80]); // 0x46 missing -> error
        let spy = Spy {
            inner: &f,
            log: RefCell::new(Vec::new()),
        };
        let mut r = KbReport::default();
        assert_eq!(read_safe(&spy, &mut r), SafeRead::Partial);
        assert_eq!(*spy.log.borrow(), vec![0x47, 0x46]);
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
}
