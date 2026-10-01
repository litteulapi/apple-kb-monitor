//! Tray icon of the daemon: StatusNotifierItem + dbusmenu in pure zbus
//! (issues #115 U1, #116 U2, #86 F05; docs/REVUE-UI-TRAY.md §4-§5).
//!
//! * Reads the same [`Watch`] as the D-Bus interface, never writes to it, and
//!   asks for refreshes through the actor [`Mailbox`] like `Refresh()` does.
//! * Runs on its own thread with its own session connection; a panic is
//!   caught and the tray restarts without touching acquisition.
//! * Registers with `org.kde.StatusNotifierWatcher` and registers again every
//!   time the watcher (re)appears (plasmashell restart).
//! * No duplicate with the Plasma widget: the SNI is withdrawn (bus name
//!   released) while a client holds a claim through
//!   `com.agenceapi.AppleKbMonitor1.Tray.ClaimTray()`, or (mode `auto`) while
//!   plasmashell runs with `com.agenceapi.devicehub` enabled in its system
//!   tray. Mode from `APPLE_KB_MONITOR_TRAY` = `auto` (default) | `always` |
//!   `never`.
//! * Signals `NewIcon` / `NewToolTip` / `NewStatus` / `LayoutUpdated` /
//!   `ItemsPropertiesUpdated` only on real changes; nothing wakes up while
//!   the keyboard state does not change.
//!
//! Icons: our symbolic set `apihub-kb-*-symbolic` (icons/hicolor) when it is
//! installed or found next to the sources (`APPLE_KB_MONITOR_ICON_DIR`
//! overrides), else Breeze / Adwaita battery names.

mod actions;
mod menu;
mod sni;
mod view;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use akm_core::{Snapshot, Watch};
use apple_kb_monitord::actor::{Mailbox, Msg};
use zbus::blocking::Connection;

use view::{IconSet, IconState, Lang, View};

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const PLASMASHELL: &str = "org.kde.plasmashell";
/// Object of the arbitration interface on the daemon's connection.
pub const CONTROL_PATH: &str = "/com/agenceapi/AppleKbMonitor1/Tray";
const WIDGET_ID: &str = "com.agenceapi.devicehub";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Always,
    Never,
}

impl Mode {
    pub fn parse(v: &str) -> Option<Self> {
        match v.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Some(Mode::Auto),
            "always" | "on" | "1" => Some(Mode::Always),
            "never" | "off" | "0" => Some(Mode::Never),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Always => "always",
            Mode::Never => "never",
        }
    }
}

/// Tray configuration (environment of the unit).
#[derive(Debug, Clone)]
pub struct Config {
    pub mode: Mode,
    /// SNI `Id` (a test instance uses its own).
    pub item_id: String,
    pub lang: Lang,
}

impl Config {
    pub fn from_env() -> Self {
        let mode = std::env::var("APPLE_KB_MONITOR_TRAY")
            .ok()
            .map(|v| {
                Mode::parse(&v).unwrap_or_else(|| {
                    tracing::warn!("APPLE_KB_MONITOR_TRAY={v}: unknown value, using auto");
                    Mode::Auto
                })
            })
            .unwrap_or(Mode::Auto);
        Self {
            mode,
            item_id: "apple-kb-monitor".into(),
            lang: Lang::from_env(),
        }
    }
}

// ── Shared state (read by the D-Bus objects) ────────────────────────────────

pub(crate) struct Shared {
    view: View,
    icon_name: String,
    attention_icon_name: String,
    icon_theme_path: String,
    has_keyboard: bool,
    lang: Lang,
    item_id: String,
    title: String,
    menu_rev: u32,
    xdg_token: Option<String>,
    visible: bool,
    mode: Mode,
}

pub(crate) type SharedRef = Arc<Mutex<Shared>>;

