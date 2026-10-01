//! The real HID source: `/dev/hidrawN` of the Apple keyboard, plus the wake
//! monitor (input report 0x13).
//!
//! Only one process should ever open the keyboard's hidraw node: the daemon
//! `apple-kb-monitord` (or `apihub-app` when the daemon is absent). The
//! node is located by `HID_ID` (vendor/product) in sysfs, never by name.

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::decode::{report_from_uevent, HidSource};
use crate::model::apple_model_from_uevent;
use crate::report::{KbReport, KbWake};

/// HIDIOCGFEATURE = _IOWR('H', 0x07, 256) — read HID Feature Report
const HIDIOCGFEATURE: libc::c_ulong = 0xC1004807;

/// The write twin of the read ioctl, sized for ONE byte: _IOWR('H', 0x06, 1).
/// The size is part of the request number, so the kernel can never send more
/// than the single report id (`WillShutdown`, wire `53 40`).
const HIDIOCSFEATURE_1: libc::c_ulong = 0xC001_4806;

/// Retries of an ioctl interrupted by a signal (EINTR) before giving up.
const EINTR_RETRIES: u32 = 3;

/// Read one Feature Report on a raw fd (GET_REPORT: read-only on the device).
///
/// The errno is captured right after the failing ioctl (never a stale one),
/// an EINTR is retried, and a 0-byte answer is an empty report (`Ok(vec![])`,
/// #134). The buffer returned starts with the report id.
pub fn hid_read_feature(fd: libc::c_int, report_id: u8) -> io::Result<Vec<u8>> {
    // The single door to the hardware: the register map decides (#219). A
    // refused id never reaches the ioctl, whatever the caller.
    crate::registry::check_read(report_id)?;
    let mut attempts = 0;
    loop {
        let mut buf = [0u8; 256];
        buf[0] = report_id;
        // SAFETY: buf is 256 bytes, the size encoded in HIDIOCGFEATURE.
        let ret = unsafe { libc::ioctl(fd, HIDIOCGFEATURE, buf.as_mut_ptr()) };
        if ret >= 0 {
            // The kernel never returns more than the 256 bytes encoded above.
            let n = (ret as usize).min(buf.len());
            return Ok(buf[..n].to_vec());
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EINTR) && attempts < EINTR_RETRIES {
            attempts += 1;
            continue;
        }
        return Err(err);
    }
}

/// THE write to the hardware: one Feature report made of the id alone, only
/// if the register map lets the named operation `op` write it (class
/// `WriteApple`, id of `op`, exact length of `op`). This door is sized for one
/// byte: an operation that carries data (`DeviceName`, 64 bytes) can never
/// pass it, whatever the caller. Every byte handed to the kernel is logged
/// first. Never retried (not even on EINTR): a command must not be sent
/// twice. The "once per session" rule is in [`crate::registry::WriteSession`].
pub fn hid_write_feature(
    fd: libc::c_int,
    op: crate::registry::WriteOp,
    report: &[u8],
) -> io::Result<()> {
    let Some((&id, data)) = report.split_first() else {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "write refused: empty report",
        ));
    };
    crate::registry::check_write_op(op, id, crate::registry::Direction::Feature)?;
    if data.len() != op.payload_len() || !data.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "write refused: {} byte(s); this door sends one report id alone (operation {})",
                report.len(),
                op.as_str()
            ),
        ));
    }
    let mut buf = [id];
    eprintln!(
        "[hid-write] {} Feature report, {} byte(s) handed to the kernel: {} (Bluetooth wire: 53 {})",
        op.as_str(),
        buf.len(),
        buf.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "),
        buf.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "),
    );
    // SAFETY: buf is 1 byte, the size encoded in the request number.
    let ret = unsafe { libc::ioctl(fd, HIDIOCSFEATURE_1, buf.as_mut_ptr()) };
    if ret < 0 {
        let err = io::Error::last_os_error();
        eprintln!("[hid-write] failed: {err}");
        return Err(err);
    }
    note_write_done();
    Ok(())
}

fn note_write_done() {
    crate::read_policy::note_hw_access();
}

impl crate::parity::FeatureSink for Hidraw {
    fn set_feature(&self, op: crate::registry::WriteOp, report: &[u8]) -> io::Result<()> {
        hid_write_feature(self.0, op, report)
    }
}

