//! Session-bus interface `com.agenceapi.AppleKbMonitor1` (read-only state).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use akm_core::history::{Clock, History, SystemClock};
use akm_core::{Snapshot, Watch};
use zbus::blocking::Connection;
use zbus::fdo::RequestNameReply;
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::actor::Mailbox;
use crate::alias::{AliasBackend, BluezAlias};
use crate::devices::{self, DevProps, Device, Shared, INTERFACE_VERSION};
use crate::events::{DeviceEvent, EventHub};
use crate::settings::{HelperBackend, SettingsBackend};
use akm_core::hid_params::Param;

pub const BUS_NAME: &str = "com.agenceapi.AppleKbMonitor1";

static SERVED_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The name this process owns (`--bus-name`), [`BUS_NAME`] before [`serve_with`] took one.
#[must_use]
pub fn bus_name() -> &'static str {
    SERVED_NAME.get().map_or(BUS_NAME, String::as_str)
}
pub const OBJECT_PATH: &str = "/com/agenceapi/AppleKbMonitor1";
pub const INTERFACE: &str = "com.agenceapi.AppleKbMonitor1";
/// `Rssi` value when no fresh measurement exists: read `RssiValid` first (R1).
pub const RSSI_UNKNOWN: i32 = 127;

/// D-Bus view of a snapshot (sentinels instead of options).
#[derive(Debug, Clone, PartialEq)]
pub struct Props {
    pub battery: i32,
    pub voltage: f64,
    pub rssi: i32,
    /// `rssi` is a measurement (false: `rssi` holds the 127 sentinel).
    pub rssi_valid: bool,
    pub connected: bool,
    pub model: String,
    /// Alias, else the keyboard's own name ("" = unknown).
    pub name: String,
    pub mac: String,
    pub last_update: u64,
    pub last_error: String,
    /// `0x4F` as `0x0050` ("" = not read yet).
    pub firmware_version: String,
    /// Latest public version known for the model ("" = not in the table).
    pub firmware_latest_known: String,
    /// `up_to_date` / `update_available` / `unknown`.
    pub firmware_status: String,
    /// Name stored in the keyboard, `0x51`-`0x54` ("" = not read yet).
    pub device_name_on_keyboard: String,
}

impl Props {
    #[must_use]
    pub fn from_snapshot(s: &Snapshot) -> Self {
        Self {
            battery: s.battery_pct().map_or(-1, akm_core::conv::pct_i32),
            voltage: s.voltage().filter(|v| v.is_finite()).unwrap_or(0.0),
            rssi: s.rssi().unwrap_or(RSSI_UNKNOWN),
            rssi_valid: s.rssi().is_some(),
            connected: s.connected,
            model: s.model().unwrap_or_default().to_string(),
            name: s.display_name().unwrap_or_default().to_string(),
            mac: s.mac().unwrap_or_default().to_string(),
            last_update: s.last_update,
            last_error: s.last_error.clone().unwrap_or_default(),
            firmware_version: s
                .firmware()
                .and_then(|f| f.version.clone())
                .unwrap_or_default(),
            firmware_latest_known: s
                .firmware()
                .and_then(|f| f.latest_known.clone())
                .unwrap_or_default(),
            firmware_status: s
                .firmware()
                .map(|f| f.status.clone())
                .filter(|st| !st.is_empty())
                .unwrap_or_else(|| "unknown".to_string()),
            device_name_on_keyboard: s
                .keyboard
                .as_ref()
                .and_then(|k| k.device.name_on_keyboard.clone())
                .unwrap_or_default(),
        }
    }
}

pub struct Monitor {
    shared: Arc<Shared>,
}

impl Monitor {
    fn props(&self) -> Props {
        Props::from_snapshot(&self.shared.watch.get())
    }
}

