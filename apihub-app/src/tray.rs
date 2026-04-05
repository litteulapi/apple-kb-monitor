//! System tray via StatusNotifierItem — pure zbus implementation.
//!
//! Replaces ksni (which uses dbus-rs busy-polling at 50ms intervals = 3% CPU)
//! with zbus async I/O that uses epoll — near-zero CPU when idle.
//!
//! Implements:
//! - org.kde.StatusNotifierItem  (icon, tooltip, scroll, activate)
//! - com.canonical.dbusmenu      (right-click menu: info, brightness, picture
//!                                 mode, MQTT, Show Window, Quit)

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::{OwnedValue, Signature, Value};

use crate::ddc;

type State = Arc<Mutex<crate::SharedState>>;

// ── org.kde.StatusNotifierItem ───────────────────────────────────────────────

struct SniItem {
    tooltip: Arc<Mutex<String>>,
    i2c_bus: String,
    show_window: Arc<AtomicBool>,
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
        let desc = self.tooltip.lock().map(|t| t.clone()).unwrap_or_default();
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
        self.show_window.store(true, Ordering::Relaxed);
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {}
    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, delta: i32, orientation: &str) {
        if orientation == "vertical" || orientation == "Vertical" {
            if let Ok((cur, _)) = ddc::ddc_read_vcp(&self.i2c_bus, 0x10) {
                let new_val = if delta > 0 {
                    (cur + 1).min(100)
                } else {
                    cur.saturating_sub(1)
                };
                let _ = ddc::ddc_write_vcp(&self.i2c_bus, 0x10, new_val);
            }
        }
    }

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
    pub const INFO_BRIGHTNESS: i32 = 4;
    pub const INFO_REMAINING: i32 = 5;
    pub const SEP1: i32 = 10;
    pub const MONITOR_SUB: i32 = 20;
    pub const BRI_10: i32 = 21;
    pub const BRI_30: i32 = 22;
    pub const BRI_50: i32 = 23;
    pub const BRI_70: i32 = 24;
    pub const BRI_100: i32 = 25;
    pub const SEP_BRI: i32 = 26;
    pub const PM_CUSTOM: i32 = 30;
    pub const PM_READER: i32 = 31;
    pub const PM_VIVID: i32 = 32;
    pub const PM_SRGB: i32 = 33;
    pub const PM_FPS1: i32 = 34;
    pub const PM_FPS2: i32 = 35;
    pub const PM_RTS: i32 = 36;
    pub const PM_CINEMA: i32 = 37;
    pub const PM_HDR: i32 = 38;
    pub const PM_DCIP3: i32 = 39;
    pub const PM_PHOTO: i32 = 40;
    pub const MQTT_SUB: i32 = 50;
    pub const MQTT_STATUS: i32 = 51;
    pub const MQTT_PUBLISH: i32 = 52;
    pub const SEP2: i32 = 60;
    pub const SHOW_WINDOW: i32 = 61;
    pub const QUIT: i32 = 62;
}

struct DbusmenuServer {
    state: State,
    i2c_bus: String,
    show_window: Arc<AtomicBool>,
    quit_flag: Arc<AtomicBool>,
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
        let snap = self.state.lock().ok();
        let mut root_children: Vec<MenuItem> = Vec::new();

        // ── Info section ──────────────────────────────────────────
        if let Some(ref snap) = snap {
            if let Some(ref kb) = snap.keyboard {
                let pct = kb
                    .battery
                    .percentage_fine
                    .or(kb.battery.percentage_interpolated)
                    .or(kb.battery.percentage)
                    .unwrap_or(0.0);
                let voltage = kb.battery.voltage.unwrap_or(0.0);
                root_children.push(info_item(
                    menu_id::INFO_BATTERY,
                    format!("Battery: {:.0}%  ({:.3}V)", pct, voltage),
                ));
                if let Some(ref rssi) = kb.radio.rssi_dbm {
                    root_children.push(info_item(
                        menu_id::INFO_RSSI,
                        format!("RSSI: {} dBm", rssi),
                    ));
                }
                let caps = if snap.caps_lock { "ON" } else { "off" };
                let num = if snap.num_lock { "ON" } else { "off" };
                root_children.push(info_item(
                    menu_id::INFO_LOCKS,
                    format!("CapsLock: {}  NumLock: {}", caps, num),
                ));
            }
            let bri = snap.ddc.data.get("brightness").map(|v| v.0).unwrap_or(0);
            let vol = snap.ddc.data.get("volume").map(|v| v.0).unwrap_or(0);
            root_children.push(info_item(
                menu_id::INFO_BRIGHTNESS,
                format!("Brightness: {}%  Volume: {}%", bri, vol),
            ));
            if let Some(ref rem) = snap.remaining_display {
                root_children.push(info_item(
                    menu_id::INFO_REMAINING,
                    format!("Remaining: {}", rem),
                ));
            }
        }

        root_children.push(sep(menu_id::SEP1));

        // ── Monitor submenu ───────────────────────────────────────
        let mut monitor = Vec::new();
        for (id, label) in [
            (menu_id::BRI_10, "Brightness 10%"),
            (menu_id::BRI_30, "Brightness 30%"),
            (menu_id::BRI_50, "Brightness 50%"),
            (menu_id::BRI_70, "Brightness 70%"),
            (menu_id::BRI_100, "Brightness 100%"),
        ] {
            monitor.push(action_item(id, label));
        }
        monitor.push(sep(menu_id::SEP_BRI));
        for (id, label) in [
            (menu_id::PM_CUSTOM, "Custom"),
            (menu_id::PM_READER, "Reader"),
            (menu_id::PM_VIVID, "Vivid"),
            (menu_id::PM_SRGB, "sRGB"),
            (menu_id::PM_FPS1, "FPS 1"),
            (menu_id::PM_FPS2, "FPS 2"),
            (menu_id::PM_RTS, "RTS"),
            (menu_id::PM_CINEMA, "Cinema"),
            (menu_id::PM_HDR, "HDR Effect"),
            (menu_id::PM_DCIP3, "DCI-P3"),
            (menu_id::PM_PHOTO, "Photo"),
        ] {
            monitor.push(action_item(id, label));
        }
        root_children.push(sub(menu_id::MONITOR_SUB, "Monitor", monitor));

