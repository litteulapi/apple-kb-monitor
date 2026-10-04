//! Side effects of the tray: open the widget's popup, Bluetooth settings, clipboard, charging
//! state from `UPower`.

use std::io::Write;
use std::process::{Command, Stdio};

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

pub use apple_kb_monitord::spawn_ui::{launch, missing_display_env, which};

/// Change `hid_apple.fnmode` as `Device.SetFnMode` does once its caller is checked: the call
/// stays inside the daemon, never a D-Bus call to itself that would pass the uid check.
///
/// # Errors
/// The refusal or failure of the change.
pub fn set_fn_mode(conn: &Connection, mac: &str, mode: i32) -> zbus::fdo::Result<()> {
    let path = apple_kb_monitord::devices::device_path(mac).ok_or_else(|| {
        zbus::fdo::Error::InvalidArgs(format!("invalid keyboard address {mac:?}"))
    })?;
    let iref = conn
        .object_server()
        .interface::<_, apple_kb_monitord::devices::Device>(&path)?;
    let dev = iref.get();
    zbus::block_on(dev.apply_fn_mode(mode, conn.inner()))
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
    if let Some(argv) = bluetooth_settings_command() {
        if let Err(e) = launch(&argv, None) {
            tracing::warn!("tray: cannot start {}: {e}", argv[0]);
        }
    } else {
        tracing::warn!("tray: no Bluetooth settings program found");
    }
}

/// Command line of a text-entry dialog (`kdialog` or `zenity`).
pub fn rename_dialog_argv(prog: &str, title: &str, prompt: &str, current: &str) -> Vec<String> {
    let v: Vec<&str> = match prog {
        // `--` ends the options: the alias (data) is never parsed as one.
        "kdialog" => vec![
            "kdialog",
            "--title",
            title,
            "--inputbox",
            prompt,
            "--",
            current,
        ],
        // `--opt=value`: GOption cannot take the value for another option.
        _ => {
            return vec![
                "zenity".to_string(),
                "--entry".to_string(),
                format!("--title={title}"),
                format!("--text={prompt}"),
                format!("--entry-text={current}"),
            ]
        }
    };
    v.into_iter().map(str::to_string).collect()
}

pub fn ask_name(title: &str, prompt: &str, current: &str) -> Result<Option<String>, ()> {
    let Some(prog) = ["kdialog", "zenity"]
        .into_iter()
        .find(|p| which(p).is_some())
    else {
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

/// kdialog / zenity: 0 = a name, 1 = cancelled.
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

/// Clipboard: Klipper over D-Bus, else `wl-copy`, `xclip`, `xsel`.
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

/// Longest wait for `UPower` in the tray loop.
pub const UPOWER_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Run `f` on a worker thread, wait at most `timeout`.
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
    let spawned = std::thread::Builder::new()
        .name("tray-query".into())
        .spawn(move || {
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

/// [`upower_charging`] bounded by [`UPOWER_TIMEOUT`]; `None` = unknown now (keep the last value).
pub fn upower_charging_bounded(
    sys: &Connection,
    mac: Option<&str>,
    busy: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Option<bool> {
    let (sys, mac) = (sys.clone(), mac.map(str::to_string));
    bounded_query(busy, UPOWER_TIMEOUT, move || {
        upower_charging(&sys, mac.as_deref())
    })
}

/// Charging state of the keyboard from `UPower`'s cache (`State` 1 = charging).
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
    fn a_dialog_that_fails_is_not_a_cancellation() {
        assert_eq!(dialog_outcome(Some(0), b"Desk\n"), Ok(Some("Desk".into())));
        assert_eq!(dialog_outcome(Some(1), b""), Ok(None));
        assert_eq!(dialog_outcome(Some(254), b""), Err(()));
        assert_eq!(dialog_outcome(None, b""), Err(()), "killed by a signal");
    }

    #[test]
    fn a_frozen_upower_never_blocks_the_tray_loop() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        let busy = Arc::new(AtomicBool::new(false));
        let t = Instant::now();
        let r = bounded_query(&busy, Duration::from_millis(200), || {
            std::thread::sleep(Duration::from_hours(1));
            true
        });
        assert_eq!(r, None);
        assert!(t.elapsed() < Duration::from_secs(1));
        let t = Instant::now();
        assert_eq!(
            bounded_query(&busy, Duration::from_secs(5), || unreachable!(
                "second call"
            )),
            None::<bool>
        );
        assert!(t.elapsed() < Duration::from_millis(100));
        let free = Arc::new(AtomicBool::new(false));
        assert_eq!(bounded_query(&free, Duration::from_secs(5), || 7), Some(7));
        assert!(!free.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn dialog_argv_per_program() {
        let k = rename_dialog_argv("kdialog", "T", "P", "cur");
        assert_eq!(
            k,
            ["kdialog", "--title", "T", "--inputbox", "P", "--", "cur"]
        );
        let z = rename_dialog_argv("zenity", "T", "P", "cur");
        assert_eq!(z[0], "zenity");
        assert!(z.contains(&"--entry-text=cur".to_string()));
    }

    #[test]
    fn alias_cannot_inject_options() {
        for evil in [
            "--version",
            "--getopenfilename",
            "--textbox /etc/passwd",
            "-h",
        ] {
            let k = rename_dialog_argv("kdialog", "T", "P", evil);
            let sep = k.iter().position(|a| a == "--").expect("separator");
            assert_eq!(k[sep + 1..], [evil.to_string()]);
            assert!(!k[..sep].iter().any(|a| a == evil));
            let z = rename_dialog_argv("zenity", "T", "P", evil);
            assert!(z.iter().all(|a| a != evil), "never a bare argument: {z:?}");
            assert!(z.contains(&format!("--entry-text={evil}")));
        }
    }

    #[test]
    fn fn_mode_goes_through_the_daemon_internal_path() {
        let me = akm_core::srclint::prod_tokens(include_str!("actions.rs"));
        let set = &me[me.find("pubfnset_fn_mode(").unwrap()..];
        let set = &set[..set.find("pubfnbluetooth_settings_command").unwrap()];
        assert!(set.contains("dev.apply_fn_mode(mode,conn.inner())"));
        assert!(
            !set.contains("call_method("),
            "never a D-Bus call to the daemon itself"
        );
        let devices = akm_core::srclint::prod_tokens(include_str!("../devices.rs"));
        let set = &devices[devices.find("asyncfnset_fn_mode(").unwrap()..];
        assert!(set.contains("caller_uid(conn,&hdr).await?;"));
        assert!(set.contains("self.apply_fn_mode(mode,conn).await"));
    }
}