#[interface(name = "com.agenceapi.AppleKbMonitor1")]
#[allow(clippy::unused_self)] // zbus interface: D-Bus methods take `&self` and owned arguments
impl Monitor {
    #[zbus(property)]
    fn battery(&self) -> i32 {
        self.props().battery
    }
    #[zbus(property)]
    fn voltage(&self) -> f64 {
        self.props().voltage
    }
    #[zbus(property)]
    fn rssi(&self) -> i32 {
        self.props().rssi
    }
    /// False when `Rssi` is the 127 sentinel (no measurement).
    #[zbus(property)]
    fn rssi_valid(&self) -> bool {
        self.props().rssi_valid
    }
    #[zbus(property)]
    fn connected(&self) -> bool {
        self.props().connected
    }
    #[zbus(property)]
    fn model(&self) -> String {
        self.props().model
    }
    #[zbus(property)]
    fn mac(&self) -> String {
        self.props().mac
    }
    /// Name to show: alias set on this computer, else own name.
    #[zbus(property)]
    fn name(&self) -> String {
        self.props().name
    }
    #[zbus(property)]
    fn last_update(&self) -> u64 {
        self.props().last_update
    }
    #[zbus(property)]
    fn last_error(&self) -> String {
        self.props().last_error
    }
    /// Firmware version read from report 0x4F, once per connection ("" = not read).
    #[zbus(property)]
    fn firmware_version(&self) -> String {
        self.props().firmware_version
    }
    /// Latest public firmware known for this model ("" = model not in the table).
    #[zbus(property)]
    fn firmware_latest_known(&self) -> String {
        self.props().firmware_latest_known
    }
    /// `up_to_date` / `update_available` / `unknown`.
    #[zbus(property)]
    fn firmware_status(&self) -> String {
        self.props().firmware_status
    }
    /// Name stored in the keyboard (`0x51`-`0x54`), "" = not read yet. Read-only.
    #[zbus(property)]
    fn device_name_on_keyboard(&self) -> String {
        self.props().device_name_on_keyboard
    }
    #[zbus(property)]
    fn revision(&self) -> u64 {
        self.shared.watch.version()
    }
    /// Full snapshot; changes are only announced (invalidated): the value comes with `StateChanged`.
    #[zbus(property(emits_changed_signal = "invalidates"))]
    fn json(&self) -> String {
        serde_json::to_string(&self.shared.watch.get()).unwrap_or_default()
    }
    /// API version (1 = root object only, 2 = device objects).
    #[zbus(property)]
    fn interface_version(&self) -> u32 {
        INTERFACE_VERSION
    }
    /// Autonomy forecast in seconds, -1 = unknown.
    #[zbus(property(emits_changed_signal = "false"))]
    fn remaining_seconds(&self) -> i64 {
        devices::remaining_seconds(&self.shared.watch.get(), SystemClock.now())
    }
    #[zbus(property)]
    fn daemon_version(&self) -> &str {
        akm_core::PKG_VERSION
    }

    /// Full snapshot as JSON (same as the `Json` property, without a variant:
    /// convenient for QML / shell clients).
    fn get_state(&self) -> String {
        serde_json::to_string(&self.shared.watch.get()).unwrap_or_default()
    }

    /// `akmctl repair` is forgetting the keyboard (it just sent
    /// `RecantConnection`): the disconnection that follows within 15 s
    /// is not notified (Apple's `SuppressDisconnectNotifications`). Writes
    /// nothing to the keyboard; returns true.
    fn expect_disconnect(&self) -> bool {
        akm_core::link::mark_expected_disconnect();
        true
    }

    /// Ask for a full read now. Refused with an error that says why: no
    /// keyboard connected (Failed), or read less than 5 min ago with the wait
    /// before the next read (`LimitsExceeded`).
    async fn refresh(&self) -> zbus::fdo::Result<()> {
        self.shared.off_bus(devices::Shared::refresh).await
    }

    /// The name stored in the keyboard was just rewritten by `akmctl rename`
    /// with the device-name option: forget the four fragments `0x51`-`0x54` (claims
    /// and cached values, nothing else) and read these four reports again
    /// (never the routine reports): now, or, when a re-read was made less
    /// than 30 s ago, when those 30 s end (deferred, never dropped). Returns
    /// `(accepted, text)`: `false` while no keyboard is connected. Writes
    /// nothing to the keyboard.
    async fn reread_name(&self) -> zbus::fdo::Result<(bool, String)> {
        self.shared.off_bus(devices::Shared::reread_name).await
    }

    /// Tell the keyboard the computer is shutting down: the one write macOS
    /// does, Feature `0x40` (`WillShutdown`, the id alone), at most once per
    /// run, only if `[apple] will_shutdown` is on and the keyboard connected.
    /// Returns `(sent, explanation)`; "nothing sent" is not an error, a failed
    /// write is (`org.freedesktop.DBus.Error.Failed`).
    /// Called by `akmctl shutdown-notify` (user unit `ExecStop=`).
    async fn notify_shutdown(&self) -> zbus::fdo::Result<(bool, String)> {
        // The write may wait for the 1 s spacing and the HIDP answer: off the bus executor.
        let o = devices::unblock(|| crate::shutdown::notify("NotifyShutdown (D-Bus)"))
            .await
            .unwrap_or_else(|e| akm_core::parity::Outcome::Failed(e.to_string()));
        shutdown_reply(&o)
    }

