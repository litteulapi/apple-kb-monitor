//! Passive listening to the keyboard's input reports (#130, #187, #129).
//!
//! The daemon never asks the keyboard anything here: no GET_REPORT, no
//! SET_REPORT, no LED write. The `/dev/hidrawN` node is opened **read-only**
//! and the interrupt reports the keyboard sends on its own are decoded:
//!
//! | id | meaning | source |
//! |---|---|---|
//! | `0x04` | SLEEP notification | WICED `RPT_ID_IN_SLEEP` [source publique] |
//! | `0x05` | FUNC_LOCK state byte | WICED `RPT_ID_IN_FUNC_LOCK` [source publique] |
//! | `0x30` | BATT_STAT | FreeBSD bthidd `BATT_STAT_REPORT_ID` [source publique] |
//! | `0x13` | ready (bit 0) / connection request (bit 1) | descriptor [mesuré] |
//! | `0x11` | Eject (bit 3) / Fn (bit 4) | descriptor [mesuré] |
//! | `0x12` | media keys (bits 0-4) | descriptor [mesuré] |
//!
//! **No keylogging**: every other report (`0x01` key presses, `0x4C`, `0xFE`,
//! anything unknown) is dropped by [`decode`] after reading its first byte;
//! only its *arrival* is noted (timestamp, [`crate::read_policy::note_input`]).
//! Of the decoded reports only state and counters are kept, never a key code.
//!
//! The listener shares the cross-process lock of the safe read policy
//! (`hid.lock`): while another process (CLI, RE tool) holds it for Feature
//! requests, the listener releases the node and retries later. It never holds
//! the lock itself, so it cannot starve a reader. A disconnection (`POLLHUP`,
//! read error, node gone) ends the session; [`run`] then waits [`Config::retry`]
//! (never a tight loop) and picks up the new node at reconnection.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

pub const ID_SLEEP: u8 = 0x04;
pub const ID_FUNC_LOCK: u8 = 0x05;
pub const ID_EJECT_FN: u8 = 0x11;
pub const ID_MEDIA: u8 = 0x12;
pub const ID_WAKE: u8 = 0x13;
pub const ID_BATT_STAT: u8 = 0x30;

/// One decoded input report. Contains state only, never a key code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassiveEvent {
    /// `0x04`: sleep notification, with its payload byte.
    Sleep { code: u8 },
    /// `0x05`: Fn-lock state byte.
    FnLock { value: u8 },
    /// `0x30`: battery status byte.
    BattStat { value: u8 },
    /// `0x13`: device ready / connection request (an all-zero report is a
    /// release, not an event).
    Wake { ready: bool, conn_request: bool },
    /// `0x11`: Eject and Fn keys (false/false = both released).
    Keys { eject: bool, fn_key: bool },
    /// `0x12`: media keys bit field (0 = all released).
    Media { bits: u8 },
}

/// Decode one input report. Looks at the report id first and, for any id that
/// is not in the table, returns `None` without reading the payload.
pub fn decode(report: &[u8]) -> Option<PassiveEvent> {
    let (&id, rest) = report.split_first()?;
    let b = *rest.first()?;
    match id {
        ID_SLEEP => Some(PassiveEvent::Sleep { code: b }),
        ID_FUNC_LOCK => Some(PassiveEvent::FnLock { value: b }),
        ID_BATT_STAT => Some(PassiveEvent::BattStat { value: b }),
        ID_WAKE => (b & 0x03 != 0).then_some(PassiveEvent::Wake {
            ready: b & 0x01 != 0,
            conn_request: b & 0x02 != 0,
        }),
        ID_EJECT_FN => Some(PassiveEvent::Keys {
            eject: b & 0x08 != 0,
            fn_key: b & 0x10 != 0,
        }),
        ID_MEDIA => Some(PassiveEvent::Media { bits: b & 0x1F }),
        _ => None,
    }
}

