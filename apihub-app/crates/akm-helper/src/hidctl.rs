//! `HID_CONTROL` SUSPEND (`0x13`) / `EXIT_SUSPEND` (`0x14`) on the HIDP control channel that
//! `bluetoothd` already owns (`docs/HID-SLEEP.md`).

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::breaker_state;
use crate::model;

pub const DRAIN_WAIT: Duration = Duration::from_secs(1);

pub const PSM_HID_CONTROL: u16 = 0x0011;

pub const AF_BLUETOOTH: i32 = 31;
pub const BTPROTO_L2CAP: i32 = 0;
const SOL_L2CAP: i32 = 6;
const L2CAP_OPTIONS: i32 = 0x01;
const L2CAP_CONNINFO: i32 = 0x02;

pub const BLUETOOTHD_EXES: &[&str] = &[
    "/usr/lib/bluetooth/bluetoothd",
    "/usr/libexec/bluetooth/bluetoothd",
    "/usr/sbin/bluetoothd",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HidControl {
    Suspend,
    ExitSuspend,
}

impl HidControl {
    #[must_use]
    pub const fn byte(self) -> u8 {
        match self {
            HidControl::Suspend => 0x13,
            HidControl::ExitSuspend => 0x14,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            HidControl::Suspend => "SUSPEND",
            HidControl::ExitSuspend => "EXIT_SUSPEND",
        }
    }

    #[must_use]
    pub const fn from_byte(b: u8) -> Option<HidControl> {
        match b {
            0x13 => Some(HidControl::Suspend),
            0x14 => Some(HidControl::ExitSuspend),
            _ => None,
        }
    }
}

/// Accept only the HID control bytes `SUSPEND` (0x13) and `EXIT_SUSPEND` (0x14).
///
/// # Errors
///
/// A message for any other byte.
pub fn check_byte(b: u8) -> Result<u8, String> {
    HidControl::from_byte(b).map(|_| b).ok_or_else(|| {
        format!("byte 0x{b:02x} refused: only 0x13 (SUSPEND) and 0x14 (EXIT_SUSPEND) may be sent")
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    /// Parse `AA:BB:CC:DD:EE:FF`.
    ///
    /// # Errors
    ///
    /// A message when `s` is not a MAC address.
    pub fn parse(s: &str) -> Result<Mac, String> {
        let b = s.as_bytes();
        let bad = || format!("invalid MAC {s:?} (expected XX:XX:XX:XX:XX:XX)");
        if b.len() != 17 {
            return Err(bad());
        }
        let mut out = [0u8; 6];
        for (i, o) in out.iter_mut().enumerate() {
            let p = &b[i * 3..i * 3 + 2];
            if i < 5 && b[i * 3 + 2] != b':' {
                return Err(bad());
            }
            if !p.iter().all(u8::is_ascii_hexdigit) {
                return Err(bad());
            }
            *o = u8::from_str_radix(std::str::from_utf8(p).map_err(|_| bad())?, 16)
                .map_err(|_| bad())?;
        }
        Ok(Mac(out))
    }

    #[must_use]
    pub fn from_bdaddr_le(b: [u8; 6]) -> Mac {
        Mac([b[5], b[4], b[3], b[2], b[1], b[0]])
    }
}

impl std::fmt::Display for Mac {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let b = self.0;
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            b[0], b[1], b[2], b[3], b[4], b[5]
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SockInfo {
    pub domain: i32,
    pub protocol: i32,
    pub sotype: i32,
    pub peer: Option<Mac>,
    pub local_psm: Option<u16>,
    pub peer_psm: Option<u16>,
    pub peer_cid: Option<u16>,
    pub hci_handle: Option<u16>,
    pub omtu: Option<u16>,
    pub imtu: Option<u16>,
}

impl SockInfo {
    #[must_use]
    pub fn is_l2cap_seqpacket(&self) -> bool {
        self.domain == AF_BLUETOOTH
            && self.protocol == BTPROTO_L2CAP
            && self.sotype == libc::SOCK_SEQPACKET
    }
}

pub trait ControlSocket {
    /// Describe the socket behind `fd`.
    ///
    /// # Errors
    ///
    /// The I/O error of `getsockopt(2)`/`getsockname(2)`.
    fn inspect(&self, fd: BorrowedFd<'_>) -> io::Result<SockInfo>;
    fn drained(&self, fd: BorrowedFd<'_>) -> Option<bool>;
}

fn getsockopt_int(fd: RawFd, level: i32, name: i32) -> io::Result<i32> {
    let mut v: libc::c_int = 0;
    let mut len =
        libc::socklen_t::try_from(std::mem::size_of::<libc::c_int>()).map_err(io::Error::other)?;
    // SAFETY: valid out-pointer and length for an int option.
    let rc = unsafe { libc::getsockopt(fd, level, name, (&raw mut v).cast(), &raw mut len) };
    if rc == 0 {
        Ok(v)
    } else {
        Err(io::Error::last_os_error())
    }
}

fn sockaddr_l2(fd: RawFd, peer: bool) -> Option<(u16, Mac, u16)> {
    let mut buf = [0u8; 32];
    let mut len = libc::socklen_t::try_from(buf.len()).ok()?;
    // SAFETY: the buffer is larger than sockaddr_l2 (14 bytes); len in/out.
    let rc = unsafe {
        if peer {
            libc::getpeername(fd, buf.as_mut_ptr().cast(), &raw mut len)
        } else {
            libc::getsockname(fd, buf.as_mut_ptr().cast(), &raw mut len)
        }
    };
    if rc != 0 || len < 12 || i32::from(u16::from_ne_bytes([buf[0], buf[1]])) != AF_BLUETOOTH {
        return None;
    }
    let psm = u16::from_le_bytes([buf[2], buf[3]]);
    let mut a = [0u8; 6];
    a.copy_from_slice(&buf[4..10]);
    let cid = u16::from_le_bytes([buf[10], buf[11]]);
    Some((psm, Mac::from_bdaddr_le(a), cid))
}

pub struct L2capControlSocket;

impl ControlSocket for L2capControlSocket {
    fn inspect(&self, fd: BorrowedFd<'_>) -> io::Result<SockInfo> {
        let raw = fd.as_raw_fd();
        let mut si = SockInfo {
            domain: getsockopt_int(raw, libc::SOL_SOCKET, libc::SO_DOMAIN)?,
            protocol: getsockopt_int(raw, libc::SOL_SOCKET, libc::SO_PROTOCOL)?,
            sotype: getsockopt_int(raw, libc::SOL_SOCKET, libc::SO_TYPE)?,
            ..SockInfo::default()
        };
        if !si.is_l2cap_seqpacket() {
            return Ok(si);
        }
        if let Some((psm, _, _)) = sockaddr_l2(raw, false) {
            si.local_psm = Some(psm);
        }
        if let Some((psm, mac, cid)) = sockaddr_l2(raw, true) {
            si.peer_psm = Some(psm);
            si.peer = Some(mac);
            si.peer_cid = Some(cid);
        }
        let mut ci = [0u8; 8];
        let mut len: libc::socklen_t = 6;
        // SAFETY: 8-byte buffer, kernel writes at most `len` (6) bytes.
        let rc = unsafe {
            libc::getsockopt(
                raw,
                SOL_L2CAP,
                L2CAP_CONNINFO,
                ci.as_mut_ptr().cast(),
                &raw mut len,
            )
        };
        if rc == 0 && len >= 2 {
            si.hci_handle = Some(u16::from_ne_bytes([ci[0], ci[1]]));
        }
        let mut lo = [0u8; 16];
        let mut len: libc::socklen_t = 12;
        // SAFETY: 16-byte buffer, kernel writes at most `len` (12) bytes.
        let rc = unsafe {
            libc::getsockopt(
                raw,
                SOL_L2CAP,
                L2CAP_OPTIONS,
                lo.as_mut_ptr().cast(),
                &raw mut len,
            )
        };
        if rc == 0 && len >= 4 {
            si.omtu = Some(u16::from_ne_bytes([lo[0], lo[1]]));
            si.imtu = Some(u16::from_ne_bytes([lo[2], lo[3]]));
        }
        Ok(si)
    }

    fn drained(&self, fd: BorrowedFd<'_>) -> Option<bool> {
        let sndbuf = getsockopt_int(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_SNDBUF).ok()?;
        let mut room: libc::c_int = 0;
        // SAFETY: TIOCOUTQ writes one int.
        let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCOUTQ, &mut room) };
        (rc == 0).then_some(room >= sndbuf)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NoCandidate,
    Ambiguous(usize),
    MacNotInTable(Mac),
    NotBluetoothd(String),
    WrongUid(u32),
    Daemon(String),
    Syscall(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NoCandidate => write!(
                f,
                "no connected L2CAP control socket (PSM 0x0011) for this keyboard in bluetoothd"
            ),
            Refusal::Ambiguous(n) => {
                write!(f, "{n} candidate control sockets, exactly one required")
            }
            Refusal::MacNotInTable(m) => write!(
                f,
                "{m} is not a connected Apple keyboard of the model table"
            ),
            Refusal::NotBluetoothd(e) => write!(f, "process is not bluetoothd (exe {e:?})"),
            Refusal::WrongUid(u) => write!(f, "bluetoothd runs as uid {u}, expected root"),
            Refusal::Daemon(e) => write!(f, "bluetoothd not found: {e}"),
            Refusal::Syscall(e) => write!(f, "{e}"),
        }
    }
}

#[must_use]
pub fn is_control_channel(mac: Mac, si: &SockInfo) -> bool {
    si.is_l2cap_seqpacket()
        && si.peer == Some(mac)
        && si.hci_handle.is_some()
        && si.peer_psm == Some(PSM_HID_CONTROL)
        && matches!(si.local_psm, Some(PSM_HID_CONTROL | 0))
}

/// Index of the HID control socket of `mac` in `socks`.
///
/// # Errors
///
/// [`Refusal`] when no single socket matches.
pub fn select_control(mac: Mac, socks: &[(i32, SockInfo)]) -> Result<usize, Refusal> {
    let mut found = socks
        .iter()
        .enumerate()
        .filter(|(_, (_, si))| is_control_channel(mac, si))
        .map(|(i, _)| i);
    match (found.next(), found.count()) {
        (None, _) => Err(Refusal::NoCandidate),
        (Some(i), 0) => Ok(i),
        (Some(_), n) => Err(Refusal::Ambiguous(n + 1)),
    }
}

#[must_use]
pub fn is_bluetoothd_exe(exe: &str, allowed: &[&str]) -> bool {
    let e = exe.strip_suffix(" (deleted)").unwrap_or(exe);
    allowed.contains(&e)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keyboard {
    pub mac: Mac,
    pub model: &'static str,
    pub pid: u32,
}

#[must_use]
pub fn keyboard_from_uevent(uevent: &str) -> Option<Keyboard> {
    let id = uevent
        .lines()
        .find_map(|l| l.strip_prefix("HID_ID="))?
        .trim();
    if u32::from_str_radix(id.split(':').next()?, 16).ok()? != 0x0005 {
        return None;
    }
    let mi = model::model_from_uevent(uevent)?;
    let mac = Mac::parse(
        uevent
            .lines()
            .find_map(|l| l.strip_prefix("HID_UNIQ="))?
            .trim(),
    )
    .ok()?;
    Some(Keyboard {
        mac,
        model: mi.model,
        pid: mi.pid,
    })
}

#[must_use]
pub fn connected_keyboards(hid_root: &Path) -> Vec<Keyboard> {
    let mut v: Vec<Keyboard> = fs::read_dir(hid_root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| fs::read_to_string(e.path().join("uevent")).ok())
        .filter_map(|u| keyboard_from_uevent(&u))
        .collect();
    v.sort_by_key(|k| k.mac);
    v.dedup_by_key(|k| k.mac);
    v
}

/// Keyboards to act on: all connected ones, or `only`.
///
/// # Errors
///
/// [`Refusal`] when none matches.
pub fn targets(connected: &[Keyboard], only: Option<Mac>) -> Result<Vec<Keyboard>, Refusal> {
    match only {
        None => Ok(connected.to_vec()),
        Some(m) => connected
            .iter()
            .find(|k| k.mac == m)
            .map(|k| vec![k.clone()])
            .ok_or(Refusal::MacNotInTable(m)),
    }
}

pub struct Env<'a> {
    pub proc_root: PathBuf,
    pub hid_root: PathBuf,
    pub allowed_exes: &'a [&'a str],
    pub required_uid: u32,
    pub daemon_pid: &'a dyn Fn() -> Result<i32, String>,
    pub run_user_root: PathBuf,
    pub daemon_alive: &'a dyn Fn(u32, Option<breaker_state::Writer>) -> bool,
    pub active_uid: &'a dyn Fn() -> Option<u32>,
}

fn system_daemon_alive(uid: u32, w: Option<breaker_state::Writer>) -> bool {
    breaker_state::daemon_alive_in(Path::new("/proc"), w, uid)
}

fn system_active_uid() -> Option<u32> {
    breaker_state::seat_active_uid(Path::new(breaker_state::SEAT0))
}

impl Env<'static> {
    #[must_use]
    pub fn system() -> Env<'static> {
        Env {
            proc_root: PathBuf::from("/proc"),
            hid_root: PathBuf::from("/sys/bus/hid/devices"),
            allowed_exes: BLUETOOTHD_EXES,
            required_uid: 0,
            daemon_pid: &bluetoothd_main_pid,
            run_user_root: PathBuf::from(breaker_state::RUN_USER_ROOT),
            daemon_alive: &system_daemon_alive,
            active_uid: &system_active_uid,
        }
    }
}

/// Breaker state published by the daemon of user `uid`.
///
/// # Errors
///
/// A message when the file exists but cannot be read or parsed.
pub fn published_breaker(run_user_root: &Path, uid: u32) -> breaker_state::Found {
    let p = breaker_state::path_for_uid(run_user_root, uid);
    match crate::fsutil::read_user_file(&p, uid, breaker_state::MAX_LEN) {
        Ok(s) => breaker_state::BreakerState::parse(&s)
            .map(Some)
            .map_err(|e| format!("{}: {e}", p.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[must_use]
pub fn breaker_verdict(env: &Env<'_>, mac: Mac, now_unix: u64) -> breaker_state::Verdict {
    let Some(uid) = (env.active_uid)() else {
        return breaker_state::Verdict::Refuse(breaker_state::Refuse::NoActiveUser);
    };
    let found = published_breaker(&env.run_user_root, uid);
    let alive = |w: Option<breaker_state::Writer>| (env.daemon_alive)(uid, w);
    breaker_state::verdict(&found, &mac.to_string(), now_unix, &alive)
}

/// Main PID of `bluetooth.service`.
///
/// # Errors
///
/// A message when systemd cannot be queried or gives no PID.
pub fn bluetoothd_main_pid() -> Result<i32, String> {
    let out = std::process::Command::new("/usr/bin/systemctl")
        .env_clear()
        .args(["show", "-p", "MainPID", "--value", "bluetooth.service"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("systemctl: {e}"))?;
    parse_main_pid(&String::from_utf8_lossy(&out.stdout))
}

/// Parse the `MainPID=` output of systemctl.
///
/// # Errors
///
/// A message when the output is malformed or the PID is 0.
pub fn parse_main_pid(s: &str) -> Result<i32, String> {
    let t = s.trim();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) || t.len() > 9 {
        return Err(format!("unexpected MainPID {t:?}"));
    }
    match t.parse::<i32>() {
        Ok(p) if p > 1 => Ok(p),
        _ => Err("bluetooth.service is not running (MainPID 0)".into()),
    }
}

fn pidfd_open(pid: i32) -> io::Result<OwnedFd> {
    // SAFETY: plain syscall, returns a new fd or -1.
    let r = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = RawFd::try_from(r).map_err(io::Error::other)?;
    // SAFETY: fd is a fresh descriptor owned by nobody else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn pidfd_getfd(pidfd: &OwnedFd, target: RawFd) -> io::Result<OwnedFd> {
    // SAFETY: plain syscall; the duplicate is close-on-exec and ours.
    let r = unsafe { libc::syscall(libc::SYS_pidfd_getfd, pidfd.as_raw_fd(), target, 0) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = RawFd::try_from(r).map_err(io::Error::other)?;
    // SAFETY: as above.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn pidfd_alive(pidfd: &OwnedFd) -> bool {
    // SAFETY: signal 0 = existence check only.
    unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            0,
            std::ptr::null::<u8>(),
            0,
        ) == 0
    }
}

fn proc_uid(proc_root: &Path, pid: i32) -> Option<u32> {
    let s = fs::read_to_string(proc_root.join(pid.to_string()).join("status")).ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// The single write path: one byte, non-blocking, never retried.
fn send_one(fd: BorrowedFd<'_>, cmd: HidControl) -> io::Result<()> {
    let b = check_byte(cmd.byte()).map_err(io::Error::other)?;
    let buf = [b];
    // SAFETY: one-byte buffer; MSG_DONTWAIT acts on this call only (the
    // O_NONBLOCK flag shared with bluetoothd is never touched).
    let n = unsafe {
        libc::send(
            fd.as_raw_fd(),
            buf.as_ptr().cast(),
            1,
            libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
        )
    };
    match n {
        1 => Ok(()),
        n if n < 0 => Err(io::Error::last_os_error()),
        n => Err(io::Error::other(format!("short send ({n} bytes)"))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Info(String),
    Warn(String),
}

/// Log sink for the hid subcommands: journal priority prefix under systemd, else stderr + syslog.
pub fn event_logger(cmd: &'static str) -> impl FnMut(Event) {
    let journal = std::env::var_os("JOURNAL_STREAM").is_some();
    move |e: Event| {
        let (warn, m) = match &e {
            Event::Info(m) => (false, m),
            Event::Warn(m) => (true, m),
        };
        if journal {
            eprintln!("<{}>{m}", if warn { 4 } else { 6 });
        } else {
            eprintln!("akm-helper {cmd}: {m}");
            syslog(warn, m);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KbOutcome {
    Sent,
    DryRun,
    /// `inspect`: the control channel was found and described, nothing sent.
    Inspected,
    /// Apple's R3: the daemon's breaker is open: nothing sent, and nothing to retry.
    BreakerOpen(String),
    Refused(Refusal),
    SendFailed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Send one `HID_CONTROL` byte (or show what would be sent with `dry_run`).
    Send(HidControl),
    /// Describe the control channel (PSM, CID, state, MTUs); never sends.
    Inspect,
}

impl Action {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Action::Send(c) => c.name(),
            Action::Inspect => "inspect",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub pid: Option<i32>,
    pub per_keyboard: Vec<(Mac, KbOutcome)>,
    pub global: Option<Refusal>,
}

impl Report {
    #[must_use]
    pub fn ok(&self) -> bool {
        self.global.is_none()
            && self.per_keyboard.iter().all(|(_, o)| {
                matches!(
                    o,
                    KbOutcome::Sent
                        | KbOutcome::DryRun
                        | KbOutcome::Inspected
                        | KbOutcome::BreakerOpen(_)
                )
            })
    }
}

fn socket_fds(proc_root: &Path, pid: i32) -> io::Result<Vec<RawFd>> {
    let mut v = Vec::new();
    for e in fs::read_dir(proc_root.join(pid.to_string()).join("fd"))? {
        let e = e?;
        let Some(n) = e.file_name().to_str().and_then(|s| s.parse::<RawFd>().ok()) else {
            continue;
        };
        if fs::read_link(e.path()).is_ok_and(|l| l.to_string_lossy().starts_with("socket:[")) {
            v.push(n);
        }
    }
    v.sort_unstable();
    Ok(v)
}

pub fn run(
    cmd: HidControl,
    only: Option<Mac>,
    dry_run: bool,
    env: &Env<'_>,
    cs: &dyn ControlSocket,
    log: &mut dyn FnMut(Event),
) -> Report {
    run_action(Action::Send(cmd), only, dry_run, env, cs, log)
}

pub fn inspect(
    only: Option<Mac>,
    env: &Env<'_>,
    cs: &dyn ControlSocket,
    log: &mut dyn FnMut(Event),
) -> Report {
    run_action(Action::Inspect, only, true, env, cs, log)
}

#[must_use]
pub fn describe_socket(mac: Mac, fd: i32, si: &SockInfo) -> String {
    format!(
        "  {mac} fd {fd}: psm local {} peer {} cid {} state {} mtu out {} in {}",
        fmt_psm(si.local_psm),
        fmt_psm(si.peer_psm),
        si.peer_cid.map_or("-".into(), |c| format!("0x{c:04x}")),
        si.hci_handle.map_or("not connected".into(), |h| format!(
            "connected (hci handle 0x{h:04x})"
        )),
        si.omtu.map_or("-".into(), |m| m.to_string()),
        si.imtu.map_or("-".into(), |m| m.to_string()),
    )
}

fn run_action(
    action: Action,
    only: Option<Mac>,
    dry_run: bool,
    env: &Env<'_>,
    cs: &dyn ControlSocket,
    log: &mut dyn FnMut(Event),
) -> Report {
    let mut rep = Report {
        pid: None,
        per_keyboard: Vec::new(),
        global: None,
    };
    let connected = connected_keyboards(&env.hid_root);
    let kbs = match targets(&connected, only) {
        Ok(k) => k,
        Err(r) => {
            log(Event::Warn(format!("refused: {r}")));
            rep.global = Some(r);
            return rep;
        }
    };
    if kbs.is_empty() {
        log(Event::Info(format!(
            "no Apple keyboard connected: {} not {}",
            action.name(),
            if action == Action::Inspect {
                "done"
            } else {
                "sent"
            }
        )));
        return rep;
    }
    let fail = |rep: &mut Report, log: &mut dyn FnMut(Event), r: Refusal| {
        log(Event::Warn(format!("refused, nothing written: {r}")));
        rep.global = Some(r);
    };
    let pid = match (env.daemon_pid)() {
        Ok(p) => p,
        Err(e) => {
            fail(&mut rep, log, Refusal::Daemon(e));
            return rep;
        }
    };
    rep.pid = Some(pid);
    let (socks, held, exe, unread) = match verify_daemon(env, pid).and_then(|(pidfd, exe)| {
        let (socks, held, unread) = collect_sockets(env, pid, &pidfd, cs)?;
        Ok((socks, held, exe, unread))
    }) {
        Ok(found) => found,
        Err(r) => {
            fail(&mut rep, log, r);
            return rep;
        }
    };
    log(Event::Info(match unread.first() {
        None => format!(
            "bluetoothd pid {pid} ({exe}): {} L2CAP socket(s)",
            socks.len()
        ),
        Some(e) => format!(
            "bluetoothd pid {pid} ({exe}): {} L2CAP socket(s), {} socket(s) unreadable ({e})",
            socks.len(),
            unread.len()
        ),
    }));
    let mut sent: Vec<usize> = Vec::new();
    for kb in &kbs {
        for (n, si) in socks.iter().filter(|(_, si)| si.peer == Some(kb.mac)) {
            log(Event::Info(describe_socket(kb.mac, *n, si)));
        }
        let outcome = match select_control(kb.mac, &socks) {
            Err(r) => {
                log(Event::Warn(format!(
                    "{} ({}): refused, nothing written: {r}",
                    kb.mac, kb.model
                )));
                KbOutcome::Refused(r)
            }
            Ok(i) => {
                let target = Target {
                    kb,
                    pid,
                    fd: socks[i].0,
                    si: &socks[i].1,
                    sock: held[i].as_fd(),
                };
                let outcome = act_on(action, dry_run, env, &target, log);
                if outcome == KbOutcome::Sent {
                    sent.push(i);
                }
                outcome
            }
        };
        rep.per_keyboard.push((kb.mac, outcome));
    }
    let waited: Vec<OwnedFd> = held
        .into_iter()
        .enumerate()
        .filter(|(i, _)| sent.contains(i))
        .map(|(_, f)| f)
        .collect();
    wait_drained(&waited, cs, log);
    rep
}

/// The daemon's pidfd and executable, once it is checked to be `bluetoothd` run by the right uid.
fn verify_daemon(env: &Env<'_>, pid: i32) -> Result<(OwnedFd, String), Refusal> {
    let pidfd = pidfd_open(pid).map_err(|e| Refusal::Syscall(format!("pidfd_open({pid}): {e}")))?;
    let exe = match fs::read_link(env.proc_root.join(pid.to_string()).join("exe")) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(e) => return Err(Refusal::NotBluetoothd(format!("unreadable: {e}"))),
    };
    if !is_bluetoothd_exe(&exe, env.allowed_exes) {
        return Err(Refusal::NotBluetoothd(exe));
    }
    match proc_uid(&env.proc_root, pid) {
        Some(u) if u == env.required_uid => Ok((pidfd, exe)),
        u => Err(Refusal::WrongUid(u.unwrap_or(u32::MAX))),
    }
}

/// The daemon's L2CAP sockets (fd number, description), our duplicates of them, and why other sockets could not be read.
type Sockets = (Vec<(i32, SockInfo)>, Vec<OwnedFd>, Vec<String>);

fn collect_sockets(
    env: &Env<'_>,
    pid: i32,
    pidfd: &OwnedFd,
    cs: &dyn ControlSocket,
) -> Result<Sockets, Refusal> {
    let fds = socket_fds(&env.proc_root, pid)
        .map_err(|e| Refusal::Syscall(format!("/proc/{pid}/fd: {e}")))?;
    let mut socks: Vec<(i32, SockInfo)> = Vec::new();
    let mut held: Vec<OwnedFd> = Vec::new();
    let mut unread: Vec<String> = Vec::new();
    for n in fds {
        let dup = match pidfd_getfd(pidfd, n) {
            Ok(d) => d,
            Err(e) => {
                unread.push(format!("fd {n}: pidfd_getfd: {e}"));
                continue;
            }
        };
        let si = match cs.inspect(dup.as_fd()) {
            Ok(si) => si,
            Err(e) => {
                unread.push(format!("fd {n}: {e}"));
                continue;
            }
        };
        if si.is_l2cap_seqpacket() {
            socks.push((n, si));
            held.push(dup);
        }
    }
    if !pidfd_alive(pidfd) {
        return Err(Refusal::Daemon(format!("pid {pid} exited during the scan")));
    }
    Ok((socks, held, unread))
}

/// The control channel of one keyboard in the daemon.
struct Target<'a> {
    kb: &'a Keyboard,
    pid: i32,
    fd: i32,
    si: &'a SockInfo,
    sock: BorrowedFd<'a>,
}

/// Inspect, or send one byte on the control channel when the breaker allows it.
fn act_on(
    action: Action,
    dry_run: bool,
    env: &Env<'_>,
    t: &Target<'_>,
    log: &mut dyn FnMut(Event),
) -> KbOutcome {
    let (kb, pid, n, si) = (t.kb, t.pid, t.fd, t.si);
    let cmd = match action {
        Action::Inspect => {
            log(Event::Info(format!(
                "{} ({} 0x{:04x}): control channel pid {pid} fd {n} psm 0x{:04x} {} mtu out {} in {}: inspected, nothing sent",
                kb.mac,
                kb.model,
                kb.pid,
                si.peer_psm.unwrap_or(0),
                si.hci_handle
                    .map_or("-".into(), |h| format!("hci 0x{h:04x}")),
                si.omtu.map_or("-".into(), |m| m.to_string()),
                si.imtu.map_or("-".into(), |m| m.to_string()),
            )));
            return KbOutcome::Inspected;
        }
        Action::Send(c) => c,
    };
    let what = format!(
        "{} ({} 0x{:04x}): pid {pid} fd {n} psm 0x{:04x} {} byte 0x{:02x} {}",
        kb.mac,
        kb.model,
        kb.pid,
        si.peer_psm.unwrap_or(0),
        si.hci_handle
            .map_or("-".into(), |h| format!("hci 0x{h:04x}")),
        cmd.byte(),
        cmd.name()
    );
    match breaker_verdict(env, kb.mac, breaker_state::now_unix()) {
        breaker_state::Verdict::Allow(a) => {
            log(Event::Info(format!(
                "{}: breaker state {a:?}: emission allowed",
                kb.mac
            )));
        }
        breaker_state::Verdict::Refuse(r) => {
            log(Event::Warn(format!("{what}: NOT sent, {r}")));
            if !dry_run {
                return KbOutcome::BreakerOpen(r.to_string());
            }
        }
    }
    if dry_run {
        log(Event::Info(format!("{what}: NOT sent (--dry-run)")));
        return KbOutcome::DryRun;
    }
    match send_one(t.sock, cmd) {
        Ok(()) => {
            log(Event::Info(format!("{what}: sent")));
            KbOutcome::Sent
        }
        Err(e) => {
            log(Event::Warn(format!(
                "{what}: send failed, not retried: {e}"
            )));
            KbOutcome::SendFailed(e.to_string())
        }
    }
}

/// Wait (bounded by `DRAIN_WAIT`) until the kernel sent what was queued on `waited`.
fn wait_drained(waited: &[OwnedFd], cs: &dyn ControlSocket, log: &mut dyn FnMut(Event)) {
    if waited.is_empty() {
        return;
    }
    let t0 = Instant::now();
    loop {
        if waited.iter().all(|f| cs.drained(f.as_fd()).unwrap_or(true)) {
            log(Event::Info(format!(
                "queue drained after {} ms",
                t0.elapsed().as_millis()
            )));
            break;
        }
        if t0.elapsed() >= DRAIN_WAIT {
            log(Event::Warn(format!(
                "queue not drained after {} ms, giving up",
                DRAIN_WAIT.as_millis()
            )));
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn fmt_psm(p: Option<u16>) -> String {
    p.map_or("-".into(), |p| format!("0x{p:04x}"))
}

pub const CONFIG_PATH: &str = "/etc/apple-kb-monitor/hid-suspend.conf";

/// Parse the helper configuration (`enabled = true|false`).
///
/// # Errors
///
/// A message naming the first invalid line.
pub fn parse_config(s: &str) -> Result<bool, String> {
    let mut enabled = None;
    for (i, raw) in s.lines().enumerate() {
        let entry = raw.split('#').next().unwrap_or("").trim();
        if entry.is_empty() {
            continue;
        }
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("line {}: expected `enabled = true|false`", i + 1))?;
        if key.trim() != "enabled" {
            return Err(format!("line {}: unknown key {:?}", i + 1, key.trim()));
        }
        let flag = match value.trim() {
            "true" => true,
            "false" => false,
            o => {
                return Err(format!(
                    "line {}: invalid value {o:?} (true or false)",
                    i + 1
                ))
            }
        };
        if enabled.replace(flag).is_some() {
            return Err(format!("line {}: `enabled` given twice", i + 1));
        }
    }
    Ok(enabled.unwrap_or(false))
}

pub const CONFIG_MAX_LEN: u64 = 4096;

#[must_use]
pub fn config_meta_ok(is_file: bool, uid: u32, mode: u32, len: u64) -> bool {
    is_file && uid == 0 && mode & 0o022 == 0 && len <= CONFIG_MAX_LEN
}

/// Read at most `CONFIG_MAX_LEN` bytes from the already-vetted descriptor.
fn read_capped(f: &fs::File) -> io::Result<String> {
    use std::io::Read;
    let mut s = String::new();
    f.take(CONFIG_MAX_LEN + 1).read_to_string(&mut s)?;
    if s.len() as u64 > CONFIG_MAX_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file larger than 4 KiB",
        ));
    }
    Ok(s)
}

/// Read and parse the helper configuration (absent = default).
///
/// # Errors
///
/// A message for an I/O error or invalid content.
pub fn read_config(path: &Path) -> Result<bool, String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let f = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let md = f.metadata().map_err(|e| e.to_string())?;
    if !config_meta_ok(md.file_type().is_file(), md.uid(), md.mode(), md.len()) {
        return Err(format!(
            "{}: must be a regular file owned by root, not group/world writable, ≤ 4 KiB",
            path.display()
        ));
    }
    let s = read_capped(&f).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_config(&s).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn syslog(warn: bool, msg: &str) {
    static IDENT: &[u8] = b"akm-hid-control\0";
    let Ok(c) = CString::new(msg.replace('\0', " ")) else {
        return;
    };
    let prio = if warn {
        libc::LOG_WARNING
    } else {
        libc::LOG_INFO
    };
    // SAFETY: static NUL-terminated ident, "%s" format with a valid C string.
    unsafe {
        libc::openlog(IDENT.as_ptr().cast(), libc::LOG_PID, libc::LOG_DAEMON);
        libc::syslog(prio, c"%s".as_ptr(), c.as_ptr());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PSM_HID_INTERRUPT: u16 = 0x0013;

    const KB: Mac = Mac([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xf1]);
    const OTHER: Mac = Mac([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);

    fn l2(mac: Mac, local: u16, peer: u16, connected: bool) -> SockInfo {
        SockInfo {
            domain: AF_BLUETOOTH,
            protocol: BTPROTO_L2CAP,
            sotype: libc::SOCK_SEQPACKET,
            peer: Some(mac),
            local_psm: Some(local),
            peer_psm: Some(peer),
            peer_cid: Some(0x40),
            hci_handle: connected.then_some(0x0b),
            omtu: Some(672),
            imtu: Some(672),
        }
    }

    #[test]
    fn socket_description_carries_the_mtus_for_akmctl() {
        let s = describe_socket(KB, 7, &l2(KB, 0, PSM_HID_CONTROL, true));
        assert_eq!(
            s,
            "  AA:BB:CC:DD:EE:F1 fd 7: psm local 0x0000 peer 0x0011 cid 0x0040 state connected (hci handle 0x000b) mtu out 672 in 672"
        );
        let mut si = l2(KB, 0, PSM_HID_CONTROL, false);
        si.omtu = None;
        si.imtu = None;
        assert!(describe_socket(KB, 7, &si).ends_with("state not connected mtu out - in -"));
        assert_eq!(Action::Inspect.name(), "inspect");
        assert_eq!(Action::Send(HidControl::Suspend).name(), "SUSPEND");
        let rep = Report {
            pid: Some(1),
            per_keyboard: vec![(KB, KbOutcome::Inspected)],
            global: None,
        };
        assert!(rep.ok());
    }

    #[test]
    fn only_0x13_and_0x14_pass_the_256_sweep() {
        let mut ok = Vec::new();
        for b in 0..=255u8 {
            let c = HidControl::from_byte(b);
            assert_eq!(c.is_some(), check_byte(b).is_ok(), "0x{b:02x}");
            if let Some(c) = c {
                assert_eq!(c.byte(), b);
                ok.push(b);
            }
        }
        assert_eq!(ok, vec![0x13, 0x14]);
        assert_eq!(HidControl::Suspend.byte(), 0x13);
        assert_eq!(HidControl::ExitSuspend.byte(), 0x14);
    }

    #[test]
    fn single_candidate_is_selected() {
        let socks = vec![
            (5, l2(KB, PSM_HID_INTERRUPT, PSM_HID_INTERRUPT, true)),
            (7, l2(KB, PSM_HID_CONTROL, PSM_HID_CONTROL, true)),
            (9, l2(OTHER, PSM_HID_CONTROL, PSM_HID_CONTROL, true)),
        ];
        assert_eq!(select_control(KB, &socks), Ok(1));
        assert_eq!(select_control(OTHER, &socks), Ok(2));
        assert_eq!(
            select_control(KB, &[(3, l2(KB, 0, PSM_HID_CONTROL, true))]),
            Ok(0)
        );
    }

    #[test]
    fn zero_or_two_candidates_are_refused() {
        assert_eq!(select_control(KB, &[]), Err(Refusal::NoCandidate));
        let two = vec![(7, l2(KB, 0x11, 0x11, true)), (8, l2(KB, 0x11, 0x11, true))];
        assert_eq!(select_control(KB, &two), Err(Refusal::Ambiguous(2)));
        let three = vec![
            (7, l2(KB, 0x11, 0x11, true)),
            (8, l2(KB, 0, 0x11, true)),
            (9, l2(KB, 0x11, 0x11, true)),
        ];
        assert_eq!(select_control(KB, &three), Err(Refusal::Ambiguous(3)));
    }

    #[test]
    fn psm_other_than_0x11_is_refused() {
        for psm in (0..=0xffffu16).step_by(1).filter(|p| *p != PSM_HID_CONTROL) {
            assert!(
                !is_control_channel(KB, &l2(KB, PSM_HID_CONTROL, psm, true)),
                "peer psm 0x{psm:04x}"
            );
            if psm != 0 {
                assert!(
                    !is_control_channel(KB, &l2(KB, psm, PSM_HID_CONTROL, true)),
                    "local psm 0x{psm:04x}"
                );
            }
        }
        assert_eq!(
            select_control(KB, &[(5, l2(KB, 0x13, 0x13, true))]),
            Err(Refusal::NoCandidate)
        );
    }

    #[test]
    fn other_socket_kinds_are_refused() {
        let base = l2(KB, 0x11, 0x11, true);
        assert!(is_control_channel(KB, &base));
        assert!(
            !is_control_channel(KB, &l2(KB, 0x11, 0x11, false)),
            "not connected"
        );
        assert!(!is_control_channel(OTHER, &base), "other peer");
        assert!(!is_control_channel(
            KB,
            &SockInfo {
                domain: libc::AF_UNIX,
                ..base.clone()
            }
        ));
        assert!(
            !is_control_channel(
                KB,
                &SockInfo {
                    protocol: 1,
                    ..base.clone()
                }
            ),
            "HCI"
        );
        assert!(!is_control_channel(
            KB,
            &SockInfo {
                sotype: libc::SOCK_STREAM,
                ..base.clone()
            }
        ));
        assert!(!is_control_channel(
            KB,
            &SockInfo {
                peer: None,
                ..base.clone()
            }
        ));
        assert!(!is_control_channel(
            KB,
            &SockInfo {
                local_psm: None,
                ..base
            }
        ));
    }

    #[test]
    fn exe_must_be_bluetoothd() {
        assert!(is_bluetoothd_exe(
            "/usr/lib/bluetooth/bluetoothd",
            BLUETOOTHD_EXES
        ));
        assert!(is_bluetoothd_exe(
            "/usr/lib/bluetooth/bluetoothd (deleted)",
            BLUETOOTHD_EXES
        ));
        assert!(is_bluetoothd_exe(
            "/usr/libexec/bluetooth/bluetoothd",
            BLUETOOTHD_EXES
        ));
        for bad in [
            "/usr/bin/sleep",
            "/tmp/bluetoothd",
            "/usr/lib/bluetooth/bluetoothd.evil",
            "/usr/lib/bluetooth/obexd",
            "bluetoothd",
            "/usr/lib/bluetooth/bluetoothd (deleted) (deleted)",
            "",
        ] {
            assert!(!is_bluetoothd_exe(bad, BLUETOOTHD_EXES), "{bad}");
        }
    }

    #[test]
    fn mac_parsing_is_strict() {
        assert_eq!(Mac::parse("aa:bb:cc:dd:ee:f1"), Ok(KB));
        assert_eq!(
            Mac::parse("AA:BB:CC:DD:EE:F1").unwrap().to_string(),
            "AA:BB:CC:DD:EE:F1"
        );
        for bad in [
            "",
            "aa:bb:cc:dd:ee",
            "aa-bb-cc-dd-ee-f1",
            "aa:bb:cc:dd:ee:f1 ",
            "0g:bb:cc:dd:ee:f1",
            "aa:bb:cc:dd:ee:f1e",
            "+a:bb:cc:dd:ee:f1",
        ] {
            assert!(Mac::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            Mac::from_bdaddr_le([0xf1, 0xee, 0xdd, 0xcc, 0xbb, 0xaa]),
            KB
        );
    }

    #[test]
    fn keyboards_come_from_the_model_table_only() {
        let a1314 = "HID_ID=0005:000005AC:00000256\nHID_NAME=kb\nHID_UNIQ=aa:bb:cc:dd:ee:f1\n";
        assert_eq!(
            keyboard_from_uevent(a1314).map(|k| (k.mac, k.pid)),
            Some((KB, 0x0256))
        );
        assert!(keyboard_from_uevent(
            "HID_ID=0003:000005AC:00000256\nHID_UNIQ=aa:bb:cc:dd:ee:f1\n"
        )
        .is_none());
        assert!(keyboard_from_uevent(
            "HID_ID=0005:000005AC:0000030D\nHID_UNIQ=aa:bb:cc:dd:ee:f1\n"
        )
        .is_none());
        assert!(keyboard_from_uevent(
            "HID_ID=0005:0000046D:0000B342\nHID_UNIQ=aa:bb:cc:dd:ee:f1\n"
        )
        .is_none());
        assert!(keyboard_from_uevent("HID_ID=0005:000005AC:00000256\nHID_UNIQ=\n").is_none());
        let kbs = vec![Keyboard {
            mac: KB,
            model: "A1314",
            pid: 0x256,
        }];
        assert_eq!(targets(&kbs, None).unwrap().len(), 1);
        assert_eq!(targets(&kbs, Some(KB)).unwrap()[0].mac, KB);
        assert_eq!(
            targets(&kbs, Some(OTHER)),
            Err(Refusal::MacNotInTable(OTHER))
        );
        assert_eq!(targets(&[], Some(KB)), Err(Refusal::MacNotInTable(KB)));
    }

    #[test]
    fn config_read_is_capped_on_the_open_descriptor() {
        use std::io::{Seek, Write};
        let path = std::env::temp_dir().join(format!("akm-cfg-cap-{}", std::process::id()));
        let mut f = fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        fs::remove_file(&path).unwrap();
        f.write_all(b"enabled = true\n").unwrap();
        f.rewind().unwrap();
        assert_eq!(read_capped(&f).unwrap(), "enabled = true\n");
        f.write_all(&[b'#'; 4096]).unwrap();
        f.rewind().unwrap();
        assert!(read_capped(&f).is_err());
    }

    #[test]
    fn config_parsing() {
        assert_eq!(parse_config(""), Ok(false));
        assert_eq!(parse_config("# enabled = true\n\n"), Ok(false));
        assert_eq!(parse_config("# c\n\nenabled = false\n"), Ok(false));
        assert_eq!(parse_config("enabled=true # on"), Ok(true));
        for bad in [
            "enabled = yes",
            "enable = true",
            "enabled",
            "enabled = true\nenabled = false",
            "x = 1",
        ] {
            assert!(parse_config(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            read_config(Path::new("/nonexistent/akm/hid-suspend.conf")),
            Ok(false),
            "absent file = disabled"
        );
        assert_eq!(
            parse_config(include_str!("../../../../systemd/hid-suspend.conf")),
            Ok(false)
        );
    }

    #[test]
    fn main_pid_parsing() {
        assert_eq!(parse_main_pid("1144\n"), Ok(1144));
        for bad in ["0\n", "1", "", "-5", "12 3", "abc", "9999999999"] {
            assert!(parse_main_pid(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn this_file_has_one_write_path() {
        let src = include_str!("hidctl.rs");
        let code: String = src.split("#[cfg(test)]").next().unwrap().to_string();
        assert_eq!(code.matches("libc::send(").count(), 1);
        let flat: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(flat.contains("libc::send(fd.as_raw_fd(),buf.as_ptr().cast(),1,"));
        assert_eq!(
            flat.matches("letbuf=[b];").count(),
            1,
            "the buffer is the checked byte alone"
        );
        for forbidden in [
            "libc::write(",
            "libc::sendmsg(",
            "libc::sendto(",
            "libc::setsockopt(",
            "libc::fcntl(",
            "write_all(",
        ] {
            assert!(!code.contains(forbidden), "{forbidden}");
        }
        for other in [
            "lib.rs",
            "main.rs",
            "cmd/hid_control.rs",
            "cmd/hid_inspect.rs",
            "cmd/keymap_helper.rs",
            "cmd/doctor_fix.rs",
        ] {
            let s = std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("src")
                    .join(other),
            )
            .unwrap();
            assert!(!s.contains("libc::send"), "{other}");
        }
    }
}
