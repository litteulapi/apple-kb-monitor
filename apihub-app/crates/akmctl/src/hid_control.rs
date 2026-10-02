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
/// The read-only inspection, an executable of its own: its polkit action
/// (`hid-inspect`, no password in the active local session) is bound to this
/// path and to nothing else, so pkexec's choice does not depend on the order
/// polkitd enumerates the actions in.
pub const HID_INSPECT_HELPER: &str = "/usr/lib/apple-kb-monitor/akm-hid-inspect";

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

/// Arguments of the read-only inspection (`akm-hid-inspect`, lock 2 of
/// #248): the keyboard and nothing else (no verb).
pub fn inspect_args(mac: &str) -> Result<Vec<String>, String> {
    Ok(vec!["--mac".into(), check_mac(mac)?])
}

/// The negotiated **outgoing** L2CAP MTU of the HIDP control channel to `mac`,
/// from the lines `akm-hid-inspect` prints (`describe_socket`): the
/// line of a socket whose peer PSM is `0x0011`, `state connected`, `mtu out N`.
/// `Err` explains why it is unknown (older helper without `mtu`, no socket,
/// several sockets, unreadable value): unknown = the write is refused.
pub fn parse_control_mtu(text: &str, mac: &str) -> Result<u16, String> {
    let mac = mac.to_ascii_uppercase();
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| l.contains(&mac) && l.contains(" fd ") && l.contains("peer 0x0011"))
        .collect();
    if lines.is_empty() {
        return Err(format!(
            "no L2CAP control socket (PSM 0x0011) to {mac} reported by akm-hid-inspect"
        ));
    }
    let connected: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| l.contains("state connected"))
        .collect();
    let line = match connected.as_slice() {
        [one] => *one,
        [] => return Err(format!("the control socket to {mac} is not connected")),
        many => return Err(format!("{} connected control sockets to {mac}, exactly one expected", many.len())),
    };
    let Some(rest) = line.split("mtu out ").nth(1) else {
        return Err(
            "akm-hid-inspect did not report the MTU: the installed helper is older than this akmctl, reinstall the package".into(),
        );
    };
    let v = rest.split_whitespace().next().unwrap_or("");
    v.parse::<u16>()
        .map_err(|_| format!("unreadable outgoing MTU {v:?} in akm-hid-inspect output"))
}

/// Read the outgoing MTU of the control channel to `mac` on the live socket
/// (`pkexec akm-hid-inspect --mac MAC`: no password in the active
/// local session, read-only `getsockopt`, nothing sent). The combined output is returned
/// with the value so the caller can journal it.
pub fn inspect_control_mtu(mac: &str) -> Result<(u16, String), String> {
    let args = inspect_args(mac)?;
    let out = Proc::new(PKEXEC)
        .arg(HID_INSPECT_HELPER)
        .args(&args)
        .output()
        .map_err(|e| format!("cannot run pkexec: {e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    match out.status.code() {
        Some(0) => {}
        Some(126) => return Err("authentication dismissed or not authorized (pkexec 126)".into()),
        Some(127) => {
            return Err(format!(
                "authentication failed or {HID_INSPECT_HELPER} not found (pkexec 127)"
            ))
        }
        Some(c) => return Err(format!("{HID_INSPECT_HELPER} refused or failed (exit {c}): {}", text.trim())),
        None => return Err(format!("{HID_INSPECT_HELPER} killed by a signal")),
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
            "no verb: akm-hid-inspect only inspects"
        );
        // m1 of the final review: the password-less action is bound to an
        // executable of its own, never to the one that can send a byte.
        assert_ne!(HID_INSPECT_HELPER, HID_CONTROL_HELPER);
        assert!(HID_INSPECT_HELPER.ends_with("/akm-hid-inspect"));
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
        // A small MTU is reported as is (the pre-flight decides).
        assert_eq!(parse_control_mtu(&CTRL.replace("out 672", "out 48"), KB), Ok(48));
        // The interrupt channel's MTU is never taken.
        assert!(parse_control_mtu(INTR, KB).is_err());
        // Another keyboard's line is never taken.
        assert!(parse_control_mtu(&CTRL.replace(KB, "11:22:33:44:55:66"), KB).is_err());
        // Not connected, two connected, older helper without mtu, garbage.
        assert!(parse_control_mtu(&CTRL.replace("state connected (hci handle 0x000b)", "state not connected"), KB)
            .unwrap_err()
            .contains("not connected"));
        assert!(parse_control_mtu(&format!("{CTRL}\n{CTRL}"), KB).unwrap_err().contains("exactly one"));
        let old = CTRL.split(" mtu out").next().unwrap();
        assert!(parse_control_mtu(old, KB).unwrap_err().contains("reinstall"));
        assert!(parse_control_mtu(&CTRL.replace("out 672", "out -"), KB).unwrap_err().contains("unreadable"));
        assert!(parse_control_mtu(&CTRL.replace("out 672", "out 70000"), KB).is_err());
        assert!(parse_control_mtu("", KB).unwrap_err().contains("no L2CAP control socket"));
    }
}