/// One write session for the whole process: `WillShutdown` goes out once per run.
static WRITE_SESSION: Mutex<crate::registry::WriteSession> =
    Mutex::new(crate::registry::WriteSession::new());

/// How long the single reader lock may be awaited before the write is skipped.
const WRITE_LOCK_WAIT: Duration = Duration::from_millis(1500);

/// Tell the keyboard the host is shutting down (Feature `0x40`), once per run,
/// on the persistent hidraw node, under the single-reader lock, the breaker and
/// the 1 s spacing. `enabled` is `[apple] will_shutdown`, `connected` the
/// daemon's view of the link.
pub fn send_will_shutdown(enabled: bool, connected: bool) -> crate::parity::Outcome {
    use crate::parity::Outcome;
    if !enabled {
        return Outcome::Disabled;
    }
    if !connected {
        return Outcome::NotConnected;
    }
    let mut session = WRITE_SESSION.lock().unwrap_or_else(|e| e.into_inner());
    if session.is_used_by(crate::registry::WriteOp::Shutdown) {
        return Outcome::AlreadySent;
    }
    let Some(_lock) = crate::read_policy::try_lock(WRITE_LOCK_WAIT) else {
        return Outcome::Busy;
    };
    let Some((fd, _)) = get_hid_fd() else {
        return Outcome::NoNode;
    };
    crate::parity::will_shutdown(
        enabled,
        connected,
        &Hidraw(fd),
        &mut session,
        crate::read_policy::breaker(),
        crate::read_policy::last_hw_access(),
        &mut std::thread::sleep,
    )
}

/// The keyboard's hidraw node opened by a short-lived command (`akmctl`)
/// for ONE guarded write, under the cross-process lock shared with the
/// daemon (released on drop). Every write still goes through
/// [`hid_write_feature`] (register map, operation, length, one-byte door),
/// after the 1 s spacing that follows the last hardware access, and is
/// recorded by the circuit breaker. Opening it writes nothing.
pub struct WriteDoor {
    file: std::fs::File,
    path: String,
    _lock: crate::read_policy::ReadLock,
}

