//! `akm-helper hid-control` — `HID_CONTROL` SUSPEND before the machine sleeps, `EXIT_SUSPEND` at wake, on the
//! HIDP control channel of `bluetoothd`, as macOS `bluetoothd` does.

use akm_helper::{tr, EX_USAGE};
use std::path::Path;
use std::process::ExitCode;

use akm_helper::hidctl::{self, Event, HidControl, L2capControlSocket, Mac};

const USAGE: &str =
    "usage: akm-helper hid-control suspend|exit-suspend [--mac XX:XX:XX:XX:XX:XX] [--dry-run]";

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
        v => {
            return Err(tr!(
                "unknown command {v}\n{USAGE}",
                v = format!("{:?}", v),
                USAGE = USAGE
            ))
        }
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
                let (m, r2) = r.split_first().ok_or_else(|| tr!("--mac needs a value"))?;
                out.mac = Some(Mac::parse(m)?);
                rest = r2;
                continue;
            }
            o => {
                return Err(tr!(
                    "unexpected argument {o}\n{USAGE}",
                    o = format!("{:?}", o),
                    USAGE = USAGE
                ))
            }
        }
        rest = r;
    }
    Ok(out)
}

pub fn run(cli: &[String]) -> ExitCode {
    let args = match parse_args(cli) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("akm-helper hid-control: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    let mut log = hidctl::event_logger("hid-control");
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
    let cmd = args.cmd;
    log(Event::Info(format!(
        "{} (0x{:02x}){}{}",
        cmd.name(),
        cmd.byte(),
        args.mac.map_or(String::new(), |m| format!(" for {m}")),
        if args.dry_run { " --dry-run" } else { "" }
    )));
    let rep = hidctl::run(
        cmd,
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
        let a = parse_args(&["exit-suspend", "--mac", "aa:bb:cc:dd:ee:f1", "--dry-run"]).unwrap();
        assert_eq!(a.cmd, HidControl::ExitSuspend);
        assert!(a.dry_run);
        assert_eq!(a.mac.unwrap().to_string(), "AA:BB:CC:DD:EE:F1");
        assert!(parse_args(&["suspend", "--dry-run", "--mac", "AA:BB:CC:DD:EE:F1"]).is_ok());
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
            &["inspect"],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }
}
