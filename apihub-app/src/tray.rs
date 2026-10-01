//! System tray via StatusNotifierItem — pure zbus implementation.
//!
//! Replaces ksni (which uses dbus-rs busy-polling at 50ms intervals = 3% CPU)
//! with zbus async I/O that uses epoll — near-zero CPU when idle.
//!
//! Implements:
//! - org.kde.StatusNotifierItem  (icon, tooltip, scroll, activate)
//! - com.canonical.dbusmenu      (right-click menu: battery/RSSI/LED info, Show Window, Quit)

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::{OwnedValue, Signature, Value};

type State = Arc<akm_core::Watch>;

/// Requests from the tray to the main thread (no polling loop in main()).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCmd {
    ShowWindow,
    Quit,
}

/// Shared by the SNI item and the menu: flags for an open window + wake-up of main().
#[derive(Clone)]
struct Ui {
    show_window: Arc<AtomicBool>,
    quit_flag: Arc<AtomicBool>,
    tx: Arc<Mutex<Sender<UiCmd>>>,
}

impl Ui {
    fn show(&self) {
        self.show_window.store(true, Ordering::Relaxed);
        let _ = self.tx.lock().map(|t| t.send(UiCmd::ShowWindow));
    }
    fn quit(&self) {
        self.quit_flag.store(true, Ordering::Relaxed);
        let _ = self.tx.lock().map(|t| t.send(UiCmd::Quit));
    }
}

// ── org.kde.StatusNotifierItem ───────────────────────────────────────────────

struct SniItem {
    state: State,
    ui: Ui,
}

/// ToolTip wire type: (icon_name, icon_pixmap[], title, description)
type ToolTipValue = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

