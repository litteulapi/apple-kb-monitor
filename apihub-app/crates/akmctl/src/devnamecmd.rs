//! `akmctl rename --device-name ...`: the name stored IN the keyboard (#248).
//!
//! Two names exist (docs/RENOMMER-CLAVIER.md):
//! * `akmctl rename <nom>`: alias of THIS computer (BlueZ `Alias`), the
//!   default, risk-free, unchanged;
//! * `akmctl rename --device-name ...`: the keyboard's own name, stored in its
//!   firmware (`0x51`-`0x54` read, `0x55` written by Lion), seen by every host.
//!
//! `--show` reads the daemon's cache (no hardware request). `--dry-run` (the
//! default without `--write-device-name`) validates the name, shows the
//! pre-flight, saves the current name (backup 0600) and prints every byte
//! that would be sent with its level of proof. `--write-device-name` runs the
//! guarded sequence of [`akm_core::devname::run`], which today REFUSES the real
//! write (`NotProven`) before touching anything. No D-Bus method, no window
//! nor tray button can reach this: only this interactive command.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use akm_core::devname::{self, Backup, Outcome, Preflight, Reconnect, RenameEnv, Request};
use akm_core::parity::FeatureSink;
use akm_core::registry::WriteSession;
use akm_core::Snapshot;

use crate::bus;
use crate::cli::{EXIT_ABSENT, EXIT_ERROR, EXIT_OK};

/// What the user asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Show,
    Rename { name: String, write: bool },
    Restore { file: PathBuf, write: bool },
}

const TWO_NAMES: &str = "Two names exist:\n\
  * alias on THIS computer (BlueZ Alias): `akmctl rename <name>`, the default, no risk, nothing written into the keyboard;\n\
  * name stored IN the keyboard (firmware, reports 0x51-0x54 / 0x55), seen by every host: `akmctl rename --device-name`.\n";

fn snapshot() -> Result<Snapshot, (u8, String)> {
    let conn = bus::connect().map_err(|e| (EXIT_ABSENT, e.to_string()))?;
    bus::get_state(&conn).map_err(|e| match e {
        bus::BusError::Absent(m) => (EXIT_ABSENT, m),
        bus::BusError::Failed(m) => (EXIT_ERROR, m),
    })
}

/// The cached 32 bytes of `0x51`-`0x54` in a snapshot.
pub fn cached_raw(s: &Snapshot) -> Option<Vec<u8>> {
    s.keyboard
        .as_ref()
        .and_then(|k| k.device.name_on_keyboard_hex.as_deref())
        .and_then(devname::unhex)
        .filter(|r| r.len() == devname::MAX_NAME_LEN)
}

/// Pre-flight facts from a snapshot (+ doctor verdict, terminal).
pub fn preflight_from(s: &Snapshot, doctor_green: bool, interactive: bool) -> Preflight {
    let k = s.keyboard.as_ref();
    Preflight {
        connected: s.connected,
        battery_pct: s.battery_pct(),
        // Input 0x30 is only listened to by the daemon, not published: the
        // percentage decides (state `normal` stays unknown here).
        battery_state_normal: None,
        breaker_open: k.is_some_and(|k| k.breaker_open),
        recent_read_failure: k.is_some_and(|k| k.incomplete) || s.kb_error.is_some(),
        doctor_green,
        interactive,
    }
}

fn doctor_green(mac: Option<&str>) -> bool {
    let r = crate::doctor::gather(mac);
    r.verdict.0 <= crate::doctor::Level::Info
        && r.keyboard.as_ref().is_some_and(|k| k.connected)
        && r.health.as_deref() == Some("connected")
}

fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

pub fn run(action: Action, mac: Option<String>) -> u8 {
    match action {
        Action::Show => show(),
        Action::Rename { name, write } => rename(&name, write, mac),
        Action::Restore { file, write } => restore(&file, write, mac),
    }
}

fn show() -> u8 {
    let s = match snapshot() {
        Ok(s) => s,
        Err((code, m)) => {
            eprintln!("akmctl: {m}");
            return code;
        }
    };
    print!("{TWO_NAMES}");
    println!("Alias (this computer): {}", s.alias().unwrap_or("(none)"));
    match s
        .keyboard
        .as_ref()
        .and_then(|k| k.device.name_on_keyboard.clone())
    {
        Some(n) => {
            println!("Stored in the keyboard (0x51-0x54, daemon cache): {n}");
            if let Some(h) = cached_raw(&s) {
                println!("  bytes: {}", devname::hex(&h));
            }
            EXIT_OK
        }
        None => {
            println!("Stored in the keyboard: not read yet in this connection (the daemon reads 0x51-0x54 once per connection; no new hardware read is made here)");
            EXIT_ERROR
        }
    }
}

