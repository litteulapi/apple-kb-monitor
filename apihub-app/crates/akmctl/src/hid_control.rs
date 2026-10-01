//! `akmctl hid-control suspend|exit-suspend [--mac MAC] [--dry-run]` (#244):
//! manual test of the byte the system units send at sleep (SUSPEND `0x13`)
//! and wake (EXIT_SUSPEND `0x14`). Runs `pkexec akm-hid-control` (polkit
//! action `com.agenceapi.AppleKbMonitor.hid-control`, administrator
//! authentication); the helper prints what it finds (bluetoothd pid, fd, MAC,
//! PSM, state) and, without `--dry-run`, sends the byte once.

use std::process::Command as Proc;

use clap::ValueEnum;

use crate::cli::{EXIT_ERROR, EXIT_OK, EXIT_USAGE};

/// Absolute paths: never resolved through `$PATH` (the helper runs as root).
const PKEXEC: &str = "/usr/bin/pkexec";
pub const HID_CONTROL_HELPER: &str = "/usr/lib/apple-kb-monitor/akm-hid-control";

/// The two operations; nothing else can be asked from the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HidControlOp {
    /// HID_CONTROL SUSPEND, byte 0x13 (what the suspend unit sends before sleep)
    Suspend,
    /// HID_CONTROL EXIT_SUSPEND, byte 0x14 (what the resume unit sends at wake)
    ExitSuspend,
}

/// Strict `XX:XX:XX:XX:XX:XX`, upper-cased (the helper checks it again).
fn check_mac(m: &str) -> Result<String, String> {
    let ok = m.len() == 17
        && m.bytes().enumerate().all(|(i, b)| {
            if i % 3 == 2 {
                b == b':'
            } else {
                b.is_ascii_hexdigit()
            }
        });
    ok.then(|| m.to_ascii_uppercase())
        .ok_or_else(|| format!("invalid MAC {m:?} (expected XX:XX:XX:XX:XX:XX)"))
}

/// Arguments of the helper.
pub fn helper_args(
    op: HidControlOp,
    mac: Option<&str>,
    dry_run: bool,
) -> Result<Vec<String>, String> {
    let mut a = vec![match op {
        HidControlOp::Suspend => "suspend".to_string(),
        HidControlOp::ExitSuspend => "exit-suspend".to_string(),
    }];
    if let Some(m) = mac {
        a.push("--mac".into());
        a.push(check_mac(m)?);
    }
    if dry_run {
        a.push("--dry-run".into());
    }
    Ok(a)
}

pub fn run(op: HidControlOp, mac: Option<&str>, dry_run: bool) -> u8 {
    let args = match helper_args(op, mac, dry_run) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("akmctl: {e}");
            return EXIT_USAGE;
        }
    };
    match Proc::new(PKEXEC)
        .arg(HID_CONTROL_HELPER)
        .args(&args)
        .status()
    {
        Ok(st) => match st.code() {
            Some(0) => EXIT_OK,
            Some(126) => {
                eprintln!("akmctl: authentication dismissed or not authorized");
                EXIT_ERROR
            }
            Some(127) => {
                eprintln!("akmctl: authentication failed or {HID_CONTROL_HELPER} not found");
                EXIT_ERROR
            }
            Some(c) => {
                eprintln!("akmctl: {HID_CONTROL_HELPER} refused or failed (exit {c}); details above and in journalctl -t akm-hid-control");
                EXIT_ERROR
            }
            None => {
                eprintln!("akmctl: {HID_CONTROL_HELPER} killed by a signal");
                EXIT_ERROR
            }
        },
        Err(e) => {
            eprintln!("akmctl: cannot run pkexec: {e}");
            EXIT_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_args_are_fixed() {
        assert_eq!(
            helper_args(HidControlOp::Suspend, None, false).unwrap(),
            ["suspend"]
        );
        assert_eq!(
            helper_args(HidControlOp::ExitSuspend, Some("04:db:56:ca:42:ee"), true).unwrap(),
            ["exit-suspend", "--mac", "04:DB:56:CA:42:EE", "--dry-run"]
        );
        for bad in [
            "",
            "04:db:56:ca:42",
            "04-db-56-ca-42-ee",
            "04:db:56:ca:42:ee;id",
            "--dry-run",
        ] {
            assert!(
                helper_args(HidControlOp::Suspend, Some(bad), false).is_err(),
                "{bad:?}"
            );
        }
    }
}
