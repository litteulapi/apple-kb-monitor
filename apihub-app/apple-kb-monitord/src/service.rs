//! Session-bus interface `com.agenceapi.AppleKbMonitor1` (read-only state).
//!
//! Object `/com/agenceapi/AppleKbMonitor1`, well-known name
//! `com.agenceapi.AppleKbMonitor1` (also the single-instance lock: whoever
//! owns the name owns the keyboard).
//!
//! Properties (all emit `PropertiesChanged`):
//! * `Battery`  i  percentage 0..100, **-1 = unknown**
//! * `Voltage`  d  volts, **0 = unknown** (HID diagnostic)
//! * `Rssi`     i  **relative dB, not dBm** (#174): on BR/EDR the gap to the
//!   controller's golden receive power range, 0 = ideal, negative = below,
//!   positive = above (legal); **127 = unknown / stale**. The name is kept for
//!   compatibility; `Json` carries `radio.rssi_rel_db` and `radio.rssi_quality`
//! * `Connected` b, `Model` s, `Mac` s (empty = unknown), `Name` s (alias,
//!   else the keyboard's own name; empty = unknown)
//! * `LastUpdate` t (unix s, 0 = never), `LastError` s (empty = none)
//! * `Revision` t (snapshot counter), `Json` s (full snapshot, schema 1)
//! * since API 2: `InterfaceVersion` u (= 2), `RemainingSeconds` x
//!   (autonomy forecast, **-1 = unknown**, computed at read time)
//! * firmware check (#227), additions: `FirmwareVersion` s (`0x0050`, **empty
//!   = not read yet**), `FirmwareLatestKnown` s (latest public version in the
//!   embedded table, empty = model not in the table), `FirmwareStatus` s
//!   (`up_to_date` / `update_available` / `unknown`, never empty)
//! * name stored in the keyboard (#248): `DeviceNameOnKeyboard` s (`0x51`-`0x54`
//!   read once per connection, **empty = not read yet**). Read-only: no D-Bus
//!   method writes the keyboard's name (only `akmctl rename --device-name`);
//!   `RereadName()` makes the daemon read it again after such a write
//!
//! Methods: `GetState() -> s` (= `Json`), `Refresh()`, `RereadName() -> (bs)` (forget
//! `0x51`-`0x54` and read these four again, now or at the end of a 30 s floor: deferred,
//! never dropped; `false` while disconnected; writes nothing to the keyboard, #248), `SetAlias(s mac, s name) -> s`
//! (BlueZ alias, `""` = restore the keyboard's own name), `History(t since) -> s` (at most 2000 points, thinned
//! by the daemon beyond; `HistoryMax(t since, u max) -> s` chooses the bound, #96; JSON array of
//! `{ts,pct,voltage?,event?,schema?,mv_0x46?,mv_0x49?,voltage_valid?}`, `voltage_valid=false` = legacy value, #180); since API 2 `GetDevices() -> ao`,
//! `BatterySets() -> s`, `NotifyShutdown() -> (b, s)` (the one write macOS does: `WillShutdown`, #191),
//! `ExpectDisconnect() -> b` (mute the next disconnection notification for 15 s while `akmctl repair`
//! forgets the keyboard; writes nothing to it, #217), `Diagnose() -> s` (the checks of the Diag tab as JSON,
//! never touches the keyboard, #120). Signal: `StateChanged(t revision, s json)`.
//!
//! Since #247 the same object also carries `com.agenceapi.AppleKbMonitor1.Keymap`
//! (special keys, manual key mapping): see [`crate::keymap`].
//!
//! API 2 adds one object per keyboard and `org.freedesktop.DBus.ObjectManager`
//! on this path: see [`crate::devices`]. Everything above is unchanged from
//! API 1 (only additions), for the Plasma widget and `apihub-app`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use akm_core::history::{Clock, History, SystemClock};
use akm_core::{Snapshot, Watch};
use zbus::blocking::Connection;
use zbus::fdo::RequestNameReply;
use zbus::interface;
use zbus::zvariant::OwnedObjectPath;

use crate::actor::Mailbox;
use crate::alias::{AliasBackend, BluezAlias};
use crate::devices::{self, DevProps, Device, Shared, INTERFACE_VERSION};
use crate::events::{DeviceEvent, EventHub};
use crate::settings::{HelperBackend, SettingsBackend};