/// What the passive listener knows about the keyboard.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PassiveState {
    /// A hidraw node is currently being listened to.
    pub listening: bool,
    /// Last `0x05` byte (`None` = unknown / never seen this session).
    pub fn_lock: Option<u8>,
    /// Unix time of the last `0x04` report (0 = never) and its payload.
    pub last_sleep_ts: u64,
    pub last_sleep_code: Option<u8>,
    /// Number of `0x13` events (ready / connection request) since start.
    pub wake_count: u64,
    pub last_wake_ts: u64,
    pub last_wake_ready: bool,
    pub last_wake_conn_request: bool,
    /// Eject / Fn keys currently held (from `0x11`).
    pub eject_pressed: bool,
    pub fn_pressed: bool,
    /// Media keys held (bits of `0x12`).
    pub media_bits: u8,
    /// Last `0x30` byte.
    pub batt_stat: Option<u8>,
    /// Number of Eject presses since start.
    pub eject_count: u64,
}

impl PassiveState {
    /// Fold an event in at unix time `now`; true if the state changed.
    pub fn apply(&mut self, ev: PassiveEvent, now: u64) -> bool {
        let before = self.clone();
        match ev {
            PassiveEvent::Sleep { code } => {
                self.last_sleep_ts = now;
                self.last_sleep_code = Some(code);
            }
            PassiveEvent::FnLock { value } => self.fn_lock = Some(value),
            PassiveEvent::BattStat { value } => self.batt_stat = Some(value),
            PassiveEvent::Wake {
                ready,
                conn_request,
            } => {
                self.wake_count += 1;
                self.last_wake_ts = now;
                self.last_wake_ready = ready;
                self.last_wake_conn_request = conn_request;
            }
            PassiveEvent::Keys { eject, fn_key } => {
                if eject && !self.eject_pressed {
                    self.eject_count += 1;
                }
                self.eject_pressed = eject;
                self.fn_pressed = fn_key;
            }
            PassiveEvent::Media { bits } => self.media_bits = bits,
        }
        *self != before
    }

    /// Link established on a node.
    pub fn connected(&mut self) {
        self.listening = true;
    }

    /// Link lost: held keys and the Fn-lock byte are no longer known;
    /// counters and timestamps are history and stay.
    pub fn disconnected(&mut self) {
        self.listening = false;
        self.eject_pressed = false;
        self.fn_pressed = false;
        self.media_bits = 0;
        self.fn_lock = None;
    }

    /// Stable JSON view (`akmctl status --json`, `passive` key). Unknown = null.
    pub fn to_json(&self) -> Value {
        let ts = |t: u64| (t != 0).then_some(t);
        json!({
            "listening": self.listening,
            "fn_lock": self.fn_lock,
            "last_sleep_event": ts(self.last_sleep_ts),
            "last_sleep_code": self.last_sleep_code,
            "wake_count": self.wake_count,
            "last_wake": ts(self.last_wake_ts),
            "last_wake_ready": ts(self.last_wake_ts).map(|_| self.last_wake_ready),
            "last_wake_conn_request": ts(self.last_wake_ts).map(|_| self.last_wake_conn_request),
            "eject_pressed": self.eject_pressed,
            "eject_count": self.eject_count,
            "fn_pressed": self.fn_pressed,
            "media_bits": self.media_bits,
            "battery_status": self.batt_stat,
        })
    }
}

// ── lock shared with the safe read policy ──────────────────────────────────

/// Is `lock` held by another reader (Feature requests in progress)? Probes
/// with a shared non-blocking `flock` and releases it at once.
pub fn lock_busy_at(lock: &Path) -> bool {
    let Ok(f) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(lock)
    else {
        return false; // no lock file: nobody has ever read
    };
    // SAFETY: valid fd owned by `f`.
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) };
    r != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK)
}

// ── listening ──────────────────────────────────────────────────────────────

