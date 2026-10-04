//! D-Bus menu of the Plasma widget (`…1.Tray`): entries, their actions and the panel request.

mod actions;
mod view;

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use akm_core::{Snapshot, Watch};
use apple_kb_monitord::actor::{Mailbox, Msg};
use zbus::blocking::Connection;
use zbus::interface;

use view::{id, View};

/// Object of the menu interface on the daemon's connection.
pub const CONTROL_PATH: &str = "/com/agenceapi/AppleKbMonitor1/Tray";
pub const IFACE: &str = "com.agenceapi.AppleKbMonitor1.Tray";

pub(crate) struct Shared {
    view: View,
}

pub(crate) type SharedRef = Arc<Mutex<Shared>>;

pub(crate) fn lock(s: &SharedRef) -> MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// Open the widget's popup (signal `PanelRequested`).
    Open,
    Refresh,
    Copy,
    Bluetooth,
    Rename,
    Repair,
    Reconnect,
    Disconnect,
    /// Only asks, through a confirmation notification.
    Forget,
    FnMode(u8),
}

pub(crate) enum Event {
    Snapshot(Box<Snapshot>),
    Action(Action),
}

pub fn action_for(item: i32) -> Option<Action> {
    Some(match item {
        id::REFRESH => Action::Refresh,
        id::COPY => Action::Copy,
        id::BLUETOOTH => Action::Bluetooth,
        id::RENAME => Action::Rename,
        id::REPAIR => Action::Repair,
        id::RECONNECT => Action::Reconnect,
        id::DISCONNECT => Action::Disconnect,
        id::FORGET => Action::Forget,
        id::FN_MEDIA => Action::FnMode(view::FN_MEDIA_FIRST),
        id::FN_FKEYS => Action::FnMode(view::FN_FKEYS_FIRST),
        _ => return None,
    })
}

/// Only an entry with an action runs; an unknown id, an information row, or an entry
/// the current menu disables (a stale menu on the caller's side), is refused.
pub fn activation(item: i32, entry: Option<&view::Entry>) -> Result<Action, String> {
    if entry.is_some_and(|e| !e.enabled()) {
        return Err(format!("menu item {item} is disabled"));
    }
    action_for(item).ok_or_else(|| format!("no menu item {item}"))
}

/// Mode `ToggleFnMode` switches to, from the current `hid_apple.fnmode`.
pub fn next_fn_mode(current: Option<i32>) -> u8 {
    if current == Some(i32::from(view::FN_FKEYS_FIRST)) {
        view::FN_MEDIA_FIRST
    } else {
        view::FN_FKEYS_FIRST
    }
}

fn read_fn_mode() -> Option<i32> {
    akm_core::hid_params::Param::FnMode
        .read_in(std::path::Path::new(akm_core::hid_params::SYSFS_DIR))
}

/// Asks the widget to open its popup.
pub fn request_panel(conn: &Connection) {
    if let Err(e) = conn.emit_signal(None::<&str>, CONTROL_PATH, IFACE, "PanelRequested", &()) {
        tracing::warn!("tray: PanelRequested not sent: {e}");
    }
}

struct Control {
    tx: Sender<Event>,
    shared: SharedRef,
}

#[interface(name = "com.agenceapi.AppleKbMonitor1.Tray")]
impl Control {
    /// The widget's right-click menu: JSON of `View::menu_json`.
    fn menu_items(&self) -> String {
        lock(&self.shared).view.menu_json()
    }
    async fn activate_menu_item(
        &self,
        id: i32,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        // The menu reaches pkexec (Fn mode, forget, rename).
        apple_kb_monitord::devices::caller_uid(conn, &hdr).await?;
        let shared = lock(&self.shared);
        let entry = shared.view.menu.iter().find(|e| e.id == id);
        let found = activation(id, entry).map_err(zbus::fdo::Error::InvalidArgs);
        drop(shared);
        let _ = self.tx.send(Event::Action(found?));
        Ok(())
    }
    /// Opens the widget's popup (notifications, global shortcut).
    fn open_panel(&self) {
        let _ = self.tx.send(Event::Action(Action::Open));
    }
    /// Media keys first <-> F1–F12 first (global shortcut).
    async fn toggle_fn_mode(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        apple_kb_monitord::devices::caller_uid(conn, &hdr).await?;
        let _ = self
            .tx
            .send(Event::Action(Action::FnMode(next_fn_mode(read_fn_mode()))));
        Ok(())
    }
}

