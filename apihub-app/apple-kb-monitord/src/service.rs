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
//!
//! Methods: `GetState() -> s` (= `Json`), `Refresh()`, `History(t since) -> s` (JSON array of
//! `{ts,pct,voltage?}`). Signal: `StateChanged(t revision, s json)`.

use std::sync::Arc;
use std::time::Duration;

use akm_core::history::History;
use akm_core::{Snapshot, Watch};
use zbus::blocking::Connection;
use zbus::fdo::RequestNameReply;
use zbus::interface;

use crate::actor::{Mailbox, Msg};

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
    watch: Arc<Watch>,
    mailbox: Arc<Mailbox>,
    history: Option<Arc<History>>,
}

impl Monitor {
    fn props(&self) -> Props {
        Props::from_snapshot(&self.watch.get())
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
        self.watch.version()
    }
    #[zbus(property)]
    fn json(&self) -> String {
        serde_json::to_string(&self.watch.get()).unwrap_or_default()
    }
    #[zbus(property)]
    fn daemon_version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    /// Full snapshot as JSON (same as the `Json` property, without a variant:
    /// convenient for QML / shell clients).
    fn get_state(&self) -> String {
        serde_json::to_string(&self.watch.get()).unwrap_or_default()
    }

    /// Ask for a full read now (no effect while the keyboard is disconnected).
    fn refresh(&self) -> zbus::fdo::Result<()> {
        if self.mailbox.send(Msg::Refresh) {
            Ok(())
        } else {
            Err(zbus::fdo::Error::Failed(
                "acquisition thread not running".into(),
            ))
        }
    }

    /// History entries with `ts >= since`, as a JSON array.
    fn history(&self, since: u64) -> zbus::fdo::Result<String> {
        let entries = self
            .history
            .as_ref()
            .map(|h| h.read_since(since))
            .unwrap_or_default();
        serde_json::to_string(&entries).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
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

/// Export the object and take the well-known name on `conn`.
pub fn serve_on(
    conn: &Connection,
    watch: Arc<Watch>,
    mailbox: Arc<Mailbox>,
    history: Option<Arc<History>>,
) -> Result<(), ServeError> {
    conn.object_server().at(
        OBJECT_PATH,
        Monitor {
            watch,
            mailbox,
            history,
        },
    )?;
    let flags = zbus::fdo::RequestNameFlags::DoNotQueue.into();
    match conn.request_name_with_flags(BUS_NAME, flags) {
        Ok(RequestNameReply::PrimaryOwner) | Ok(RequestNameReply::AlreadyOwner) => Ok(()),
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
    let conn = Connection::session()?;
    serve_on(&conn, watch.clone(), mailbox, history)?;
    spawn_emitter(conn.clone(), watch);
    Ok(conn)
}

/// Emit change signals for every new snapshot.
pub fn spawn_emitter(conn: Connection, watch: Arc<Watch>) {
    let _ = std::thread::Builder::new()
        .name("dbus-emitter".into())
        .spawn(move || {
            let mut seen = watch.version();
            let mut prev = Props::from_snapshot(&watch.get());
            loop {
                let Some(snap) = watch.wait_newer(seen, Duration::from_secs(60)) else {
                    continue;
                };
                seen = snap.version;
                let now = Props::from_snapshot(&snap);
                if let Err(e) = emit(&conn, &prev, &now, &snap) {
                    tracing::warn!("cannot emit D-Bus signals: {e}");
                }
                prev = now;
            }
        });
}

fn emit(conn: &Connection, prev: &Props, now: &Props, snap: &Snapshot) -> zbus::Result<()> {
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