    /// Rename the keyboard `mac` on this computer (`BlueZ` alias; nothing is
    /// written into the keyboard). `""` restores its own name. Returns the
    /// name now in effect.
    async fn set_alias(
        &self,
        mac: &str,
        name: &str,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        let caller = crate::devices::describe_caller(conn, &hdr).await;
        self.shared.rename(mac, name, caller).await
    }

    /// History entries with `ts >= since`, as a JSON array of at most 2000
    /// points: beyond, the daemon thins the series (first, last, every
    /// battery replacement, evenly spaced samples).
    async fn history(&self, since: u64) -> zbus::fdo::Result<String> {
        self.shared
            .off_bus(move |s| s.history_json(since, s.followed_mac().as_deref()))
            .await
    }

    /// [`Self::history`] thinned to at most `max` points (0 = 2000, never
    /// above 20000).
    async fn history_max(&self, since: u64, max: u32) -> zbus::fdo::Result<String> {
        self.shared
            .off_bus(move |s| s.history_json_max(since, max, s.followed_mac().as_deref()))
            .await
    }

    /// The checks shown by the widget's DIAG tab and the settings module's Diagnosis page, run by
    /// the daemon, as JSON (`{schema, daemon_version, passed, total, checks: [{id, label, ok,
    /// detail}]}`). Never touches the keyboard; every program run is
    /// bounded to 3 s. See [`crate::diagnose`].
    async fn diagnose(&self) -> zbus::fdo::Result<String> {
        // Programs are run: off the bus executor.
        let probe = self.shared.diag.clone();
        crate::devices::unblock(move || {
            crate::diagnose::to_json(&crate::diagnose::run_checks(probe.as_ref(), true)).to_string()
        })
        .await
    }

    /// Object paths of the known keyboards (API 2).
    fn get_devices(&self) -> Vec<OwnedObjectPath> {
        self.shared.device_paths()
    }

    /// Battery sets with their lifetime, JSON array (API 2).
    async fn battery_sets(&self) -> zbus::fdo::Result<String> {
        self.shared
            .off_bus(|s| s.battery_sets_json(s.followed_mac().as_deref()))
            .await
    }

    #[zbus(signal)]
    async fn state_changed(
        ctxt: &zbus::object_server::SignalContext<'_>,
        revision: u64,
        json: &str,
    ) -> zbus::Result<()>;
}

/// Why the service could not start.
#[derive(Debug)]
pub enum ServeError {
    /// Another process owns the name (and therefore the keyboard).
    NameTaken,
    Bus(zbus::Error),
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServeError::NameTaken => {
                write!(f, "{BUS_NAME} is already owned: another monitor is running")
            }
            ServeError::Bus(e) => write!(f, "session bus: {e}"),
        }
    }
}

impl From<zbus::Error> for ServeError {
    fn from(e: zbus::Error) -> Self {
        ServeError::Bus(e)
    }
}

/// Everything the service needs.
pub struct ServeOptions {
    pub watch: Arc<Watch>,
    pub mailbox: Arc<Mailbox>,
    pub history: Option<Arc<History>>,
    /// Events of the actor, turned into device signals.
    pub events: Arc<EventHub>,
    /// Write path of the `hid_apple` parameters.
    pub settings: Arc<dyn SettingsBackend>,
    /// Alias (name on this computer) backend.
    pub alias: Arc<dyn AliasBackend>,
    /// Well-known name to take (tests / side-by-side instances).
    pub bus_name: String,
    /// What `Diagnose()` looks at (the real system; a fixture in tests).
    pub diag: Arc<dyn crate::diagnose::Probe>,
    /// Settings remembered per keyboard.
    pub reapply: Option<Arc<crate::reapply::Reapplier>>,
}

impl ServeOptions {
    pub fn new(watch: Arc<Watch>, mailbox: Arc<Mailbox>, history: Option<Arc<History>>) -> Self {
        Self {
            watch,
            mailbox,
            history,
            events: EventHub::new(),
            settings: Arc::new(HelperBackend::default()),
            alias: Arc::new(BluezAlias::default()),
            bus_name: BUS_NAME.to_string(),
            diag: Arc::new(crate::diagnose::SystemProbe),
            reapply: None,
        }
    }
}

