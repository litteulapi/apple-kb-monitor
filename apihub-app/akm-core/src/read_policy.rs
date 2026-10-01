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
//! * **short**: at least [`MIN_GAP`] between two requests, [`BUDGET`] per read,
//!   stop at the first failure of any kind (a HIDP timeout costs ~3.5 s).
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
pub const MIN_GAP: Duration = Duration::from_millis(250);
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
}

pub fn gate(age: Option<Duration>) -> Gate {
    match age {
        Some(a) if a < ACTIVE_WINDOW => Gate::Allowed,
        _ => Gate::Idle,
    }
}

// ── single reader ──────────────────────────────────────────────────────────

static IN_PROCESS: Mutex<()> = Mutex::new(());

/// Path of the cross-process lock (also used by the CLI / RE tools).
pub fn lock_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("apple-kb-monitor").join("hid.lock")
}

/// Held for the duration of a read.
pub struct ReadLock {
    _file: std::fs::File,
    _guard: std::sync::MutexGuard<'static, ()>,
}

/// Take both locks, waiting at most `wait`; `None` if another reader is active.
pub fn try_lock(wait: Duration) -> Option<ReadLock> {
    let guard = IN_PROCESS.try_lock().ok()?;
    let p = lock_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
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
}

impl<'a> SafeSource<'a> {
    pub fn new(inner: &'a dyn HidSource) -> Self {
        Self {
            inner,
            last: std::cell::Cell::new(None),
            sent: std::cell::Cell::new(0),
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
        if let Some(l) = self.last.get() {
            let e = l.elapsed();
            if e < MIN_GAP {
                std::thread::sleep(MIN_GAP - e);
            }
        }
        self.sent.set(self.sent.get() + 1);
        let r = self.inner.feature(report_id);
        self.last.set(Some(Instant::now()));
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
    let safe = SafeSource::new(src);
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
        let f = fixture();
        let safe = SafeSource::new(&f);
        let t = Instant::now();
        safe.feature(0x47).unwrap();
        safe.feature(0x46).unwrap();
        safe.feature(0x49).unwrap();
        assert!(t.elapsed() >= MIN_GAP * 2);
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
