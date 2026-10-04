//! `akm-helper hid-inspect` — read-only description of the HIDP control channel on `bluetoothd`'s sockets,
//! with the negotiated L2CAP MTUs.

use akm_helper::{tr, EX_USAGE};
use std::process::ExitCode;

use akm_helper::hidctl::{self, Event, L2capControlSocket, Mac};

const USAGE: &str = "usage: akm-helper hid-inspect [--mac XX:XX:XX:XX:XX:XX]";

fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Option<Mac>, String> {
    let a: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    match a.as_slice() {
        [] => Ok(None),
        ["--mac", m] => Mac::parse(m).map(Some),
        ["--mac"] => Err(tr!("--mac needs a value\n{USAGE}", USAGE = USAGE)),
        [o, ..] => Err(tr!(
            "unexpected argument {o}\n{USAGE}",
            o = format!("{:?}", o),
            USAGE = USAGE
        )),
    }
}

pub fn run(argv: &[String]) -> ExitCode {
    let mac = match parse_args(argv) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("akm-helper hid-inspect: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    let mut log = hidctl::event_logger("hid-inspect");
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

    #[test]
    fn this_executable_cannot_send() {
        let src = include_str!("hid_inspect.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
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
            concat!("libc", "::send"),
            concat!("libc", "::write"),
            "read_config",
        ] {
            assert!(!code.contains(forbidden), "{forbidden}");
        }
        assert!(code.contains("hidctl::inspect("));
    }
}