#[cfg(any(test, feature = "testbus"))]
/// Export the object and take the well-known name on `conn`.
///
/// # Errors
/// [`ServeError`] when the object cannot be exported or the name taken.
pub fn serve_on(
    conn: &Connection,
    watch: Arc<Watch>,
    mailbox: Arc<Mailbox>,
    history: Option<Arc<History>>,
) -> Result<(), ServeError> {
    export_on(conn, &ServeOptions::new(watch, mailbox, history)).map(|_| ())
}

#[cfg(any(test, feature = "testbus"))]
/// Export the root object (v1 interface + `ObjectManager`) and take the name.
///
/// # Errors
/// [`ServeError`] when the object cannot be exported or the name taken.
pub fn export_on(conn: &Connection, o: &ServeOptions) -> Result<Arc<Shared>, ServeError> {
    let shared = export_objects(conn, o)?;
    take_name(conn, o)?;
    Ok(shared)
}

fn export_objects(conn: &Connection, o: &ServeOptions) -> Result<Arc<Shared>, ServeError> {
    let shared = Arc::new(Shared {
        watch: o.watch.clone(),
        mailbox: o.mailbox.clone(),
        history: o.history.clone(),
        settings: o.settings.clone(),
        alias: o.alias.clone(),
        diag: o.diag.clone(),
        reapply: o.reapply.clone(),
        devices: Mutex::new(Vec::new()),
    });
    conn.object_server().at(
        OBJECT_PATH,
        Monitor {
            shared: shared.clone(),
        },
    )?;
    conn.object_server()
        .at(OBJECT_PATH, zbus::fdo::ObjectManager)?;
    // Special keys and manual key mapping, see crate::keymap.
    conn.object_server().at(
        OBJECT_PATH,
        crate::keymap::KeymapIface::new(
            crate::keymap::KeymapPaths::default(),
            Arc::new(crate::keymap::Pkexec::default()),
        ),
    )?;
    Ok(shared)
}

/// Last step of the start-up: an activating call is delivered as soon as the name is owned.
fn take_name(conn: &Connection, o: &ServeOptions) -> Result<(), ServeError> {
    let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
    let name = zbus::names::WellKnownName::try_from(o.bus_name.as_str())
        .map_err(|e| ServeError::Bus(e.into()))?;
    match conn.request_name_with_flags(name, flags) {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Ok(()),
        Ok(_) | Err(zbus::Error::NameTaken) => Err(ServeError::NameTaken),
        Err(e) => Err(e.into()),
    }
}

#[cfg(any(test, feature = "testbus"))]
/// Connect to the session bus, export, take the name and start the thread that turns snapshot
/// changes into `PropertiesChanged` + `StateChanged`.
///
/// # Errors
/// [`ServeError`] when the bus is unreachable, the object cannot be exported or the name taken.
pub fn serve(
    watch: Arc<Watch>,
    mailbox: Arc<Mailbox>,
    history: Option<Arc<History>>,
) -> Result<Connection, ServeError> {
    serve_with(&ServeOptions::new(watch, mailbox, history))
}

/// [`serve`] with every option (API 2: device signals from `events`).
///
/// # Errors
/// [`ServeError`] when the bus is unreachable, the object cannot be exported or the name taken.
pub fn serve_with(o: &ServeOptions) -> Result<Connection, ServeError> {
    serve_exporting(o, |_| {})
}

/// [`serve_with`], `export` adding the process's other objects before the name is taken.
///
/// # Errors
/// As [`serve_with`].
pub fn serve_exporting(
    o: &ServeOptions,
    export: impl FnOnce(&Connection),
) -> Result<Connection, ServeError> {
    let conn = Connection::session()?;
    let shared = export_objects(&conn, o)?;
    export(&conn);
    take_name(&conn, o)?;
    let _ = SERVED_NAME.set(o.bus_name.clone());
    // Device object of an already-known keyboard before the first change.
    sync_devices(&conn, &shared, &o.watch.get(), &mut HashMap::new());
    spawn_emitter(conn.clone(), shared.clone());
    spawn_event_forwarder(conn.clone(), shared, &o.events);
    Ok(conn)
}

