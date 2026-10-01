//! Session-bus interface `com.agenceapi.AppleKbMonitor1` (read-only state).
//!
//! Object `/com/agenceapi/AppleKbMonitor1`, well-known name
//! `com.agenceapi.AppleKbMonitor1` (also the single-instance lock: whoever
//! owns the name owns the keyboard).
//!
//! Properties (all emit `PropertiesChanged`):
//! * `Battery`  i  percentage 0..100, **-1 = unknown**
//! * `Voltage`  d  volts, **0 = unknown** (HID diagnostic)
//! * `Rssi`     i  dBm, **127 = unknown / stale** (MGMT convention; on
//!   BR/EDR 0 is a valid value: inside the golden receive power range)
//! * `Connected` b, `Model` s, `Mac` s (empty = unknown)
//! * `LastUpdate` t (unix s, 0 = never), `LastError` s (empty = none)
//! * `Revision` t (snapshot counter), `Json` s (full snapshot, schema 1)
//! * since API 2: `InterfaceVersion` u (= 2), `RemainingSeconds` x
//!   (autonomy forecast, **-1 = unknown**, computed at read time)
//!
//! Methods: `GetState() -> s` (= `Json`), `Refresh()`, `History(t since) -> s` (JSON array of
//! `{ts,pct,voltage?,event?}`); since API 2 `GetDevices() -> ao`,
//! `BatterySets() -> s`. Signal: `StateChanged(t revision, s json)`.
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
    pub mac: String,
    pub last_update: u64,
    pub last_error: String,
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
            mac: s.mac().unwrap_or_default().to_string(),
            last_update: s.last_update,
            last_error: s.last_error.clone().unwrap_or_default(),
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
    #[zbus(property)]
    fn last_update(&self) -> u64 {
        self.props().last_update
    }
    #[zbus(property)]
    fn last_error(&self) -> String {
        self.props().last_error
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

    /// Ask for a full read now (no effect while the keyboard is disconnected).
    fn refresh(&self) -> zbus::fdo::Result<()> {
        self.shared.refresh()
    }

    /// History entries with `ts >= since`, as a JSON array.
    fn history(&self, since: u64) -> zbus::fdo::Result<String> {
        self.shared.history_json(since)
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
    /// Well-known name to take (tests / side-by-side instances).
    pub bus_name: String,
}

impl ServeOptions {
    pub fn new(watch: Arc<Watch>, mailbox: Arc<Mailbox>, history: Option<Arc<History>>) -> Self {
        Self {
            watch,
            mailbox,
            history,
            events: EventHub::new(),
            settings: Arc::new(HelperBackend),
            bus_name: BUS_NAME.to_string(),
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
        if let Err(e) = devices::ensure_device(conn, shared, mac, snap.model().unwrap_or_default())
        {
            tracing::warn!("cannot export device {mac}: {e}");
        }
    }
    let macs = shared
        .devices
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    for mac in macs {
        let last_model = prev
            .get(&mac)
            .map(|p| p.base.model.clone())
            .unwrap_or_default();
        let now = DevProps::for_mac(snap, &mac, &last_model);
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
        if o.last_update != n.last_update {
            d.last_update_changed(ctx).await?;
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
    let model = shared.watch.get().model().unwrap_or_default().to_string();
    let Some(path) = devices::ensure_device(conn, shared, ev.mac(), &model)? else {
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
        if prev.mac != now.mac {
            m.mac_changed(ctx).await?;
        }
        if prev.last_update != now.last_update {
            m.last_update_changed(ctx).await?;
        }
        if prev.last_error != now.last_error {
            m.last_error_changed(ctx).await?;
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

        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(89.6);
        k.battery.voltage = Some(2.81);
        k.radio.rssi_dbm = Some(-48);
        k.device.model = Some("A1314".into());
        k.device.mac = Some("04:DB:56:CA:42:EE".into());
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
        assert_eq!(p.mac, "04:DB:56:CA:42:EE");
    }
}
