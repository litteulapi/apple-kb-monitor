//! `akm-hid-control` — HID_CONTROL SUSPEND (`0x13`) before the machine
//! sleeps, EXIT_SUSPEND (`0x14`) at wake, on the HIDP control channel of
//! `bluetoothd`, as macOS `bluetoothd` does (#244, `docs/VEILLE-HID.md`).
//!
//! ```text
//! akm-hid-control suspend      [--mac XX:XX:XX:XX:XX:XX] [--dry-run]
//! akm-hid-control exit-suspend [--mac XX:XX:XX:XX:XX:XX] [--dry-run]
//! akm-hid-control inspect      [--mac XX:XX:XX:XX:XX:XX]
//! ```
//!
//! Runs as root: from the system units `apple-kb-monitor-suspend.service` /
//! `apple-kb-monitor-resume.service`, or through `pkexec` (polkit action
//! `com.agenceapi.AppleKbMonitor.hid-control`, `akmctl hid-control`).
//! One byte per keyboard at most, never retried; `--dry-run` shows the socket
//! it would use (pid, fd, MAC, PSM, state) and writes nothing. `inspect`
//! only describes the control channel, with its negotiated L2CAP MTUs
//! (`getsockopt(L2CAP_OPTIONS)`, read-only): the pre-flight of
//! `akmctl rename --device-name --write-device-name` reads the outgoing MTU
//! there (#248). `/etc/apple-kb-monitor/hid-suspend.conf` `enabled = false`
//! turns the sending off (`inspect` and `--dry-run` still run).
//!
//! Exit: 0 = done or nothing to do (no keyboard connected, disabled),
//! 1 = refused or failed for at least one keyboard, 64 = usage.

use std::path::Path;
use std::process::ExitCode;

use akm_helper::fsutil;
use akm_helper::hidctl::{self, Action, Event, HidControl, L2capControlSocket, Mac};

const EX_USAGE: u8 = 64;
const USAGE: &str =
    "usage: akm-hid-control suspend|exit-suspend [--mac XX:XX:XX:XX:XX:XX] [--dry-run]\n       akm-hid-control inspect [--mac XX:XX:XX:XX:XX:XX]";

#[derive(Debug, PartialEq, Eq)]
struct Args {
    action: Action,
    mac: Option<Mac>,
    dry_run: bool,
}

fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Args, String> {
    let a: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let (verb, mut rest) = a.split_first().ok_or(USAGE)?;
    let action = match *verb {
        "suspend" => Action::Send(HidControl::Suspend),
        "exit-suspend" => Action::Send(HidControl::ExitSuspend),
        "inspect" => Action::Inspect,
        v => return Err(format!("unknown command {v:?}\n{USAGE}")),
    };
    let mut out = Args {
        action,
        mac: None,
        // `inspect` never sends: it is a dry run by construction.
        dry_run: action == Action::Inspect,
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
            args.action.name(),
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
    let rep = match args.action {
        Action::Inspect => {
            log(Event::Info(format!(
                "inspect (read-only){}",
                args.mac.map_or(String::new(), |m| format!(" for {m}"))
            )));
            hidctl::inspect(args.mac, &hidctl::Env::system(), &L2capControlSocket, &mut log)
        }
        Action::Send(cmd) => {
            log(Event::Info(format!(
                "{} (0x{:02x}){}{}",
                cmd.name(),
                cmd.byte(),
                args.mac.map_or(String::new(), |m| format!(" for {m}")),
                if args.dry_run { " --dry-run" } else { "" }
            )));
            hidctl::run(
                cmd,
                args.mac,
                args.dry_run,
                &hidctl::Env::system(),
                &L2capControlSocket,
                &mut log,
            )
        }
    };
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
                action: Action::Send(HidControl::Suspend),
                mac: None,
                dry_run: false
            })
        );
        let a = parse_args(&["exit-suspend", "--mac", "aa:bb:cc:dd:ee:f1", "--dry-run"]).unwrap();
        assert_eq!(a.action, Action::Send(HidControl::ExitSuspend));
        assert!(a.dry_run);
        assert_eq!(a.mac.unwrap().to_string(), "AA:BB:CC:DD:EE:F1");
        assert!(parse_args(&["suspend", "--dry-run", "--mac", "AA:BB:CC:DD:EE:F1"]).is_ok());
        // inspect: read-only by construction (dry run), with or without --mac.
        let a = parse_args(&["inspect", "--mac", "AA:BB:CC:DD:EE:F1"]).unwrap();
        assert_eq!(a.action, Action::Inspect);
        assert!(a.dry_run);
        assert_eq!(parse_args(&["inspect"]).unwrap().action, Action::Inspect);
        assert!(parse_args(&["inspect", "--dry-run"]).is_err(), "already implied");
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
                "AA:BB:CC:DD:EE:F1",
                "--mac",
                "AA:BB:CC:DD:EE:F1",
            ],
            &["suspend", "extra"],
            &["SUSPEND"],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }
}
