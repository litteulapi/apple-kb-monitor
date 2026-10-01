//! `akmctl shutdown-notify`: ask the daemon to send `WillShutdown` (#191).
//!
//! Apple's driver sends one report to the keyboard when the computer shuts down
//! or restarts: Feature `0x40`, the id alone (wire `53 40`). This command only
//! asks the daemon (`NotifyShutdown` on the session bus); the daemon is the
//! single owner of the keyboard and applies the rules: `[apple] will_shutdown`
//! on, keyboard connected, once per run, circuit breaker, 1 s spacing.
//! It is what the user unit `apple-kb-monitor-shutdown.service` runs in
//! `ExecStop=`. akmctl itself never opens the keyboard.

use serde_json::{json, Value};

use crate::bus;
use crate::cli::{EXIT_ABSENT, EXIT_ERROR, EXIT_OK};

/// Is `[apple] will_shutdown` on in this user's `config.toml` (default on)?
pub fn configured() -> bool {
    akm_core::config::load(&akm_core::config::default_path()).0.will_shutdown
}

/// Text of the `status` line.
pub fn status_text(enabled: bool) -> String {
    if enabled {
        "on (WillShutdown sent once at shutdown, as macOS; [apple] will_shutdown)".into()
    } else {
        "off ([apple] will_shutdown = false in config.toml)".into()
    }
}

/// `status --json` object.
pub fn status_json(enabled: bool) -> Value {
    json!({
        "enabled": enabled,
        "default": true,
        "config_key": "[apple] will_shutdown",
        "report": "Feature 0x40, id only (wire 53 40)",
    })
}

/// Does this `systemctl is-system-running` output mean "shutting down"?
pub fn is_stopping(output: &str) -> bool {
    output.trim() == "stopping"
}

/// Absolute path: never resolved through `$PATH`.
const SYSTEMCTL: &str = "/usr/bin/systemctl";

fn system_stopping() -> bool {
    std::process::Command::new(SYSTEMCTL)
        .arg("is-system-running")
        .output()
        .is_ok_and(|o| is_stopping(&String::from_utf8_lossy(&o.stdout)))
}

/// Ask the daemon; exit 0 when it answered (sent or deliberately not),
/// 1 on a D-Bus or write failure, 2 when the daemon is absent.
/// `only_if_stopping`: the unit's guard, nothing is asked unless the system
/// is shutting down (a logout or a service restart is not a shutdown).
pub fn run(only_if_stopping: bool) -> u8 {
    if only_if_stopping && !system_stopping() {
        println!("system is not shutting down: nothing sent");
        return EXIT_OK;
    }
    let conn = match bus::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("akmctl: {e}");
            return EXIT_ABSENT;
        }
    };
    if !bus::daemon_present(&conn) {
        eprintln!("akmctl: daemon not running ({} absent on the session bus): nothing sent", bus::BUS_NAME);
        return EXIT_ABSENT;
    }
    let p = match bus::proxy(&conn) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("akmctl: {e}");
            return EXIT_ERROR;
        }
    };
    match p.call::<_, _, (bool, String)>("NotifyShutdown", &()) {
        Ok((sent, text)) => {
            println!("{text}");
            if !sent && text.contains("failed") {
                EXIT_ERROR
            } else {
                EXIT_OK
            }
        }
        Err(e) => {
            eprintln!("akmctl: NotifyShutdown: {e}");
            EXIT_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_is_only_the_shutdown_state() {
        assert!(is_stopping("stopping\n"));
        for s in ["running", "degraded", "starting", "initializing", "maintenance", "offline", "", "unknown"] {
            assert!(!is_stopping(s), "{s}");
        }
    }

    #[test]
    fn the_command_parses() {
        use clap::Parser;
        let c = crate::cli::Cli::try_parse_from(["akmctl", "shutdown-notify", "--only-if-stopping"]).unwrap();
        assert!(matches!(c.command, crate::cli::Command::ShutdownNotify { only_if_stopping: true }));
        let c = crate::cli::Cli::try_parse_from(["akmctl", "shutdown-notify"]).unwrap();
        assert!(matches!(c.command, crate::cli::Command::ShutdownNotify { only_if_stopping: false }));
    }

    #[test]
    fn the_choice_is_visible_in_both_views() {
        assert!(status_text(true).starts_with("on") && status_text(true).contains("as macOS"));
        assert!(status_text(false).starts_with("off") && status_text(false).contains("will_shutdown = false"));
        let j = status_json(true);
        assert_eq!(j["enabled"], true);
        assert_eq!(j["default"], true);
        assert_eq!(status_json(false)["enabled"], false);
    }
}