pub(crate) fn lock(s: &SharedRef) -> MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Open,
    Refresh,
    Copy,
    Bluetooth,
    Rename,
    /// `akmctl repair` in a terminal (#147).
    Repair,
    Hide,
}

pub(crate) enum Event {
    Snapshot(Box<Snapshot>),
    /// `NameOwnerChanged(name, new_owner)`.
    Owner(String, Option<String>),
    Claim(String),
    Release(String),
    Unhide,
    Action(Action),
}

// ── Icon set discovery ──────────────────────────────────────────────────────

const PROBE: &str = "hicolor/scalable/status/apihub-kb-battery-050-symbolic.svg";

fn data_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    match std::env::var_os("XDG_DATA_HOME") {
        Some(h) if !h.is_empty() => v.push(PathBuf::from(h)),
        _ => {
            if let Some(h) = std::env::var_os("HOME") {
                v.push(Path::new(&h).join(".local/share"));
            }
        }
    }
    let sys = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    v.extend(sys.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    v
}

/// (icon set, IconThemePath).
fn discover_icons() -> (IconSet, String) {
    if let Some(d) = std::env::var_os("APPLE_KB_MONITOR_ICON_DIR") {
        let d = PathBuf::from(d);
        if d.join(PROBE).is_file() {
            return (IconSet::Ours, d.to_string_lossy().into_owned());
        }
    }
    let dirs = data_dirs();
    if dirs.iter().any(|d| d.join("icons").join(PROBE).is_file()) {
        return (IconSet::Ours, String::new());
    }
    // Development tree / uninstalled build.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../icons");
    if src.join(PROBE).is_file() {
        if let Ok(c) = src.canonicalize() {
            return (IconSet::Ours, c.to_string_lossy().into_owned());
        }
    }
    let breeze = dirs.iter().any(|d| {
        d.join("icons/breeze/status/16/battery-050-symbolic.svg")
            .is_file()
    });
    (
        if breeze {
            IconSet::Breeze
        } else {
            IconSet::Adwaita
        },
        String::new(),
    )
}

// ── Plasma widget detection (mode auto) ─────────────────────────────────────

/// Whether the system tray of `plasma-org.kde.plasma.desktop-appletsrc`
/// lists our widget among its enabled items.
pub fn widget_in_systray(appletsrc: &str) -> bool {
    appletsrc.lines().any(|l| {
        l.strip_prefix("extraItems=")
            .is_some_and(|v| v.split(',').any(|i| i.trim() == WIDGET_ID))
    })
}

fn widget_enabled() -> bool {
    let cfg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")));
    cfg.and_then(|c| {
        std::fs::read_to_string(c.join("plasma-org.kde.plasma.desktop-appletsrc")).ok()
    })
    .is_some_and(|s| widget_in_systray(&s))
}

/// Whether the SNI should be on the bus.
pub fn want_visible(mode: Mode, hidden_by_user: bool, claimed: bool, plasma_widget: bool) -> bool {
    match mode {
        Mode::Never => false,
        Mode::Always => !hidden_by_user,
        Mode::Auto => !hidden_by_user && !claimed && !plasma_widget,
    }
}

// ── Public entry point ──────────────────────────────────────────────────────

/// Start the tray thread. `control` is the daemon's session connection (the
/// one owning `com.agenceapi.AppleKbMonitor1`): the arbitration interface is
/// exported there so that clients find it under the daemon's name.
pub fn spawn(watch: Arc<Watch>, mailbox: Arc<Mailbox>, control: Option<Connection>) {
    spawn_with(watch, mailbox, control, Config::from_env());
}

