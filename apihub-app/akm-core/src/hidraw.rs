//! The real HID source: `/dev/hidrawN` of the Apple keyboard.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::decode::{report_from_uevent, HidSource};
use crate::model::apple_model_from_uevent;
use crate::report::{KbReport, KbWake};

const HIDIOCGFEATURE: libc::c_ulong = 0xC100_4807;

const HIDIOCGINPUT: libc::c_ulong = 0xC100_480A;

const HIDIOCSFEATURE_1: libc::c_ulong = 0xC001_4806;

const HIDIOCSFEATURE_65: libc::c_ulong = 0xC041_4806;

const LONG_NAME_DATA: usize = crate::devname::LONG_DEVICE_NAME_LEN;

const EINTR_RETRIES: u32 = 3;

/// Read one Feature Report on a raw fd (`GET_REPORT`: read-only on the device).
///
/// # Errors
///
/// The refusal of the register map, else the `ioctl` error.
pub fn hid_read_feature(fd: libc::c_int, report_id: u8) -> io::Result<Vec<u8>> {
    // The single door to the hardware: the register map decides.
    crate::registry::check_read(report_id)?;
    hid_get_report(fd, HIDIOCGFEATURE, report_id)
}

/// Read one Input Report on a raw fd (`GET_REPORT` type Input: read-only on the device).
///
/// # Errors
///
/// The refusal of the register map, else the `ioctl` error.
pub fn hid_read_input(fd: libc::c_int, report_id: u8) -> io::Result<Vec<u8>> {
    crate::registry::check_read_input(report_id)?;
    hid_get_report(fd, HIDIOCGINPUT, report_id)
}

fn hid_get_report(fd: libc::c_int, request: libc::c_ulong, report_id: u8) -> io::Result<Vec<u8>> {
    let mut attempts = 0;
    loop {
        let mut buf = [0u8; 256];
        buf[0] = report_id;
        // SAFETY: buf is 256 bytes, the size encoded in both request numbers.
        let ret = unsafe { libc::ioctl(fd, request, buf.as_mut_ptr()) };
        if let Ok(n) = usize::try_from(ret) {
            // The kernel never returns more than the 256 bytes encoded above.
            return Ok(buf[..n.min(buf.len())].to_vec());
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EINTR) && attempts < EINTR_RETRIES {
            attempts += 1;
            continue;
        }
        return Err(err);
    }
}

/// THE write to the hardware.
///
/// # Errors
///
/// `PermissionDenied` when the registry refuses the write, else the `ioctl` error.
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
    if data.len() != op.payload_len() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "write refused: {} data byte(s), operation {} sends exactly {}",
                data.len(),
                op.as_str(),
                op.payload_len()
            ),
        ));
    }
    let ret = match (op.payload_len(), op) {
        (0, _) => {
            let mut buf = [id];
            log_write(op, &buf);
            // SAFETY: buf is 1 byte, the size encoded in the request number.
            unsafe { libc::ioctl(fd, HIDIOCSFEATURE_1, buf.as_mut_ptr()) }
        }
        (LONG_NAME_DATA, crate::registry::WriteOp::DeviceName) => {
            let mut buf = [0u8; 1 + LONG_NAME_DATA];
            buf[0] = id;
            buf[1..].copy_from_slice(data);
            log_write(op, &buf);
            // SAFETY: buf is 65 bytes, the size encoded in the request number.
            unsafe { libc::ioctl(fd, HIDIOCSFEATURE_65, buf.as_mut_ptr()) }
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "write refused: no door for operation {} with {} data byte(s)",
                    op.as_str(),
                    data.len()
                ),
            ));
        }
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        diag(true, &format!("[hid-write] failed: {err}"));
        return Err(err);
    }
    note_write_done();
    Ok(())
}

fn log_write(op: crate::registry::WriteOp, buf: &[u8]) {
    let hex = buf
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    diag(
        false,
        &format!(
            "[hid-write] {} Feature report, {} byte(s) handed to the kernel: {hex} (Bluetooth wire: 53 {hex})",
            op.as_str(),
            buf.len(),
        ),
    );
}

/// Trace line: to `tracing` when the process installed a subscriber (the daemon's journal, with
/// its level), else to stderr (the CLIs' audit trace).
pub(crate) fn diag(warn: bool, msg: &str) {
    if tracing::dispatcher::has_been_set() {
        if warn {
            tracing::warn!("{msg}");
        } else {
            tracing::info!("{msg}");
        }
    } else {
        eprintln!("{msg}");
    }
}