pub fn spawn(watch: Arc<Watch>, mailbox: Arc<Mailbox>, control: Option<Connection>) {
    let Some(conn) = control else { return };
    let shared: SharedRef = Arc::new(Mutex::new(Shared {
        view: View::build(&watch.get(), None),
    }));
    let (tx, rx) = mpsc::channel::<Event>();
    let ctl = Control {
        tx: tx.clone(),
        shared: shared.clone(),
    };
    if let Err(e) = conn.object_server().at(CONTROL_PATH, ctl) {
        tracing::warn!("tray: cannot export {CONTROL_PATH}: {e}");
        return;
    }
    {
        let tx = tx.clone();
        let watch = watch.clone();
        let _ = std::thread::Builder::new()
            .name("tray-watch".into())
            .spawn(move || {
                let mut seen = watch.version();
                loop {
                    if let Some(s) = watch.wait_newer(seen, Duration::from_hours(1)) {
                        seen = s.version;
                        if tx.send(Event::Snapshot(Box::new(s))).is_err() {
                            break;
                        }
                    }
                }
            });
    }
    let _ = std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || {
            let mut tray = Tray {
                shared,
                tx,
                mailbox,
                watch,
                charging: false,
                system: None,
                upower_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            };
            tray.apply(&tray.watch.get());
            for ev in rx {
                match ev {
                    Event::Snapshot(s) => tray.apply(&s),
                    Event::Action(a) => tray.action(a, &conn),
                }
            }
        });
}

fn rename_flow(conn: &Connection, mailbox: &Mailbox, mac: &str, current: &str) {
    let title = &tr!("Rename keyboard");
    let prompt = &tr!("New name (empty = the keyboard's own name):");
    let text = match actions::ask_name(title, prompt, current) {
        Ok(Some(t)) => t,
        Ok(None) => return,
        Err(()) => {
            tracing::info!("tray: no working dialog program, opening the widget");
            request_panel(conn);
            return;
        }
    };
    let backend = apple_kb_monitord::alias::BluezAlias::default();
    match apple_kb_monitord::alias::rename(&backend, mailbox, mac, &text, "tray (Rename keyboard…)")
    {
        Ok(now) => tracing::info!("tray: keyboard renamed to {now:?}"),
        Err(e) => {
            tracing::warn!("tray: rename refused: {e}");
            apple_kb_monitord::notify::send(title, &e.to_string(), "dialog-error");
        }
    }
}

struct Tray {
    shared: SharedRef,
    tx: Sender<Event>,
    mailbox: Arc<Mailbox>,
    watch: Arc<Watch>,
    charging: bool,
    system: Option<Connection>,
    upower_busy: Arc<std::sync::atomic::AtomicBool>,
}

impl Tray {
    fn system_bus(&mut self) -> Option<&Connection> {
        if self.system.is_none() {
            self.system = Connection::system().ok();
        }
        self.system.as_ref()
    }

    fn apply(&mut self, snap: &Snapshot) {
        let charging = if snap.connected && snap.keyboard.is_some() {
            let mac = snap.mac().map(str::to_string);
            let busy = self.upower_busy.clone();
            let last = self.charging;
            self.system_bus().is_some_and(|sys| {
                actions::upower_charging_bounded(sys, mac.as_deref(), &busy).unwrap_or(last)
            })
        } else {
            false
        };
        self.charging = charging;
        lock(&self.shared).view = View::build(snap, read_fn_mode());
    }