#[interface(name = "org.kde.StatusNotifierItem")]
impl SniItem {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }
    #[zbus(property)]
    fn id(&self) -> &str {
        "apihub-app"
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        "ApiHub"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        "apihub-scarab"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn attention_icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn overlay_icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }
    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }
    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }
    #[zbus(property)]
    fn tool_tip(&self) -> ToolTipValue {
        let desc = self.state.get().tooltip_text();
        ("apihub-scarab".into(), Vec::new(), "ApiHub".into(), desc)
    }
    #[zbus(property)]
    fn window_id(&self) -> i32 {
        0
    }
    #[zbus(property, name = "ItemIsMenu")]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> zbus::zvariant::OwnedObjectPath {
        zbus::zvariant::OwnedObjectPath::try_from("/MenuBar").unwrap()
    }

    fn activate(&self, _x: i32, _y: i32) {
        self.ui.show();
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {}
    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, _delta: i32, _orientation: &str) {}

    #[zbus(signal)]
    async fn new_icon(ctxt: &zbus::object_server::SignalContext<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_title(ctxt: &zbus::object_server::SignalContext<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_status(
        ctxt: &zbus::object_server::SignalContext<'_>,
        status: &str,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_tool_tip(ctxt: &zbus::object_server::SignalContext<'_>) -> zbus::Result<()>;
}

// ── com.canonical.dbusmenu ───────────────────────────────────────────────────
//
// The DBusMenu GetLayout return type is recursive: (ia{sv}av) where each
// variant in av is itself (ia{sv}av). We build it with zvariant's
// StructureBuilder / Dict / Array using only Value-level APIs to avoid
// trait-resolution issues with the dual-zvariant dependency tree.

/// Stable menu item IDs used by the desktop host to invoke actions.
mod menu_id {
    pub const INFO_BATTERY: i32 = 1;
    pub const INFO_RSSI: i32 = 2;
    pub const INFO_LOCKS: i32 = 3;
    pub const INFO_REMAINING: i32 = 5;
    pub const SEP1: i32 = 10;
    pub const SEP2: i32 = 60;
    pub const SHOW_WINDOW: i32 = 61;
    pub const QUIT: i32 = 62;
}

struct DbusmenuServer {
    state: State,
    ui: Ui,
    revision: Arc<AtomicU32>,
}

/// An intermediate menu node before conversion to Value.
struct MenuItem {
    id: i32,
    props: Vec<(&'static str, Value<'static>)>,
    children: Vec<MenuItem>,
}

impl MenuItem {
    /// Convert to the recursive DBusMenu Value: (ia{sv}av)
    ///
    /// Uses StructureBuilder::append_field and Dict::append to operate
    /// purely at the Value level, avoiding trait-resolution issues with
    /// the dual-zvariant versions in the dependency graph.
    fn into_value(self) -> Value<'static> {
        use zbus::zvariant::{Array, Dict, StructureBuilder};

        let mut dict = Dict::new(
            Signature::from_str_unchecked("s"),
            Signature::from_str_unchecked("v"),
        );
        for (k, v) in self.props {
            let _ = dict.append(Value::from(k), Value::Value(Box::new(v)));
        }

        let children_vals: Vec<Value<'static>> = self
            .children
            .into_iter()
            .map(|c| Value::Value(Box::new(c.into_value())))
            .collect();
        let children_arr = Array::from(children_vals);

        Value::Structure(
            StructureBuilder::new()
                .append_field(Value::I32(self.id))
                .append_field(Value::Dict(dict))
                .append_field(Value::Array(children_arr))
                .build(),
        )
    }
}

impl DbusmenuServer {
    /// Build the full menu tree from current shared state.
    fn build_layout(&self) -> Value<'static> {
        let snap = Some(self.state.get());
        let mut root_children: Vec<MenuItem> = Vec::new();

        // ── Info section ──────────────────────────────────────────
        if let Some(ref snap) = snap {
            if let Some(ref kb) = snap.keyboard {
                let pct = kb.battery_pct().unwrap_or(0.0);
                // Measured voltage (reports 0x46/0xFF), rounded to 0.01 V.
                let mut label = match kb.battery.voltage {
                    Some(v) => format!(
                        "Keyboard indication: {:.0}%  ({})",
                        pct,
                        crate::view::volts_text(v)
                    ),
                    None => format!("Keyboard indication: {:.0}%", pct),
                };
                if let Some(e) = crate::view::estimate_text(&kb.battery) {
                    label += &format!("  \u{b7}  estimate {e}");
                }
                root_children.push(info_item(menu_id::INFO_BATTERY, label));
                // Relative BR/EDR value, no unit (#174).
                if let Some(rssi) = kb.radio.rel_db() {
                    root_children.push(info_item(
                        menu_id::INFO_RSSI,
                        format!("Signal: {}", crate::view::rssi_text(Some(rssi))),
                    ));
                }
                let caps = if snap.caps_lock { "ON" } else { "off" };
                let num = if snap.num_lock { "ON" } else { "off" };
                root_children.push(info_item(
                    menu_id::INFO_LOCKS,
                    format!("CapsLock: {}  NumLock: {}", caps, num),
                ));
            }
            if let Some(ref rem) = snap.remaining_display {
                root_children.push(info_item(
                    menu_id::INFO_REMAINING,
                    format!("Remaining: {}", rem),
                ));
            }
        }

        root_children.push(sep(menu_id::SEP1));

        root_children.push(sep(menu_id::SEP2));
        root_children.push(action_item(menu_id::SHOW_WINDOW, "Show Window"));
        root_children.push(action_item(menu_id::QUIT, "Quit"));

        // Root node
        MenuItem {
            id: 0,
            props: vec![("children-display", Value::from("submenu"))],
            children: root_children,
        }
        .into_value()
    }

    /// Dispatch an item click by menu id.
    fn handle_event(&self, id: i32) {
        use menu_id::*;
        match id {
            SHOW_WINDOW => self.ui.show(),
            QUIT => self.ui.quit(),
            _ => {}
        }
    }
}

#[interface(name = "com.canonical.dbusmenu")]
impl DbusmenuServer {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    /// GetLayout(parentId, recursionDepth, propertyNames) -> (revision, layout)
    fn get_layout(
        &self,
        _parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, Value<'_>) {
        (self.revision.load(Ordering::Relaxed), self.build_layout())
    }

    /// GetGroupProperties — minimal impl (host uses GetLayout).
    fn get_group_properties(
        &self,
        _ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        Vec::new()
    }

    /// GetProperty — minimal impl.
    fn get_property(&self, _id: i32, _name: &str) -> zbus::fdo::Result<OwnedValue> {
        Err(zbus::fdo::Error::InvalidArgs("use GetLayout".into()))
    }

    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id == "clicked" {
            self.handle_event(id);
        }
    }

    fn event_group(&self, events: Vec<(i32, String, Value<'_>, u32)>) -> Vec<i32> {
        for (id, event_id, _, _) in &events {
            if event_id == "clicked" {
                self.handle_event(*id);
            }
        }
        Vec::new()
    }

    /// Signal the host to re-fetch layout (dynamic info items update on open).
    /// Note: `fetch_add(1, Relaxed)` on `AtomicU32` wraps at u32::MAX naturally
    /// (defined behavior per Rust atomics spec). The revision is only used by the
    /// DBusMenu host to detect changes — monotonicity across wrap is irrelevant.
    fn about_to_show(&self, _id: i32) -> bool {
        self.revision.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        self.revision.fetch_add(1, Ordering::Relaxed);
        (ids, Vec::new())
    }

    #[zbus(signal)]
    async fn layout_updated(
        ctxt: &zbus::object_server::SignalContext<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn items_properties_updated(
        ctxt: &zbus::object_server::SignalContext<'_>,
        updated_props: Vec<(i32, HashMap<String, OwnedValue>)>,
        removed_props: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn item_activation_requested(
        ctxt: &zbus::object_server::SignalContext<'_>,
        id: i32,
        timestamp: u32,
    ) -> zbus::Result<()>;
}

// ── Menu item builders ───────────────────────────────────────────────────────

fn info_item(id: i32, label: String) -> MenuItem {
    MenuItem {
        id,
        props: vec![
            ("label", Value::from(label)),
            ("enabled", Value::from(false)),
        ],
        children: Vec::new(),
    }
}

fn action_item(id: i32, label: &str) -> MenuItem {
    MenuItem {
        id,
        props: vec![("label", Value::from(String::from(label)))],
        children: Vec::new(),
    }
}

fn sep(id: i32) -> MenuItem {
    MenuItem {
        id,
        props: vec![("type", Value::from("separator"))],
        children: Vec::new(),
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Spawn the system tray on a dedicated thread (fire-and-forget).
///
/// The thread owns the D-Bus connection and serves requests via epoll —
/// near-zero CPU when idle. Clicks are forwarded to main() through `tx`.
pub fn spawn(state: State, show_window: Arc<AtomicBool>, quit_flag: Arc<AtomicBool>, tx: Sender<UiCmd>) {
    let ui = Ui { show_window, quit_flag, tx: Arc::new(Mutex::new(tx)) };
    std::thread::Builder::new()
        .name("tray-sni".into())
        .spawn(move || {
            // Retry: the session bus may not be ready yet at login. `run` only
            // returns on error (it parks forever on success).
            loop {
                if ui.quit_flag.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(e) = run(state.clone(), ui.clone()) {
                    eprintln!("[tray] error: {} — retrying in 10s", e);
                }
                std::thread::sleep(std::time::Duration::from_secs(10));
            }
        })
        .expect("failed to spawn tray thread");
}

const WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// Register the item with the watcher; the watcher may have just claimed its
/// name and not export its interface yet, so retry with a growing delay.
fn register_with_backoff(conn: &Connection, bus_name: &str) -> bool {
    let mut delay = 1u64;
    for attempt in 1..=8 {
        match conn.call_method(
            Some(WATCHER),
            "/StatusNotifierWatcher",
            Some(WATCHER),
            "RegisterStatusNotifierItem",
            &bus_name,
        ) {
            Ok(_) => {
                eprintln!("[tray] registered with StatusNotifierWatcher");
                return true;
            }
            Err(e) => {
                eprintln!("[tray] register attempt {}/8 failed: {}", attempt, e);
                std::thread::sleep(std::time::Duration::from_secs(delay));
                delay = (delay * 2).min(30);
            }
        }
    }
    false
}

fn run(state: State, ui: Ui) -> zbus::Result<()> {
    let sni = SniItem { state: state.clone(), ui: ui.clone() };
    let menu = DbusmenuServer { state, ui, revision: Arc::new(AtomicU32::new(1)) };

    let pid = std::process::id();
    let bus_name = format!("org.kde.StatusNotifierItem-{}-1", pid);

    let conn = Connection::session()?;
    conn.request_name(bus_name.as_str())?;

    conn.object_server().at("/StatusNotifierItem", sni)?;
    conn.object_server().at("/MenuBar", menu)?;

    // Subscribe BEFORE the first registration so a watcher appearing in
    // between is not missed.
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)?;
    let owner_changes = dbus.receive_name_owner_changed()?;

    // Initial registration, with backoff, if a watcher is already on the bus.
    // If the panel starts later, the NameOwnerChanged loop below registers us.
    if dbus
        .name_has_owner(WATCHER.try_into().expect("valid bus name"))
        .unwrap_or(false)
    {
        register_with_backoff(&conn, &bus_name);
    }

    // Stay registered for the whole life of the app: every time a
    // StatusNotifierWatcher (re)appears — plasmashell restart, panel crash —
    // register again. The signal iterator blocks on epoll (no polling).
    for sig in owner_changes {
        let Ok(args) = sig.args() else { continue };
        if args.name().as_str() == WATCHER && args.new_owner().is_some() {
            eprintln!("[tray] StatusNotifierWatcher (re)appeared — registering");
            register_with_backoff(&conn, &bus_name);
        }
    }
    // Signal stream ended: let the caller rebuild the connection.
    Err(zbus::Error::Failure("NameOwnerChanged stream ended".into()))
}