/// Why a listening session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Peer gone (`POLLHUP`, EOF): the keyboard disconnected.
    Hangup,
    /// Read / poll error.
    Error,
    /// Asked to stop.
    Stopped,
    /// Another reader holds `hid.lock`: node released.
    Busy,
}

/// Open the node read-only and non-blocking. Never writable.
pub fn open_node(path: &Path) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
}

/// Longest wait of one `poll`: bounds the reaction time to stop / lock / gate.
const POLL_MS: libc::c_int = 2000;

/// Read reports from `fd` until it ends. `gate` is called after every wake-up
/// (report or 2 s timeout) and may end the session. `on_event` receives the
/// decoded reports; every report, decoded or not, only refreshes the
/// last-input timestamp.
pub fn listen_fd(
    fd: libc::c_int,
    poll_ms: libc::c_int,
    gate: &mut dyn FnMut() -> Option<Exit>,
    on_event: &mut dyn FnMut(PassiveEvent),
) -> Exit {
    let mut buf = [0u8; 64];
    loop {
        if let Some(e) = gate() {
            return e;
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let ret = unsafe { libc::poll(&mut pfd, 1, poll_ms) };
        if ret < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Exit::Error;
        }
        if ret == 0 {
            continue;
        }
        if pfd.revents & libc::POLLIN != 0 {
            // SAFETY: buf is valid for buf.len() bytes.
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n < 0 {
                match io::Error::last_os_error().raw_os_error() {
                    Some(libc::EAGAIN) | Some(libc::EINTR) => continue,
                    _ => return Exit::Error,
                }
            }
            if n == 0 {
                return Exit::Hangup;
            }
            crate::read_policy::note_input(); // timestamp only
            if let Some(ev) = decode(&buf[..n as usize]) {
                on_event(ev);
            }
            continue;
        }
        // POLLHUP / POLLERR / POLLNVAL without data: node gone (else poll
        // would return at once forever, 100 % CPU).
        return if pfd.revents & libc::POLLHUP != 0 {
            Exit::Hangup
        } else {
            Exit::Error
        };
    }
}

/// What the supervisor tells its sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// A node was opened.
    Connected(PathBuf),
    Event(PassiveEvent),
    /// The session ended (disconnection, lock taken by another reader, stop).
    Disconnected(Exit),
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Cross-process lock of the read policy.
    pub lock: PathBuf,
    /// Wait between two attempts to find / open the node.
    pub retry: Duration,
    /// `poll` timeout (reaction time to stop / lock).
    pub poll_ms: libc::c_int,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lock: crate::read_policy::lock_path(),
            retry: Duration::from_secs(5),
            poll_ms: POLL_MS,
        }
    }
}

/// Sleep `d` in slices, returning early (true) once `stop` is set.
fn sleep_or_stop(stop: &AtomicBool, d: Duration) -> bool {
    let end = std::time::Instant::now() + d;
    while std::time::Instant::now() < end {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20).min(d));
    }
    stop.load(Ordering::Relaxed)
}

/// Listen to the node `find` returns, forever, until `stop` is set: open,
/// listen until the link drops, wait `retry`, look for the (new) node again.
/// Blocking; run it on its own thread ([`spawn`]).
pub fn run(
    cfg: &Config,
    stop: &AtomicBool,
    find: &dyn Fn() -> Option<PathBuf>,
    sink: &mut dyn FnMut(Msg),
) {
    while !stop.load(Ordering::Relaxed) {
        if let Some(path) = find() {
            if !lock_busy_at(&cfg.lock) {
                if let Ok(file) = open_node(&path) {
                    sink(Msg::Connected(path));
                    let lock = cfg.lock.clone();
                    let mut gate = || {
                        if stop.load(Ordering::Relaxed) {
                            Some(Exit::Stopped)
                        } else if lock_busy_at(&lock) {
                            Some(Exit::Busy)
                        } else {
                            None
                        }
                    };
                    let exit = listen_fd(file.as_raw_fd(), cfg.poll_ms, &mut gate, &mut |ev| {
                        sink(Msg::Event(ev))
                    });
                    drop(file);
                    sink(Msg::Disconnected(exit));
                }
            }
        }
        if sleep_or_stop(stop, cfg.retry) {
            break;
        }
    }
}

