//! `akm-hid-inspect` — read-only description of the HIDP control channel of
//! the Apple keyboards on `bluetoothd`'s sockets, with the negotiated L2CAP
//! MTUs (`getsockopt(L2CAP_OPTIONS)`, `getsockname`, `getpeername` on a
//! duplicated descriptor). The pre-flight of `akmctl rename --device-name`
//! reads the outgoing MTU there (#248).
//!
//! ```text
//! akm-hid-inspect [--mac XX:XX:XX:XX:XX:XX]
//! ```
//!
//! It is its own executable so that polkit can tell it apart from
//! `akm-hid-control` by its PATH: action
//! `com.agenceapi.AppleKbMonitor.hid-inspect` (`allow_active = yes`, no
//! password in the active local session) is bound to this file and to
//! nothing else, while `akm-hid-control` (which can send a byte) stays
//! `auth_admin`. Two actions on one path, told apart by `exec.argv1`, were
//! matched in the order polkitd happens to enumerate them.
//!
//! This program takes no verb and cannot send: it only calls
//! `hidctl::inspect`, from which the `send` path is not reachable. The
//! `hid-suspend.conf` switch does not concern it (nothing is sent).
//!
//! Exit: 0 = inspected (or no keyboard connected), 1 = refused or failed,
//! 64 = usage.

use std::process::ExitCode;

use akm_helper::fsutil;
use akm_helper::hidctl::{self, Event, L2capControlSocket, Mac};

const EX_USAGE: u8 = 64;
const USAGE: &str = "usage: akm-hid-inspect [--mac XX:XX:XX:XX:XX:XX]";

/// Nothing but an optional `--mac MAC`, once. Every verb of
/// `akm-hid-control` (`suspend`, `exit-suspend`, even `inspect`) and every
/// other option is refused.
fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Option<Mac>, String> {
    let a: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    match a.as_slice() {
        [] => Ok(None),
        ["--mac", m] => Mac::parse(m).map(Some),
        ["--mac"] => Err(format!("--mac needs a value\n{USAGE}")),
        [o, ..] => Err(format!("unexpected argument {o:?}\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    fsutil::lock_umask();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mac = match parse_args(&argv) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("akm-hid-inspect: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    let mut log = |e: Event| {
        let (warn, m) = match &e {
            Event::Info(m) => (false, m),
            Event::Warn(m) => (true, m),
        };
        eprintln!("akm-hid-inspect: {m}");
        hidctl::syslog(warn, m);
    };
    // SAFETY: geteuid(2) has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        log(Event::Warn("must run as root (pkexec)".into()));
        return ExitCode::from(1);
    }
    log(Event::Info(format!(
        "inspect (read-only){}",
        mac.map_or(String::new(), |m| format!(" for {m}"))
    )));
    let rep = hidctl::inspect(mac, &hidctl::Env::system(), &L2capControlSocket, &mut log);
    if rep.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// m1 of the final review: this executable accepts only the inspection.
    #[test]
    fn only_an_optional_mac_is_accepted() {
        assert_eq!(parse_args::<&str>(&[]), Ok(None));
        let m = parse_args(&["--mac", "aa:bb:cc:dd:ee:f1"])
            .unwrap()
            .unwrap();
        assert_eq!(m.to_string(), "AA:BB:CC:DD:EE:F1");
        for bad in [
            &["suspend"][..],
            &["exit-suspend"],
            &["inspect"],
            &["inspect", "--mac", "AA:BB:CC:DD:EE:F1"],
            &["suspend", "--mac", "AA:BB:CC:DD:EE:F1"],
            &["--dry-run"],
            &["--mac"],
            &["--mac", "x"],
            &["--mac", "AA:BB:CC:DD:EE:F1", "suspend"],
            &["--mac", "AA:BB:CC:DD:EE:F1", "--mac", "AA:BB:CC:DD:EE:F1"],
            &["--byte", "0x13"],
            &["0x13"],
            &[""],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }

    /// The source of this executable names nothing that sends: no verb, no
    /// `hidctl::run`, no HID_CONTROL command, no write of any kind.
    #[test]
    fn this_executable_cannot_send() {
        let src = include_str!("akm-hid-inspect.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        // Comment lines are documentation, not code.
        let code: String = code
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "hidctl::run",
            "HidControl",
            "Action::Send",
            "send_one",
            // (split: hidctl's own scan of this file looks for the first one)
            concat!("libc", "::send"),
            concat!("libc", "::write"),
            "read_config",
        ] {
            assert!(!code.contains(forbidden), "{forbidden}");
        }
        assert!(code.contains("hidctl::inspect("));
    }
}