pub fn spawn_with(
    watch: Arc<Watch>,
    mailbox: Arc<Mailbox>,
    control: Option<Connection>,
    cfg: Config,
) {
    if cfg.mode == Mode::Never {
        tracing::info!("tray disabled (APPLE_KB_MONITOR_TRAY=never)");
    }
    let (icon_set, icon_theme_path) = discover_icons();
    let snap = watch.get();
    let view = View::build(&snap, false, None, cfg.lang);
    let shared: SharedRef = Arc::new(Mutex::new(Shared {
        icon_name: view.icon.name(icon_set),
        attention_icon_name: IconState::Caution.name(icon_set),
        view,
        icon_theme_path,
        has_keyboard: snap.keyboard.is_some(),
        lang: cfg.lang,
        item_id: cfg.item_id.clone(),
        title: match cfg.lang {
            Lang::Fr => "Clavier Apple".into(),
            Lang::En => "Apple keyboard".into(),
        },
        menu_rev: 1,
        xdg_token: None,
        visible: false,
        mode: cfg.mode,
    }));
    let (tx, rx) = mpsc::channel::<Event>();

    if let Some(c) = &control {
        let ctl = sni::Control {
            tx: tx.clone(),
            shared: shared.clone(),
        };
        if let Err(e) = c.object_server().at(CONTROL_PATH, ctl) {
            tracing::warn!("tray: cannot export {CONTROL_PATH}: {e}");
        }
    }

    // Snapshot forwarder: blocks on the watch's condition variable.
    {
        let tx = tx.clone();
        let watch = watch.clone();
        let _ = std::thread::Builder::new()
            .name("tray-watch".into())
            .spawn(move || {
                let mut seen = watch.version();
                loop {
                    if let Some(s) = watch.wait_newer(seen, Duration::from_secs(3600)) {
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
                rx,
                mailbox,
                watch,
                control,
                icon_set,
                cfg,
                bucket: None,
                charging: false,
                claims: HashSet::new(),
                hidden_by_user: false,
                plasma_widget: false,
                system: None,
                upower_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            };
            let mut delay = Duration::from_secs(2);
            loop {
                let started = Instant::now();
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tray.run()));
                match res {
                    Ok(Ok(())) => return,
                    Ok(Err(e)) => tracing::warn!("tray: {e}; restarting in {delay:?}"),
                    Err(_) => tracing::error!("tray: panic caught; restarting in {delay:?}"),
                }
                std::thread::sleep(delay);
                delay = if started.elapsed() > Duration::from_secs(300) {
                    Duration::from_secs(2)
                } else {
                    (delay * 2).min(Duration::from_secs(60))
                };
            }
        });
}

/// "Rename keyboard…": ask for a name, set the BlueZ alias. Without a dialog
/// program the window (which has a name field) is opened instead.
fn rename_flow(
    conn: &Connection,
    token: Option<String>,
    lang: Lang,
    mailbox: &Mailbox,
    mac: &str,
    current: &str,
) {
    let title = lang.t("Renommer le clavier", "Rename keyboard");
    let prompt = lang.t(
        "Nouveau nom (vide = nom d'origine du clavier) :",
        "New name (empty = the keyboard's own name):",
    );
    let text = match actions::ask_name(title, prompt, current) {
        Ok(Some(t)) => t,
        Ok(None) => return,
        Err(()) => {
            tracing::info!("tray: no dialog program, opening the window");
            actions::open_window(conn, token);
            return;
        }
    };
    let backend = apple_kb_monitord::alias::BluezAlias::default();
    match apple_kb_monitord::alias::rename(&backend, mailbox, mac, &text) {
        Ok(now) => tracing::info!("tray: keyboard renamed to {now:?}"),
        Err(e) => {
            tracing::warn!("tray: rename refused: {e}");
            apple_kb_monitord::notify::send(title, &e.to_string(), "dialog-error");
        }
    }
}

// ── Tray loop ───────────────────────────────────────────────────────────────

struct Tray {
    shared: SharedRef,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    mailbox: Arc<Mailbox>,
    watch: Arc<Watch>,
    control: Option<Connection>,
    icon_set: IconSet,
    cfg: Config,
    bucket: Option<u8>,
    charging: bool,
    claims: HashSet<String>,
    hidden_by_user: bool,
    plasma_widget: bool,
    system: Option<Connection>,
    /// A UPower query is pending (#234).
    upower_busy: Arc<std::sync::atomic::AtomicBool>,
}