/// Emit change signals for every new snapshot.
pub fn spawn_emitter(conn: Connection, shared: Arc<Shared>) {
    let _ = std::thread::Builder::new()
        .name("dbus-emitter".into())
        .spawn(move || {
            let watch = shared.watch.clone();
            let mut seen = watch.version();
            let first = watch.get();
            let mut prev = Props::from_snapshot(&first);
            let mut dev_prev: HashMap<String, DevProps> = HashMap::new();
            sync_devices(&conn, &shared, &first, &mut dev_prev);
            let mut params = keymap_params(&shared);
            loop {
                let snap = watch.wait_newer(seen, Duration::from_mins(1));
                // Changed outside the daemon (akmctl, sysfs): seen at the next pass.
                let now_params = keymap_params(&shared);
                if now_params != params {
                    params = now_params;
                    zbus::block_on(emit_keymap_params(conn.inner()));
                }
                let Some(snap) = snap else {
                    continue;
                };
                seen = snap.version;
                let now = Props::from_snapshot(&snap);
                if let Err(e) = emit(&conn, &prev, &now, &snap) {
                    tracing::warn!("cannot emit D-Bus signals: {e}");
                }
                sync_devices(&conn, &shared, &snap, &mut dev_prev);
                prev = now;
            }
        });
}

fn sync_devices(
    conn: &Connection,
    shared: &Arc<Shared>,
    snap: &Snapshot,
    prev: &mut HashMap<String, DevProps>,
) {
    if let Some(mac) = snap.mac() {
        let existed = shared
            .devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|m| m.eq_ignore_ascii_case(mac));
        if !existed {
            // The object is born in its final state.
            prev.entry(mac.to_ascii_uppercase())
                .or_insert_with(|| DevProps::for_mac(&Snapshot::default(), mac, ("", "")));
        }
        if let Err(e) = devices::ensure_device(
            conn,
            shared,
            mac,
            snap.model().unwrap_or_default(),
            snap.display_name().unwrap_or_default(),
        ) {
            tracing::warn!("cannot export device {mac}: {e}");
        }
    }
    // The other keyboards of the roster get their object too.
    for d in snap.devices.iter().filter(|d| !d.primary) {
        let known = shared
            .devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|m| m.eq_ignore_ascii_case(&d.mac));
        if !known {
            prev.entry(d.mac.to_ascii_uppercase())
                .or_insert_with(|| DevProps::for_mac(&Snapshot::default(), &d.mac, ("", "")));
        }
        if let Err(e) = devices::ensure_device(conn, shared, &d.mac, "", &d.name) {
            tracing::warn!("cannot export device {}: {e}", d.mac);
        }
    }
    let macs = shared
        .devices
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    for mac in macs {
        let (last_model, last_name) = prev
            .get(&mac)
            .map(|p| (p.base.model.clone(), p.base.name.clone()))
            .unwrap_or_default();
        let now = DevProps::for_mac(snap, &mac, (&last_model, &last_name));
        match prev.get(&mac) {
            Some(old) if *old != now => {
                if let Err(e) = emit_device(conn, &mac, old, &now) {
                    tracing::warn!("cannot emit device signals: {e}");
                }
            }
            Some(_) => continue,
            None => {}
        }
        prev.insert(mac, now);
    }
}

/// `FnMode`, `SwapOptCmd` and `IsoLayout` as every device object shows them.
fn keymap_params(shared: &Shared) -> [i32; 3] {
    [Param::FnMode, Param::SwapOptCmd, Param::IsoLayout].map(|p| shared.settings.get(p))
}

/// `PropertiesChanged` of `FnMode`, `SwapOptCmd` and `IsoLayout` on every device object, after the
/// daemon changed or re-read them: sysfs has no change notification.
pub async fn emit_keymap_params(conn: &zbus::Connection) {
    let server = conn.object_server();
    let Ok(monitor) = server.interface::<_, Monitor>(OBJECT_PATH).await else {
        return;
    };
    let paths = monitor.get().await.shared.device_paths();
    for path in paths {
        let Ok(iref) = server.interface::<_, Device>(&path).await else {
            continue;
        };
        let (ctx, d) = (iref.signal_context(), iref.get().await);
        let r = async {
            d.fn_mode_changed(ctx).await?;
            d.swap_opt_cmd_changed(ctx).await?;
            d.iso_layout_changed(ctx).await
        };
        if let Err(e) = r.await {
            tracing::warn!("cannot emit the keyboard parameters of {path}: {e}");
        }
    }
}

