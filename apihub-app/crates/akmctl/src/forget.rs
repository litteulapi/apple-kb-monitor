//! Clean forget of a CONNECTED keyboard, the way macOS 26.5 does it (#217),
//! used by `akmctl repair` only (never by D-Bus, the window or the tray).
//!
//! `bluetoothd` `FUN_1005a3f64` ("HID device ... will unpair"): if the device
//! is connected, `FUN_1005a41e4` sends, for a classic Apple HID such as the
//! A1314, a SET Feature of one byte `0x41` `RecantConnection` (wire `53 41`),
//! then waits **2000 ms** for the disconnection ("HID recanted
//! Successfully" / "HID timedout waiting to recant"), then the pairing is
//! removed (RE-GHIDRA-IOBLUETOOTH.md §5.2, [décompilé + listing]). The effect
//! of `0x41` on the keyboard itself (simple drop or forgetting this host) is
//! **not measured**.
//!
//! Order here, stopping at the first failure, never retrying, every byte and
//! decision logged: pre-flight (connected, link healthy, breaker closed,
//! interactive terminal) -> BACKUP of the host-side pairing facts (never a
//! link key) -> explanation + typed confirmation -> ONE `0x41` through the
//! register map (operation `Forget`, once per session) -> the daemon mutes
//! the disconnection notification -> 2000 ms -> link dropped or not ->
//! ONLY THEN `Adapter1.RemoveDevice`. If `0x41` fails (refused, keyboard
//! silent), nothing is removed: the caller goes back to wake + reconnect.

use std::path::PathBuf;
use std::time::Duration;

use akm_core::parity::FeatureSink;
use akm_core::registry::{Direction, WriteOp, WriteSession};
use serde::{Deserialize, Serialize};

/// `RecantConnection`.
pub const RECANT_ID: u8 = 0x41;
/// Wait for the link to drop after `0x41` [décompilé] (`bluetoothd`).
pub const RECANT_WAIT: Duration = Duration::from_millis(2000);

/// Host-side pairing facts saved before the forget. Never a link key: the
/// key stays in `/var/lib/bluetooth` (root) and is never read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetBackup {
    pub schema: u32,
    pub created_unix: u64,
    pub mac: String,
    pub name: String,
    pub alias: Option<String>,
    pub paired: bool,
    pub bonded: bool,
    pub trusted: bool,
    pub adapter_path: String,
    pub adapter_address: Option<String>,
    pub device_path: String,
}

impl ForgetBackup {
    pub const SCHEMA: u32 = 1;
}

/// Save a forget backup (0600, never overwritten) in `dir`.
pub fn write_backup(dir: &std::path::Path, b: &ForgetBackup) -> std::io::Result<PathBuf> {
    let json = serde_json::to_string_pretty(b).map_err(std::io::Error::other)?;
    akm_core::devname::write_private_file(
        dir,
        &akm_core::devname::backup_file_name("forget", b.created_unix),
        &json,
    )
}

/// Facts checked before the forget.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preflight {
    pub connected: bool,
    /// `akmctl doctor`: daemon link health `connected`, no KO finding.
    pub link_healthy: bool,
    pub breaker_open: bool,
    /// stdin and stdout are a terminal.
    pub interactive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightFail {
    NotConnected,
    LinkNotHealthy,
    BreakerOpen,
    NotInteractive,
}

impl PreflightFail {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::NotConnected => "keyboard not connected",
            Self::LinkNotHealthy => {
                "link not healthy (`akmctl doctor`: health must be `connected`, no KO)"
            }
            Self::BreakerOpen => "circuit breaker open: the keyboard stopped answering",
            Self::NotInteractive => "not an interactive terminal",
        }
    }
}

pub fn preflight(p: &Preflight) -> Vec<PreflightFail> {
    let mut v = Vec::new();
    if !p.connected {
        v.push(PreflightFail::NotConnected);
    }
    if !p.link_healthy {
        v.push(PreflightFail::LinkNotHealthy);
    }
    if p.breaker_open {
        v.push(PreflightFail::BreakerOpen);
    }
    if !p.interactive {
        v.push(PreflightFail::NotInteractive);
    }
    v
}

