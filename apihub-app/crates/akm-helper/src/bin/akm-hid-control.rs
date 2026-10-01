//! `akm-hid-control` — HID_CONTROL SUSPEND (`0x13`) before the machine
//! sleeps, EXIT_SUSPEND (`0x14`) at wake, on the HIDP control channel of
//! `bluetoothd`, as macOS `bluetoothd` does (#244, `docs/VEILLE-HID.md`).
//!
//! ```text
//! akm-hid-control suspend      [--mac XX:XX:XX:XX:XX:XX] [--dry-run]
//! akm-hid-control exit-suspend [--mac XX:XX:XX:XX:XX:XX] [--dry-run]
//! ```
//!
//! Runs as root: from the system units `apple-kb-monitor-suspend.service` /
//! `apple-kb-monitor-resume.service`, or through `pkexec` (polkit action
//! `com.agenceapi.AppleKbMonitor.hid-control`, `akmctl hid-control`).
//! One byte per keyboard at most, never retried; `--dry-run` shows the socket
//! it would use (pid, fd, MAC, PSM, state) and writes nothing.
//! `/etc/apple-kb-monitor/hid-suspend.conf` `enabled = false` turns it off.
//!
//! Exit: 0 = done or nothing to do (no keyboard connected, disabled),
//! 1 = refused or failed for at least one keyboard, 64 = usage.

use std::path::Path;
use std::process::ExitCode;

use akm_helper::fsutil;
use akm_helper::hidctl::{self, Event, HidControl, L2capControlSocket, Mac};

const EX_USAGE: u8 = 64;
const USAGE: &str =
    "usage: akm-hid-control suspend|exit-suspend [--mac XX:XX:XX:XX:XX:XX] [--dry-run]";

#[derive(Debug, PartialEq, Eq)]
struct Args {
    cmd: HidControl,
    mac: Option<Mac>,
    dry_run: bool,
}

fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Args, String> {
    let a: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let (verb, mut rest) = a.split_first().ok_or(USAGE)?;
    let cmd = match *verb {
        "suspend" => HidControl::Suspend,
        "exit-suspend" => HidControl::ExitSuspend,
        v => return Err(format!("unknown command {v:?}\n{USAGE}")),
    };
    let mut out = Args {
        cmd,
        mac: None,
        dry_run: false,
    };
    while let Some((t, r)) = rest.split_first() {
        match *t {
            "--dry-run" if !out.dry_run => out.dry_run = true,
            "--mac" if out.mac.is_none() => {
                let (m, r2) = r.split_first().ok_or("--mac needs a value")?;
                out.mac = Some(Mac::parse(m)?);
                rest = r2;
                continue;
            }
            o => return Err(format!("unexpected argument {o:?}\n{USAGE}")),
        }
        rest = r;
    }
    Ok(out)
}

fn main() -> ExitCode {
    fsutil::lock_umask();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("akm-hid-control: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    // Under systemd stderr already is the journal; otherwise (pkexec) also syslog.
    let journal = std::env::var_os("JOURNAL_STREAM").is_some();
    let mut log = |e: Event| {
        let (warn, m) = match &e {
            Event::Info(m) => (false, m),
            Event::Warn(m) => (true, m),
        };
        if journal {
            eprintln!("<{}>{m}", if warn { 4 } else { 6 });
        } else {
            eprintln!("akm-hid-control: {m}");
            hidctl::syslog(warn, m);
        }
    };
    if unsafe { libc::geteuid() } != 0 {
        log(Event::Warn(
            "must run as root (system unit or pkexec)".into(),
        ));
        return ExitCode::from(1);
    }
    let enabled = match hidctl::read_config(Path::new(hidctl::CONFIG_PATH)) {
        Ok(b) => b,
        Err(e) => {
            log(Event::Warn(format!(
                "invalid configuration, feature OFF: {e}"
            )));
            false
        }
    };
    if !enabled && !args.dry_run {
        log(Event::Info(format!(
            "{} not sent: disabled ({})",
            args.cmd.name(),
            hidctl::CONFIG_PATH
        )));
        return ExitCode::SUCCESS;
    }
    if !enabled {
        log(Event::Info(format!(
            "disabled in {} (dry run continues)",
            hidctl::CONFIG_PATH
        )));
    }
    log(Event::Info(format!(
        "{} (0x{:02x}){}{}",
        args.cmd.name(),
        args.cmd.byte(),
        args.mac.map_or(String::new(), |m| format!(" for {m}")),
        if args.dry_run { " --dry-run" } else { "" }
    )));
    let rep = hidctl::run(
        args.cmd,
        args.mac,
        args.dry_run,
        &hidctl::Env::system(),
        &L2capControlSocket,
        &mut log,
    );
    if rep.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_are_strict() {
        assert_eq!(
            parse_args(&["suspend"]),
            Ok(Args {
                cmd: HidControl::Suspend,
                mac: None,
                dry_run: false
            })
        );
        let a = parse_args(&["exit-suspend", "--mac", "04:db:56:ca:42:ee", "--dry-run"]).unwrap();
        assert_eq!(a.cmd, HidControl::ExitSuspend);
        assert!(a.dry_run);
        assert_eq!(a.mac.unwrap().to_string(), "04:DB:56:CA:42:EE");
        assert!(parse_args(&["suspend", "--dry-run", "--mac", "04:DB:56:CA:42:EE"]).is_ok());
        for bad in [
            &[][..],
            &["0x13"],
            &["hard-reset"],
            &["control", "3"],
            &["suspend", "--byte", "0x11"],
            &["suspend", "--mac"],
            &["suspend", "--mac", "x"],
            &["suspend", "--dry-run", "--dry-run"],
            &[
                "suspend",
                "--mac",
                "04:DB:56:CA:42:EE",
                "--mac",
                "04:DB:56:CA:42:EE",
            ],
            &["suspend", "extra"],
            &["SUSPEND"],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }
}
