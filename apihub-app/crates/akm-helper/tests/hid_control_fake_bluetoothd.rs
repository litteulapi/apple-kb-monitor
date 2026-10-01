//! #244: a fake `bluetoothd` (child process) holds two AF_UNIX SEQPACKET
//! socketpair ends that play the HIDP control (PSM 0x11) and interrupt
//! (PSM 0x13) channels. The test proves that `pidfd_getfd` duplicates the
//! child's descriptor and that the peer of the control channel, and only it,
//! receives exactly one byte, 0x13 or 0x14; dry run and a wrong executable
//! write nothing. No Bluetooth socket and no real process is touched.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use akm_helper::hidctl::{
    run, ControlSocket, Env, Event, HidControl, KbOutcome, Mac, Refusal, SockInfo, AF_BLUETOOTH,
    BTPROTO_L2CAP,
};

const KB: &str = "AA:BB:CC:DD:EE:F1";

fn socketpair() -> (OwnedFd, OwnedFd) {
    let mut fds = [0 as RawFd; 2];
    // SAFETY: valid out-array; no CLOEXEC on purpose (the child inherits one end).
    assert_eq!(
        unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0, fds.as_mut_ptr()) },
        0
    );
    // SAFETY: fresh descriptors.
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

fn cloexec(fd: &OwnedFd, on: bool) {
    // SAFETY: plain fcntl on our own descriptor.
    unsafe {
        libc::fcntl(
            fd.as_raw_fd(),
            libc::F_SETFD,
            if on { libc::FD_CLOEXEC } else { 0 },
        )
    };
}

fn ino(fd: &OwnedFd) -> u64 {
    fs::metadata(format!("/proc/self/fd/{}", fd.as_raw_fd()))
        .unwrap()
        .ino()
}

/// Describes the child's socketpair ends as L2CAP channels to `KB`.
struct FakeL2cap {
    by_ino: HashMap<u64, u16>,
    /// `drained()` answers `Some(false)` this many times before `Some(true)`
    /// (`u32::MAX` = never drained).
    undrained: std::cell::Cell<u32>,
    /// Number of `drained()` calls.
    drain_polls: std::cell::Cell<u32>,
}

impl ControlSocket for FakeL2cap {
    fn inspect(&self, fd: BorrowedFd<'_>) -> io::Result<SockInfo> {
        let i = fs::metadata(format!("/proc/self/fd/{}", fd.as_raw_fd()))?.ino();
        Ok(match self.by_ino.get(&i) {
            Some(&psm) => SockInfo {
                domain: AF_BLUETOOTH,
                protocol: BTPROTO_L2CAP,
                sotype: libc::SOCK_SEQPACKET,
                peer: Some(Mac::parse(KB).unwrap()),
                local_psm: Some(psm),
                peer_psm: Some(psm),
                peer_cid: Some(0x40),
                hci_handle: Some(0x0b),
                omtu: Some(if psm == 0x0011 { 672 } else { 48 }),
                imtu: Some(672),
            },
            None => SockInfo {
                domain: libc::AF_UNIX,
                ..SockInfo::default()
            },
        })
    }
    fn drained(&self, _fd: BorrowedFd<'_>) -> Option<bool> {
        self.drain_polls.set(self.drain_polls.get() + 1);
        let n = self.undrained.get();
        if n == 0 {
            return Some(true);
        }
        if n != u32::MAX {
            self.undrained.set(n - 1);
        }
        Some(false)
    }
}

/// The remote socketpair ends are deliberately inheritable (no CLOEXEC), so a
/// child spawned by another test running at the same time would inherit
/// them: the fakes are built and torn down one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Fake {
    child: Child,
    ctrl_local: OwnedFd,
    intr_local: OwnedFd,
    cs: FakeL2cap,
    exe: String,
    hid_root: PathBuf,
    /// Fake `/run/user`: the daemon's published breaker state goes under
    /// `<run_user_root>/<uid>/apple-kb-monitor/breaker.state`.
    run_user_root: PathBuf,
    /// What the fake liveness check answers for the daemon.
    daemon_alive: std::cell::Cell<bool>,
    /// The active user of the seat as the fake logind reports it.
    active_uid: std::cell::Cell<Option<u32>>,
    /// The uid `bluetoothd` must run as.
    required_uid: std::cell::Cell<u32>,
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.hid_root);
        let _ = fs::remove_dir_all(&self.run_user_root);
    }
}

impl Fake {
    fn uid() -> u32 {
        // SAFETY: getuid never fails.
        unsafe { libc::getuid() }
    }