fn print_proof_state() {
    println!(
        "Proof of the sequence: {:?}. Unknowns (docs/RENOMMER-CLAVIER.md §5.3):",
        devname::SEQUENCE_PROOF
    );
    for u in devname::UNKNOWNS {
        println!("  - {u}");
    }
    println!("Passive experiments that would lift them:");
    for e in devname::VALIDATION_EXPERIMENTS {
        println!("  - {e}");
    }
}

fn print_preflight(p: &Preflight) {
    let fails = devname::preflight(p);
    if fails.is_empty() {
        println!("Pre-flight: ok");
    } else {
        for f in fails {
            println!("Pre-flight: FAILED - {}", f.describe());
        }
    }
}

fn rename(name: &str, write: bool, mac: Option<String>) -> u8 {
    print!("{TWO_NAMES}");
    let frames = match devname::prepare(name) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("akmctl: name refused: {e}");
            return EXIT_ERROR;
        }
    };
    println!("\nNew name stored in the keyboard: {name:?}\nFrames Apple's rename would send (Lion 10.7 setDeviceName:):");
    print!("{}", devname::render_frames(&frames));
    if write {
        return guarded(&Request::Rename(name.to_string()), mac);
    }
    // Dry run: pre-flight and backup of the current name, nothing written.
    println!("\nDRY RUN: nothing is written to the keyboard.");
    match snapshot() {
        Ok(s) => {
            print_preflight(&preflight_from(
                &s,
                doctor_green(mac.as_deref()),
                interactive(),
            ));
            match cached_raw(&s) {
                Some(raw) => {
                    let m = s.mac().unwrap_or("unknown").to_string();
                    match Backup::new(&m, &raw, now_unix(), "daemon-cache")
                        .map_err(|e| e.to_string())
                        .and_then(|b| devname::write_backup(&devname::state_dir(), &b).map_err(|e| e.to_string()))
                    {
                        Ok(p) => println!("Backup of the current name: {} (0600)", p.display()),
                        Err(e) => println!("Backup: FAILED - {e}"),
                    }
                }
                None => println!("Backup: not possible yet (the daemon has not read 0x51-0x54 in this connection)"),
            }
        }
        Err((_, m)) => println!("Pre-flight: daemon unavailable ({m})"),
    }
    print_proof_state();
    println!("\nThe real write would need --write-device-name, an interactive terminal and the name typed again; it is REFUSED while the proof is {:?}.", devname::SEQUENCE_PROOF);
    EXIT_OK
}

fn restore(file: &Path, write: bool, mac: Option<String>) -> u8 {
    let b = match devname::read_backup(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("akmctl: backup refused: {e}");
            return EXIT_ERROR;
        }
    };
    println!(
        "Backup {}: name {:?}, keyboard {}, saved at {}",
        file.display(),
        b.name,
        b.mac,
        devname::utc_stamp(b.created_unix)
    );
    match devname::frames_for_restore(&b) {
        Ok(f) => print!("{}", devname::render_frames(&f)),
        Err(e) => {
            eprintln!("akmctl: backup refused: {e}");
            return EXIT_ERROR;
        }
    }
    if !write {
        println!("DRY RUN: nothing written. Add --write-device-name to restore (same protocol, new confirmation).");
        print_proof_state();
        return EXIT_OK;
    }
    guarded(&Request::Restore(b), mac)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The real environment: daemon snapshot, doctor, terminal, backup dir, the
/// hidraw write door. Today never reaches the door (`NotProven`).
struct RealEnv {
    mac: Option<String>,
    door: Option<akm_core::hidraw::WriteDoor>,
}

/// A sink that refuses: used when the door cannot be opened.
struct Closed(String);

impl FeatureSink for Closed {
    fn set_feature(&self, _op: akm_core::registry::WriteOp, _report: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other(self.0.clone()))
    }
}

