//! Start desktop programs from the daemon: own scope, display variables, reaped child.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// First executable `bin` on `PATH`.
#[must_use]
pub fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Start a desktop program outside the daemon's cgroup when possible (`systemd-run --user
/// --scope`): restarting the service must not kill the window it opened. `activation_token`
/// (from the notification server) lets the compositor focus its window.
///
/// # Errors
/// When the program (or `systemd-run`) cannot be spawned.
pub fn launch(argv: &[&str], activation_token: Option<&str>) -> std::io::Result<()> {
    let scoped = which("systemd-run").is_some() && std::env::var_os("INVOCATION_ID").is_some();
    let mut cmd = command_for(argv, scoped, activation_token);
    for (k, v) in missing_display_env() {
        cmd.env(k, v);
    }
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let prog = argv[0].to_string();
    let _ = std::thread::Builder::new()
        .name("ui-reap".into())
        .spawn(move || {
            if let Err(e) = wait_start(child, START_CHECK) {
                tracing::warn!("{prog} failed to start: {e}");
                crate::notify::send_with(
                    &prog,
                    &e,
                    "dialog-error",
                    akm_core::alerts::Urgency::Normal,
                    true,
                );
            }
        });
    Ok(())
}

fn command_for(argv: &[&str], scoped: bool, activation_token: Option<&str>) -> Command {
    let mut c = if scoped {
        let mut c = Command::new("systemd-run");
        c.args(["--user", "--scope", "--collect", "--quiet", "--"]);
        c.args(argv);
        c
    } else {
        let mut c = Command::new(argv[0]);
        c.args(&argv[1..]);
        c
    };
    if let Some(t) = activation_token {
        c.env("XDG_ACTIVATION_TOKEN", t);
    }
    c
}

/// How long a launched program is watched for an immediate failure.
const START_CHECK: Duration = Duration::from_secs(2);

/// `Err(exit status + end of stderr)` when the child ends in failure within `window`.
fn wait_start(mut child: std::process::Child, window: Duration) -> Result<(), String> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) if st.success() => return Ok(()),
            Ok(Some(st)) => {
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut e, &mut err);
                }
                let tail: String = err
                    .trim()
                    .lines()
                    .last()
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect();
                return Err(if tail.is_empty() {
                    format!("{st}")
                } else {
                    format!("{st}: {tail}")
                });
            }
            Ok(None) if start.elapsed() >= window => {
                // Still running: drop stderr (no pipe left full), reap later.
                drop(child.stderr.take());
                let _ = child.wait();
                return Ok(());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Display variables the user manager knows and this process lacks.
#[must_use]
pub fn missing_display_env() -> Vec<(String, String)> {
    const KEYS: [&str; 3] = ["WAYLAND_DISPLAY", "DISPLAY", "XAUTHORITY"];
    if KEYS.iter().all(|k| std::env::var_os(k).is_some()) {
        return Vec::new();
    }
    let Ok(out) = Command::new("systemctl")
        .args(["--user", "show-environment"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    pick_display_env(&String::from_utf8_lossy(&out.stdout), |k| {
        std::env::var_os(k).is_some()
    })
}

fn pick_display_env(text: &str, present: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    const KEYS: [&str; 3] = ["WAYLAND_DISPLAY", "DISPLAY", "XAUTHORITY"];
    text.lines()
        .filter_map(|l| l.split_once('='))
        .filter(|(k, v)| KEYS.contains(k) && !present(k) && !v.is_empty())
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_that_fails_at_once_is_reported() {
        let spawn = |script: &str| {
            Command::new("sh")
                .args(["-c", script])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        };
        let e =
            wait_start(spawn("echo 'cannot open display' >&2; exit 3"), START_CHECK).unwrap_err();
        assert!(e.contains("cannot open display") && e.contains('3'), "{e}");
        assert!(wait_start(spawn("sleep 1"), Duration::from_millis(200)).is_ok());
        assert!(wait_start(spawn("exit 0"), START_CHECK).is_ok());
    }

    #[test]
    fn display_variables_come_from_the_user_manager_when_missing() {
        let env = "HOME=/h\nWAYLAND_DISPLAY=wayland-1\nDISPLAY=:1\nXAUTHORITY=\n";
        assert_eq!(
            pick_display_env(env, |_| false),
            vec![
                ("WAYLAND_DISPLAY".to_string(), "wayland-1".to_string()),
                ("DISPLAY".to_string(), ":1".to_string())
            ]
        );
        assert!(
            pick_display_env(env, |_| true).is_empty(),
            "own values kept"
        );
    }

    #[test]
    fn under_systemd_the_program_gets_its_own_scope() {
        let c = command_for(&["konsole", "-e", "akmctl", "repair"], true, None);
        assert_eq!(c.get_program(), "systemd-run");
        let args: Vec<_> = c.get_args().collect();
        assert_eq!(
            args,
            [
                "--user",
                "--scope",
                "--collect",
                "--quiet",
                "--",
                "konsole",
                "-e",
                "akmctl",
                "repair"
            ]
        );
        let c = command_for(&["konsole", "-e"], false, Some("tok-1"));
        assert_eq!(c.get_program(), "konsole");
        let env: Vec<_> = c.get_envs().collect();
        assert_eq!(
            env,
            [(
                std::ffi::OsStr::new("XDG_ACTIVATION_TOKEN"),
                Some(std::ffi::OsStr::new("tok-1"))
            )],
            "the notification's token reaches the window"
        );
    }

    #[test]
    fn which_finds_sh() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-binary-akm").is_none());
    }
}
