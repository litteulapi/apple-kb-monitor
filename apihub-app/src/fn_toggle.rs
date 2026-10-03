//! `apihub-app --toggle-fn`: switch the top row between media keys first and
//! F1–F12 first (#99). It is the command of the KDE global shortcut declared
//! in `data/com.agenceapi.AppleKbMonitor.shortcuts.desktop`, and the KRunner
//! action "Toggle the function keys" (#98).
//!
//! Thin client: the mode is read from the daemon's keyboard object
//! (`Device.FnMode`) and changed with `Device.SetFnMode`; the daemon checks
//! the caller and opens the polkit authentication. Nothing here touches
//! sysfs or the keyboard.

use apple_kb_monitord::devices::DEVICE_INTERFACE;
use apple_kb_monitord::service::{BUS_NAME, INTERFACE, OBJECT_PATH};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::i18n::{tr, trf};

/// Command-line flag handled by `main()` before any window is opened.
pub const FLAG: &str = "--toggle-fn";

/// `hid_apple.fnmode`: media keys first (kernel default) / F1–F12 first.
pub const MEDIA_FIRST: i32 = 1;
pub const FKEYS_FIRST: i32 = 2;

/// The mode the toggle switches to. F1–F12 first goes back to media keys;
/// media keys (1, or 3 = auto, which behaves the same on an Apple keyboard)
/// go to F1–F12; the two other kernel values (0 = Fn disabled, 4 = F-keys
/// disabled) return to the kernel default. `None`: `hid_apple` is not loaded
/// or the value is unknown, nothing is written.
pub fn next_mode(current: i32) -> Option<i32> {
    match current {
        FKEYS_FIRST => Some(MEDIA_FIRST),
        1 | 3 => Some(FKEYS_FIRST),
        0 | 4 => Some(MEDIA_FIRST),
        _ => None,
    }
}

/// What a mode does, in words.
pub fn mode_text(mode: i32) -> String {
    match mode {
        1 | 3 => tr("media keys first").into(),
        2 => tr("F1\u{2013}F12 first").into(),
        0 => tr("Fn key has no effect").into(),
        4 => tr("F-keys disabled").into(),
        m => trf("unknown mode {}", &[&m]),
    }
}

fn device_prop(conn: &Connection, path: &OwnedObjectPath, name: &str) -> zbus::Result<OwnedValue> {
    conn.call_method(
        Some(BUS_NAME),
        path.as_str(),
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(DEVICE_INTERFACE, name),
    )?
    .body()
    .deserialize()
}

/// The keyboard object the daemon exposes: the connected one, else the first.
pub fn keyboard_path(conn: &Connection) -> Result<OwnedObjectPath, String> {
    let paths: Vec<OwnedObjectPath> = conn
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "GetDevices",
            &(),
        )
        .and_then(|m| m.body().deserialize())
        .map_err(|e| crate::actions::daemon_error(&e))?;
    let connected = |p: &&OwnedObjectPath| {
        device_prop(conn, p, "Connected")
            .ok()
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(false)
    };
    paths
        .iter()
        .find(connected)
        .or(paths.first())
        .cloned()
        .ok_or_else(|| tr("no keyboard known to the daemon").to_string())
}

/// Current `hid_apple.fnmode` as the daemon reads it (negative = not loaded).
pub fn current_mode(conn: &Connection, path: &OwnedObjectPath) -> Result<i32, String> {
    device_prop(conn, path, "FnMode")
        .map_err(|e| e.to_string())
        .and_then(|v| i32::try_from(v).map_err(|e| e.to_string()))
}

/// Toggle the Fn mode through the daemon. Returns `(old, new)`. Blocks until
/// the user answered the polkit dialog the daemon opens.
pub fn toggle(conn: &Connection) -> Result<(i32, i32), String> {
    let path = keyboard_path(conn)?;
    let old = current_mode(conn, &path)?;
    let new =
        next_mode(old).ok_or_else(|| trf("Fn mode unknown ({}): is hid_apple loaded?", &[&old]))?;
    conn.call_method(
        Some(BUS_NAME),
        path.as_str(),
        Some(DEVICE_INTERFACE),
        "SetFnMode",
        &(new,),
    )
    .map_err(|e| trf("Fn mode not changed: {}", &[&e]))?;
    Ok((old, new))
}