/// The outside world (simulated in the tests).
pub trait ForgetEnv {
    fn preflight(&mut self) -> Preflight;
    fn save_backup(&mut self) -> Result<PathBuf, String>;
    /// Show what will happen and how to re-pair, read the typed word.
    fn explain_and_confirm(&mut self) -> bool;
    /// Open the hardware door (after the confirmation only).
    fn open_door(&mut self) -> Result<(), String>;
    fn sink(&self) -> &dyn FeatureSink;
    /// Ask the daemon to mute the disconnection notification.
    fn expect_disconnect(&mut self);
    fn sleep(&mut self, d: Duration);
    fn connected(&mut self) -> bool;
    fn remove_device(&mut self) -> Result<(), String>;
    fn log(&mut self, line: &str);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing written, nothing removed.
    Preflight(Vec<PreflightFail>),
    BackupFailed(String),
    Cancelled,
    /// The door could not be opened or the map refused: nothing sent, nothing removed.
    NotSent(String),
    /// `0x41` failed (refused by the keyboard, silent): nothing removed.
    RecantFailed(String),
    /// `0x41` sent, pairing removed.
    Removed {
        link_dropped: bool,
        backup: PathBuf,
    },
    /// `0x41` sent, but BlueZ refused the removal.
    RemoveFailed(String),
}