    #[allow(clippy::too_many_lines)] // one arm per menu action
    fn action(&mut self, a: Action, conn: &Connection) {
        match a {
            Action::Refresh => {
                let (reply, rx) = apple_kb_monitord::actor::RefreshReply::channel();
                if !self.mailbox.send(Msg::Refresh(reply)) {
                    tracing::warn!("tray: refresh ignored, acquisition thread not running");
                    return;
                }
                // A refused refresh is said, not swallowed.
                let _ = std::thread::Builder::new()
                    .name("tray-refresh".into())
                    .spawn(move || {
                        let Ok(o) = rx.recv_timeout(Duration::from_secs(30)) else {
                            return;
                        };
                        if let Some(text) = apple_kb_monitord::actor::refresh_text(o) {
                            tracing::info!("tray: refresh not taken: {text}");
                            apple_kb_monitord::notify::send_with(
                                &tr!("No new reading"),
                                &text,
                                "dialog-information",
                                akm_core::alerts::Urgency::Normal,
                                true,
                            );
                        }
                    });
            }
            Action::Open => request_panel(conn),
            Action::Bluetooth => {
                let _ = std::thread::Builder::new()
                    .name("tray-bt".into())
                    .spawn(actions::open_bluetooth_settings);
            }
            Action::Repair => {
                let _ = std::thread::Builder::new()
                    .name("tray-repair".into())
                    .spawn(|| apple_kb_monitord::repair::launch_repair(None));
            }
            Action::Reconnect => {
                tracing::info!("tray: reconnection asked from the menu");
                if apple_kb_monitord::repair::reconnect_in_flight() {
                    // Say it, instead of a click that does nothing.
                    tracing::info!(
                        "tray: an attempt is already waiting for BlueZ, nothing new sent"
                    );
                    apple_kb_monitord::notify::send_with(
                        &tr!("Reconnection in progress"),
                        &tr!("An attempt is already waiting for BlueZ (40 s at most)."),
                        "network-bluetooth",
                        akm_core::alerts::Urgency::Normal,
                        true,
                    );
                } else if !apple_kb_monitord::repair::user_reconnect() {
                    tracing::warn!("tray: link keeper not running, nothing asked");
                }
            }
            Action::Disconnect => {
                let mac = self.watch.get().mac().map(str::to_string);
                tracing::info!("tray: disconnection asked from the menu");
                if !apple_kb_monitord::repair::user_disconnect(mac.as_deref()) {
                    tracing::warn!("tray: link keeper not running, nothing asked");
                }
            }
            Action::Forget => {
                let snap = self.watch.get();
                let Some(mac) = snap.mac().map(str::to_string) else {
                    return;
                };
                let name = snap
                    .display_name()
                    .or(snap.model())
                    .unwrap_or(mac.as_str())
                    .to_string();
                // Only asks: the removal needs the button of the notification.
                if !apple_kb_monitord::forget::request(&mac, &name) {
                    tracing::warn!("tray: forget of {mac} not asked");
                }
            }
            Action::FnMode(mode) => {
                let snap = self.watch.get();
                let Some(mac) = snap.mac().map(str::to_string) else {
                    return;
                };
                let (tx, watch, c) = (self.tx.clone(), self.watch.clone(), conn.clone());
                let _ = std::thread::Builder::new()
                    .name("tray-fnmode".into())
                    .spawn(move || {
                        tracing::info!("tray: Fn mode {mode} asked from the menu");
                        let res = actions::set_fn_mode(&c, &mac, i32::from(mode));
                        if let Err(e) = res {
                            tracing::warn!("tray: Fn mode not changed: {e}");
                            apple_kb_monitord::notify::send(
                                &tr!("Fn mode not changed"),
                                &fn_mode_error_text(&e),
                                "dialog-error",
                            );
                        }
                        // Show the mode now in effect (read back from sysfs).
                        let _ = tx.send(Event::Snapshot(Box::new(watch.get())));
                    });
            }
            Action::Rename => {
                let snap = self.watch.get();
                let Some(mac) = snap.mac().map(str::to_string) else {
                    return;
                };
                let current = snap.display_name().unwrap_or_default().to_string();
                let mailbox = self.mailbox.clone();
                let c = conn.clone();
                let _ = std::thread::Builder::new()
                    .name("tray-rename".into())
                    .spawn(move || rename_flow(&c, &mailbox, &mac, &current));
            }
            Action::Copy => {
                let text = view::clipboard_text(&self.watch.get(), self.charging, now_unix());
                let c = conn.clone();
                let _ = std::thread::Builder::new()
                    .name("tray-copy".into())
                    .spawn(move || {
                        if actions::copy_to_clipboard(&c, &text) {
                            tracing::info!("tray: information copied to the clipboard");
                        } else {
                            tracing::warn!("tray: no clipboard available");
                        }
                    });
            }
        }
    }
}

