//! `akmctl hid-control suspend|exit-suspend [--mac MAC] [--dry-run]`.

use akm_core::tr;
use std::process::Command as Proc;

use clap::ValueEnum;

use crate::cli::{EXIT_ERROR, EXIT_OK, EXIT_USAGE};

use akm_core::paths::{HELPER, PKEXEC};
/// `argv[1]` of the helper: it selects the polkit action (hid-inspect needs no password).
pub const INSPECT_CMD: &str = "hid-inspect";
pub const CONTROL_CMD: &str = "hid-control";

/// The two operations; nothing else can be asked from the command line.
/// Their help is the translated help of the `op` argument (cli.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HidControlOp {
    Suspend,
    ExitSuspend,
}

fn check_mac(m: &str) -> Result<String, String> {
    let ok = m.len() == 17
        && m.bytes().enumerate().all(|(i, b)| {
            if i % 3 == 2 {
                b == b':'
            } else {
                b.is_ascii_hexdigit()
            }
        });
    ok.then(|| m.to_ascii_uppercase()).ok_or_else(|| {
        tr!(
            "invalid MAC {m} (expected XX:XX:XX:XX:XX:XX)",
            m = format!("{:?}", m)
        )
    })
}

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

pub fn inspect_args(mac: &str) -> Result<Vec<String>, String> {
    Ok(vec!["--mac".into(), check_mac(mac)?])
}

pub fn parse_control_mtu(text: &str, mac: &str) -> Result<u16, String> {
    let mac = mac.to_ascii_uppercase();
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| l.contains(&mac) && l.contains(" fd ") && l.contains("peer 0x0011"))
        .collect();
    if lines.is_empty() {
        return Err(tr!(
            "no L2CAP control socket (PSM 0x0011) to {mac} reported by akm-helper hid-inspect",
            mac = mac
        ));
    }
    let connected: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| l.contains("state connected"))
        .collect();
    let line = match connected.as_slice() {
        [one] => *one,
        [] => {
            return Err(tr!(
                "the control socket to {mac} is not connected",
                mac = mac
            ))
        }
        many => {
            return Err(tr!(
                "{len} connected control sockets to {mac}, exactly one expected",
                len = many.len(),
                mac = mac
            ))
        }
    };
    let Some(rest) = line.split("mtu out ").nth(1) else {
        return Err(
            tr!("akm-helper hid-inspect did not report the MTU: the installed helper is older than this akmctl, reinstall the package"),
        );
    };
    let v = rest.split_whitespace().next().unwrap_or("");
    v.parse::<u16>().map_err(|_| {
        tr!(
            "unreadable outgoing MTU {v} in akm-helper hid-inspect output",
            v = format!("{:?}", v)
        )
    })
}

pub fn inspect_control_mtu(mac: &str) -> Result<(u16, String), String> {
    let args = inspect_args(mac)?;
    let out = Proc::new(PKEXEC)
        .arg(HELPER)
        .arg(INSPECT_CMD)
        .args(&args)
        .output()
        .map_err(|e| tr!("cannot run pkexec: {e}", e = e))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if let Some(e) = crate::pkexec::failure(out.status.code(), &format!("{HELPER} {INSPECT_CMD}")) {
        return Err(e);
    }
    match out.status.code() {
        Some(0) => {}
        Some(c) => {
            return Err(tr!(
                "{HELPER} {INSPECT_CMD} refused or failed (exit {c}): {trim}",
                HELPER = HELPER,
                INSPECT_CMD = INSPECT_CMD,
                c = c,
                trim = text.trim()
            ))
        }
        None => {
            return Err(tr!(
                "{HELPER} {INSPECT_CMD} killed by a signal",
                HELPER = HELPER,
                INSPECT_CMD = INSPECT_CMD
            ))
        }
    }
    parse_control_mtu(&text, mac).map(|m| (m, text))
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
        .arg(HELPER)
        .arg(CONTROL_CMD)
        .args(&args)
        .status()
    {
        Ok(st) => match control_failure(st.code()) {
            None => EXIT_OK,
            Some(e) => {
                eprintln!("akmctl: {e}");
                EXIT_ERROR
            }
        },
        Err(e) => {
            eprintln!("akmctl: {}", tr!("cannot run pkexec: {e}", e = e));
            EXIT_ERROR
        }
    }
}

