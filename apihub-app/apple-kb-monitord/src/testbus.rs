//! Test support: a private `dbus-daemon` that can start NOTHING.
//!
//! The stock session configuration (`dbus-daemon --session`,
//! `dbus-run-session`) lists the service directories of the machine: a test
//! that calls `com.agenceapi.AppleKbMonitor1` before its own fake owns the
//! name would D-Bus-activate the installed `apple-kb-monitord`, a real daemon
//! reading the real keyboard (incident of 2026-10-02). The bus started here
//! reads a configuration without any `<servicedir>`: a name nobody owns is
//! an error, never a program.
//!
//! [`rerun`] re-executes the running test binary with BOTH the session and
//! the system bus addresses pointing at such a bus, so code under test that
//! opens the "system" bus never reaches the real BlueZ, UPower or logind.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// Bus configuration without any service directory (see the module).
pub const BUS_CONFIG: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=@TMP@</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;

/// A `dbus-daemon` nobody else talks to; killed on drop, also when the test
/// panics.
pub struct PrivateBus {
    child: Child,
    pub addr: String,
    config: PathBuf,
}

impl PrivateBus {
    /// Kill the bus now (tests of "the bus restarted").
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        self.kill();
        let _ = std::fs::remove_file(&self.config);
    }
}

/// `None` when `dbus-daemon` is not installed (the test is skipped).
pub fn private_bus() -> Option<PrivateBus> {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let tmp = std::env::temp_dir();
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let config = tmp.join(format!("akm-testbus-{}-{n}.conf", std::process::id()));
    std::fs::write(&config, BUS_CONFIG.replace("@TMP@", &tmp.to_string_lossy())).ok()?;
    let spawned = Command::new("dbus-daemon")
        .arg(format!("--config-file={}", config.display()))
        .args(["--nofork", "--print-address"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else {
        let _ = std::fs::remove_file(&config);
        return None;
    };
    let mut addr = String::new();
    let read = child
        .stdout
        .take()
        .map(|o| BufReader::new(o).read_line(&mut addr));
    let bus = PrivateBus {
        child,
        addr: addr.trim().to_string(),
        config,
    };
    matches!(read, Some(Ok(n)) if n > 0).then_some(bus)
}

pub fn connect(addr: &str) -> zbus::Result<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::address(addr).and_then(|b| b.build())
}

/// The running test binary, to be run again as a child whose session AND
/// system buses are one private bus. Keep the returned bus alive until the
/// child ended. `None` when `dbus-daemon` is not installed.
pub fn rerun() -> Option<(PrivateBus, Command)> {
    let bus = private_bus()?;
    let mut cmd = Command::new(std::env::current_exe().ok()?);
    cmd.env("DBUS_SESSION_BUS_ADDRESS", &bus.addr)
        .env("DBUS_SYSTEM_BUS_ADDRESS", &bus.addr)
        // Nothing of the desktop session leaks into the child.
        .env_remove("DBUS_STARTER_ADDRESS")
        .env_remove("DBUS_STARTER_BUS_TYPE")
        .stdin(Stdio::null());
    Some((bus, cmd))
}

/// Run the test `name` of the running binary again, alone, on a private bus,
/// with `inner_env=1` and `envs` in its environment; panics unless that run
/// passed. Returns false when `dbus-daemon` is not installed (test skipped).
pub fn rerun_test(name: &str, inner_env: &str, envs: &[(&str, &str)]) -> bool {
    let Some((_bus, mut cmd)) = rerun() else {
        eprintln!("SKIP: dbus-daemon not installed");
        return false;
    };
    let out = cmd
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(inner_env, "1")
        .envs(envs.iter().copied())
        .output()
        .expect("run on the private bus");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let tail: String = text
        .chars()
        .rev()
        .take(4000)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    assert!(out.status.success(), "inner run failed:\n{tail}");
    assert!(text.contains("1 passed"), "inner test did not run:\n{tail}");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: no service directory, so nothing can be activated.
    #[test]
    fn the_private_bus_can_activate_nothing() {
        assert!(!BUS_CONFIG.contains("servicedir"));
        assert!(!BUS_CONFIG.contains("standard_session"));
        assert!(!BUS_CONFIG.contains("include"));
        let Some(bus) = private_bus() else {
            eprintln!("SKIP: dbus-daemon not installed");
            return;
        };
        let c = connect(&bus.addr).unwrap();
        let names: Vec<String> = c
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "ListActivatableNames",
                &(),
            )
            .unwrap()
            .body()
            .deserialize()
            .unwrap();
        assert_eq!(
            names,
            ["org.freedesktop.DBus"],
            "nothing but the bus itself"
        );
        // The daemon's name is an error, not a started program.
        let r = c.call_method(
            Some(crate::service::BUS_NAME),
            crate::service::OBJECT_PATH,
            Some(crate::service::INTERFACE),
            "GetState",
            &(),
        );
        assert!(r.is_err());
    }

    #[test]
    fn a_rerun_points_both_buses_at_the_private_one() {
        let Some((bus, cmd)) = rerun() else {
            eprintln!("SKIP: dbus-daemon not installed");
            return;
        };
        let env: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        for key in ["DBUS_SESSION_BUS_ADDRESS", "DBUS_SYSTEM_BUS_ADDRESS"] {
            assert!(
                env.contains(&(key.to_string(), Some(bus.addr.clone()))),
                "{key}"
            );
        }
    }
}
