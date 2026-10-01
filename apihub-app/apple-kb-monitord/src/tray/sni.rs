//! `org.kde.StatusNotifierItem` at `/StatusNotifierItem`, and the tray
//! arbitration interface `com.agenceapi.AppleKbMonitor1.Tray` exported on the
//! daemon's own connection (docs/REVUE-UI-TRAY.md §4, §5.2).

use std::sync::mpsc::Sender;

use zbus::interface;
use zbus::message::Header;

use super::{lock, now_unix, Action, Event, SharedRef};

/// StatusNotifierItem tooltips are rich text (HTML subset): the keyboard name
/// is data (BlueZ alias, writable by any account), so `<img src=http://...>`
/// would make the shell fetch a URL (#205). Escape what the host would parse.
pub use apple_kb_monitord::notify::escape_markup;

/// ToolTip wire type: (icon_name, icon_pixmap[], title, description).
pub type ToolTip = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

pub struct Item {
    pub shared: SharedRef,
    pub tx: Sender<Event>,
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "Hardware"
    }
    #[zbus(property)]
    fn id(&self) -> String {
        lock(&self.shared).item_id.clone()
    }
    #[zbus(property)]
    fn title(&self) -> String {
        lock(&self.shared).title.clone()
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        lock(&self.shared).view.status.as_str()
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        lock(&self.shared).icon_name.clone()
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> String {
        lock(&self.shared).icon_theme_path.clone()
    }
    #[zbus(property)]
    fn attention_icon_name(&self) -> String {
        lock(&self.shared).attention_icon_name.clone()
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
    fn attention_movie_name(&self) -> &str {
        ""
    }
    /// Computed on read: the "updated … ago" line is always current.
    #[zbus(property)]
    fn tool_tip(&self) -> ToolTip {
        let s = lock(&self.shared);
        (
            s.icon_name.clone(),
            Vec::new(),
            escape_markup(&s.view.tooltip_title),
            escape_markup(&s.view.tooltip_body(s.lang, now_unix(), s.has_keyboard)),
        )
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
        zbus::zvariant::OwnedObjectPath::try_from(super::MENU_PATH)
            .expect("constant object path is valid")
    }

    /// Left click: open the window (D-Bus activation of `apihub-app`).
    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.tx.send(Event::Action(Action::Open));
    }
    /// Middle click: immediate refresh.
    fn secondary_activate(&self, _x: i32, _y: i32) {
        let _ = self.tx.send(Event::Action(Action::Refresh));
    }
    /// Hosts that do not use dbusmenu would call this: nothing to show.
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn scroll(&self, _delta: i32, _orientation: &str) {}
    /// KDE extension: activation token for the window we are about to open
    /// (focus without focus stealing under Wayland).
    fn provide_xdg_activation_token(&self, token: String) {
        lock(&self.shared).xdg_token = Some(token).filter(|t| !t.is_empty());
    }

    #[zbus(signal)]
    pub async fn new_icon(ctxt: &zbus::object_server::SignalContext<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    pub async fn new_attention_icon(
        ctxt: &zbus::object_server::SignalContext<'_>,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    pub async fn new_title(ctxt: &zbus::object_server::SignalContext<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    pub async fn new_status(
        ctxt: &zbus::object_server::SignalContext<'_>,
        status: &str,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    pub async fn new_tool_tip(ctxt: &zbus::object_server::SignalContext<'_>) -> zbus::Result<()>;
}

/// Arbitration with the Plasma widget: while at least one client holds a
/// claim, the SNI is withdrawn. A claim dies with its client's unique name
/// (plasmashell restarted → the icon comes back by itself).
pub struct Control {
    pub tx: Sender<Event>,
    pub shared: SharedRef,
}

impl Control {
    fn claim(&self, hdr: &Header<'_>, instance: &str) -> zbus::fdo::Result<()> {
        let sender = hdr
            .sender()
            .map(|s| s.to_string())
            .ok_or_else(|| zbus::fdo::Error::Failed("no sender".into()))?;
        let _ = self.tx.send(Event::Claim(sender, instance.to_string()));
        Ok(())
    }
    fn release(&self, hdr: &Header<'_>, instance: &str) {
        if let Some(s) = hdr.sender() {
            let _ = self.tx.send(Event::Release(s.to_string(), instance.to_string()));
        }
    }
}

#[interface(name = "com.agenceapi.AppleKbMonitor1.Tray")]
impl Control {
    /// Withdraw the tray icon for as long as the caller is on the bus.
    fn claim_tray(&self, #[zbus(header)] hdr: Header<'_>) -> zbus::fdo::Result<()> {
        self.claim(&hdr, "")
    }
    /// Drop the caller's claim.
    fn release_tray(&self, #[zbus(header)] hdr: Header<'_>) {
        self.release(&hdr, "");
    }
    /// Same, for one widget instance of the caller (plasmashell hosts all of
    /// them under one bus name): the icon comes back when the last instance
    /// is released or when the caller leaves the bus (#253). Idempotent.
    fn claim_tray_for(&self, instance: String, #[zbus(header)] hdr: Header<'_>) -> zbus::fdo::Result<()> {
        self.claim(&hdr, &instance)
    }
    fn release_tray_for(&self, instance: String, #[zbus(header)] hdr: Header<'_>) {
        self.release(&hdr, &instance);
    }
    /// Show the icon again after "Quit (hide icon)".
    fn show_tray(&self) {
        let _ = self.tx.send(Event::Unhide);
    }
    /// Whether the SNI item is currently registered on the bus.
    #[zbus(property)]
    fn visible(&self) -> bool {
        lock(&self.shared).visible
    }
    /// `auto` | `always` | `never` (env `APPLE_KB_MONITOR_TRAY`).
    #[zbus(property)]
    fn mode(&self) -> String {
        lock(&self.shared).mode.as_str().to_string()
    }
}

#[cfg(test)]
mod escape_tests {
    use super::escape_markup;

    #[test]
    fn html_in_a_name_is_neutralised() {
        let evil = r#"<img src="http://127.0.0.1:9/x">Clavier & <b>co</b>"#;
        let e = escape_markup(evil);
        assert!(!e.contains('<') && !e.contains('>') && !e.contains('"'));
        assert_eq!(
            e,
            "&lt;img src=&quot;http://127.0.0.1:9/x&quot;&gt;Clavier &amp; &lt;b&gt;co&lt;/b&gt;"
        );
        assert_eq!(escape_markup("Clavier de alice #1 \u{2014} 99%"), "Clavier de alice #1 \u{2014} 99%");
    }
}
