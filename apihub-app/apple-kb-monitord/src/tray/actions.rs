//! Side effects of the tray: open the window, Bluetooth settings, clipboard,
//! charging state from UPower. Each runs off the D-Bus dispatch threads and
//! never touches the keyboard (UPower is read from its cache).

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// Well-known name of the `apihub-app` window (`DBusActivatable=true`).
pub const APP_NAME: &str = "com.agenceapi.ApiHub";
pub const APP_PATH: &str = "/com/agenceapi/ApiHub";

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
/// window it opened. The child is reaped by a short-lived thread.
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
    if let Some(t) = token {
        cmd.env("XDG_ACTIVATION_TOKEN", t);
        cmd.env("DESKTOP_STARTUP_ID", t);
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let _ = std::thread::Builder::new()
        .name("tray-reap".into())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

/// Open the window: `org.freedesktop.Application.Activate` on
/// `com.agenceapi.ApiHub` (D-Bus activation, single instance), falling back to
/// `apihub-app --show`.
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
            tracing::debug!("tray: {APP_NAME} not activatable ({e}), running apihub-app --show");
            if let Err(e) = launch(&["apihub-app", "--show"], token.as_deref()) {
                tracing::warn!("tray: cannot start apihub-app: {e}");
            }
        }
    }
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

    #[test]
    fn which_finds_sh() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-binary-akm").is_none());
    }
}
