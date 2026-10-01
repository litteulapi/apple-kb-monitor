//! Safe HID read policy of the daemon (#142, #177; docs/RECONNEXION-PAIRAGE.md §3.6).
//!
//! Three link losses were captured or logged in the middle of GET_REPORT
//! bursts (29/09 15:09, 01/10 04:00:13, 01/10 12:13:08): the keyboard went
//! radio-silent right after a vendor Feature request and stopped page-scanning
//! (supervision timeout, then `Host is down`). Until the BCM2042 firmware is
//! understood, the daemon reads as little as possible:
//!
//! * **allow-list**: only `0x47` (declared Battery Strength), `0x46` (battery
//!   voltage, mV LE [mesuré]) and `0x49` (filtered voltage [mesuré]). Never
//!   `0xFE`, never an undeclared or unknown id, never a scan; no `0xEA` probe
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

/// The only Feature Reports the daemon may request.
pub const ALLOWED: [u8; 3] = [0x47, 0x46, 0x49];
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

pub fn is_allowed(id: u8) -> bool {
    ALLOWED.contains(&id)
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
    let guard = IN_PROCESS.try_lock().ok()?;
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
    let end = Instant::now() + wait;
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

/// A [`HidSource`] that refuses (without any I/O) every id outside
/// [`ALLOWED`] and spaces the requests by [`MIN_GAP`].
pub struct SafeSource<'a> {
    inner: &'a dyn HidSource,
    last: std::cell::Cell<Option<Instant>>,
    sent: std::cell::Cell<u32>,
    /// `None` = the process-wide breaker.
    breaker: Option<&'a Mutex<Breaker>>,
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
        }
    }
    /// Same with a private breaker (tests).
    pub fn with_breaker(inner: &'a dyn HidSource, breaker: &'a Mutex<Breaker>) -> Self {
        Self {
            breaker: Some(breaker),
            ..Self::new(inner)
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
        if !is_allowed(report_id) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("report {report_id:#04x} is not in the safe read list"),
            ));
        }
        if !self.breaker().allow() {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "circuit breaker open: the keyboard stopped answering",
            ));
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

/// Read the allowed reports into `report`. Stops at the first failure.
pub fn read_safe(src: &dyn HidSource, report: &mut KbReport) -> SafeRead {
    read_with(SafeSource::new(src), report)
}

fn read_with(safe: SafeSource<'_>, report: &mut KbReport) -> SafeRead {
    let start = Instant::now();
    let mut complete = true;
    for id in [0x47u8, 0x46, 0x49] {
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
    report.incomplete = !complete;
    if complete {
        SafeRead::Complete
    } else {
        SafeRead::Partial
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
        return (report, SafeRead::Skipped(Gate::Allowed));
    }
    let outcome = match gate(last_input_age(now)) {
        Gate::Allowed if tripped() => SafeRead::Skipped(Gate::Tripped),
        Gate::Allowed => match try_lock(LOCK_WAIT) {
            Some(_lock) => read_safe(src, &mut report),
            None => SafeRead::Skipped(Gate::Busy),
        },
        g => SafeRead::Skipped(g),
    };
    report.battery.percentage_fine = report.battery.percentage;
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
            assert_eq!(is_allowed(id), [0x46, 0x47, 0x49].contains(&id));
        }
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
            assert_eq!(read_with(s, &mut r), SafeRead::Partial);
        }
        assert_eq!(dead.0.get(), TRIP_AFTER);
        // breaker open: further reads send nothing
        for _ in 0..10 {
            let mut r = KbReport::default();
            let s = SafeSource::with_breaker(&dead, &br);
            assert_eq!(read_with(s, &mut r), SafeRead::Partial);
        }
        assert_eq!(dead.0.get(), TRIP_AFTER, "no request while open");
        // sign of life -> one probe only
        br.lock().unwrap().alive();
        let mut r = KbReport::default();
        read_with(SafeSource::with_breaker(&dead, &br), &mut r);
        assert_eq!(dead.0.get(), TRIP_AFTER + 1);
        read_with(SafeSource::with_breaker(&dead, &br), &mut r);
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