fn note_write_done() {
    crate::read_policy::note_hw_access();
}

impl crate::parity::FeatureSink for Hidraw {
    fn set_feature(&self, op: crate::registry::WriteOp, report: &[u8]) -> io::Result<()> {
        hid_write_feature(self.0, op, report)
    }
}

static WRITE_SESSION: Mutex<crate::registry::WriteSession> =
    Mutex::new(crate::registry::WriteSession::new());

const WRITE_LOCK_WAIT: Duration = Duration::from_millis(1500);

/// Tell the keyboard the host is shutting down (Feature `0x40`), once per run, on the persistent
/// hidraw node, under the single-reader lock, the breaker and the 1 s spacing.
pub fn send_will_shutdown(enabled: bool, connected: bool) -> crate::parity::Outcome {
    use crate::parity::Outcome;
    if !enabled {
        return Outcome::Disabled;
    }
    if !connected {
        return Outcome::NotConnected;
    }
    let mut session = WRITE_SESSION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        &Hidraw(fd.as_raw_fd()),
        &mut session,
        crate::read_policy::breaker(),
        crate::read_policy::last_hw_access(),
        &mut std::thread::sleep,
    )
}

/// Hidraw node opened by a short-lived command for ONE guarded write (plus the name read-back),
/// under the cross-process lock shared with the daemon.
pub struct WriteDoor {
    file: std::fs::File,
    path: String,
    /// `HID_UNIQ` of the node (the keyboard the daemon's state must concern).
    mac: String,
    _lock: crate::read_policy::ReadLock,
}

fn daemon_breaker_verdict(mac: &str) -> crate::breaker_state::Verdict {
    let found = crate::breaker_state::read_own(&crate::read_policy::breaker_state_path());
    // SAFETY: getuid(2) has no preconditions.
    let uid = unsafe { libc::getuid() };
    let alive = |w: Option<crate::breaker_state::Writer>| {
        crate::breaker_state::daemon_alive_in(Path::new("/proc"), w, uid)
    };
    crate::breaker_state::verdict(&found, mac, crate::breaker_state::now_unix(), &alive)
}

impl WriteDoor {
    /// Open the BCM2042 node of the keyboard with this MAC, never another one.
    ///
    /// # Errors
    ///
    /// A message saying why no node of that keyboard can be opened (nothing is written).
    pub fn open_for(mac: &str) -> Result<Self, String> {
        let dev = find_apple_hidraw_for_mac_in(Path::new("/sys"), mac)
            .ok_or_else(|| format!("no hidraw node of keyboard {mac}: nothing written"))?;
        let d = Self::open_node(dev)?;
        crate::devname::check_door_mac(&d.mac(), mac)?;
        Ok(d)
    }

    fn open_node(dev: String) -> Result<Self, String> {
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
            mac: crate::model::mac_from_uevent(&uevent).unwrap_or_default(),
            _lock: lock,
        })
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// `HID_UNIQ` of the opened node (upper-case).
    #[must_use]
    pub fn mac(&self) -> String {
        self.mac.to_ascii_uppercase()
    }

    /// Read `0x51`-`0x54` now, in this connection: the 32 data bytes of the name.
    ///
    /// # Errors
    ///
    /// The I/O error of a read, or `InvalidData` for a malformed report.
    pub fn read_name(&self) -> io::Result<Vec<u8>> {
        use std::os::fd::AsRawFd;
        let wait =
            crate::read_policy::wait_before(crate::read_policy::last_hw_access(), Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        let raw = Hidraw(self.file.as_raw_fd());
        crate::devname::read_name_from(&crate::read_policy::SafeSource::new(&raw))
    }
}

impl crate::devname::NameDoor for WriteDoor {
    fn read_name(&self) -> io::Result<Vec<u8>> {
        WriteDoor::read_name(self)
    }
    fn as_sink(&self) -> &dyn crate::parity::FeatureSink {
        self
    }
    fn mac(&self) -> String {
        WriteDoor::mac(self)
    }
}