impl RenameEnv for RealEnv {
    fn preflight(&mut self) -> Preflight {
        match snapshot() {
            Ok(s) => preflight_from(&s, doctor_green(self.mac.as_deref()), interactive()),
            Err(_) => Preflight::default(),
        }
    }
    fn cached_raw(&mut self) -> Option<Vec<u8>> {
        snapshot().ok().as_ref().and_then(cached_raw)
    }
    fn mac(&mut self) -> String {
        snapshot()
            .ok()
            .and_then(|s| s.mac().map(str::to_string))
            .unwrap_or_else(|| "unknown".into())
    }
    fn now_unix(&mut self) -> u64 {
        now_unix()
    }
    fn save_backup(&mut self, b: &Backup) -> std::io::Result<PathBuf> {
        devname::write_backup(&devname::state_dir(), b)
    }
    fn confirm(&mut self, prompt: &str, expected: &str) -> bool {
        print!("{prompt}");
        let _ = std::io::stdout().flush();
        if !interactive() {
            println!("\nrefused: interactive confirmation required");
            return false;
        }
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).is_err() {
            return false;
        }
        let ok = line.trim_end_matches(['\n', '\r']) == expected;
        if ok {
            // Only now is the hardware node opened (under the HID lock).
            match akm_core::hidraw::WriteDoor::open() {
                Ok(d) => self.door = Some(d),
                Err(e) => eprintln!("[devname] write door not opened: {e}"),
            }
        }
        ok
    }
    fn sink(&self) -> &dyn FeatureSink {
        static CLOSED: std::sync::OnceLock<Closed> = std::sync::OnceLock::new();
        match &self.door {
            Some(d) => d,
            None => CLOSED.get_or_init(|| Closed("no hidraw write door: nothing written".into())),
        }
    }
    fn wait_reconnect(&mut self, max: Duration) -> Reconnect {
        // Release the HID lock first: the daemon must read the name again.
        self.door = None;
        println!(
            "\u{2192} Switch the keyboard OFF (power button, 3 s, light off), wait 5 s, switch it ON.\n  \
             The name is only read back after a reconnection. Waiting at most {} s...",
            max.as_secs()
        );
        let end = Instant::now() + max;
        let mut seen_down = false;
        while Instant::now() < end {
            if let Ok(s) = snapshot() {
                if !s.connected {
                    seen_down = true;
                } else if seen_down {
                    if let Some(r) = cached_raw(&s) {
                        return Reconnect::Back(r);
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        Reconnect::Timeout
    }
    fn log(&mut self, line: &str) {
        eprintln!("{line}");
    }
}

fn guarded(req: &Request, mac: Option<String>) -> u8 {
    let mut env = RealEnv { mac, door: None };
    let mut session = WriteSession::new();
    let o = devname::run(req, devname::SEQUENCE_PROOF, &mut session, &mut env);
    report(&o)
}

/// Text for the user and exit code of an outcome.
pub fn report(o: &Outcome) -> u8 {
    match o {
        Outcome::NotProven => {
            println!("\nREFUSED (NotProven): the exact bytes Apple sends to rename this keyboard are not established; nothing was touched (no backup, no confirmation, no write).");
            print_proof_state();
            EXIT_ERROR
        }
        Outcome::Verified { backup } => {
            println!("\u{2713} Name written and read back identical.");
            if let Some(b) = backup {
                println!("  backup kept: {}", b.display());
            }
            EXIT_OK
        }
        Outcome::Mismatch { read, backup } => {
            println!(
                "\u{2717} The name read back differs: {}",
                devname::hex(read)
            );
            if let Some(b) = backup {
                println!(
                    "  Guided restore (same protocol, new confirmation, new session):\n  akmctl rename --device-name --restore {} --write-device-name",
                    b.display()
                );
            }
            EXIT_ERROR
        }
        Outcome::NoReconnect => {
            println!("\u{2717} The keyboard did not come back in time. Nothing else is attempted. Switch it off and on, then `akmctl rename --device-name --show`.");
            EXIT_ERROR
        }
        other => {
            println!(
                "Stopped: {other:?}. {}",
                if other.wrote() {
                    "A frame may have been sent."
                } else {
                    "Nothing was written."
                }
            );
            EXIT_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akm_core::KbReport;

    #[test]
    fn preflight_from_a_snapshot() {
        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(99.0);
        k.device.name_on_keyboard_hex = Some(
            "436c617669657220".to_string()
                + "6465206d61726961"
                + "2023310000000000"
                + "0000000000000000",
        );
        let s = Snapshot {
            connected: true,
            keyboard: Some(k),
            ..Default::default()
        };
        let p = preflight_from(&s, true, true);
        assert!(devname::preflight(&p).is_empty(), "{p:?}");
        assert_eq!(
            devname::name_from_raw(&cached_raw(&s).unwrap()).unwrap(),
            "Clavier de maria #1"
        );
        let mut s2 = s.clone();
        s2.keyboard.as_mut().unwrap().breaker_open = true;
        s2.keyboard.as_mut().unwrap().incomplete = true;
        let f = devname::preflight(&preflight_from(&s2, false, false));
        assert_eq!(f.len(), 4);
        assert!(cached_raw(&Snapshot::default()).is_none());
    }

    #[test]
    fn every_outcome_is_an_error_except_verified() {
        assert_eq!(report(&Outcome::Verified { backup: None }), EXIT_OK);
        for o in [
            Outcome::NotProven,
            Outcome::Cancelled,
            Outcome::NoReconnect,
            Outcome::NoCachedName,
            Outcome::Mismatch {
                read: vec![0; 32],
                backup: Some("/x".into()),
            },
        ] {
            assert_eq!(report(&o), EXIT_ERROR);
        }
    }

    #[test]
    fn this_command_uses_the_production_proof_only() {
        let src = include_str!("devnamecmd.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(prod.contains("devname::SEQUENCE_PROOF, &mut session"));
        assert!(!prod.contains(&["SequenceProof", "::Proven"].concat()));
    }
}