/// Per-connection state of one `run`.
struct Session {
    conn: Connection,
    bus_name: String,
    owned: bool,
    watcher: bool,
    plasmashell: bool,
    /// Registration pending (watcher absent or call failed).
    need_register: bool,
    retry_at: Option<Instant>,
    retry_delay: Duration,
}

impl Tray {
    fn run(&mut self) -> zbus::Result<()> {
        let conn = Connection::session()?;
        conn.object_server().at(
            ITEM_PATH,
            sni::Item {
                shared: self.shared.clone(),
                tx: self.tx.clone(),
            },
        )?;
        conn.object_server().at(
            MENU_PATH,
            menu::Menu {
                shared: self.shared.clone(),
                tx: self.tx.clone(),
            },
        )?;
        let dbus = zbus::blocking::fdo::DBusProxy::new(&conn)?;
        // Subscribe BEFORE probing, so that no appearance is missed.
        let owners = dbus.receive_name_owner_changed()?;
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        {
            let tx = self.tx.clone();
            let alive = alive.clone();
            let _ = std::thread::Builder::new()
                .name("tray-owners".into())
                .spawn(move || {
                    for sig in owners {
                        if !alive.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }
                        let Ok(a) = sig.args() else { continue };
                        let name = a.name().to_string();
                        let new = a.new_owner().as_ref().map(|o| o.to_string());
                        // Unique names only matter when they leave (claims).
                        let watched = name == WATCHER
                            || name == PLASMASHELL
                            || (name.starts_with(':') && new.is_none());
                        if !watched {
                            continue;
                        }
                        if tx.send(Event::Owner(name, new)).is_err() {
                            break;
                        }
                    }
                });
        }
        let has = |n: &str| {
            zbus::names::BusName::try_from(n)
                .ok()
                .and_then(|b| dbus.name_has_owner(b).ok())
                .unwrap_or(false)
        };
        let mut s = Session {
            bus_name: format!("org.kde.StatusNotifierItem-{}-1", std::process::id()),
            owned: false,
            watcher: has(WATCHER),
            plasmashell: has(PLASMASHELL),
            need_register: true,
            retry_at: None,
            retry_delay: Duration::from_secs(1),
            conn,
        };
        // Claims held by clients that left while we were down are void.
        self.claims.retain(|c| has(c));
        self.plasma_widget = s.plasmashell && widget_enabled();
        self.apply(&self.watch.get(), &s.conn);

