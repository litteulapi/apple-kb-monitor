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

use crate::decode::{build_report, report_from_uevent, HidSource};
use crate::model::apple_model_from_uevent;
use crate::report::{KbReport, KbWake};

/// HIDIOCGFEATURE = _IOWR('H', 0x07, 256) — read HID Feature Report
const HIDIOCGFEATURE: libc::c_ulong = 0xC1004807;

/// Read one Feature Report on a raw fd (GET_REPORT: read-only on the device).
pub fn hid_read_feature(fd: libc::c_int, report_id: u8) -> Option<Vec<u8>> {
    let mut buf = [0u8; 256];
    buf[0] = report_id;
    // SAFETY: buf is 256 bytes, the size encoded in HIDIOCGFEATURE.
    let ret = unsafe { libc::ioctl(fd, HIDIOCGFEATURE, buf.as_mut_ptr()) };
    (ret > 0).then(|| buf[..ret as usize].to_vec())
}

/// A borrowed hidraw file descriptor as a [`HidSource`].
pub struct Hidraw(pub libc::c_int);

impl HidSource for Hidraw {
    fn feature(&self, report_id: u8) -> io::Result<Vec<u8>> {
        hid_read_feature(self.0, report_id).ok_or_else(io::Error::last_os_error)
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
    let r = build_report(&uevent, kernel, &Hidraw(fd), wake);
    if r.is_none() {
        // Keyboard not responding: reopen next time.
        close_hid_fd();
    }
    r
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
    fn wake_report_detection() {
        assert!(is_wake_report(&[0x13, 1, 2]));
        assert!(!is_wake_report(&[0x12]));
        assert!(!is_wake_report(&[]));
    }

    #[test]
    fn hid_read_feature_on_invalid_fd_is_none() {
        assert!(hid_read_feature(-1, crate::decode::HID_BATTERY_PRECISE).is_none());
        assert!(Hidraw(-1).feature(0xEA).is_err());
    }
}