#[allow(clippy::float_cmp)] // change detection of published values: exact equality is meant
fn emit_device(conn: &Connection, mac: &str, old: &DevProps, now: &DevProps) -> zbus::Result<()> {
    let Some(path) = devices::device_path(mac) else {
        return Ok(());
    };
    let iref = conn.object_server().interface::<_, Device>(&path)?;
    let ctx = iref.signal_context();
    let d = iref.get();
    let (o, n) = (&old.base, &now.base);
    zbus::block_on(async {
        if o.battery != n.battery {
            d.battery_changed(ctx).await?;
        }
        if o.voltage != n.voltage {
            d.voltage_changed(ctx).await?;
        }
        if o.rssi != n.rssi {
            d.rssi_changed(ctx).await?;
        }
        if o.model != n.model {
            d.model_changed(ctx).await?;
        }
        if o.name != n.name {
            d.name_changed(ctx).await?;
        }
        if o.last_update != n.last_update {
            d.last_update_changed(ctx).await?;
        }
        if o.firmware_version != n.firmware_version {
            d.firmware_version_changed(ctx).await?;
        }
        if o.firmware_latest_known != n.firmware_latest_known {
            d.firmware_latest_known_changed(ctx).await?;
        }
        if o.firmware_status != n.firmware_status {
            d.firmware_status_changed(ctx).await?;
        }
        if old.empty_at != now.empty_at || old.rate_pct_per_day != now.rate_pct_per_day {
            d.empty_at_changed(ctx).await?;
            d.discharge_rate_changed(ctx).await?;
        }
        if old.installed_at != now.installed_at {
            d.batteries_installed_at_changed(ctx).await?;
        }
        if o.connected != n.connected {
            d.connected_changed(ctx).await?;
            Device::connection_changed(ctx, n.connected, n.battery).await?;
        }
        Ok(())
    })
}

fn spawn_event_forwarder(conn: Connection, shared: Arc<Shared>, hub: &EventHub) {
    let rx = hub.subscribe();
    let _ = std::thread::Builder::new()
        .name("dbus-events".into())
        .spawn(move || {
            for ev in rx {
                if let Err(e) = forward(&conn, &shared, &ev) {
                    tracing::warn!("cannot emit {ev:?}: {e}");
                }
            }
        });
}

fn forward(conn: &Connection, shared: &Arc<Shared>, ev: &DeviceEvent) -> zbus::Result<()> {
    if let DeviceEvent::Forgotten { mac } = ev {
        devices::remove_device(conn, shared, mac);
        return Ok(());
    }
    let snap = shared.watch.get();
    let (model, name) = (
        snap.model().unwrap_or_default(),
        snap.display_name().unwrap_or_default(),
    );
    let Some(path) = devices::ensure_device(conn, shared, ev.mac(), model, name)? else {
        return Ok(());
    };
    let iref = conn.object_server().interface::<_, Device>(&path)?;
    let ctx = iref.signal_context();
    let pct = akm_core::conv::pct_i32;
    zbus::block_on(async {
        match ev {
            DeviceEvent::BatteryLevelCrossed { crossing: c, .. } => {
                Device::battery_level_crossed(
                    ctx,
                    u32::from(c.threshold),
                    pct(c.pct),
                    c.urgency.as_str(),
                )
                .await
            }
            DeviceEvent::BatteryReplaced { replacement: r, .. } => {
                Device::battery_replaced(
                    ctx,
                    r.ts,
                    r.pct_before.map_or(-1, pct),
                    pct(r.pct_after),
                    r.voltage_before.unwrap_or(0.0),
                    r.voltage_after.unwrap_or(0.0),
                )
                .await
            }
            // ConnectionChanged follows the published state (sync_devices).
            DeviceEvent::Link(_) | DeviceEvent::Forgotten { .. } => Ok(()),
        }
    })
}