/// What the user reads when the Fn mode was not changed.
fn fn_mode_error_text(e: &zbus::fdo::Error) -> String {
    match e {
        zbus::fdo::Error::LimitsExceeded(msg) => match crate::settings::cooldown_secs(msg) {
            Some(n) => tr!(
                "A Fn mode change just finished: try again in {w}.",
                w = format!("{n} s")
            ),
            None if *msg == crate::settings::pending_text() => {
                tr!("A Fn mode change is already waiting for authentication.")
            }
            None => msg.clone(),
        },
        other => zbus::DBusError::description(other)
            .unwrap_or_default()
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activate_menu_item_takes_only_the_item_id() {
        use zbus::object_server::Interface;
        let (tx, _rx) = mpsc::channel();
        let ctl = Control {
            tx,
            shared: Arc::new(Mutex::new(Shared {
                view: View::build(&Snapshot::default(), None),
            })),
        };
        let mut xml = String::new();
        ctl.introspect_to_writer(&mut xml, 0);
        let m = xml
            .split("<method name=\"ActivateMenuItem\">")
            .nth(1)
            .unwrap();
        let m = &m[..m.find("</method>").unwrap()];
        assert_eq!(m.matches("<arg ").count(), 1, "{m}");
        assert!(m.contains("type=\"i\""), "{m}");
    }

    #[test]
    fn control_methods_reaching_pkexec_check_the_callers_uid() {
        let code = akm_core::srclint::prod_tokens(include_str!("tray.rs"));
        for m in ["asyncfnactivate_menu_item(", "asyncfntoggle_fn_mode("] {
            let body = &code[code.find(m).unwrap()..];
            let body = &body[..body.find("self.tx").unwrap()];
            assert!(
                body.contains("devices::caller_uid(conn,&hdr).await?"),
                "{m}"
            );
        }
    }

    #[test]
    fn fn_mode_refusal_is_said_in_words_not_as_a_dbus_name() {
        let e = zbus::fdo::Error::LimitsExceeded(
            "a change of the Fn mode just ended, retry in 3 s".into(),
        );
        let fr = fn_mode_error_text(&e);
        assert_eq!(fr, "A Fn mode change just finished: try again in 3 s.");
        assert!(!fn_mode_error_text(&e).contains("org.freedesktop"));
        let busy = zbus::fdo::Error::LimitsExceeded;
        assert_eq!(
            fn_mode_error_text(&busy(crate::settings::cooldown_text(7))),
            "A Fn mode change just finished: try again in 7 s."
        );
        assert_eq!(
            fn_mode_error_text(&busy(crate::settings::pending_text())),
            "A Fn mode change is already waiting for authentication."
        );
        assert_eq!(
            fn_mode_error_text(&busy("no thread for the request: x".into())),
            "no thread for the request: x"
        );
    }

    #[test]
    fn open_and_quit_rows_are_gone() {
        let row = |id, props| view::Entry { id, props };
        assert!(activation(11, None).is_err() && activation(21, None).is_err());
        // 1 was the header information row.
        assert!(activation(1, Some(&row(1, vec![]))).is_err());
    }

    #[test]
    fn a_disabled_entry_is_not_run() {
        let off = view::Entry {
            id: id::REPAIR,
            props: vec![("enabled", view::Prop::Bool(false))],
        };
        assert!(activation(id::REPAIR, Some(&off)).is_err());
    }

    #[test]
    fn toggle_goes_to_the_other_mode() {
        assert_eq!(next_fn_mode(Some(2)), view::FN_MEDIA_FIRST);
        assert_eq!(next_fn_mode(Some(1)), view::FN_FKEYS_FIRST);
        assert_eq!(next_fn_mode(None), view::FN_FKEYS_FIRST);
    }
}