    /// Publish a breaker state as the daemon would (file owned by us, 0644).
    fn publish(&self, text: &str) {
        let p = akm_helper::breaker_state::path_for_uid(&self.run_user_root, Self::uid());
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, text).unwrap();
    }

    fn state(&self, open: bool, counter: u32, age_s: u64, mac: &str) -> String {
        akm_helper::breaker_state::BreakerState {
            mac: Some(mac.into()),
            open,
            counter,
            written_unix: akm_helper::breaker_state::now_unix().saturating_sub(age_s),
            pid: 4242,
            starttime: Some(1),
        }
        .render()
    }
}

fn spawn(tag: &str) -> Fake {
    let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (ctrl_local, ctrl_remote) = socketpair();
    let (intr_local, intr_remote) = socketpair();
    cloexec(&ctrl_local, true);
    cloexec(&intr_local, true);
    let by_ino = HashMap::from([
        (ino(&ctrl_remote), 0x0011u16),
        (ino(&intr_remote), 0x0013u16),
    ]);
    let child = Command::new("sleep")
        .arg("30")
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    // the parent copies are closed: only the child holds the remote ends now
    drop(ctrl_remote);
    drop(intr_remote);
    let exe = fs::read_link(format!("/proc/{}/exe", child.id()))
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let hid_root = std::env::temp_dir().join(format!("akm-hidctl-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&hid_root);
    let dev = hid_root.join("0005:05AC:0256.0001");
    fs::create_dir_all(&dev).unwrap();
    fs::write(
        dev.join("uevent"),
        format!(
            "HID_ID=0005:000005AC:00000256\nHID_NAME=A1314\nHID_UNIQ={}\n",
            KB.to_lowercase()
        ),
    )
    .unwrap();
    let run_user_root =
        std::env::temp_dir().join(format!("akm-hidctl-run-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&run_user_root);
    fs::create_dir_all(&run_user_root).unwrap();
    Fake {
        child,
        ctrl_local,
        intr_local,
        cs: FakeL2cap {
            by_ino,
            undrained: std::cell::Cell::new(0),
            drain_polls: std::cell::Cell::new(0),
        },
        exe,
        hid_root,
        run_user_root,
        daemon_alive: std::cell::Cell::new(false),
        active_uid: std::cell::Cell::new(Some(Fake::uid())),
        required_uid: std::cell::Cell::new(Fake::uid()),
        _serial: serial,
    }
}

fn recv_all(fd: &OwnedFd) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let mut b = [0u8; 64];
        // SAFETY: valid buffer.
        let n = unsafe {
            libc::recv(
                fd.as_raw_fd(),
                b.as_mut_ptr().cast(),
                b.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if n <= 0 {
            return out;
        }
        out.push(b[..n as usize].to_vec());
    }
}

fn go(
    f: &Fake,
    cmd: HidControl,
    dry: bool,
    exes: &[&str],
    mac: Option<&str>,
) -> (akm_helper::hidctl::Report, Vec<Event>) {
    let pid = f.child.id() as i32;
    let pidf = move || Ok(pid);
    let alive = |uid: u32, _w: Option<akm_helper::breaker_state::Writer>| {
        uid == Fake::uid() && f.daemon_alive.get()
    };
    let active = || f.active_uid.get();
    let env = Env {
        proc_root: PathBuf::from("/proc"),
        hid_root: f.hid_root.clone(),
        allowed_exes: exes,
        required_uid: f.required_uid.get(),
        daemon_pid: &pidf,
        run_user_root: f.run_user_root.clone(),
        daemon_alive: &alive,
        active_uid: &active,
    };
    let mut ev = Vec::new();
    let rep = run(
        cmd,
        mac.map(|m| Mac::parse(m).unwrap()),
        dry,
        &env,
        &f.cs,
        &mut |e| ev.push(e),
    );
    (rep, ev)
}

#[test]
fn pidfd_getfd_duplicates_and_peer_receives_exactly_one_byte() {
    for (cmd, byte) in [
        (HidControl::Suspend, 0x13u8),
        (HidControl::ExitSuspend, 0x14u8),
    ] {
        let f = spawn(&format!("{byte:x}"));
        let exe = f.exe.clone();
        // dry run: everything found, nothing written
        let (rep, ev) = go(&f, cmd, true, &[&exe], None);
        assert!(rep.ok(), "{ev:?}");
        assert_eq!(rep.per_keyboard[0].1, KbOutcome::DryRun);
        assert!(ev.iter().any(|e| matches!(e, Event::Info(m) if m.contains("NOT sent") && m.contains("psm 0x0011"))), "{ev:?}");
        assert!(recv_all(&f.ctrl_local).is_empty());
        // real run against the fake: one byte on the control channel only
        let (rep, ev) = go(&f, cmd, false, &[&exe], Some(KB));
        assert_eq!(rep.per_keyboard[0].1, KbOutcome::Sent, "{ev:?}");
        assert_eq!(rep.pid, Some(f.child.id() as i32));
        assert_eq!(
            recv_all(&f.ctrl_local),
            vec![vec![byte]],
            "exactly one 1-byte packet"
        );
        assert!(
            recv_all(&f.intr_local).is_empty(),
            "the interrupt channel (PSM 0x13) gets nothing"
        );
    }
}

#[test]
fn inspect_reports_the_control_channel_mtu_and_sends_nothing() {
    let f = spawn("inspect");
    let exe = f.exe.clone();
    let pid = f.child.id() as i32;
    let pidf = move || Ok(pid);
    let alive = |_: u32, _: Option<akm_helper::breaker_state::Writer>| false;
    let active = || Some(Fake::uid());
    let env = Env {
        proc_root: PathBuf::from("/proc"),
        hid_root: f.hid_root.clone(),
        allowed_exes: &[&exe],
        required_uid: Fake::uid(),
        daemon_pid: &pidf,
        run_user_root: f.run_user_root.clone(),
        daemon_alive: &alive,
        active_uid: &active,
    };
    let mut ev = Vec::new();
    let rep = akm_helper::hidctl::inspect(Some(Mac::parse(KB).unwrap()), &env, &f.cs, &mut |e| {
        ev.push(e)
    });
    assert!(rep.ok(), "{ev:?}");
    assert_eq!(
        rep.per_keyboard,
        vec![(Mac::parse(KB).unwrap(), KbOutcome::Inspected)]
    );
    // The per-socket line akmctl parses: control channel, connected, mtu out 672.
    let line = ev
        .iter()
        .find_map(|e| match e {
            Event::Info(m) if m.contains("peer 0x0011") && m.contains("fd ") => Some(m.clone()),
            _ => None,
        })
        .expect("control socket line");
    assert!(
        line.contains("state connected") && line.contains("mtu out 672 in 672"),
        "{line}"
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Info(m) if m.contains("inspected, nothing sent"))),
        "{ev:?}"
    );
    // The interrupt channel is described too (its own MTU), but never selected.
    assert!(
        ev.iter().any(
            |e| matches!(e, Event::Info(m) if m.contains("peer 0x0013") && m.contains("mtu out 48"))
        ),
        "{ev:?}"
    );
    assert!(
        recv_all(&f.ctrl_local).is_empty(),
        "nothing on the control channel"
    );
    assert!(recv_all(&f.intr_local).is_empty());
}

#[test]
fn wrong_exe_or_unknown_mac_writes_nothing() {
    let f = spawn("refuse");
    let (rep, _) = go(
        &f,
        HidControl::Suspend,
        false,
        &["/usr/lib/bluetooth/bluetoothd"],
        None,
    );
    assert!(matches!(rep.global, Some(Refusal::NotBluetoothd(_))));
    let exe = f.exe.clone();
    let (rep, _) = go(
        &f,
        HidControl::Suspend,
        false,
        &[&exe],
        Some("11:22:33:44:55:66"),
    );
    assert!(matches!(rep.global, Some(Refusal::MacNotInTable(_))));
    assert!(recv_all(&f.ctrl_local).is_empty());
    assert!(recv_all(&f.intr_local).is_empty());
}

/// Apple's R3 reaches the root helper (#251): with the daemon's breaker open
/// the control channel receives NOTHING and the unit still exits 0; with it
/// closed the byte goes out as before; a stale or unreadable state of a
/// running daemon is refused, that of a dead daemon is ignored.
#[test]
fn the_daemons_breaker_blocks_the_hid_control_byte() {
    let f = spawn("breaker");
    let exe = f.exe.clone();
    // 1. open, fresh -> refused, exit 0, nothing on either channel
    f.publish(&f.state(true, 3, 2, KB));
    f.daemon_alive.set(true);
    let (rep, ev) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert!(
        matches!(rep.per_keyboard[0].1, KbOutcome::BreakerOpen(_)),
        "{ev:?}"
    );
    assert!(rep.ok(), "nothing to do is not a failure");
    assert!(ev.iter().any(|e| matches!(e, Event::Warn(m) if m.contains("NOT sent") && m.contains("breaker open") && m.contains("3 requests"))), "{ev:?}");
    assert!(
        recv_all(&f.ctrl_local).is_empty(),
        "no byte while the breaker is open"
    );
    assert!(recv_all(&f.intr_local).is_empty());
    // an open breaker of a daemon that just died is still Apple's verdict
    f.daemon_alive.set(false);
    let (rep, _) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert!(matches!(rep.per_keyboard[0].1, KbOutcome::BreakerOpen(_)));
    assert!(recv_all(&f.ctrl_local).is_empty());
    // dry run: the verdict is logged, nothing sent either way
    f.daemon_alive.set(true);
    let (rep, ev) = go(&f, HidControl::Suspend, true, &[&exe], None);
    assert_eq!(rep.per_keyboard[0].1, KbOutcome::DryRun);
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Warn(m) if m.contains("breaker open"))),
        "{ev:?}"
    );
    assert!(recv_all(&f.ctrl_local).is_empty());
    // 2. closed, fresh -> sent (unchanged behaviour)
    f.publish(&f.state(false, 1, 2, KB));
    let (rep, ev) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert_eq!(rep.per_keyboard[0].1, KbOutcome::Sent, "{ev:?}");
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Info(m) if m.contains("breaker state Closed"))),
        "{ev:?}"
    );
    assert_eq!(recv_all(&f.ctrl_local), vec![vec![0x13u8]]);
    // 3. stale (> 60 s) while the daemon runs -> refused; daemon gone -> sent
    f.publish(&f.state(false, 0, 61, KB));
    let (rep, ev) = go(&f, HidControl::ExitSuspend, false, &[&exe], None);
    assert!(
        matches!(rep.per_keyboard[0].1, KbOutcome::BreakerOpen(_)),
        "{ev:?}"
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Warn(m) if m.contains("61 s old"))),
        "{ev:?}"
    );
    assert!(recv_all(&f.ctrl_local).is_empty());
    f.daemon_alive.set(false);
    let (rep, _) = go(&f, HidControl::ExitSuspend, false, &[&exe], None);
    assert_eq!(rep.per_keyboard[0].1, KbOutcome::Sent);
    assert_eq!(recv_all(&f.ctrl_local), vec![vec![0x14u8]]);
    // 4. unreadable state: refused with a live daemon, ignored without one
    f.daemon_alive.set(true);
    f.publish("schema=9\n");
    let (rep, ev) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert!(
        matches!(rep.per_keyboard[0].1, KbOutcome::BreakerOpen(_)),
        "{ev:?}"
    );
    assert!(recv_all(&f.ctrl_local).is_empty());
    f.daemon_alive.set(false);
    let (rep, _) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert_eq!(rep.per_keyboard[0].1, KbOutcome::Sent);
    assert_eq!(recv_all(&f.ctrl_local), vec![vec![0x13u8]]);
    // 5. the daemon follows another keyboard: this one is not concerned
    f.daemon_alive.set(true);
    f.publish(&f.state(true, 3, 1, "11:22:33:44:55:66"));
    let (rep, _) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert_eq!(rep.per_keyboard[0].1, KbOutcome::Sent);
    assert_eq!(recv_all(&f.ctrl_local), vec![vec![0x13u8]]);
    // 6. a state file under another uid's directory is never read (#256:
    //    only the active user's breaker counts)
    let foreign = akm_helper::breaker_state::path_for_uid(&f.run_user_root, Fake::uid() + 1);
    fs::create_dir_all(foreign.parent().unwrap()).unwrap();
    fs::write(&foreign, f.state(false, 0, 0, KB)).unwrap();
    f.publish(&f.state(false, 0, 0, KB));
    let (rep, ev) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert_eq!(
        rep.per_keyboard[0].1,
        KbOutcome::Sent,
        "the foreign uid is not the active user: never read; {ev:?}"
    );
    assert_eq!(recv_all(&f.ctrl_local), vec![vec![0x13u8]]);
    assert!(
        recv_all(&f.intr_local).is_empty(),
        "the interrupt channel never got anything"
    );
}