/// Entry point of `apihub-app --toggle-fn`: exit code 0 = changed.
pub fn run() -> i32 {
    let res = Connection::session()
        .map_err(|e| trf("no session bus: {}", &[&e]))
        .and_then(|c| toggle(&c));
    match res {
        Ok((_, new)) => {
            println!("{}", trf("Fn mode: {}", &[&mode_text(new)]));
            0
        }
        Err(e) => {
            eprintln!("apihub-app {FLAG}: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::{FakeDaemon, MAC};

    #[test]
    fn toggle_alternates_between_media_keys_and_f_keys() {
        assert_eq!(next_mode(1), Some(2));
        assert_eq!(next_mode(3), Some(2));
        assert_eq!(next_mode(2), Some(1));
        // Fn disabled / F-keys disabled: back to the kernel default.
        assert_eq!(next_mode(0), Some(1));
        assert_eq!(next_mode(4), Some(1));
        // hid_apple not loaded (-1) or a value the kernel does not have.
        assert_eq!(next_mode(-1), None);
        assert_eq!(next_mode(7), None);
        // Two presses come back to the start.
        for m in [1, 2] {
            assert_eq!(next_mode(m).and_then(next_mode), Some(m));
        }
        assert_eq!(mode_text(2), "F1\u{2013}F12 first");
        assert_eq!(mode_text(9), "unknown mode 9");
    }

    /// The shortcut goes through `Device.SetFnMode` of the daemon, on a
    /// private bus with a fake daemon: no sysfs, no keyboard.
    #[test]
    fn toggle_calls_the_daemon_device_method() {
        let Some(fake) = FakeDaemon::start(1) else {
            eprintln!("skipped: no dbus-daemon");
            return;
        };
        let conn = fake.client();
        let path = keyboard_path(&conn).unwrap();
        assert_eq!(
            path.as_str(),
            apple_kb_monitord::devices::device_path(MAC)
                .unwrap()
                .as_str()
        );
        assert_eq!(toggle(&conn), Ok((1, 2)));
        assert_eq!(toggle(&conn), Ok((2, 1)));
        assert_eq!(fake.set_calls(), vec![2, 1]);
        // hid_apple not loaded: nothing is written.
        fake.force_mode(-1);
        let err = toggle(&conn).unwrap_err();
        assert!(err.contains("hid_apple"), "{err}");
        assert_eq!(fake.set_calls(), vec![2, 1]);
        // The daemon refuses (polkit dismissed): the error is reported.
        fake.force_mode(2);
        fake.refuse(true);
        let err = toggle(&conn).unwrap_err();
        assert!(err.starts_with("Fn mode not changed"), "{err}");
    }

    #[test]
    fn toggle_without_daemon_fails_cleanly() {
        let Some(bus) = crate::testbus::private_bus() else {
            return;
        };
        let conn = crate::testbus::connect(&bus.addr).unwrap();
        let err = toggle(&conn).unwrap_err();
        assert!(
            err.contains("systemctl --user start apple-kb-monitord"),
            "{err}"
        );
    }

    /// #99: the shortcut is visible in System Settings > Shortcuts and has
    /// no key until the user gives it one.
    #[test]
    fn global_shortcut_file_declares_the_toggle_without_a_default_key() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let f = std::fs::read_to_string(
            root.join("data/com.agenceapi.AppleKbMonitor.shortcuts.desktop"),
        )
        .unwrap();
        let group = |name: &str| -> Vec<&str> {
            f.lines()
                .skip_while(|l| l.trim() != format!("[{name}]"))
                .skip(1)
                .take_while(|l| !l.starts_with('['))
                .collect()
        };
        let main = group("Desktop Entry");
        assert!(main.contains(&"Actions=ToggleFnMode;"));
        assert!(main.contains(&"X-KDE-GlobalShortcutType=Service"));
        assert!(main.contains(&"Exec=apihub-app"));
        let action = group("Desktop Action ToggleFnMode");
        assert!(action.contains(&format!("Exec=apihub-app {FLAG}").as_str()));
        assert!(action.iter().any(|l| l.starts_with("Name=")));
        assert!(action.iter().any(|l| l.starts_with("Name[fr]=")));
        // No default key anywhere: the user assigns one.
        for l in f.lines().filter(|l| l.starts_with("X-KDE-Shortcuts")) {
            assert_eq!(l, "X-KDE-Shortcuts=", "default key declared: {l}");
        }
        assert_eq!(
            f.lines().filter(|l| *l == "X-KDE-Shortcuts=").count(),
            2,
            "the launch entry and the action both declare an empty shortcut"
        );
        // The PKGBUILD installs it where kglobalacceld looks for it.
        let pkgbuild = std::fs::read_to_string(root.join("PKGBUILD")).unwrap();
        assert!(pkgbuild
            .contains("usr/share/kglobalaccel/com.agenceapi.AppleKbMonitor.shortcuts.desktop"));
    }
}