impl crate::parity::FeatureSink for WriteDoor {
    fn set_feature(&self, op: crate::registry::WriteOp, report: &[u8]) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        let allowed = crate::read_policy::breaker()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .allow();
        if !allowed {
            return Err(io::Error::other("circuit breaker open: nothing written"));
        }
        // Apple's R3 belongs to the daemon's breaker, not to this short-lived process (whose own
        // breaker is always fresh): its published state decides.
        match daemon_breaker_verdict(&self.mac) {
            crate::breaker_state::Verdict::Allow(a) => {
                diag(
                    false,
                    &format!("[hid-write] daemon breaker state {a:?}: write allowed"),
                );
            }
            crate::breaker_state::Verdict::Refuse(r) => {
                diag(
                    true,
                    &format!("[hid-write] refused by the daemon's breaker: {r}"),
                );
                return Err(io::Error::other(format!("daemon's circuit breaker: {r}")));
            }
        }
        let wait =
            crate::read_policy::wait_before(crate::read_policy::last_hw_access(), Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        let r = hid_write_feature(self.file.as_raw_fd(), op, report);
        crate::read_policy::breaker()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

    fn input(&self, report_id: u8) -> io::Result<Vec<u8>> {
        hid_read_input(self.0, report_id)
    }
}

/// First Apple keyboard hidraw (`/dev/hidrawN`) under a sysfs root.
#[must_use]
pub fn find_apple_hidraw_in(sys: &Path) -> Option<String> {
    let mut entries: Vec<_> = std::fs::read_dir(sys.join("class/hidraw"))
        .ok()?
        .flatten()
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    entries.into_iter().find_map(|entry| {
        let uevent = std::fs::read_to_string(entry.path().join("device/uevent")).ok()?;
        apple_model_from_uevent(&uevent)?;
        Some(format!("/dev/{}", entry.file_name().to_string_lossy()))
    })
}

/// Hidraw node (`/dev/hidrawN`) of the Apple keyboard whose `HID_UNIQ` is `mac`, under a sysfs root.
#[must_use]
pub fn find_apple_hidraw_for_mac_in(sys: &Path, mac: &str) -> Option<String> {
    let mut entries: Vec<_> = std::fs::read_dir(sys.join("class/hidraw"))
        .ok()?
        .flatten()
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    entries.into_iter().find_map(|entry| {
        let uevent = std::fs::read_to_string(entry.path().join("device/uevent")).ok()?;
        apple_model_from_uevent(&uevent)?;
        crate::model::mac_from_uevent(&uevent)?
            .eq_ignore_ascii_case(mac.trim())
            .then(|| format!("/dev/{}", entry.file_name().to_string_lossy()))
    })
}

/// First Apple keyboard hidraw device on this machine.
#[must_use]
pub fn find_apple_hidraw() -> Option<String> {
    find_apple_hidraw_in(Path::new("/sys"))
}

fn hidraw_uevent(sys: &Path, dev_path: &str) -> String {
    std::fs::read_to_string(
        sys.join("class/hidraw")
            .join(dev_path.trim_start_matches("/dev/"))
            .join("device/uevent"),
    )
    .unwrap_or_default()
}

/// HID uevent of the keyboard with this MAC (`HID_UNIQ`), from `bus/hid/devices`.
#[must_use]
pub fn hid_uevent_for_mac_in(sys: &Path, mac: &str) -> Option<String> {
    hid_device_for_mac_in(sys, mac).map(|(_, u)| u)
}

fn hid_device_for_mac_in(sys: &Path, mac: &str) -> Option<(std::path::PathBuf, String)> {
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
            .then_some((d, u))
    })
}

/// Index of the Bluetooth controller (`hciN`) the keyboard with this MAC is connected through.
#[must_use]
pub fn hci_index_for_mac_in(sys: &Path, mac: &str) -> Option<u16> {
    let (dev, _) = hid_device_for_mac_in(sys, mac)?;
    let real = std::fs::canonicalize(dev).ok()?;
    let mut parts = real.iter().map(|c| c.to_str().unwrap_or(""));
    parts.find(|c| *c == "bluetooth")?;
    parts.next()?.strip_prefix("hci")?.parse().ok()
}

/// MAC of the first Apple keyboard known to the kernel HID bus (connected).
#[must_use]
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

/// Report built from sysfs only (identity + kernel battery), without opening hidraw.
#[must_use]
pub fn report_from_sysfs_in(sys: &Path, mac: &str) -> Option<KbReport> {
    let uevent = hid_uevent_for_mac_in(sys, mac)?;
    let kernel = crate::power::kernel_battery_in(sys, mac);
    let mut r = report_from_uevent(&uevent, kernel);
    r.bluetooth.connected = true;
    Some(r)
}