pub const BUS_NAME: &str = "com.agenceapi.AppleKbMonitor1";
pub const OBJECT_PATH: &str = "/com/agenceapi/AppleKbMonitor1";
pub const INTERFACE: &str = "com.agenceapi.AppleKbMonitor1";
/// `Rssi` value when no fresh measurement exists.
pub const RSSI_UNKNOWN: i32 = 127;

/// D-Bus view of a snapshot (sentinels instead of options).
#[derive(Debug, Clone, PartialEq)]
pub struct Props {
    pub battery: i32,
    pub voltage: f64,
    pub rssi: i32,
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
    /// Name stored in the keyboard, `0x51`-`0x54` ("" = not read yet, #248).
    pub device_name_on_keyboard: String,
}

impl Props {
    pub fn from_snapshot(s: &Snapshot) -> Self {
        Self {
            battery: s
                .battery_pct()
                .map_or(-1, |p| p.round().clamp(0.0, 100.0) as i32),
            voltage: s.voltage().filter(|v| v.is_finite()).unwrap_or(0.0),
            rssi: s.rssi().unwrap_or(RSSI_UNKNOWN),
            connected: s.connected,
            model: s.model().unwrap_or_default().to_string(),
            name: s.display_name().unwrap_or_default().to_string(),
            mac: s.mac().unwrap_or_default().to_string(),
            last_update: s.last_update,
            last_error: s.last_error.clone().unwrap_or_default(),
            firmware_version: s.firmware().and_then(|f| f.version.clone()).unwrap_or_default(),
            firmware_latest_known: s.firmware().and_then(|f| f.latest_known.clone()).unwrap_or_default(),
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
    /// Name to show: alias set on this computer, else own name (#141).
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
    #[zbus(property)]
    fn json(&self) -> String {
        serde_json::to_string(&self.shared.watch.get()).unwrap_or_default()
    }
    /// API version (1 = root object only, 2 = device objects).
    #[zbus(property)]
    fn interface_version(&self) -> u32 {
        INTERFACE_VERSION
    }
    /// Autonomy forecast in seconds, -1 = unknown (#83).
    #[zbus(property)]
    fn remaining_seconds(&self) -> i64 {
        devices::remaining_seconds(&self.shared.watch.get(), SystemClock.now())
    }
    #[zbus(property)]
    fn daemon_version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    /// Full snapshot as JSON (same as the `Json` property, without a variant:
    /// convenient for QML / shell clients).
    fn get_state(&self) -> String {
        serde_json::to_string(&self.shared.watch.get()).unwrap_or_default()
    }

    /// `akmctl repair` is forgetting the keyboard (it just sent
    /// `RecantConnection`, #217): the disconnection that follows within 15 s
    /// is not notified (Apple's `SuppressDisconnectNotifications`). Writes
    /// nothing to the keyboard; returns true.
    fn expect_disconnect(&self) -> bool {
        akm_core::link::mark_expected_disconnect();
        true
    }

    /// Ask for a full read now (no effect while the keyboard is disconnected).
    fn refresh(&self) -> zbus::fdo::Result<()> {
        self.shared.refresh()
    }

    /// The name stored in the keyboard was just rewritten by `akmctl rename
    /// --device-name` (#248): forget the four fragments `0x51`-`0x54` (claims
    /// and cached values, nothing else) and read these four reports again
    /// (never the routine reports): now, or, when a re-read was made less
    /// than 30 s ago, when those 30 s end (deferred, never dropped). Returns
    /// `(accepted, text)`: `false` while no keyboard is connected. Writes
    /// nothing to the keyboard.
    fn reread_name(&self) -> zbus::fdo::Result<(bool, String)> {
        self.shared.reread_name()
    }

    /// Tell the keyboard the computer is shutting down: the one write macOS
    /// does, Feature `0x40` (`WillShutdown`, the id alone), at most once per
    /// run, only if `[apple] will_shutdown` is on and the keyboard connected
    /// (#191). Returns `(sent, explanation)`; "nothing sent" is not an error.
    /// Called by `akmctl shutdown-notify` (user unit `ExecStop=`).
    async fn notify_shutdown(&self) -> (bool, String) {
        // The write may wait for the 1 s spacing and the HIDP answer: off the bus executor.
        let o = std::thread::spawn(|| crate::shutdown::notify("NotifyShutdown (D-Bus)"))
            .join()
            .unwrap_or_else(|_| akm_core::parity::Outcome::Failed("writer panicked".into()));
        (o.is_sent(), o.describe())
    }

    /// Rename the keyboard `mac` on this computer (BlueZ alias; nothing is
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
    /// battery replacement, evenly spaced samples; #96).
    fn history(&self, since: u64) -> zbus::fdo::Result<String> {
        self.shared.history_json(since)
    }

    /// [`Self::history`] thinned to at most `max` points (0 = 2000, never
    /// above 20000).
    fn history_max(&self, since: u64, max: u32) -> zbus::fdo::Result<String> {
        self.shared.history_json_max(since, max)
    }

    /// The checks of the window's "Diag" tab, run by the daemon, as JSON
    /// (`{schema, daemon_version, passed, total, checks: [{id, label, ok,
    /// detail}]}`, #120). Never touches the keyboard; every program run is
    /// bounded to 3 s. See [`crate::diagnose`].
    async fn diagnose(&self) -> String {
        // Programs are run: off the bus executor.
        let probe = self.shared.diag.clone();
        crate::devices::unblock(move || {
            let lang = crate::notify::Lang::detect();
            crate::diagnose::to_json(&crate::diagnose::run_checks(probe.as_ref(), lang, true))
                .to_string()
        })
        .await
    }

    /// Object paths of the known keyboards (API 2).
    fn get_devices(&self) -> Vec<OwnedObjectPath> {
        self.shared.device_paths()
    }

    /// Battery sets with their lifetime, JSON array (API 2, #85).
    fn battery_sets(&self) -> zbus::fdo::Result<String> {
        self.shared.battery_sets_json()
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
        }
    }
}

/// Export the object and take the well-known name on `conn` (API 1 entry
/// point, kept for compatibility).
pub fn serve_on(
    conn: &Connection,
    watch: Arc<Watch>,
    mailbox: Arc<Mailbox>,
    history: Option<Arc<History>>,
) -> Result<(), ServeError> {
    export_on(conn, &ServeOptions::new(watch, mailbox, history)).map(|_| ())
}

/// Export the root object (v1 interface + ObjectManager) and take the name.
pub fn export_on(conn: &Connection, o: &ServeOptions) -> Result<Arc<Shared>, ServeError> {
    let shared = Arc::new(Shared {
        watch: o.watch.clone(),
        mailbox: o.mailbox.clone(),
        history: o.history.clone(),
        settings: o.settings.clone(),
        alias: o.alias.clone(),
        diag: o.diag.clone(),
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
    // Special keys and manual key mapping (#247), see crate::keymap.
    conn.object_server().at(
        OBJECT_PATH,
        crate::keymap::KeymapIface::new(Default::default(), Arc::new(crate::keymap::Pkexec::default())),
    )?;
    let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
    let name = zbus::names::WellKnownName::try_from(o.bus_name.as_str())
        .map_err(|e| ServeError::Bus(e.into()))?;
    match conn.request_name_with_flags(name, flags) {
        Ok(RequestNameReply::PrimaryOwner) | Ok(RequestNameReply::AlreadyOwner) => Ok(shared),
        Ok(_) => Err(ServeError::NameTaken),
        Err(zbus::Error::NameTaken) => Err(ServeError::NameTaken),
        Err(e) => Err(e.into()),
    }
}

/// Connect to the session bus, export, take the name and start the thread
/// that turns snapshot changes into `PropertiesChanged` + `StateChanged`.
pub fn serve(
    watch: Arc<Watch>,
    mailbox: Arc<Mailbox>,
    history: Option<Arc<History>>,
) -> Result<Connection, ServeError> {
    serve_with(ServeOptions::new(watch, mailbox, history))
}

/// [`serve`] with every option (API 2: device signals from `events`).
pub fn serve_with(o: ServeOptions) -> Result<Connection, ServeError> {
    let conn = Connection::session()?;
    let shared = export_on(&conn, &o)?;
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
            let mut prev_forecast = first.forecast.clone();
            let mut dev_prev: HashMap<String, DevProps> = HashMap::new();
            sync_devices(&conn, &shared, &first, &mut dev_prev);
            loop {
                let Some(snap) = watch.wait_newer(seen, Duration::from_secs(60)) else {
                    continue;
                };
                seen = snap.version;
                let now = Props::from_snapshot(&snap);
                let forecast_changed = prev_forecast != snap.forecast;
                if let Err(e) = emit(&conn, &prev, &now, &snap, forecast_changed) {
                    tracing::warn!("cannot emit D-Bus signals: {e}");
                }
                sync_devices(&conn, &shared, &snap, &mut dev_prev);
                prev = now;
                prev_forecast = snap.forecast.clone();
            }
        });
}

/// Export the object of the keyboard in `snap` and emit the changes of
/// every device object (`PropertiesChanged`, `ConnectionChanged`).
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
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|m| m.eq_ignore_ascii_case(mac));
        if !existed {
            // The object is born in its final state: compare it to the
            // "absent keyboard" state so the first connection is signalled (#169).
            prev.entry(mac.to_ascii_uppercase()).or_insert_with(|| {
                DevProps::for_mac(&Snapshot::default(), mac, ("", ""))
            });
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
    let macs = shared
        .devices
        .lock()
        .unwrap_or_else(|e| e.into_inner())
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
            d.remaining_seconds_changed(ctx).await?;
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

/// Turn actor events into device signals.
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
    let pct = |p: f64| p.round().clamp(0.0, 100.0) as i32;
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
            DeviceEvent::Link(_) => Ok(()),
        }
    })
}