        // ── MQTT submenu ──────────────────────────────────────────
        let mqtt_connected = snap.as_ref().map(|s| s.mqtt_connected).unwrap_or(false);
        root_children.push(sub(
            menu_id::MQTT_SUB,
            "MQTT",
            vec![
                info_item(
                    menu_id::MQTT_STATUS,
                    format!(
                        "Status: {}",
                        if mqtt_connected { "Connected" } else { "Disconnected" }
                    ),
                ),
                action_item(menu_id::MQTT_PUBLISH, "Publish Now"),
            ],
        ));

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
        let bus = &self.i2c_bus;
        match id {
            BRI_10 => { let _ = ddc::ddc_write_vcp(bus, 0x10, 10); }
            BRI_30 => { let _ = ddc::ddc_write_vcp(bus, 0x10, 30); }
            BRI_50 => { let _ = ddc::ddc_write_vcp(bus, 0x10, 50); }
            BRI_70 => { let _ = ddc::ddc_write_vcp(bus, 0x10, 70); }
            BRI_100 => { let _ = ddc::ddc_write_vcp(bus, 0x10, 100); }
            PM_CUSTOM => { let _ = ddc::ddc_write_vcp(bus, 0x15, 45); }
            PM_READER => { let _ = ddc::ddc_write_vcp(bus, 0x15, 1); }
            PM_VIVID => { let _ = ddc::ddc_write_vcp(bus, 0x15, 20); }
            PM_SRGB => { let _ = ddc::ddc_write_vcp(bus, 0x15, 15); }
            PM_FPS1 => { let _ = ddc::ddc_write_vcp(bus, 0x15, 30); }
            PM_FPS2 => { let _ = ddc::ddc_write_vcp(bus, 0x15, 31); }
            PM_RTS => { let _ = ddc::ddc_write_vcp(bus, 0x15, 39); }
            PM_CINEMA => { let _ = ddc::ddc_write_vcp(bus, 0x15, 46); }
            PM_HDR => { let _ = ddc::ddc_write_vcp(bus, 0x15, 22); }
            PM_DCIP3 => { let _ = ddc::ddc_write_vcp(bus, 0x15, 24); }
            PM_PHOTO => { let _ = ddc::ddc_write_vcp(bus, 0x15, 48); }
            MQTT_PUBLISH => eprintln!("[tray] publish requested"),
            SHOW_WINDOW => self.show_window.store(true, Ordering::Relaxed),
            QUIT => self.quit_flag.store(true, Ordering::Relaxed),
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

fn sub(id: i32, label: &str, children: Vec<MenuItem>) -> MenuItem {
    MenuItem {
        id,
        props: vec![
            ("label", Value::from(String::from(label))),
            ("children-display", Value::from("submenu")),
        ],
        children,
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Spawn the system tray on a dedicated thread (fire-and-forget, like ksni).
///
/// The thread owns the D-Bus connection and serves requests via epoll —
/// near-zero CPU when idle (vs ksni's 50ms busy-poll loop).
pub fn spawn(
    tooltip: Arc<Mutex<String>>,
    state: State,
    i2c_bus: String,
    show_window: Arc<AtomicBool>,
    quit_flag: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("tray-sni".into())
        .spawn(move || {
            if let Err(e) = run(tooltip, state, i2c_bus, show_window, quit_flag) {
                eprintln!("[tray] fatal: {}", e);
            }
        })
        .expect("failed to spawn tray thread");
}

fn run(
    tooltip: Arc<Mutex<String>>,
    state: State,
    i2c_bus: String,
    show_window: Arc<AtomicBool>,
    quit_flag: Arc<AtomicBool>,
) -> zbus::Result<()> {
    let sni = SniItem {
        tooltip,
        i2c_bus: i2c_bus.clone(),
        show_window: show_window.clone(),
    };
    let menu = DbusmenuServer {
        state,
        i2c_bus,
        show_window,
        quit_flag,
        revision: Arc::new(AtomicU32::new(1)),
    };

    let pid = std::process::id();
    let bus_name = format!("org.kde.StatusNotifierItem-{}-1", pid);

    let conn = Connection::session()?;
    conn.request_name(bus_name.as_str())?;

    conn.object_server().at("/StatusNotifierItem", sni)?;
    conn.object_server().at("/MenuBar", menu)?;

    // Register with the host panel's StatusNotifierWatcher
    match conn.call_method(
        Some("org.kde.StatusNotifierWatcher"),
        "/StatusNotifierWatcher",
        Some("org.kde.StatusNotifierWatcher"),
        "RegisterStatusNotifierItem",
        &bus_name,
    ) {
        Ok(_) => eprintln!("[tray] registered with StatusNotifierWatcher"),
        Err(e) => eprintln!("[tray] watcher unavailable (icon may not appear): {}", e),
    }

    // Block forever. zbus serves D-Bus messages internally via its async
    // runtime — the thread sleeps on epoll until a message arrives.
    loop {
        std::thread::park();
    }
}
