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
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| ())?;
    if !out.status.success() {
        return Ok(None); // cancelled
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(Some(text.trim_end_matches(['\n', '\r']).to_string()))
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

    #[test]
    fn which_finds_sh() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-binary-akm").is_none());
    }
}