impl WriteDoor {
    /// Open the BCM2042 node, or explain why not (nothing is written).
    pub fn open() -> Result<Self, String> {
        let dev = find_apple_hidraw().ok_or("no Apple keyboard found (hidraw)")?;
        let uevent = std::fs::read_to_string(format!(
            "/sys/class/hidraw/{}/device/uevent",
            dev.trim_start_matches("/dev/")
        ))
        .unwrap_or_default();
        if crate::model::family_from_uevent(&uevent) != crate::model::Family::Bcm2042 {
            return Err(format!("{dev} is not a BCM2042 keyboard: nothing written"));
        }
        let lock = crate::read_policy::try_lock(WRITE_LOCK_WAIT)
            .ok_or("another reader holds the HID lock (the daemon is reading): nothing written, retry in a moment")?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&dev)
            .map_err(|e| format!("cannot open {dev}: {e}"))?;
        Ok(Self {
            file,
            path: dev,
            _lock: lock,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

impl crate::parity::FeatureSink for WriteDoor {
    fn set_feature(&self, op: crate::registry::WriteOp, report: &[u8]) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        let allowed = crate::read_policy::breaker()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .allow();
        if !allowed {
            return Err(io::Error::other("circuit breaker open: nothing written"));
        }
        let wait =
            crate::read_policy::wait_before(crate::read_policy::last_hw_access(), Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        let r = hid_write_feature(self.file.as_raw_fd(), op, report);
        crate::read_policy::breaker()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record(r.is_ok());
        r
    }
}

/// A borrowed hidraw file descriptor as a [`HidSource`].
pub struct Hidraw(pub libc::c_int);

impl HidSource for Hidraw {
    fn feature(&self, report_id: u8) -> io::Result<Vec<u8>> {
        hid_read_feature(self.0, report_id)
    }
}

/// First Apple keyboard hidraw (`/dev/hidrawN`) under a sysfs root.
pub fn find_apple_hidraw_in(sys: &Path) -> Option<String> {
    let mut entries: Vec<_> = std::fs::read_dir(sys.join("class/hidraw"))
        .ok()?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    entries.into_iter().find_map(|entry| {
        let uevent = std::fs::read_to_string(entry.path().join("device/uevent")).ok()?;
        apple_model_from_uevent(&uevent)?;
        Some(format!("/dev/{}", entry.file_name().to_string_lossy()))
    })
}

/// First Apple keyboard hidraw device on this machine.
pub fn find_apple_hidraw() -> Option<String> {
    find_apple_hidraw_in(Path::new("/sys"))
}

/// HID uevent of the device behind `/dev/hidrawN`.
fn hidraw_uevent(sys: &Path, dev_path: &str) -> String {
    std::fs::read_to_string(
        sys.join("class/hidraw")
            .join(dev_path.trim_start_matches("/dev/"))
            .join("device/uevent"),
    )
    .unwrap_or_default()
}

/// HID uevent of the keyboard with this MAC (`HID_UNIQ`), from `bus/hid/devices`.
pub fn hid_uevent_for_mac_in(sys: &Path, mac: &str) -> Option<String> {
    let mut devs: Vec<_> = std::fs::read_dir(sys.join("bus/hid/devices"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    devs.sort();
    devs.into_iter().find_map(|d| {
        let u = std::fs::read_to_string(d.join("uevent")).ok()?;
        let uniq = u.lines().find_map(|l| l.strip_prefix("HID_UNIQ="))?;
        (uniq.trim().eq_ignore_ascii_case(mac.trim()) && apple_model_from_uevent(&u).is_some())
            .then_some(u)
    })
}

/// MAC of the first Apple keyboard known to the kernel HID bus (connected).
pub fn find_apple_keyboard_mac_in(sys: &Path) -> Option<String> {
    let mut devs: Vec<_> = std::fs::read_dir(sys.join("bus/hid/devices"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    devs.sort();
    devs.into_iter().find_map(|d| {
        let u = std::fs::read_to_string(d.join("uevent")).ok()?;
        apple_model_from_uevent(&u)?;
        crate::model::mac_from_uevent(&u)
    })
}

/// Report built from sysfs only (identity + kernel battery), without opening
/// hidraw. `None` if the keyboard is unknown to the kernel.
pub fn report_from_sysfs_in(sys: &Path, mac: &str) -> Option<KbReport> {
    let uevent = hid_uevent_for_mac_in(sys, mac)?;
    let kernel = crate::power::kernel_battery_in(sys, mac);
    let mut r = report_from_uevent(&uevent, kernel);
    r.bluetooth.connected = true;
    Some(r)
}

/// [`report_from_sysfs_in`] on `/sys`.
pub fn report_from_sysfs(mac: &str) -> Option<KbReport> {
    report_from_sysfs_in(Path::new("/sys"), mac)
}

// ── Persistent fd ─────────────────────────────────────────────────────────

/// Persistent HID fd + path — opened once, reused while valid.
static HID_FD: Mutex<Option<(libc::c_int, String)>> = Mutex::new(None);

fn get_hid_fd() -> Option<(libc::c_int, String)> {
    let mut fd_lock = HID_FD.lock().ok()?;
    if let Some((fd, ref path)) = *fd_lock {
        // SAFETY: F_GETFD on an integer fd has no memory effect.
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0 {
            return Some((fd, path.clone()));
        }
        *fd_lock = None;
    }
    let path = find_apple_hidraw()?;
    let c_path = std::ffi::CString::new(path.as_str()).ok()?;
    // SAFETY: valid NUL-terminated path.
    let raw_fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if raw_fd < 0 {
        return None;
    }
    *fd_lock = Some((raw_fd, path.clone()));
    Some((raw_fd, path))
}

/// Close the persistent fd (keyboard gone, or another process takes over).
pub fn close_hid_fd() {
    if let Ok(mut fd_lock) = HID_FD.lock() {
        if let Some((fd, _)) = fd_lock.take() {
            // SAFETY: fd was opened by us and is dropped from the cache.
            unsafe { libc::close(fd) };
        }
    }
}

/// Is the persistent hidraw fd currently open in this process?
pub fn hid_fd_open() -> bool {
    HID_FD.lock().map(|g| g.is_some()).unwrap_or(false)
}

/// Read keyboard telemetry via HID Feature Reports (persistent fd).
/// `None` if no Apple hidraw node can be opened or the keyboard does not answer.
pub fn read_keyboard() -> Option<KbReport> {
    let wake = ensure_wake_monitor();
    let (fd, path) = get_hid_fd()?;
    let sys = Path::new("/sys");
    let uevent = hidraw_uevent(sys, &path);
    let kernel =
        crate::model::mac_from_uevent(&uevent).and_then(|m| crate::power::kernel_battery(&m));
    let wake = KbWake {
        last_age_s: wake.last().map(|t| t.elapsed().as_secs_f64()),
        count: wake.count(),
    };
    // Safe read policy (#177): allow-list 0x47/0x46/0x49, only while the
    // keyboard is in use, one reader, stop at the first failure.
    let (r, outcome) =
        crate::read_policy::build_report_safe(&uevent, kernel, &Hidraw(fd), wake, Instant::now());
    if outcome == crate::read_policy::SafeRead::Partial {
        // Keyboard not responding: reopen next time.
        close_hid_fd();
    }
    Some(r)
}

// ── Wake event monitor (Input Report 0x13) ──────────────────────────────

/// Shared wake state: instant of the last event and a running counter.
#[derive(Default)]
pub struct WakeState {
    last: Mutex<Option<Instant>>,
    count: AtomicU64,
}

impl WakeState {
    pub fn record(&self) {
        if let Ok(mut l) = self.last.lock() {
            *l = Some(Instant::now());
        }
        self.count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn last(&self) -> Option<Instant> {
        self.last.lock().ok().and_then(|l| *l)
    }
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }
}

static WAKE: OnceLock<Arc<WakeState>> = OnceLock::new();
/// Cleared to make the monitor release the hidraw node (another process owns it).
static WAKE_ENABLED: AtomicBool = AtomicBool::new(true);

/// Enable / disable the wake monitor. Disabled, it closes its hidraw fd within
/// ~2 s and stops looking for the node.
pub fn set_wake_monitor_enabled(on: bool) {
    WAKE_ENABLED.store(on, Ordering::Relaxed);
}

/// Start the process-wide wake monitor (idempotent) and return its state.
pub fn ensure_wake_monitor() -> Arc<WakeState> {
    set_wake_monitor_enabled(true);
    WAKE.get_or_init(|| {
        let st = Arc::new(WakeState::default());
        let lw = st.clone();
        let spawned = std::thread::Builder::new()
            .name("kb-wake-monitor".into())
            .spawn(move || loop {
                if WAKE_ENABLED.load(Ordering::Relaxed) {
                    if let Some(path) = find_apple_hidraw() {
                        wake_loop(&path, &lw);
                    }
                }
                std::thread::sleep(Duration::from_secs(5));
            });
        if let Err(e) = spawned {
            eprintln!("[keyboard] cannot spawn wake monitor: {}", e);
        }
        st
    })
    .clone()
}

/// Instant of the last wake event, if one was seen.
pub fn last_wake() -> Option<Instant> {
    WAKE.get().and_then(|w| w.last())
}

/// Number of wake events seen since the monitor started.
pub fn wake_count() -> u64 {
    WAKE.get().map_or(0, |w| w.count())
}

/// True if an input report is the vendor wake/connection event (report 0x13).
pub fn is_wake_report(report: &[u8]) -> bool {
    report.first() == Some(&0x13)
}

/// Read input reports until the device disappears, errors or the monitor is disabled.
fn wake_loop(path: &str, lw: &WakeState) {
    let Ok(c_path) = std::ffi::CString::new(path) else {
        return;
    };
    // SAFETY: valid NUL-terminated path.
    let fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return;
    }
    let mut buf = [0u8; 64];
    while WAKE_ENABLED.load(Ordering::Relaxed) {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let ret = unsafe { libc::poll(&mut pfd, 1, 2000) };
        if ret < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        if ret == 0 {
            continue;
        }
        // POLLHUP/POLLERR/POLLNVAL: node gone (else poll spins at 100 % CPU).
        if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            break;
        }
        // SAFETY: buf is valid for buf.len() bytes.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error().raw_os_error();
            if e == Some(libc::EAGAIN) || e == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        if n == 0 {
            break;
        }
        // Timestamp only (never the content): gates the safe reads (#177).
        crate::read_policy::note_input();
        if is_wake_report(&buf[..n as usize]) {
            lw.record();
        }
    }
    // SAFETY: fd opened above.
    unsafe { libc::close(fd) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_state_records_events() {
        let w = WakeState::default();
        assert!(w.last().is_none());
        assert_eq!(w.count(), 0);
        w.record();
        w.record();
        assert_eq!(w.count(), 2);
        assert!(w.last().unwrap().elapsed().as_secs() < 5);
    }

    #[test]
    fn only_the_id_only_operations_reach_the_ioctl() {
        // On an invalid fd an allowed report fails in the ioctl (EBADF,
        // nothing was sent anywhere); every other (operation, id) is refused
        // before it. The 256 ids, every named operation.
        use crate::registry::WriteOp;
        for op in WriteOp::ALL {
            for id in 0..=255u8 {
                let e = hid_write_feature(-1, op, &[id]).unwrap_err();
                if op.ids().contains(&id) && op.payload_len() == 0 {
                    assert_eq!(
                        e.raw_os_error(),
                        Some(libc::EBADF),
                        "{} {id:#04x}",
                        op.as_str()
                    );
                } else {
                    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{id:#04x}");
                    assert_eq!(e.raw_os_error(), None, "{id:#04x} must not reach the ioctl");
                }
            }
        }
        // Never a report with data, nor an empty one; the 65-byte name frame
        // of DeviceName can never pass this one-byte door.
        let mut name = vec![0x55u8];
        name.extend_from_slice(&[b'A'; 64]);
        for (op, r) in [
            (WriteOp::Shutdown, &[][..]),
            (WriteOp::Shutdown, &[0x40, 0x03][..]),
            (WriteOp::Shutdown, &[0x40, 0, 0][..]),
            (WriteOp::DeviceName, &name[..]),
            (WriteOp::DeviceName, &[0x55][..]),
        ] {
            let e = hid_write_feature(-1, op, r).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(e.raw_os_error(), None);
        }
    }

    #[test]
    fn disabled_or_disconnected_never_touches_the_node() {
        assert_eq!(send_will_shutdown(false, true), crate::parity::Outcome::Disabled);
        assert_eq!(send_will_shutdown(true, false), crate::parity::Outcome::NotConnected);
    }

    #[test]
    fn wake_report_detection() {
        assert!(is_wake_report(&[0x13, 1, 2]));
        assert!(!is_wake_report(&[0x12]));
        assert!(!is_wake_report(&[]));
    }

    #[test]
    fn hid_read_feature_on_invalid_fd_reports_its_own_errno() {
        // Poison errno (ENOENT) first: the error must come from the ioctl.
        // SAFETY: valid NUL-terminated path; the open fails, nothing to close.
        let fd = unsafe { libc::open(c"/nonexistent-akm-test".as_ptr(), libc::O_RDONLY) };
        assert!(fd < 0);
        let e = hid_read_feature(-1, crate::decode::HID_BATTERY_STRENGTH).unwrap_err();
        assert_eq!(e.raw_os_error(), Some(libc::EBADF));
        assert_eq!(
            Hidraw(-1).feature(0x47).unwrap_err().raw_os_error(),
            Some(libc::EBADF)
        );
    }

    #[test]
    fn only_the_register_map_decides_what_reaches_the_ioctl() {
        // On an invalid fd an allowed id fails in the ioctl (EBADF); every
        // other id is refused before it (PermissionDenied): none of the 256
        // ids reaches the hardware unless its class allows it (#219).
        for id in 0..=255u8 {
            let e = hid_read_feature(-1, id).unwrap_err();
            let allowed = crate::registry::classify_feature(id).daemon_may_read();
            if allowed {
                assert_eq!(e.raw_os_error(), Some(libc::EBADF), "{id:#04x}");
            } else {
                assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{id:#04x}");
                assert_eq!(e.raw_os_error(), None, "{id:#04x} must not reach the ioctl");
            }
            assert_eq!(Hidraw(-1).feature(id).unwrap_err().kind() == io::ErrorKind::PermissionDenied, !allowed);
        }
    }
}
