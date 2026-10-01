//! `akm-keymap-helper` — installs the udev hwdb file of the manual key
//! mapping (#247). Started through `pkexec` only (polkit action
//! `com.agenceapi.AppleKbMonitor.install-keymap`).
//!
//! ```text
//! akm-keymap-helper install    # /run/user/$PKEXEC_UID/apple-kb-monitor/keymap.hwdb → /etc/udev/hwdb.d/90-apple-kb-monitor.hwdb
//! akm-keymap-helper remove     # back to the kernel mapping (defaults re-applied, file removed)
//! akm-keymap-helper rollback   # previous file (90-apple-kb-monitor.hwdb.akm-bak)
//! ```
//!
//! No path, no value on the command line: the only input is the fixed file
//! of the requesting user (`PKEXEC_UID`, set by pkexec), owned by that user,
//! validated line by line against a whitelist and re-rendered canonically.
//! Then `systemd-hwdb update` and `udevadm trigger --settle
//! --subsystem-match=input --action=change`. No grab, no uinput.

use std::process::ExitCode;

use akm_helper::fsutil;
use akm_helper::keymap_install::{parse_args, parse_uid, run, Paths, SystemRunner};

const EX_USAGE: u8 = 64;

fn main() -> ExitCode {
    fsutil::lock_umask();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verb = match parse_args(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("akm-keymap-helper: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    let uid = match std::env::var("PKEXEC_UID").map_err(|_| "PKEXEC_UID missing: run through pkexec".to_string()).and_then(|s| parse_uid(&s)) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("akm-keymap-helper: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    match run(verb, &Paths::for_uid(uid), uid, &SystemRunner) {
        Ok(m) => {
            println!("{m}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("akm-keymap-helper: {m}");
            ExitCode::from(1)
        }
    }
}
