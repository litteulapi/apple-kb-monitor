//! `akm-doctor-fix` — applies the corrections `akmctl doctor` proposes (#106).
//! Started through `pkexec` (polkit action
//! `com.agenceapi.AppleKbMonitor.doctor-fix`, `auth_admin`); with `--dry-run`
//! it only reads and can be run unprivileged.
//!
//! ```text
//! akm-doctor-fix <bluez-conf|upower-conf|adapter-autosuspend>... [--restart] [--dry-run]
//! ```
//!
//! * `bluez-conf`: `/etc/bluetooth/main.conf`, `FastConnectable = true` in
//!   `[General]`, `ReconnectUUIDs` / `ReconnectAttempts` / `ReconnectIntervals`
//!   in `[Policy]` (removed from any other section, where bluetoothd ignores
//!   them);
//! * `upower-conf`: `/etc/UPower/UPower.conf`, `NoPollBatteries = true`;
//! * `adapter-autosuspend`: `/etc/udev/rules.d/61-akm-bt-adapter-no-autosuspend.rules`
//!   (content compiled in), then `udevadm control --reload` and
//!   `udevadm trigger` on the adapter `8087:0026`;
//! * `--restart`: `systemctl restart bluetooth.service` / `upower.service`
//!   after their file (never done otherwise: it drops every Bluetooth link);
//! * `--dry-run`: says what would change; writes nothing, runs nothing.
//!
//! Security stance: a closed list of identifiers and two switches. No path,
//! no value, no environment variable and no stdin picks a file, a key or a
//! command; no shell; the programs run are absolute paths with fixed
//! arguments and an empty environment. Every file is rewritten atomically
//! (0644 root:root, symlinks refused) and the previous content is kept next
//! to it as `<name>.akm-bak`, only when the content really changes: running
//! a correction twice changes nothing the second time.

use std::process::ExitCode;

use akm_helper::doctor_apply::{run, Paths};
use akm_helper::doctor_fix::parse_args;
use akm_helper::fsutil;
use akm_helper::keymap_install::SystemRunner;

const EX_USAGE: u8 = 64;

fn main() -> ExitCode {
    fsutil::lock_umask();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let req = match parse_args(&args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("akm-doctor-fix: {e}");
            return ExitCode::from(EX_USAGE);
        }
    };
    let (lines, result) = run(&req, &Paths::system(), &SystemRunner);
    for l in lines {
        println!("{l}");
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("akm-doctor-fix: {e}");
            ExitCode::from(1)
        }
    }
}