        let res = self.event_loop(&mut s);
        alive.store(false, std::sync::atomic::Ordering::Relaxed);
        if s.owned {
            let _ = s.conn.release_name(s.bus_name.as_str());
        }
        res
    }

    fn event_loop(&mut self, s: &mut Session) -> zbus::Result<()> {
        loop {
            self.reconcile(s);
            let ev = match s.retry_at {
                Some(t) => match self
                    .rx
                    .recv_timeout(t.saturating_duration_since(Instant::now()))
                {
                    Ok(ev) => Some(ev),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return Ok(()),
                },
                None => match self.rx.recv() {
                    Ok(ev) => Some(ev),
                    Err(_) => return Ok(()),
                },
            };
            let Some(ev) = ev else {
                s.retry_at = None; // retry now
                continue;
            };
            match ev {
                Event::Snapshot(snap) => self.apply(&snap, &s.conn),
                Event::Owner(name, new) => {
                    if name == WATCHER {
                        s.watcher = new.is_some();
                        if s.watcher {
                            tracing::info!("tray: StatusNotifierWatcher (re)appeared");
                            s.need_register = true;
                            s.retry_at = None;
                            s.retry_delay = Duration::from_secs(1);
                        }
                    } else if name == PLASMASHELL {
                        s.plasmashell = new.is_some();
                        self.plasma_widget = s.plasmashell && widget_enabled();
                    } else if new.is_none() && self.claims.remove(&name) {
                        tracing::info!("tray: claim of {name} ended (client left)");
                    }
                }
                Event::Claim(who) => {
                    tracing::info!("tray: claimed by {who}");
                    self.claims.insert(who);
                }
                Event::Release(who) => {
                    if self.claims.remove(&who) {
                        tracing::info!("tray: released by {who}");
                    }
                }
                Event::Unhide => self.hidden_by_user = false,
                Event::Action(a) => self.action(a, &s.conn),
            }
        }
    }

    /// Take / release the SNI name and (re)register as needed.
    fn reconcile(&mut self, s: &mut Session) {
        let want = want_visible(
            self.cfg.mode,
            self.hidden_by_user,
            !self.claims.is_empty(),
            self.plasma_widget,
        );
        if want && !s.owned {
            match s.conn.request_name(s.bus_name.as_str()) {
                Ok(()) => {
                    s.owned = true;
                    s.need_register = true;
                    s.retry_at = None;
                }
                Err(e) => tracing::warn!("tray: cannot own {}: {e}", s.bus_name),
            }
        } else if !want && s.owned {
            // The watcher drops the item when its name disappears.
            if let Err(e) = s.conn.release_name(s.bus_name.as_str()) {
                tracing::warn!("tray: cannot release {}: {e}", s.bus_name);
            }
            s.owned = false;
            tracing::info!("tray: icon withdrawn");
        }
        if s.owned && s.need_register && s.watcher && s.retry_at.is_none() {
            match s.conn.call_method(
                Some(WATCHER),
                "/StatusNotifierWatcher",
                Some(WATCHER),
                "RegisterStatusNotifierItem",
                &(s.bus_name.as_str(),),
            ) {
                Ok(_) => {
                    tracing::info!("tray: registered {} with the watcher", s.bus_name);
                    s.need_register = false;
                    s.retry_delay = Duration::from_secs(1);
                }
                Err(e) => {
                    tracing::warn!(
                        "tray: registration failed ({e}), retry in {:?}",
                        s.retry_delay
                    );
                    s.retry_at = Some(Instant::now() + s.retry_delay);
                    s.retry_delay = (s.retry_delay * 2).min(Duration::from_secs(30));
                }
            }
        }
        let visible = s.owned && !s.need_register;
        if lock(&self.shared).visible != visible {
            lock(&self.shared).visible = visible;
            self.emit_control_visible();
        }
    }

    fn emit_control_visible(&self) {
        let Some(c) = &self.control else { return };
        let Ok(iref) = c.object_server().interface::<_, sni::Control>(CONTROL_PATH) else {
            return;
        };
        let ctx = iref.signal_context().to_owned();
        let m = iref.get();
        let _ = zbus::block_on(m.visible_changed(&ctx));
    }

    fn system_bus(&mut self) -> Option<&Connection> {
        if self.system.is_none() {
            self.system = Connection::system().ok();
        }
        self.system.as_ref()
    }

    /// New snapshot → new view → signals for what changed.
    fn apply(&mut self, snap: &Snapshot, conn: &Connection) {
        let charging = if snap.connected && snap.keyboard.is_some() {
            let mac = snap.mac().map(str::to_string);
            // Bounded: a frozen UPower must not stop the tray loop (#234).
            let busy = self.upower_busy.clone();
            let last = self.charging;
            self.system_bus()
                .map(|sys| actions::upower_charging_bounded(sys, mac.as_deref(), &busy).unwrap_or(last))
                .unwrap_or(false)
        } else {
            false
        };
        self.charging = charging;
        let new = View::build(snap, charging, self.bucket, self.cfg.lang);
        if let IconState::Level { bucket, .. } = new.icon {
            self.bucket = Some(bucket);
        } else {
            self.bucket = None;
        }
        let icon_name = new.icon.name(self.icon_set);

        let (icon_changed, status_changed, tip_changed, menu_diff, rev) = {
            let mut sh = lock(&self.shared);
            let old = std::mem::replace(&mut sh.view, new);
            let icon_changed = sh.icon_name != icon_name;
            sh.icon_name = icon_name;
            sh.has_keyboard = snap.keyboard.is_some();
            let status_changed = old.status != sh.view.status;
            let tip_changed = old.tooltip_title != sh.view.tooltip_title
                || old.tooltip_lines != sh.view.tooltip_lines
                || old.last_update != sh.view.last_update
                || icon_changed;
            let d = menu::diff(&old, &sh.view);
            if !matches!(d, menu::MenuDiff::Same) {
                sh.menu_rev = sh.menu_rev.wrapping_add(1).max(1);
            }
            (icon_changed, status_changed, tip_changed, d, sh.menu_rev)
        };
        let status = lock(&self.shared).view.status.as_str();
        if let Err(e) = emit(
            conn,
            icon_changed,
            status_changed.then_some(status),
            tip_changed,
            menu_diff,
            rev,
        ) {
            tracing::debug!("tray: signal emission failed: {e}");
        }
    }

    fn action(&mut self, a: Action, conn: &Connection) {
        match a {
            Action::Refresh => {
                if !self.mailbox.send(Msg::Refresh) {
                    tracing::warn!("tray: refresh ignored, acquisition thread not running");
                }
            }
            Action::Hide => {
                tracing::info!(
                    "tray: hidden from the menu (ShowTray() or a restart brings it back)"
                );
                self.hidden_by_user = true;
            }
            Action::Open => {
                let token = lock(&self.shared).xdg_token.take();
                let c = conn.clone();
                let _ = std::thread::Builder::new()
                    .name("tray-open".into())
                    .spawn(move || actions::open_window(&c, token));
            }
            Action::Bluetooth => {
                let _ = std::thread::Builder::new()
                    .name("tray-bt".into())
                    .spawn(actions::open_bluetooth_settings);
            }
            Action::Repair => {
                let _ = std::thread::Builder::new()
                    .name("tray-repair".into())
                    .spawn(apple_kb_monitord::repair::launch_repair);
            }
            Action::Rename => {
                let snap = self.watch.get();
                let Some(mac) = snap.mac().map(str::to_string) else {
                    return;
                };
                let current = snap.display_name().unwrap_or_default().to_string();
                let (lang, mailbox) = (self.cfg.lang, self.mailbox.clone());
                let (c, token) = (conn.clone(), lock(&self.shared).xdg_token.take());
                let _ = std::thread::Builder::new()
                    .name("tray-rename".into())
                    .spawn(move || rename_flow(&c, token, lang, &mailbox, &mac, &current));
            }
            Action::Copy => {
                let text = view::clipboard_text(
                    &self.watch.get(),
                    self.charging,
                    self.cfg.lang,
                    now_unix(),
                );
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

fn emit(
    conn: &Connection,
    icon: bool,
    status: Option<&str>,
    tooltip: bool,
    menu_diff: menu::MenuDiff,
    rev: u32,
) -> zbus::Result<()> {
    let item = conn.object_server().interface::<_, sni::Item>(ITEM_PATH)?;
    let ictx = item.signal_context().to_owned();
    let menu_ref = conn.object_server().interface::<_, menu::Menu>(MENU_PATH)?;
    let mctx = menu_ref.signal_context().to_owned();
    zbus::block_on(async {
        if icon {
            sni::Item::new_icon(&ictx).await?;
        }
        if let Some(s) = status {
            sni::Item::new_status(&ictx, s).await?;
        }
        if tooltip {
            sni::Item::new_tool_tip(&ictx).await?;
        }
        match menu_diff {
            menu::MenuDiff::Same => {}
            menu::MenuDiff::Layout => menu::Menu::layout_updated(&mctx, rev, 0).await?,
            menu::MenuDiff::Props { updated, removed } => {
                menu::Menu::items_properties_updated(&mctx, updated, removed).await?
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes() {
        assert_eq!(Mode::parse("AUTO"), Some(Mode::Auto));
        assert_eq!(Mode::parse(""), Some(Mode::Auto));
        assert_eq!(Mode::parse("always"), Some(Mode::Always));
        assert_eq!(Mode::parse("never"), Some(Mode::Never));
        assert_eq!(Mode::parse("bogus"), None);
    }

    #[test]
    fn visibility_rules() {
        assert!(want_visible(Mode::Auto, false, false, false));
        assert!(!want_visible(Mode::Auto, false, true, false), "claimed");
        assert!(
            !want_visible(Mode::Auto, false, false, true),
            "widget in systray"
        );
        assert!(
            !want_visible(Mode::Auto, true, false, false),
            "hidden by user"
        );
        assert!(want_visible(Mode::Always, false, true, true));
        assert!(!want_visible(Mode::Always, true, false, false));
        assert!(!want_visible(Mode::Never, false, false, false));
    }

    #[test]
    fn widget_detection() {
        let cfg = "[Containments][8][Applets][9]\nplugin=org.kde.plasma.systemtray\n\
                   [Containments][10][General]\nextraItems=org.kde.plasma.battery,com.agenceapi.devicehub,org.kde.kscreen\n";
        assert!(widget_in_systray(cfg));
        assert!(!widget_in_systray(
            "extraItems=org.kde.plasma.battery,org.kde.kscreen,\nplugin=com.agenceapi.devicehub\n"
        ));
        assert!(!widget_in_systray("knownItems=com.agenceapi.devicehub\n"));
    }

    /// Live check on the user's session bus, without hardware: a test SNI
    /// with a fake snapshot. Run by hand:
    /// `AKM_TRAY_LIVE_SECS=60 cargo test -p apple-kb-monitord --bin apple-kb-monitord live_tray -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_tray() {
        use akm_core::KbReport;
        let secs: u64 = std::env::var("AKM_TRAY_LIVE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let watch = Arc::new(Watch::new());
        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(57.0);
        k.battery.voltage = Some(2.71);
        k.radio.rssi_dbm = None;
        k.device.model = Some("Apple Wireless Keyboard (A1314, aluminum, ISO) [TEST]".into());
        k.device.mac = Some("00:00:00:00:00:00".into());
        watch.publish(Snapshot {
            connected: true,
            keyboard: Some(k),
            remaining_display: Some("≈ 41 j".into()),
            last_update: now_unix(),
            ..Default::default()
        });
        let control = Connection::session().unwrap();
        control
            .request_name("com.agenceapi.AppleKbMonitorTrayTest")
            .unwrap();
        println!(
            "pid {} item org.kde.StatusNotifierItem-{}-1 control com.agenceapi.AppleKbMonitorTrayTest",
            std::process::id(),
            std::process::id()
        );
        spawn_with(
            watch.clone(),
            Mailbox::new(),
            Some(control),
            Config {
                mode: Mode::parse(&std::env::var("APPLE_KB_MONITOR_TRAY").unwrap_or_default())
                    .unwrap_or(Mode::Auto),
                item_id: "apple-kb-monitor-test".into(),
                lang: Lang::from_env(),
            },
        );
        // Halfway: battery drops to 8 % (icon, status, tooltip, menu change).
        std::thread::sleep(Duration::from_secs(secs / 2));
        let mut s = watch.get();
        if let Some(k) = s.keyboard.as_mut() {
            k.battery.percentage_fine = Some(8.0);
        }
        s.last_update = now_unix();
        watch.publish(s);
        println!("battery set to 8 %");
        std::thread::sleep(Duration::from_secs(secs - secs / 2));
    }
}