#[test]
fn two_control_candidates_write_nothing() {
    let mut f = spawn("ambiguous");
    // describe the interrupt end as a second control channel: ambiguous
    for v in f.cs.by_ino.values_mut() {
        *v = 0x0011;
    }
    let exe = f.exe.clone();
    let (rep, _) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert_eq!(
        rep.per_keyboard[0].1,
        KbOutcome::Refused(Refusal::Ambiguous(2))
    );
    assert!(!rep.ok());
    assert!(recv_all(&f.ctrl_local).is_empty());
    assert!(recv_all(&f.intr_local).is_empty());
}

/// #256: only the ACTIVE user's breaker counts. An open state in a directory
/// that is not the active user's is never read; no active user = fail closed;
/// a state dated in the future from a dead daemon no longer blocks.
#[test]
fn only_the_active_users_breaker_counts_and_future_states_expire() {
    let f = spawn("active");
    let exe = f.exe.clone();
    // open, fresh, daemon alive: but this uid is not the active user
    f.publish(&f.state(true, 3, 1, KB));
    f.daemon_alive.set(true);
    f.active_uid.set(Some(Fake::uid() + 1));
    let (rep, ev) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert_eq!(rep.per_keyboard[0].1, KbOutcome::Sent, "{ev:?}");
    assert_eq!(recv_all(&f.ctrl_local), vec![vec![0x13u8]]);
    // no active user known: nothing sent, exit 0
    f.active_uid.set(None);
    let (rep, ev) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert!(
        matches!(&rep.per_keyboard[0].1, KbOutcome::BreakerOpen(m) if m.contains("no active user")),
        "{ev:?}"
    );
    assert!(rep.ok());
    assert!(recv_all(&f.ctrl_local).is_empty());
    // a future-dated open state left by a dead daemon: the byte goes
    f.active_uid.set(Some(Fake::uid()));
    f.daemon_alive.set(false);
    f.publish(&format!(
        "schema=2\nmac={KB}\nopen=1\ncounter=3\nwritten_unix=999999999999\npid=4242\nstarttime=1\n"
    ));
    let (rep, ev) = go(&f, HidControl::ExitSuspend, false, &[&exe], None);
    assert_eq!(rep.per_keyboard[0].1, KbOutcome::Sent, "{ev:?}");
    assert_eq!(recv_all(&f.ctrl_local), vec![vec![0x14u8]]);
    // ... and is refused while its writer runs (fail closed)
    f.daemon_alive.set(true);
    let (rep, ev) = go(&f, HidControl::ExitSuspend, false, &[&exe], None);
    assert!(
        matches!(&rep.per_keyboard[0].1, KbOutcome::BreakerOpen(m) if m.contains("future")),
        "{ev:?}"
    );
    assert!(recv_all(&f.ctrl_local).is_empty());
}