/// Handle of the listener thread; dropping it stops the thread (within one
/// poll timeout).
pub struct Handle {
    stop: Arc<AtomicBool>,
}

impl Handle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Run [`run`] on a dedicated thread (`kb-passive`).
pub fn spawn(
    cfg: Config,
    find: impl Fn() -> Option<PathBuf> + Send + 'static,
    mut sink: impl FnMut(Msg) + Send + 'static,
) -> io::Result<Handle> {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    std::thread::Builder::new()
        .name("kb-passive".into())
        .spawn(move || run(&cfg, &s2, &find, &mut sink))?;
    Ok(Handle { stop })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn hex(s: &str) -> Vec<u8> {
        let c: String = s.split_whitespace().collect();
        (0..c.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&c[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn decodes_each_report() {
        assert_eq!(decode(&hex("04 00")), Some(PassiveEvent::Sleep { code: 0 }));
        assert_eq!(decode(&hex("04 01")), Some(PassiveEvent::Sleep { code: 1 }));
        assert_eq!(
            decode(&hex("05 02")),
            Some(PassiveEvent::FnLock { value: 2 })
        );
        assert_eq!(
            decode(&hex("30 00")),
            Some(PassiveEvent::BattStat { value: 0 })
        );
        assert_eq!(
            decode(&hex("13 01")),
            Some(PassiveEvent::Wake {
                ready: true,
                conn_request: false
            })
        );
        assert_eq!(
            decode(&hex("13 02")),
            Some(PassiveEvent::Wake {
                ready: false,
                conn_request: true
            })
        );
        assert_eq!(decode(&hex("13 00")), None);
        assert_eq!(decode(&hex("13 fc")), None);
        assert_eq!(
            decode(&hex("11 08")),
            Some(PassiveEvent::Keys {
                eject: true,
                fn_key: false
            })
        );
        assert_eq!(
            decode(&hex("11 10")),
            Some(PassiveEvent::Keys {
                eject: false,
                fn_key: true
            })
        );
        assert_eq!(
            decode(&hex("11 00")),
            Some(PassiveEvent::Keys {
                eject: false,
                fn_key: false
            })
        );
        assert_eq!(decode(&hex("12 05")), Some(PassiveEvent::Media { bits: 5 }));
        assert_eq!(decode(&hex("12 e5")), Some(PassiveEvent::Media { bits: 5 }));
    }

    #[test]
    fn never_decodes_keystrokes_or_vendor_secrets() {
        // 0x01 key press, 0x4C (host key material), 0xFE, 0x47, unknown, empty,
        // truncated.
        for f in [
            "01 00 00 04 00 00 00 00 00",
            "01 02 00 1a 1b 1c 00 00 00",
            "4c 03 00 00",
            "fe 01",
            "47 63",
            "99 99",
            "",
            "04",
            "11",
        ] {
            assert_eq!(decode(&hex(f)), None, "{f}");
        }
    }

    #[test]
    fn state_tracks_events_and_resets_on_disconnect() {
        let mut s = PassiveState::default();
        s.connected();
        assert!(s.apply(PassiveEvent::FnLock { value: 2 }, 10));
        assert!(
            !s.apply(PassiveEvent::FnLock { value: 2 }, 11),
            "same value"
        );
        assert!(s.apply(PassiveEvent::Sleep { code: 1 }, 20));
        assert!(s.apply(
            PassiveEvent::Wake {
                ready: true,
                conn_request: false
            },
            30
        ));
        assert!(s.apply(
            PassiveEvent::Keys {
                eject: true,
                fn_key: false
            },
            31
        ));
        assert!(s.apply(
            PassiveEvent::Keys {
                eject: false,
                fn_key: false
            },
            32
        ));
        assert!(s.apply(
            PassiveEvent::Keys {
                eject: true,
                fn_key: true
            },
            33
        ));
        assert_eq!((s.wake_count, s.eject_count, s.last_sleep_ts), (1, 2, 20));
        let j = s.to_json();
        assert_eq!(j["fn_lock"], 2);
        assert_eq!(j["last_sleep_event"], 20);
        assert_eq!(j["eject_pressed"], true);
        s.disconnected();
        assert!(!s.listening && !s.eject_pressed && !s.fn_pressed);
        assert_eq!(s.fn_lock, None);
        assert_eq!((s.wake_count, s.last_sleep_ts), (1, 20), "history kept");
        let j = PassiveState::default().to_json();
        assert!(j["fn_lock"].is_null() && j["last_sleep_event"].is_null());
        assert!(j["last_wake_ready"].is_null());
    }

    fn pipe() -> (libc::c_int, libc::c_int) {
        let mut fds = [0; 2];
        // SAFETY: fds has room for 2 ints.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    }

    fn write_all(fd: libc::c_int, b: &[u8]) {
        // SAFETY: valid buffer / fd.
        assert_eq!(
            unsafe { libc::write(fd, b.as_ptr().cast(), b.len()) },
            b.len() as isize
        );
    }

    #[test]
    fn listen_fd_decodes_then_ends_on_hangup() {
        let (r, w) = pipe();
        write_all(w, &hex("05 02"));
        let mut got = vec![];
        // The first read drains "05 02"; the writer closes before the 2nd.
        let mut calls = 0;
        let mut gate = || {
            calls += 1;
            if calls == 2 {
                // after the first report: close the writer, then keep going
                // SAFETY: closing our own fd.
                unsafe { libc::close(w) };
            }
            None
        };
        let exit = listen_fd(r, 50, &mut gate, &mut |e| got.push(e));
        // SAFETY: closing our own fd.
        unsafe { libc::close(r) };
        assert_eq!(exit, Exit::Hangup);
        assert_eq!(got, vec![PassiveEvent::FnLock { value: 2 }]);
    }

    #[test]
    fn listen_fd_stops_without_spinning() {
        let (r, w) = pipe();
        let start = std::time::Instant::now();
        let mut n = 0;
        let mut gate = || {
            n += 1;
            (n > 3).then_some(Exit::Stopped)
        };
        let exit = listen_fd(r, 40, &mut gate, &mut |_| panic!("no report"));
        assert_eq!(exit, Exit::Stopped);
        // 3 idle polls of 40 ms: bounded by the poll timeout, not a busy loop.
        assert!(start.elapsed() >= Duration::from_millis(100));
        // SAFETY: closing our own fds.
        unsafe {
            libc::close(r);
            libc::close(w)
        };
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("akm-passive-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fifo(p: &Path) {
        let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
        // SAFETY: valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    }

    fn open_writer(p: &Path) -> File {
        std::fs::OpenOptions::new().write(true).open(p).unwrap()
    }

    #[test]
    fn supervisor_follows_disconnection_and_reconnection() {
        use std::io::Write;
        let d = tmpdir("sup");
        let node = d.join("hidraw0");
        fifo(&node);
        let cfg = Config {
            lock: d.join("hid.lock"),
            retry: Duration::from_millis(30),
            poll_ms: 30,
        };
        let (tx, rx) = mpsc::channel();
        let n2 = node.clone();
        let h = spawn(
            cfg,
            move || n2.exists().then(|| n2.clone()),
            move |m| {
                let _ = tx.send(m);
            },
        )
        .unwrap();
        let t = Duration::from_secs(5);
        // session 1
        assert_eq!(rx.recv_timeout(t).unwrap(), Msg::Connected(node.clone()));
        let mut w = open_writer(&node);
        w.write_all(&hex("11 08")).unwrap();
        assert_eq!(
            rx.recv_timeout(t).unwrap(),
            Msg::Event(PassiveEvent::Keys {
                eject: true,
                fn_key: false
            })
        );
        // a key press is dropped, nothing is emitted for it
        w.write_all(&hex("01 00 00 04 00 00 00 00 00")).unwrap();
        std::thread::sleep(Duration::from_millis(150)); // one report per read
        w.write_all(&hex("04 00")).unwrap();
        let mut seen = vec![];
        while let Ok(m) = rx.recv_timeout(Duration::from_millis(300)) {
            seen.push(m);
        }
        assert!(
            seen.contains(&Msg::Event(PassiveEvent::Sleep { code: 0 })),
            "{seen:?}"
        );
        assert!(seen
            .iter()
            .all(|m| matches!(m, Msg::Event(PassiveEvent::Sleep { .. }))));
        // disconnection: writer closed
        drop(w);
        assert_eq!(rx.recv_timeout(t).unwrap(), Msg::Disconnected(Exit::Hangup));
        // reconnection: a "new node" (same path, new writer)
        assert_eq!(rx.recv_timeout(t).unwrap(), Msg::Connected(node.clone()));
        let mut w = open_writer(&node);
        w.write_all(&hex("13 01")).unwrap();
        assert_eq!(
            rx.recv_timeout(t).unwrap(),
            Msg::Event(PassiveEvent::Wake {
                ready: true,
                conn_request: false
            })
        );
        drop(w);
        drop(h);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn supervisor_waits_without_node_and_yields_to_a_busy_lock() {
        let d = tmpdir("lock");
        let node = d.join("hidraw1");
        let lockp = d.join("hid.lock");
        let held = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lockp)
            .unwrap();
        // SAFETY: valid fd.
        assert_eq!(
            unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        assert!(lock_busy_at(&lockp));
        fifo(&node);
        let cfg = Config {
            lock: lockp.clone(),
            retry: Duration::from_millis(30),
            poll_ms: 30,
        };
        let (tx, rx) = mpsc::channel();
        let n2 = node.clone();
        let h = spawn(
            cfg,
            move || Some(n2.clone()),
            move |m| {
                let _ = tx.send(m);
            },
        )
        .unwrap();
        // Lock held: the node is not even opened.
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        drop(held);
        assert!(!lock_busy_at(&lockp));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Msg::Connected(node.clone())
        );
        // lock taken again mid-session: node released
        let held = std::fs::OpenOptions::new()
            .write(true)
            .open(&lockp)
            .unwrap();
        // SAFETY: valid fd.
        assert_eq!(
            unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Msg::Disconnected(Exit::Busy)
        );
        drop(held);
        drop(h);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn supervisor_does_not_spin_when_no_node() {
        let cfg = Config {
            lock: PathBuf::from("/nonexistent/hid.lock"),
            retry: Duration::from_millis(50),
            poll_ms: 30,
        };
        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c2 = calls.clone();
        let h = spawn(
            cfg,
            move || {
                c2.fetch_add(1, Ordering::Relaxed);
                None
            },
            |_| {},
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(520));
        drop(h);
        let n = calls.load(Ordering::Relaxed);
        assert!((5..=14).contains(&n), "{n} searches in 520 ms");
    }

    #[test]
    fn a1314_descriptor_declares_the_active_reports_only() {
        // 0x11/0x12/0x13 are declared Input; 0x04/0x05/0x30 are not (they
        // are only seen if the firmware ever sends them: #187).
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/a1314_iso/report_descriptor.hex");
        let txt = std::fs::read_to_string(p).unwrap();
        let rd = hex(&txt);
        let r = crate::discover::parse_rdesc(&rd);
        for id in [ID_EJECT_FN, ID_MEDIA, ID_WAKE] {
            assert!(r.has_input_report(id), "{id:#04x}");
        }
        for id in [ID_SLEEP, ID_FUNC_LOCK, ID_BATT_STAT] {
            assert!(!r.has_input_report(id), "{id:#04x}");
        }
    }
}