/// Discrete properties that differ between two published states, with their new value.
#[allow(clippy::float_cmp)] // change detection of published values: exact equality is meant
fn changed_props(prev: &Props, now: &Props) -> Vec<(&'static str, Value<'static>)> {
    let mut v: Vec<(&'static str, Value<'static>)> = Vec::new();
    let mut put = |name, changed: bool, val: Value<'static>| {
        if changed {
            v.push((name, val));
        }
    };
    put("Battery", prev.battery != now.battery, now.battery.into());
    put("Voltage", prev.voltage != now.voltage, now.voltage.into());
    put(
        "RssiValid",
        prev.rssi_valid != now.rssi_valid,
        now.rssi_valid.into(),
    );
    put("Rssi", prev.rssi != now.rssi, now.rssi.into());
    put(
        "Connected",
        prev.connected != now.connected,
        now.connected.into(),
    );
    put("Model", prev.model != now.model, now.model.clone().into());
    put("Name", prev.name != now.name, now.name.clone().into());
    put("Mac", prev.mac != now.mac, now.mac.clone().into());
    put(
        "LastUpdate",
        prev.last_update != now.last_update,
        now.last_update.into(),
    );
    put(
        "LastError",
        prev.last_error != now.last_error,
        now.last_error.clone().into(),
    );
    put(
        "FirmwareVersion",
        prev.firmware_version != now.firmware_version,
        now.firmware_version.clone().into(),
    );
    put(
        "FirmwareLatestKnown",
        prev.firmware_latest_known != now.firmware_latest_known,
        now.firmware_latest_known.clone().into(),
    );
    put(
        "FirmwareStatus",
        prev.firmware_status != now.firmware_status,
        now.firmware_status.clone().into(),
    );
    put(
        "DeviceNameOnKeyboard",
        prev.device_name_on_keyboard != now.device_name_on_keyboard,
        now.device_name_on_keyboard.clone().into(),
    );
    v
}

/// One `PropertiesChanged` for every changed property (`Json` only invalidated: the full snapshot
/// travels once, in `StateChanged`).
fn emit(conn: &Connection, prev: &Props, now: &Props, snap: &Snapshot) -> zbus::Result<()> {
    let iref = conn.object_server().interface::<_, Monitor>(OBJECT_PATH)?;
    let ctx = iref.signal_context();
    let mut changed = changed_props(prev, now);
    changed.push(("Revision", snap.version.into()));
    let changed: HashMap<&str, &Value<'_>> = changed.iter().map(|(k, v)| (*k, v)).collect();
    zbus::block_on(async {
        zbus::fdo::Properties::properties_changed(
            ctx,
            zbus::names::InterfaceName::from_static_str_unchecked(INTERFACE),
            &changed,
            &["Json"],
        )
        .await?;
        let json = serde_json::to_string(snap).unwrap_or_default();
        Monitor::state_changed(ctx, snap.version, &json).await
    })
}

