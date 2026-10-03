//! Side effects of the tray: open the window, Bluetooth settings, clipboard,
//! charging state from UPower. Each runs off the D-Bus dispatch threads and
//! never touches the keyboard (UPower is read from its cache).

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// Well-known name of the `apihub-app` window (`DBusActivatable=true`); same
/// value as `apihub_app::instance::{APP_ID, APP_PATH}` (the crates cannot
/// depend on each other, `tests/` of the workspace checks both).
pub const APP_NAME: &str = "com.agenceapi.AppleKbMonitor";
pub const APP_PATH: &str = "/com/agenceapi/AppleKbMonitor";

pub fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Start a desktop program outside the daemon's cgroup when possible
/// (`systemd-run --user --scope`): restarting the service must not kill the
/// window it opened. The daemon may predate `import-environment`: the
/// display variables it lacks are taken from the user manager (C10). A
/// program that fails within [`START_CHECK`] is journalled and notified,
/// never reported "started" in silence.
pub fn launch(argv: &[&str], token: Option<&str>) -> std::io::Result<()> {
    let mut cmd = if which("systemd-run").is_some() && std::env::var_os("INVOCATION_ID").is_some() {
        let mut c = Command::new("systemd-run");
        c.args(["--user", "--scope", "--collect", "--quiet", "--"]);
        c.args(argv);
        c
    } else {
        let mut c = Command::new(argv[0]);
        c.args(&argv[1..]);
        c
    };
    for (k, v) in missing_display_env() {
        cmd.env(k, v);
    }
    if let Some(t) = token {
        cmd.env("XDG_ACTIVATION_TOKEN", t);
        cmd.env("DESKTOP_STARTUP_ID", t);
    }
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let prog = argv[0].to_string();
    let _ = std::thread::Builder::new()
        .name("tray-reap".into())
        .spawn(move || {
            if let Err(e) = wait_start(child, START_CHECK) {
                tracing::warn!("tray: {prog} failed to start: {e}");
                apple_kb_monitord::notify::send_with(
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

/// How long a launched program is watched for an immediate failure.
pub const START_CHECK: Duration = Duration::from_secs(2);

/// `Err(exit status + end of stderr)` when the child ends in failure within
/// `window`; `Ok` when it is still running then (reaped later) or exited 0.
pub fn wait_start(mut child: std::process::Child, window: Duration) -> Result<(), String> {
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

/// `WAYLAND_DISPLAY` / `DISPLAY` / `XAUTHORITY` the daemon lacks, from the
/// user manager's environment (`systemctl --user show-environment`).
fn missing_display_env() -> Vec<(String, String)> {
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

/// Open the window: `org.freedesktop.Application.Activate` on
/// `com.agenceapi.AppleKbMonitor` (D-Bus activation, single instance), falling back to
/// plain `apihub-app` (single instance + activation token via env).
pub fn open_window(conn: &Connection, token: Option<String>) {
    let mut platform: HashMap<&str, Value<'_>> = HashMap::new();
    if let Some(t) = &token {
        platform.insert("activation-token", Value::from(t.as_str()));
        platform.insert("desktop-startup-id", Value::from(t.as_str()));
    }
    match conn.call_method(
        Some(APP_NAME),
        APP_PATH,
        Some("org.freedesktop.Application"),
        "Activate",
        &(platform,),
    ) {
        Ok(_) => tracing::info!("tray: window activated through {APP_NAME}"),
        Err(e) => {
            tracing::debug!("tray: {APP_NAME} not activatable ({e}), running apihub-app");
            if let Err(e) = launch(&["apihub-app"], token.as_deref()) {
                tracing::warn!("tray: cannot start apihub-app: {e}");
            }
        }
    }
}

/// Interface and method of the daemon's keyboard object that change the Fn
/// mode (`devices.rs`: caller uid checked, value validated, polkit helper).
pub const DEVICE_IFACE: &str = "com.agenceapi.AppleKbMonitor1.Device";
pub const SET_FN_MODE: &str = "SetFnMode";

/// Change `hid_apple.fnmode` through the daemon, the same path as the KDE
/// module and `akmctl` use (`Device.SetFnMode`); the daemon opens the polkit
/// authentication. Blocks until the user answered.
pub fn set_fn_mode(conn: &Connection, mac: &str, mode: i32) -> zbus::Result<()> {
    let path = apple_kb_monitord::devices::device_path(mac)
        .ok_or_else(|| zbus::Error::Failure(format!("invalid keyboard address {mac:?}")))?;
    conn.call_method(
        Some(apple_kb_monitord::service::BUS_NAME),
        path.as_str(),
        Some(DEVICE_IFACE),
        SET_FN_MODE,
        &(mode,),
    )?;
    Ok(())
}

/// Bluetooth settings of the running desktop (first available).
pub fn bluetooth_settings_command() -> Option<Vec<&'static str>> {
    const CANDIDATES: [&[&str]; 4] = [
        &["kcmshell6", "kcm_bluetooth"],
        &["systemsettings", "kcm_bluetooth"],
        &["gnome-control-center", "bluetooth"],
        &["blueman-manager"],
    ];
    CANDIDATES
        .iter()
        .find(|c| which(c[0]).is_some())
        .map(|c| c.to_vec())
}

pub fn open_bluetooth_settings() {
    match bluetooth_settings_command() {
        Some(argv) => {
            if let Err(e) = launch(&argv, None) {
                tracing::warn!("tray: cannot start {}: {e}", argv[0]);
            }
        }
        None => tracing::warn!("tray: no Bluetooth settings program found"),
    }
}

/// Command line of a text-entry dialog (`kdialog` or `zenity`).
pub fn rename_dialog_argv(prog: &str, title: &str, prompt: &str, current: &str) -> Vec<String> {
    let v: Vec<&str> = match prog {
        // `--` ends the options: the alias (data) is never parsed as one (#207).
        "kdialog" => vec![
            "kdialog", "--title", title, "--inputbox", prompt, "--", current,
        ],
        // `--opt=value`: GOption cannot take the value for another option.
        _ => return vec![
            "zenity".to_string(),
            "--entry".to_string(),
            format!("--title={title}"),
            format!("--text={prompt}"),
            format!("--entry-text={current}"),
        ],
    };
    v.into_iter().map(str::to_string).collect()
}

/// Ask the user for a new name. `None` = cancelled, or no dialog program
/// (the caller then opens the window, which has a name field).
pub fn ask_name(title: &str, prompt: &str, current: &str) -> Result<Option<String>, ()> {
    let Some(prog) = ["kdialog", "zenity"].into_iter().find(|p| which(p).is_some()) else {
        return Err(());
    };
    let argv = rename_dialog_argv(prog, title, prompt, current);
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .envs(missing_display_env())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| ())?;
    let r = dialog_outcome(out.status.code(), &out.stdout);
    if r.is_err() {
        tracing::warn!("tray: {prog} failed ({}), not a cancellation", out.status);
    }
    r
}

/// kdialog / zenity: 0 = a name, 1 = cancelled; anything else (no display,
/// killed) is a failure, never taken for a cancellation (C10).
fn dialog_outcome(code: Option<i32>, stdout: &[u8]) -> Result<Option<String>, ()> {
    match code {
        Some(0) => {
            let text = String::from_utf8_lossy(stdout);
            Ok(Some(text.trim_end_matches(['\n', '\r']).to_string()))
        }
        Some(1) => Ok(None),
        _ => Err(()),
    }
}

/// Clipboard: Klipper over D-Bus (works without `WAYLAND_DISPLAY` in the
/// unit), else `wl-copy`, `xclip`, `xsel`.
pub fn copy_to_clipboard(conn: &Connection, text: &str) -> bool {
    if conn
        .call_method(
            Some("org.kde.klipper"),
            "/klipper",
            Some("org.kde.klipper.klipper"),
            "setClipboardContents",
            &(text,),
        )
        .is_ok()
    {
        return true;
    }
    let tools: [(&str, &[&str], &str); 3] = [
        ("wl-copy", &[], "WAYLAND_DISPLAY"),
        ("xclip", &["-selection", "clipboard"], "DISPLAY"),
        ("xsel", &["--clipboard", "--input"], "DISPLAY"),
    ];
    for (bin, args, env) in tools {
        if std::env::var_os(env).is_none() || which(bin).is_none() {
            continue;
        }
        let Ok(mut child) = Command::new(bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        let ok = child
            .stdin
            .take()
            .is_some_and(|mut i| i.write_all(text.as_bytes()).is_ok());
        // wl-copy / xclip fork a server and the parent exits at once.
        let _ = child.wait();
        if ok {
            return true;
        }
    }
    false
}

/// Longest wait for UPower in the tray loop (#234).
pub const UPOWER_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Run `f` on a worker thread, wait at most `timeout`. `None` when it does
/// not answer in time, or when the previous call is still pending (`busy`):
/// a frozen peer never blocks the caller nor piles up threads (#234).
pub fn bounded_query<T: Send + 'static>(
    busy: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    timeout: std::time::Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    use std::sync::atomic::Ordering;
    if busy.swap(true, Ordering::AcqRel) {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let flag = busy.clone();
    let spawned = std::thread::Builder::new().name("tray-query".into()).spawn(move || {
        let r = f();
        flag.store(false, Ordering::Release);
        let _ = tx.send(r);
    });
    if spawned.is_err() {
        busy.store(false, Ordering::Release);
        return None;
    }
    rx.recv_timeout(timeout).ok()
}

/// [`upower_charging`] bounded by [`UPOWER_TIMEOUT`]; `None` = unknown now
/// (keep the last value).
pub fn upower_charging_bounded(
    sys: &Connection,
    mac: Option<&str>,
    busy: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Option<bool> {
    let (sys, mac) = (sys.clone(), mac.map(str::to_string));
    bounded_query(busy, UPOWER_TIMEOUT, move || upower_charging(&sys, mac.as_deref()))
}

/// Charging state of the keyboard from UPower's cache (`State` 1 =
/// charging). Read-only on the system bus; `false` when unknown.
pub fn upower_charging(sys: &Connection, mac: Option<&str>) -> bool {
    let res = (|| -> zbus::Result<bool> {
        let reply = sys.call_method(
            Some("org.freedesktop.UPower"),
            "/org/freedesktop/UPower",
            Some("org.freedesktop.UPower"),
            "EnumerateDevices",
            &(),
        )?;
        let paths: Vec<OwnedObjectPath> = reply.body().deserialize()?;
        let mac = mac.map(str::to_ascii_lowercase);
        for p in paths {
            if !akm_core::model::is_keyboard_upower_path(p.as_str()) {
                continue;
            }
            let get = |prop: &str| -> zbus::Result<OwnedValue> {
                let r = sys.call_method(
                    Some("org.freedesktop.UPower"),
                    p.as_str(),
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &("org.freedesktop.UPower.Device", prop),
                )?;
                r.body().deserialize::<OwnedValue>()
            };
            if let Some(m) = &mac {
                let native: String = get("NativePath")?.try_into().unwrap_or_default();
                if !native.to_ascii_lowercase().contains(m.as_str()) {
                    continue;
                }
            }
            let state: u32 = get("State")?.try_into().unwrap_or(0);
            return Ok(state == 1);
        }
        Ok(false)
    })();
    res.unwrap_or_else(|e| {
        tracing::debug!("tray: UPower state unavailable: {e}");
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C10: a program that fails at once is reported, with its stderr; one
    /// still running after the window is a start.
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
    fn a_dialog_that_fails_is_not_a_cancellation() {
        assert_eq!(
            dialog_outcome(Some(0), b"Bureau\n"),
            Ok(Some("Bureau".into()))
        );
        assert_eq!(dialog_outcome(Some(1), b""), Ok(None));
        assert_eq!(dialog_outcome(Some(254), b""), Err(()));
        assert_eq!(dialog_outcome(None, b""), Err(()), "killed by a signal");
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
    fn a_frozen_upower_never_blocks_the_tray_loop() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        let busy = Arc::new(AtomicBool::new(false));
        let t = Instant::now();
        let r = bounded_query(&busy, Duration::from_millis(200), || {
            std::thread::sleep(Duration::from_secs(3600));
            true
        });
        assert_eq!(r, None);
        assert!(t.elapsed() < Duration::from_secs(1));
        // Still pending: no second call, answer at once.
        let t = Instant::now();
        assert_eq!(bounded_query(&busy, Duration::from_secs(5), || unreachable!("second call")), None::<bool>);
        assert!(t.elapsed() < Duration::from_millis(100));
        let free = Arc::new(AtomicBool::new(false));
        assert_eq!(bounded_query(&free, Duration::from_secs(5), || 7), Some(7));
        assert!(!free.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn dialog_argv_per_program() {
        let k = rename_dialog_argv("kdialog", "T", "P", "cur");
        assert_eq!(k, ["kdialog", "--title", "T", "--inputbox", "P", "--", "cur"]);
        let z = rename_dialog_argv("zenity", "T", "P", "cur");
        assert_eq!(z[0], "zenity");
        assert!(z.contains(&"--entry-text=cur".to_string()));
    }

    /// #207 : an alias is data, never an option of kdialog / zenity.
    #[test]
    fn alias_cannot_inject_options() {
        for evil in ["--version", "--getopenfilename", "--textbox /etc/passwd", "-h"] {
            let k = rename_dialog_argv("kdialog", "T", "P", evil);
            let sep = k.iter().position(|a| a == "--").expect("separator");
            assert_eq!(k[sep + 1..], [evil.to_string()]);
            assert!(!k[..sep].iter().any(|a| a == evil));
            let z = rename_dialog_argv("zenity", "T", "P", evil);
            assert!(z.iter().all(|a| a != evil), "never a bare argument: {z:?}");
            assert!(z.contains(&format!("--entry-text={evil}")));
        }
    }

    /// The tray must target the name the window really claims (#148).
    #[test]
    fn app_name_matches_window_and_service_file() {
        let window = include_str!("../../../src/instance.rs");
        assert!(window.contains(&format!("APP_ID: &str = \"{APP_NAME}\"")));
        assert!(window.contains(&format!("APP_PATH: &str = \"{APP_PATH}\"")));
        let svc = include_str!("../../../../dbus/com.agenceapi.AppleKbMonitor.service");
        assert!(svc.contains(&format!("Name={APP_NAME}\n")));
        assert_eq!(APP_PATH, format!("/{}", APP_NAME.replace('.', "/")));
    }

    /// The tray's Fn entry calls the method the daemon really exports.
    #[test]
    fn fn_mode_goes_through_the_daemon_device_method() {
        let devices = include_str!("../devices.rs");
        assert!(devices.contains(&format!("#[interface(name = \"{DEVICE_IFACE}\")]")));
        assert!(devices.contains("async fn set_fn_mode("));
        assert_eq!(SET_FN_MODE, "SetFnMode");
    }

    #[test]
    fn which_finds_sh() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-binary-akm").is_none());
    }
}