fn emit(
    conn: &Connection,
    prev: &Props,
    now: &Props,
    snap: &Snapshot,
    forecast_changed: bool,
) -> zbus::Result<()> {
    let iref = conn.object_server().interface::<_, Monitor>(OBJECT_PATH)?;
    let ctx = iref.signal_context();
    let m = iref.get();
    zbus::block_on(async {
        if prev.battery != now.battery {
            m.battery_changed(ctx).await?;
        }
        if prev.voltage != now.voltage {
            m.voltage_changed(ctx).await?;
        }
        if prev.rssi != now.rssi {
            m.rssi_changed(ctx).await?;
        }
        if prev.connected != now.connected {
            m.connected_changed(ctx).await?;
        }
        if prev.model != now.model {
            m.model_changed(ctx).await?;
        }
        if prev.name != now.name {
            m.name_changed(ctx).await?;
        }
        if prev.mac != now.mac {
            m.mac_changed(ctx).await?;
        }
        if prev.last_update != now.last_update {
            m.last_update_changed(ctx).await?;
        }
        if prev.last_error != now.last_error {
            m.last_error_changed(ctx).await?;
        }
        if prev.firmware_version != now.firmware_version {
            m.firmware_version_changed(ctx).await?;
        }
        if prev.firmware_latest_known != now.firmware_latest_known {
            m.firmware_latest_known_changed(ctx).await?;
        }
        if prev.firmware_status != now.firmware_status {
            m.firmware_status_changed(ctx).await?;
        }
        if prev.device_name_on_keyboard != now.device_name_on_keyboard {
            m.device_name_on_keyboard_changed(ctx).await?;
        }
        if forecast_changed {
            m.remaining_seconds_changed(ctx).await?;
        }
        m.revision_changed(ctx).await?;
        m.json_changed(ctx).await?;
        let json = serde_json::to_string(snap).unwrap_or_default();
        Monitor::state_changed(ctx, snap.version, &json).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use akm_core::KbReport;

    #[test]
    fn props_use_documented_sentinels() {
        let p = Props::from_snapshot(&Snapshot::default());
        assert_eq!(
            (p.battery, p.voltage, p.rssi, p.connected),
            (-1, 0.0, RSSI_UNKNOWN, false)
        );
        assert!(p.model.is_empty() && p.mac.is_empty() && p.last_error.is_empty());
        assert!(p.name.is_empty());

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
            (p.firmware_version.as_str(), p.firmware_latest_known.as_str(), p.firmware_status.as_str()),
            ("", "", "unknown")
        );
        let mut k = KbReport::default();
        k.firmware.version = Some("0x0050".into());
        akm_core::firmware::assess_report(Some(0x0256), &mut k.firmware);
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
        k.device.name_on_keyboard = Some("Clavier de alice #1".into());
        let s = Snapshot {
            keyboard: Some(k),
            ..Default::default()
        };
        assert_eq!(
            Props::from_snapshot(&s).device_name_on_keyboard,
            "Clavier de alice #1"
        );
    }
}