/// What went wrong with `pkexec akm-helper hid-control`, from its exit code; `None` on success.
fn control_failure(code: Option<i32>) -> Option<String> {
    if let Some(e) = crate::pkexec::failure(code, &format!("{HELPER} {CONTROL_CMD}")) {
        return Some(e);
    }
    match code {
        Some(0) => None,
        Some(c) => Some(tr!("{HELPER} {CONTROL_CMD} refused or failed (exit {c}); details above and in journalctl -t akm-hid-control", HELPER = HELPER, CONTROL_CMD = CONTROL_CMD, c = c)),
        None => Some(tr!(
            "{HELPER} {CONTROL_CMD} killed by a signal",
            HELPER = HELPER,
            CONTROL_CMD = CONTROL_CMD
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkexec_codes_come_from_pkexec_rs_and_the_rest_from_the_helper() {
        assert_eq!(control_failure(Some(0)), None);
        assert_eq!(
            control_failure(Some(126)).as_deref(),
            Some("authentication dismissed")
        );
        assert!(control_failure(Some(127))
            .unwrap()
            .starts_with("not authorized"));
        assert!(control_failure(Some(1)).unwrap().contains("(exit 1)"));
        assert!(control_failure(None).unwrap().contains("signal"));
    }

    #[test]
    fn helper_args_are_fixed() {
        assert_eq!(
            helper_args(HidControlOp::Suspend, None, false).unwrap(),
            ["suspend"]
        );
        assert_eq!(
            helper_args(HidControlOp::ExitSuspend, Some("aa:bb:cc:dd:ee:f1"), true).unwrap(),
            ["exit-suspend", "--mac", "AA:BB:CC:DD:EE:F1", "--dry-run"]
        );
        for bad in [
            "",
            "aa:bb:cc:dd:ee",
            "aa-bb-cc-dd-ee-f1",
            "aa:bb:cc:dd:ee:f1;id",
            "--dry-run",
        ] {
            assert!(
                helper_args(HidControlOp::Suspend, Some(bad), false).is_err(),
                "{bad:?}"
            );
            assert!(inspect_args(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            inspect_args("aa:bb:cc:dd:ee:f1").unwrap(),
            ["--mac", "AA:BB:CC:DD:EE:F1"],
            "no verb: akm-helper hid-inspect only inspects"
        );
        // never to the one that can send a byte.
        assert_eq!(INSPECT_CMD, "hid-inspect");
        assert_ne!(INSPECT_CMD, CONTROL_CMD);
    }

    const KB: &str = "AA:BB:CC:DD:EE:F1";
    const CTRL: &str = "akm-hid-control:   AA:BB:CC:DD:EE:F1 fd 23: psm local 0x0000 peer 0x0011 cid 0x0041 state connected (hci handle 0x000b) mtu out 672 in 672";
    const INTR: &str = "akm-hid-control:   AA:BB:CC:DD:EE:F1 fd 24: psm local 0x0000 peer 0x0013 cid 0x0042 state connected (hci handle 0x000b) mtu out 48 in 672";

    #[test]
    fn control_mtu_is_read_from_the_connected_control_socket_line_only() {
        let text = format!(
            "akm-hid-control: inspect (read-only) for {KB}\nakm-hid-control: bluetoothd pid 1144 (/usr/lib/bluetooth/bluetoothd): 2 L2CAP socket(s)\n{CTRL}\n{INTR}\nakm-hid-control: {KB} (A1314 0x0256): control channel pid 1144 fd 23 psm 0x0011 hci 0x000b mtu out 672 in 672: inspected, nothing sent\n"
        );
        assert_eq!(parse_control_mtu(&text, KB), Ok(672));
        assert_eq!(parse_control_mtu(&text, "aa:bb:cc:dd:ee:f1"), Ok(672));
        assert_eq!(
            parse_control_mtu(&CTRL.replace("out 672", "out 48"), KB),
            Ok(48)
        );
        // The interrupt channel's MTU is never taken.
        assert!(parse_control_mtu(INTR, KB).is_err());
        // Another keyboard's line is never taken.
        assert!(parse_control_mtu(&CTRL.replace(KB, "11:22:33:44:55:66"), KB).is_err());
        assert!(parse_control_mtu(
            &CTRL.replace("state connected (hci handle 0x000b)", "state not connected"),
            KB
        )
        .unwrap_err()
        .contains("not connected"));
        assert!(parse_control_mtu(&format!("{CTRL}\n{CTRL}"), KB)
            .unwrap_err()
            .contains("exactly one"));
        let old = CTRL.split(" mtu out").next().unwrap();
        assert!(parse_control_mtu(old, KB)
            .unwrap_err()
            .contains("reinstall"));
        assert!(parse_control_mtu(&CTRL.replace("out 672", "out -"), KB)
            .unwrap_err()
            .contains("unreadable"));
        assert!(parse_control_mtu(&CTRL.replace("out 672", "out 70000"), KB).is_err());
        assert!(parse_control_mtu("", KB)
            .unwrap_err()
            .contains("no L2CAP control socket"));
    }
}