#[must_use]
pub fn report_from_sysfs(mac: &str) -> Option<KbReport> {
    report_from_sysfs_in(Path::new("/sys"), mac)
}

type FdCache = Mutex<Option<(Arc<OwnedFd>, String)>>;

/// The persistent node: users hold an `Arc`, so closing the cache never frees a number in use.
static HID_FD: FdCache = Mutex::new(None);

fn get_hid_fd() -> Option<(Arc<OwnedFd>, String)> {
    cached_fd(&HID_FD, || {
        let path = find_apple_hidraw()?;
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .ok()?;
        Some((f.into(), path))
    })
}

fn cached_fd(
    cache: &FdCache,
    open: impl FnOnce() -> Option<(OwnedFd, String)>,
) -> Option<(Arc<OwnedFd>, String)> {
    let mut g = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if g.is_none() {
        let (fd, path) = open()?;
        *g = Some((Arc::new(fd), path));
    }
    g.as_ref().map(|(fd, path)| (fd.clone(), path.clone()))
}

/// Close the persistent fd (keyboard gone, or another process takes over).
pub fn close_hid_fd() {
    HID_FD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
}

/// Read keyboard telemetry via HID Feature Reports (persistent fd); `wake` comes from the
/// passive listener, the only reader of the input stream.
#[must_use]
pub fn read_keyboard(wake: KbWake) -> Option<KbReport> {
    let (fd, path) = get_hid_fd()?;
    let sys = Path::new("/sys");
    let uevent = hidraw_uevent(sys, &path);
    let kernel =
        crate::model::mac_from_uevent(&uevent).and_then(|m| crate::power::kernel_battery(&m));
    // Safe read policy: allow-list 0x47/0x46/0x49, only while the keyboard is in use, one
    // reader, stop at the first failure.
    let (r, outcome) = crate::read_policy::build_report_safe(
        &uevent,
        kernel,
        &Hidraw(fd.as_raw_fd()),
        wake,
        Instant::now(),
    );
    if outcome == crate::read_policy::SafeRead::Partial {
        // Keyboard not responding: reopen next time.
        close_hid_fd();
    }
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traces_go_to_the_subscriber_when_there_is_one() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        struct Count(Arc<AtomicUsize>);
        impl tracing::Subscriber for Count {
            fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
            fn event(&self, e: &tracing::Event<'_>) {
                if *e.metadata().level() == tracing::Level::WARN {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
            }
            fn enter(&self, _: &tracing::span::Id) {}
            fn exit(&self, _: &tracing::span::Id) {}
        }
        let n = Arc::new(AtomicUsize::new(0));
        tracing::subscriber::with_default(Count(n.clone()), || {
            diag(true, "[hid-write] failed: test");
        });
        assert_eq!(n.load(Ordering::SeqCst), 1, "a warning, with its level");
    }

    #[test]
    fn closing_the_cache_keeps_a_borrowed_fd_open() {
        let cache: FdCache = Mutex::new(None);
        let open = || {
            let f = std::fs::File::open("/dev/null").ok()?;
            Some((OwnedFd::from(f), "/dev/null".to_string()))
        };
        let (held, _) = cached_fd(&cache, open).unwrap();
        let (again, _) = cached_fd(&cache, || None).unwrap();
        assert_eq!(
            held.as_raw_fd(),
            again.as_raw_fd(),
            "one open per cache fill"
        );
        cache.lock().unwrap().take();
        drop(again);
        // SAFETY: F_GETFD has no memory effect.
        assert!(unsafe { libc::fcntl(held.as_raw_fd(), libc::F_GETFD) } >= 0);
        assert!(
            cached_fd(&cache, || None).is_none(),
            "emptied cache reopens"
        );
    }

    #[test]
    fn only_the_id_only_operations_reach_the_ioctl() {
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
        let mut name = vec![0x55u8];
        name.extend_from_slice(&[b'A'; 64]);
        for (op, r) in [
            (WriteOp::Shutdown, &[][..]),
            (WriteOp::Shutdown, &[0x40, 0x03][..]),
            (WriteOp::Shutdown, &[0x40, 0, 0][..]),
            (WriteOp::Shutdown, &name[..]),
            (WriteOp::Forget, &name[..]),
            (WriteOp::DeviceName, &[0x55][..]),
            (WriteOp::DeviceName, &name[..64]),
            (WriteOp::DeviceName, &[&name[..], &[0][..]].concat()),
        ] {
            let e = hid_write_feature(-1, op, r).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{op:?} {r:?}");
            assert_eq!(e.raw_os_error(), None);
        }
    }

    #[test]
    fn the_65_byte_door_takes_device_name_0x55_and_nothing_else() {
        use crate::registry::WriteOp;
        let iowr = |n: libc::c_ulong| (3 << 30) | (n << 16) | (0x48 << 8) | 6;
        assert_eq!(HIDIOCSFEATURE_1, iowr(1));
        assert_eq!(HIDIOCSFEATURE_65, iowr(65));
        assert_eq!(1 + LONG_NAME_DATA, 65);
        for op in WriteOp::ALL {
            for id in 0..=255u8 {
                let mut r = vec![id];
                r.extend_from_slice(&[0x41; 64]);
                let e = hid_write_feature(-1, op, &r).unwrap_err();
                if op == WriteOp::DeviceName && id == 0x55 {
                    assert_eq!(e.raw_os_error(), Some(libc::EBADF), "{id:#04x}");
                } else {
                    assert_eq!(
                        e.kind(),
                        io::ErrorKind::PermissionDenied,
                        "{op:?} {id:#04x}"
                    );
                    assert_eq!(
                        e.raw_os_error(),
                        None,
                        "{op:?} {id:#04x} must not reach the ioctl"
                    );
                }
            }
        }
        for n in (0..=70usize).chain([255]) {
            if n == 64 {
                continue;
            }
            let mut r = vec![0x55u8];
            r.extend_from_slice(&vec![0u8; n]);
            let e = hid_write_feature(-1, WriteOp::DeviceName, &r).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{n} data bytes");
            assert_eq!(e.raw_os_error(), None, "{n} data bytes");
        }
        let f = &crate::devname::frames_for("alex").unwrap()[0];
        assert_eq!(f.report.len(), 65);
        assert_eq!(
            hid_write_feature(-1, f.op, &f.report)
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EBADF)
        );
    }

    #[test]
    fn disabled_or_disconnected_never_touches_the_node() {
        assert_eq!(
            send_will_shutdown(false, true),
            crate::parity::Outcome::Disabled
        );
        assert_eq!(
            send_will_shutdown(true, false),
            crate::parity::Outcome::NotConnected
        );
    }

    #[test]
    fn wake_report_detection() {}

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
    fn only_input_0x30_reaches_the_input_ioctl() {
        let iowr = |nr: libc::c_ulong, n: libc::c_ulong| (3 << 30) | (n << 16) | (0x48 << 8) | nr;
        assert_eq!(HIDIOCGINPUT, iowr(0x0A, 256));
        assert_eq!(HIDIOCGFEATURE, iowr(0x07, 256));
        let mut reached = Vec::new();
        for id in 0..=255u8 {
            let e = hid_read_input(-1, id).unwrap_err();
            let e2 = Hidraw(-1).input(id).unwrap_err();
            if e.raw_os_error() == Some(libc::EBADF) {
                reached.push(id);
                assert_eq!(e2.raw_os_error(), Some(libc::EBADF));
            } else {
                assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{id:#04x}");
                assert_eq!(e.raw_os_error(), None, "{id:#04x} must not reach the ioctl");
                assert_eq!(e2.kind(), io::ErrorKind::PermissionDenied, "{id:#04x}");
            }
        }
        assert_eq!(reached, vec![0x30]);
        let e = crate::decode::Fixture::new().input(0x30).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn only_the_register_map_decides_what_reaches_the_ioctl() {
        for id in 0..=255u8 {
            let e = hid_read_feature(-1, id).unwrap_err();
            let allowed = crate::registry::classify_feature(id).daemon_may_read();
            if allowed {
                assert_eq!(e.raw_os_error(), Some(libc::EBADF), "{id:#04x}");
            } else {
                assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{id:#04x}");
                assert_eq!(e.raw_os_error(), None, "{id:#04x} must not reach the ioctl");
            }
            assert_eq!(
                Hidraw(-1).feature(id).unwrap_err().kind() == io::ErrorKind::PermissionDenied,
                !allowed
            );
        }
    }
}
