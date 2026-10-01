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

const KB: &str = "04:DB:56:CA:42:EE";

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
            },
            None => SockInfo {
                domain: libc::AF_UNIX,
                ..SockInfo::default()
            },
        })
    }
    fn drained(&self, _fd: BorrowedFd<'_>) -> Option<bool> {
        Some(true)
    }
}

struct Fake {
    child: Child,
    ctrl_local: OwnedFd,
    intr_local: OwnedFd,
    cs: FakeL2cap,
    exe: String,
    hid_root: PathBuf,
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.hid_root);
    }
}

fn spawn(tag: &str) -> Fake {
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
    Fake {
        child,
        ctrl_local,
        intr_local,
        cs: FakeL2cap { by_ino },
        exe,
        hid_root,
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
    let env = Env {
        proc_root: PathBuf::from("/proc"),
        hid_root: f.hid_root.clone(),
        allowed_exes: exes,
        // SAFETY: getuid never fails.
        required_uid: unsafe { libc::getuid() },
        daemon_pid: &pidf,
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