/// `NotifyShutdown` reply: the outcome text, or a D-Bus error when the write failed.
fn shutdown_reply(o: &akm_core::parity::Outcome) -> zbus::fdo::Result<(bool, String)> {
    match o {
        akm_core::parity::Outcome::Failed(_) => Err(zbus::fdo::Error::Failed(o.describe())),
        _ => Ok((o.is_sent(), o.describe())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_shutdown_write_is_a_dbus_error() {
        use akm_core::parity::Outcome;
        assert!(matches!(
            shutdown_reply(&Outcome::Failed("EIO".into())),
            Err(zbus::fdo::Error::Failed(m)) if m.contains("EIO")
        ));
        assert!(shutdown_reply(&Outcome::Sent).unwrap().0);
        assert!(!shutdown_reply(&Outcome::NotConnected).unwrap().0);
    }
    use akm_core::KbReport;

    #[test]
    fn one_change_one_entry_and_no_json_copy() {
        let prev = Props::from_snapshot(&Snapshot::default());
        let mut now = prev.clone();
        now.battery = 42;
        now.name = "Kb".into();
        let names: Vec<&str> = changed_props(&prev, &now).iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["Battery", "Name"]);
        assert!(
            changed_props(&prev, &prev).is_empty(),
            "{:?}",
            changed_props(&prev, &prev)
        );
        let flat = akm_core::srclint::prod_tokens(include_str!("service.rs"));
        assert!(flat.contains("#[zbus(property(emits_changed_signal=\"invalidates\"))]fnjson("));
    }

    #[test]
    fn own_calls_target_the_served_name() {
        assert_eq!(bus_name(), BUS_NAME, "default before serve_with");
        for src in [include_str!("tray/actions.rs"), include_str!("notify.rs")] {
            let src = akm_core::srclint::prod_tokens(src);
            assert!(!src.contains("Some(apple_kb_monitord::service::BUS_NAME)"));
            assert!(!src.contains("Some(\"com.agenceapi.AppleKbMonitor1\")"));
        }
    }

    #[test]
    fn blocking_methods_run_off_the_bus_executor() {
        // A sync body blocks zbus's single executor thread: while Refresh and
        // RereadName wait for an actor that never answers, a property read is still served.
        let Some(bus) = crate::testbus::private_bus() else {
            return;
        };
        let server = crate::testbus::connect(&bus.addr).unwrap();
        // An actor that takes commands and never answers.
        let mailbox = Mailbox::new();
        let (tx, _held) = std::sync::mpsc::channel();
        mailbox.install(tx);
        serve_on(&server, Arc::new(Watch::new()), mailbox, None).unwrap();
        for m in ["Refresh", "RereadName"] {
            let client = crate::testbus::connect(&bus.addr).unwrap();
            let busy = std::thread::spawn(move || {
                let _ = client.call_method(Some(BUS_NAME), OBJECT_PATH, Some(INTERFACE), m, &());
            });
            std::thread::sleep(Duration::from_millis(200));
            let probe = crate::testbus::connect(&bus.addr).unwrap();
            let t = std::time::Instant::now();
            zbus::blocking::fdo::PropertiesProxy::builder(&probe)
                .destination(BUS_NAME)
                .unwrap()
                .path(OBJECT_PATH)
                .unwrap()
                .build()
                .unwrap()
                .get(
                    zbus::names::InterfaceName::from_static_str_unchecked(INTERFACE),
                    "Battery",
                )
                .unwrap();
            assert!(
                t.elapsed() < Duration::from_secs(1),
                "{m} blocked the executor for {:?}",
                t.elapsed()
            );
            busy.join().unwrap();
        }
        // Methods without a cheap way to hold them open: matched on production tokens.
        let flat = akm_core::srclint::prod_tokens(include_str!("service.rs"));
        for m in [
            "history",
            "history_max",
            "battery_sets",
            "diagnose",
            "notify_shutdown",
        ] {
            assert!(
                flat.contains(&format!("asyncfn{m}(&self")),
                "{m} must be async"
            );
        }
    }

    #[test]
    fn props_use_documented_sentinels() {
        let p = Props::from_snapshot(&Snapshot::default());
        assert_eq!(
            (p.battery, p.voltage, p.rssi, p.connected),
            (-1, 0.0, RSSI_UNKNOWN, false)
        );
        assert!(!p.rssi_valid, "127 is a sentinel, not a measure (R1)");
        assert!(p.model.is_empty() && p.mac.is_empty() && p.last_error.is_empty());
        assert!(p.name.is_empty(), "{:?}", p.name);

        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(89.6);
        k.battery.voltage = Some(2.81);
        k.radio.rssi_dbm = Some(-48);
        k.device.model = Some("A1314".into());
        k.device.mac = Some("AA:BB:CC:DD:EE:F1".into());
        k.device.name = Some("Own name".into());
        let s = Snapshot {
            connected: true,
            keyboard: Some(k),
            last_update: 7,
            ..Default::default()
        };
        let p = Props::from_snapshot(&s);
        assert_eq!((p.rssi_valid, p.rssi), (true, -48));
        assert_eq!(
            (p.battery, p.voltage, p.rssi, p.connected, p.last_update),
            (90, 2.81, -48, true, 7)
        );
        assert_eq!(p.mac, "AA:BB:CC:DD:EE:F1");
        assert_eq!(p.name, "Own name");
    }

    #[test]
    fn firmware_props_use_documented_sentinels() {
        let p = Props::from_snapshot(&Snapshot::default());
        assert_eq!(
            (
                p.firmware_version.as_str(),
                p.firmware_latest_known.as_str(),
                p.firmware_status.as_str()
            ),
            ("", "", "unknown")
        );
        let mut k = KbReport::default();
        k.firmware.version = Some("0x0050".into());
        akm_core::firmware::assess_report(Some(0x0239), &mut k.firmware);
        let s = Snapshot {
            keyboard: Some(k),
            ..Default::default()
        };
        let p = Props::from_snapshot(&s);
        assert_eq!(p.firmware_version, "0x0050");
        assert_eq!(p.firmware_latest_known, "0x0050");
        assert_eq!(p.firmware_status, "up_to_date");
    }

    #[test]
    fn device_name_on_keyboard_prop() {
        assert_eq!(
            Props::from_snapshot(&Snapshot::default()).device_name_on_keyboard,
            ""
        );
        let mut k = KbReport::default();
        k.device.name_on_keyboard = Some("Alice's keyboard #1".into());
        let s = Snapshot {
            keyboard: Some(k),
            ..Default::default()
        };
        assert_eq!(
            Props::from_snapshot(&s).device_name_on_keyboard,
            "Alice's keyboard #1"
        );
    }
}