/// #261: `bluetoothd` running under another uid than required is refused
/// (`WrongUid`) and nothing is written.
#[test]
fn bluetoothd_with_the_wrong_uid_is_refused() {
    let f = spawn("wronguid");
    let exe = f.exe.clone();
    f.required_uid.set(Fake::uid() + 1);
    let (rep, ev) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert!(!rep.ok(), "{ev:?}");
    assert_eq!(rep.global, Some(Refusal::WrongUid(Fake::uid())), "{ev:?}");
    assert!(rep.per_keyboard.is_empty());
    assert!(recv_all(&f.ctrl_local).is_empty());
    assert!(recv_all(&f.intr_local).is_empty());
}

/// #261: the wait for the queue to drain is observed: polled until drained,
/// and bounded by DRAIN_WAIT when it never drains.
#[test]
fn the_drain_wait_polls_until_drained_and_is_bounded() {
    let f = spawn("drain");
    let exe = f.exe.clone();
    f.cs.undrained.set(3);
    let (rep, ev) = go(&f, HidControl::Suspend, false, &[&exe], None);
    assert_eq!(rep.per_keyboard[0].1, KbOutcome::Sent, "{ev:?}");
    assert_eq!(f.cs.drain_polls.get(), 4, "3 x not drained, then drained");
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Info(m) if m.contains("queue drained"))),
        "{ev:?}"
    );
    assert_eq!(recv_all(&f.ctrl_local), vec![vec![0x13u8]]);
    f.cs.undrained.set(u32::MAX);
    f.cs.drain_polls.set(0);
    let t0 = std::time::Instant::now();
    let (rep, ev) = go(&f, HidControl::Suspend, false, &[&exe], None);
    let took = t0.elapsed();
    assert_eq!(rep.per_keyboard[0].1, KbOutcome::Sent, "{ev:?}");
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Warn(m) if m.contains("not drained after 1000 ms"))),
        "{ev:?}"
    );
    assert!(took >= akm_helper::hidctl::DRAIN_WAIT, "{took:?}");
    assert!(took < akm_helper::hidctl::DRAIN_WAIT * 3, "{took:?}");
    assert!(f.cs.drain_polls.get() > 1);
    assert_eq!(recv_all(&f.ctrl_local), vec![vec![0x13u8]]);
}
