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
//! that would be sent with its level of proof (established by disassembly,
//! E1). `--write-device-name` runs the guarded sequence of
//! [`akm_core::devname::run`], which writes only once THREE locks are lifted:
//! `[apple] allow_device_name_write = true` in `config.toml`; the outgoing
//! MTU of the L2CAP control channel, read here through
//! `pkexec akm-hid-control inspect --mac <MAC>` (read-only), ≥ 66; the name
//! typed again in the terminal. No D-Bus method, no window nor tray button can
//! reach this: only this interactive command.

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

/// Pre-flight facts from a snapshot (+ doctor verdict, terminal, MTU probe).
pub fn preflight_from(
    s: &Snapshot,
    doctor_green: bool,
    interactive: bool,
    control_mtu: Option<u16>,
) -> Preflight {
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
        control_mtu,
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

/// Lock 1, as read from `config.toml` (default false).
fn config_allows_write() -> bool {
    akm_core::config::load(&akm_core::config::default_path())
        .0
        .allow_device_name_write
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
    println!("Proof of the sequence: {}.", devname::SEQUENCE_PROOF.describe());
    println!("Settled by disassembly (docs/RE-NOM-PROPRE-E1.md):");
    for u in devname::RESOLVED_BY_DISASSEMBLY {
        println!("  - {u}");
    }
    println!("RISKS NOT MEASURED (no disassembly can give them; docs/RENOMMER-CLAVIER.md §5.3):");
    for u in devname::UNKNOWNS {
        println!("  - {u}");
    }
    println!("Experiments:");
    for e in devname::VALIDATION_EXPERIMENTS {
        println!("  - {e}");
    }
}

fn print_locks(allow: bool) {
    println!("Three locks, all required for a real write:");
    println!(
        "  1. configuration: [apple] allow_device_name_write = {} {}",
        allow,
        if allow {
            "(lifted)"
        } else {
            "(CLOSED, the default). To lift it, add to $XDG_CONFIG_HOME/apple-kb-monitor/config.toml (~/.config/...):\n       [apple]\n       allow_device_name_write = true"
        }
    );
    println!(
        "  2. outgoing MTU of the L2CAP control channel >= {}: read on the live socket at --write-device-name only (`pkexec akm-hid-control inspect --mac <MAC>`, administrator authentication, getsockopt read-only, nothing sent); unknown or smaller = refused",
        devname::MIN_CONTROL_MTU
    );
    println!("  3. the name typed again, exactly, in an interactive terminal");
}

fn print_preflight(p: &Preflight, dry_run: bool) {
    let fails: Vec<_> = devname::preflight(p)
        .into_iter()
        .filter(|f| !(dry_run && f.is_mtu()))
        .collect();
    if fails.is_empty() {
        println!("Pre-flight: ok");
    } else {
        for f in fails {
            println!("Pre-flight: FAILED - {}", f.describe());
        }
    }
    if dry_run {
        println!("Pre-flight: control-channel MTU not probed in a dry run (needs pkexec); probed and required >= {} at --write-device-name", devname::MIN_CONTROL_MTU);
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
    println!("\nNew name stored in the keyboard: {name:?}\nFrames Apple's rename sends (Lion 10.7.5 setDeviceName:, established by disassembly):");
    print!("{}", devname::render_frames(&frames));
    if write {
        return guarded(&Request::Rename(name.to_string()), mac);
    }
    // Dry run: pre-flight and backup of the current name, nothing written.
    println!("\nDRY RUN: nothing is written to the keyboard.");
    match snapshot() {
        Ok(s) => {
            print_preflight(
                &preflight_from(&s, doctor_green(mac.as_deref()), interactive(), None),
                true,
            );
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
    print_locks(config_allows_write());
    println!("\nThe real write needs --write-device-name and the three locks above.");
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
        println!("DRY RUN: nothing written. Add --write-device-name to restore (same protocol, same three locks, new confirmation).");
        print_proof_state();
        print_locks(config_allows_write());
        return EXIT_OK;
    }
    guarded(&Request::Restore(b), mac)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The real environment: daemon snapshot, doctor, terminal, MTU probe
/// (`pkexec akm-hid-control inspect`, once per command), backup dir, the
/// hidraw write door.
struct RealEnv {
    mac: Option<String>,
    door: Option<akm_core::hidraw::WriteDoor>,
    /// Result of the single MTU probe of this command (`None` = not run yet).
    mtu: Option<Result<u16, String>>,
}

/// A sink that refuses: used when the door cannot be opened.
struct Closed(String);

impl FeatureSink for Closed {
    fn set_feature(&self, _op: akm_core::registry::WriteOp, _report: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other(self.0.clone()))
    }
}

impl RealEnv {
    /// Lock 2: the outgoing MTU of the control channel, probed once (the
    /// second pre-flight call, just before the write, reuses the value).
    fn control_mtu(&mut self, snapshot_mac: Option<&str>) -> Option<u16> {
        if self.mtu.is_none() {
            let Some(mac) = self.mac.clone().or_else(|| snapshot_mac.map(str::to_string)) else {
                eprintln!("[devname] MTU probe skipped: the keyboard's MAC is unknown");
                self.mtu = Some(Err("MAC unknown".into()));
                return None;
            };
            eprintln!(
                "[devname] lock 2: reading the outgoing MTU of the L2CAP control channel to {mac} (pkexec akm-hid-control inspect --mac {mac}: read-only, nothing sent)"
            );
            let r = match crate::hid_control::inspect_control_mtu(&mac) {
                Ok((m, text)) => {
                    for l in text.lines().filter(|l| l.contains(" fd ") || l.contains("inspected")) {
                        eprintln!("[devname]   {}", l.trim());
                    }
                    eprintln!("[devname] control-channel outgoing MTU = {m} (required >= {})", devname::MIN_CONTROL_MTU);
                    Ok(m)
                }
                Err(e) => {
                    eprintln!("[devname] control-channel MTU unknown: {e}");
                    Err(e)
                }
            };
            self.mtu = Some(r);
        }
        self.mtu.as_ref().and_then(|r| r.as_ref().ok().copied())
    }
}

impl RenameEnv for RealEnv {
    fn preflight(&mut self) -> Preflight {
        match snapshot() {
            Ok(s) => {
                let mtu = self.control_mtu(s.mac());
                preflight_from(&s, doctor_green(self.mac.as_deref()), interactive(), mtu)
            }
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
                    if let Some(raw) = cached_raw(&s) {
                        // BlueZ's Device1.Name (what Apple would check through
                        // the HCI remote name), informative only.
                        let bluez_name = s
                            .keyboard
                            .as_ref()
                            .and_then(|k| k.device.name.clone());
                        return Reconnect::Back { raw, bluez_name };
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
    let mut env = RealEnv {
        mac,
        door: None,
        mtu: None,
    };
    let mut session = WriteSession::new();
    let o = devname::run(
        req,
        devname::SEQUENCE_PROOF,
        config_allows_write(),
        &mut session,
        &mut env,
    );
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
        Outcome::ConfigDisabled => {
            println!(
                "\nREFUSED (lock 1, configuration): [apple] allow_device_name_write is not true; nothing was touched (no pre-flight, no MTU probe, no backup, no confirmation, no write).\n\
                 To lift this lock, add to $XDG_CONFIG_HOME/apple-kb-monitor/config.toml (~/.config/apple-kb-monitor/config.toml):\n\n{}\n\n\
                 Read docs/RENOMMER-CLAVIER.md §5.3 and §6 first: the firmware's HANDSHAKE to SET 0x55 (U5) and persistence across a battery change (U3) are NOT measured.",
                devname::CONFIG_HOWTO
            );
            EXIT_ERROR
        }
        Outcome::Preflight(fails) if fails.iter().any(|f| f.is_mtu()) => {
            println!("\nREFUSED (lock 2, control-channel MTU): nothing was written (no backup, no confirmation).");
            for f in fails {
                println!("  - {}", f.describe());
            }
            println!("  The MTU is read on the live L2CAP socket by `pkexec akm-hid-control inspect --mac <MAC>` (read-only). If the helper did not report it, reinstall the package; if it is below {}, this keyboard cannot take the 66-byte frame in one piece from Linux.", devname::MIN_CONTROL_MTU);
            EXIT_ERROR
        }
        Outcome::Verified { backup, bluez_name } => {
            println!("\u{2713} Name written and read back identical (0x51-0x54).");
            if let Some(n) = bluez_name {
                println!("  BlueZ Device1.Name now: {n:?} (informative; BlueZ may still show its cached name until its next remote name request)");
            }
            if let Some(b) = backup {
                println!("  backup kept: {}", b.display());
            }
            EXIT_OK
        }
        Outcome::Mismatch { read, backup, bluez_name } => {
            println!(
                "\u{2717} The name read back differs: {}",
                devname::hex(read)
            );
            if let Some(n) = bluez_name {
                println!("  BlueZ Device1.Name now: {n:?}");
            }
            if let Some(b) = backup {
                println!(
                    "  Guided restore (same protocol, same three locks, new confirmation, new session):\n  akmctl rename --device-name --restore {} --write-device-name",
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
        let p = preflight_from(&s, true, true, Some(672));
        assert!(devname::preflight(&p).is_empty(), "{p:?}");
        assert_eq!(
            devname::name_from_raw(&cached_raw(&s).unwrap()).unwrap(),
            "Clavier de maria #1"
        );
        // MTU unknown or too small is a pre-flight failure of its own.
        let p = preflight_from(&s, true, true, None);
        assert_eq!(
            devname::preflight(&p),
            vec![devname::PreflightFail::ControlMtuUnknown]
        );
        let p = preflight_from(&s, true, true, Some(48));
        assert_eq!(
            devname::preflight(&p),
            vec![devname::PreflightFail::ControlMtuTooSmall(48)]
        );
        let mut s2 = s.clone();
        s2.keyboard.as_mut().unwrap().breaker_open = true;
        s2.keyboard.as_mut().unwrap().incomplete = true;
        let f = devname::preflight(&preflight_from(&s2, false, false, Some(672)));
        assert_eq!(f.len(), 4);
        assert!(cached_raw(&Snapshot::default()).is_none());
    }

    #[test]
    fn every_outcome_is_an_error_except_verified() {
        assert_eq!(
            report(&Outcome::Verified {
                backup: None,
                bluez_name: None
            }),
            EXIT_OK
        );
        for o in [
            Outcome::NotProven,
            Outcome::ConfigDisabled,
            Outcome::Preflight(vec![devname::PreflightFail::ControlMtuUnknown]),
            Outcome::Preflight(vec![devname::PreflightFail::ControlMtuTooSmall(48)]),
            Outcome::Preflight(vec![devname::PreflightFail::NotConnected]),
            Outcome::Cancelled,
            Outcome::NoReconnect,
            Outcome::NoCachedName,
            Outcome::Mismatch {
                read: vec![0; 32],
                backup: Some("/x".into()),
                bluez_name: Some("alex".into()),
            },
        ] {
            assert_eq!(report(&o), EXIT_ERROR, "{o:?}");
        }
    }

    #[test]
    fn this_command_uses_the_production_proof_and_the_configuration_lock_only() {
        let src = include_str!("devnamecmd.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(prod.contains("devname::SEQUENCE_PROOF,"));
        assert!(prod.contains("config_allows_write(),\n        &mut session"));
        assert!(!prod.contains(&["SequenceProof", "::"].concat()));
        // The MTU comes from the read-only `inspect` verb, never from a dry run
        // of a HID_CONTROL byte; and the only pkexec call is in hid_control.rs.
        assert!(prod.contains("inspect_control_mtu("));
        assert!(!prod.contains("HidControlOp"));
        assert!(!prod.contains("pkexec\""));
        // No fixed MTU is assumed anywhere in this command.
        assert!(!prod.contains("Some(672)") && !prod.contains("Some(66)"));
    }
}