pub fn run(session: &mut WriteSession, env: &mut dyn ForgetEnv) -> Outcome {
    let fails = preflight(&env.preflight());
    if !fails.is_empty() {
        for f in &fails {
            env.log(&format!("[forget] pre-flight failed: {}", f.describe()));
        }
        return Outcome::Preflight(fails);
    }
    let backup = match env.save_backup() {
        Ok(p) => {
            env.log(&format!(
                "[forget] backup written: {} (0600, no link key)",
                p.display()
            ));
            p
        }
        Err(e) => {
            env.log(&format!("[forget] decision: stop, backup failed: {e}"));
            return Outcome::BackupFailed(e);
        }
    };
    if !env.explain_and_confirm() {
        env.log("[forget] decision: cancelled (typed confirmation does not match); nothing written, nothing removed");
        return Outcome::Cancelled;
    }
    if let Err(e) = env.open_door() {
        env.log(&format!(
            "[forget] decision: stop, {e}; nothing written, nothing removed"
        ));
        return Outcome::NotSent(e);
    }
    if let Err(e) = session.authorize(WriteOp::Forget, RECANT_ID, Direction::Feature, &[]) {
        env.log(&format!("[forget] decision: stop, {e}; nothing removed"));
        return Outcome::NotSent(e.to_string());
    }
    env.log("[forget] write Forget Feature 0x41, 1 byte: 41 (wire 53 41)");
    if let Err(e) = env.sink().set_feature(WriteOp::Forget, &[RECANT_ID]) {
        env.log(&format!(
            "[forget] decision: RecantConnection not accepted ({e}); NOTHING removed, back to wake + reconnect, no new attempt"
        ));
        return Outcome::RecantFailed(e.to_string());
    }
    env.expect_disconnect();
    env.log("[forget] daemon told to mute the disconnection; waiting 2000 ms for the link to drop");
    env.sleep(RECANT_WAIT);
    let link_dropped = !env.connected();
    env.log(if link_dropped {
        "[forget] link dropped (HID recanted successfully)"
    } else {
        "[forget] link still up after 2000 ms (Apple: timed out waiting to recant, goes on)"
    });
    match env.remove_device() {
        Ok(()) => {
            env.log("[forget] pairing removed (Adapter1.RemoveDevice)");
            Outcome::Removed {
                link_dropped,
                backup,
            }
        }
        Err(e) => {
            env.log(&format!(
                "[forget] decision: stop, RemoveDevice failed: {e}"
            ));
            Outcome::RemoveFailed(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io;

    #[derive(Default)]
    struct Sim {
        pre: Preflight,
        typed_ok: bool,
        backup_fails: bool,
        door_fails: bool,
        write_fails: bool,
        still_connected: bool,
        remove_fails: bool,
        events: RefCell<Vec<String>>,
        writes: RefCell<Vec<(WriteOp, Vec<u8>)>>,
        slept: Vec<Duration>,
    }

    impl FeatureSink for Sim {
        fn set_feature(&self, op: WriteOp, report: &[u8]) -> io::Result<()> {
            self.writes.borrow_mut().push((op, report.to_vec()));
            self.events
                .borrow_mut()
                .push(format!("write {}", akm_core::devname::hex(report)));
            if self.write_fails {
                Err(io::Error::from_raw_os_error(libc::ETIMEDOUT))
            } else {
                Ok(())
            }
        }
    }

    impl ForgetEnv for Sim {
        fn preflight(&mut self) -> Preflight {
            self.pre.clone()
        }
        fn save_backup(&mut self) -> Result<PathBuf, String> {
            self.events.borrow_mut().push("backup".into());
            if self.backup_fails {
                Err("disk full".into())
            } else {
                Ok("/sim/forget-backup.json".into())
            }
        }
        fn explain_and_confirm(&mut self) -> bool {
            self.events.borrow_mut().push("confirm".into());
            self.typed_ok
        }
        fn open_door(&mut self) -> Result<(), String> {
            self.events.borrow_mut().push("door".into());
            if self.door_fails {
                Err("no hidraw".into())
            } else {
                Ok(())
            }
        }
        fn sink(&self) -> &dyn FeatureSink {
            self
        }
        fn expect_disconnect(&mut self) {
            self.events.borrow_mut().push("mute".into());
        }
        fn sleep(&mut self, d: Duration) {
            self.slept.push(d);
            self.events
                .borrow_mut()
                .push(format!("sleep {}", d.as_millis()));
        }
        fn connected(&mut self) -> bool {
            self.still_connected
        }
        fn remove_device(&mut self) -> Result<(), String> {
            self.events.borrow_mut().push("remove".into());
            if self.remove_fails {
                Err("org.bluez.Error.Failed".into())
            } else {
                Ok(())
            }
        }
        fn log(&mut self, _line: &str) {}
    }

    fn go() -> Sim {
        Sim {
            pre: Preflight {
                connected: true,
                link_healthy: true,
                breaker_open: false,
                interactive: true,
            },
            typed_ok: true,
            ..Sim::default()
        }
    }

    fn ev(s: &Sim) -> Vec<String> {
        s.events.borrow().clone()
    }

    #[test]
    fn apple_sequence_once_after_backup_and_confirmation() {
        let mut s = go();
        let o = run(&mut WriteSession::new(), &mut s);
        assert_eq!(
            o,
            Outcome::Removed {
                link_dropped: true,
                backup: "/sim/forget-backup.json".into()
            }
        );
        assert_eq!(
            ev(&s),
            [
                "backup",
                "confirm",
                "door",
                "write 41",
                "mute",
                "sleep 2000",
                "remove"
            ]
        );
        assert_eq!(*s.writes.borrow(), vec![(WriteOp::Forget, vec![0x41])]);
        assert_eq!(s.slept, vec![RECANT_WAIT]);
    }

    #[test]
    fn link_still_up_after_2_s_goes_on_like_apple() {
        let mut s = go();
        s.still_connected = true;
        assert!(matches!(
            run(&mut WriteSession::new(), &mut s),
            Outcome::Removed {
                link_dropped: false,
                ..
            }
        ));
    }

    #[test]
    fn a_failed_recant_removes_nothing_and_is_not_retried() {
        let mut s = go();
        s.write_fails = true;
        assert!(matches!(
            run(&mut WriteSession::new(), &mut s),
            Outcome::RecantFailed(_)
        ));
        assert_eq!(s.writes.borrow().len(), 1);
        assert!(!ev(&s).iter().any(|e| e == "remove" || e == "mute"));
    }

    #[test]
    fn refusals_write_and_remove_nothing() {
        type Setup = fn(&mut Sim);
        let cases: [(Setup, &str); 7] = [
            (|s| s.pre.connected = false, "pre"),
            (|s| s.pre.link_healthy = false, "pre"),
            (|s| s.pre.breaker_open = true, "pre"),
            (|s| s.pre.interactive = false, "pre"),
            (|s| s.backup_fails = true, "backup"),
            (|s| s.typed_ok = false, "cancel"),
            (|s| s.door_fails = true, "door"),
        ];
        for (setup, what) in cases {
            let mut s = go();
            setup(&mut s);
            let o = run(&mut WriteSession::new(), &mut s);
            assert!(s.writes.borrow().is_empty(), "{what}: {o:?}");
            assert!(!ev(&s).iter().any(|e| e == "remove"), "{what}");
            // Never a write before the backup and the confirmation.
            if what == "pre" {
                assert!(ev(&s).is_empty());
            }
        }
        // Backup failure stops before the confirmation.
        let mut s = go();
        s.backup_fails = true;
        assert!(matches!(
            run(&mut WriteSession::new(), &mut s),
            Outcome::BackupFailed(_)
        ));
        assert_eq!(ev(&s), ["backup"]);
    }

    #[test]
    fn once_per_session() {
        let mut session = WriteSession::new();
        let mut s = go();
        assert!(matches!(run(&mut session, &mut s), Outcome::Removed { .. }));
        let mut s2 = go();
        assert!(
            matches!(run(&mut session, &mut s2), Outcome::NotSent(ref e) if e.contains("already sent"))
        );
        assert!(s2.writes.borrow().is_empty());
        assert!(!ev(&s2).iter().any(|e| e == "remove"));
    }

    #[test]
    fn remove_failure_is_reported() {
        let mut s = go();
        s.remove_fails = true;
        assert!(matches!(
            run(&mut WriteSession::new(), &mut s),
            Outcome::RemoveFailed(_)
        ));
    }

    #[test]
    fn backup_never_holds_a_key_and_is_private() {
        let b = ForgetBackup {
            schema: ForgetBackup::SCHEMA,
            created_unix: 0,
            mac: "04:DB:56:CA:42:EE".into(),
            name: "Clavier de maria #1".into(),
            alias: Some("Clavier de maria #1".into()),
            paired: true,
            bonded: true,
            trusted: true,
            adapter_path: "/org/bluez/hci0".into(),
            adapter_address: Some("00:11:22:33:44:55".into()),
            device_path: "/org/bluez/hci0/dev_04_DB_56_CA_42_EE".into(),
        };
        let json = serde_json::to_string(&b).unwrap().to_lowercase();
        for k in ["key", "ltk", "irk", "secret"] {
            assert!(!json.contains(k), "{k}");
        }
        let dir = std::env::temp_dir().join(format!("akm-forget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = write_backup(&dir, &b).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(p
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("forget-backup-"));
        assert!(write_backup(&dir, &b).is_err(), "never overwritten");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn forget_is_reachable_from_repair_only() {
        // No other source of the workspace names the operation or this module.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut stack = vec![
            root.join("akm-core/src"),
            root.join("apple-kb-monitord/src"),
            root.join("crates"),
            root.join("src"),
        ];
        let needle_op = ["WriteOp", "::Forget"].concat();
        let needle_run = ["forget", "::run("].concat();
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if !p.ends_with("target") {
                        stack.push(p);
                    }
                    continue;
                }
                if p.extension().is_none_or(|x| x != "rs") {
                    continue;
                }
                let t = std::fs::read_to_string(&p).unwrap();
                let code: String = t
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let allowed_op = p.ends_with("crates/akmctl/src/forget.rs")
                    || p.ends_with("akm-core/src/registry.rs")
                    || p.ends_with("akm-core/src/hidraw.rs");
                if code.contains(&needle_op) {
                    assert!(allowed_op, "{} names the Forget operation", p.display());
                }
                if code.contains(&needle_run) {
                    assert!(
                        p.ends_with("crates/akmctl/src/repair.rs"),
                        "{} calls forget::run",
                        p.display()
                    );
                }
            }
        }
    }
}
