//! D-Bus API v2 (#93): one object per keyboard under
//! `/com/agenceapi/AppleKbMonitor1/devices/<MAC_WITH_UNDERSCORES>`,
//! interface `com.agenceapi.AppleKbMonitor1.Device`, announced through
//! `org.freedesktop.DBus.ObjectManager` on `/com/agenceapi/AppleKbMonitor1`.
//! The v1 interface on the root object is unchanged (additive only).
//!
//! Properties (`PropertiesChanged` emitted):
//! * `Mac` s, `Model` s, `Name` s (alias, else own name; "" unknown), `Connected` b
//! * `Battery` i (-1 unknown), `Voltage` d (0 unknown), `Rssi` i (relative dB on BR/EDR, not dBm, #174; 127 unknown)
//! * `LastUpdate` t (0 never)
//! * `RemainingSeconds` x (-1 unknown; computed at read time from `EmptyAt`)
//! * `DischargeRate` d (% per day, 0 unknown), `EmptyAt` t (0 unknown)
//! * `BatteriesInstalledAt` t (0 unknown)
//! * `FnMode` i, `SwapOptCmd` i, `IsoLayout` i (`hid_apple`, global to the
//!   module; -100 when not loaded)
//!
//! Methods: `Refresh()`, `History(t since) -> s` (at most 2000 points),
//! `HistoryMax(t since, u max) -> s`, `BatterySets() -> s`,
//! `SetAlias(s) -> s` (BlueZ alias, validated; "" = reset),
//! `SetFnMode(i)` (caller uid checked, validated, then delegated to the
//! privileged helper through polkit; `NotSupported` without it, `LimitsExceeded`
//! while another authentication is pending). `SwapOptCmd`/`IsoLayout` are
//! read-only (no polkit action, #203).
//!
//! Signals: `BatteryLevelCrossed(u threshold, i battery, s urgency)`,
//! `ConnectionChanged(b connected, i battery)`,
//! `BatteryReplaced(t ts, i battery_before, i battery_after, d voltage_before, d voltage_after)`.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use akm_core::batteries;
use akm_core::hid_params::Param;
use akm_core::history::{Clock, History, SystemClock};
#[allow(unused_imports)] // named by the documentation
use akm_core::history_limits::DEFAULT_MAX_POINTS;
use akm_core::{Snapshot, Watch};
use zbus::interface;
use zbus::object_server::SignalContext;
use zbus::zvariant::OwnedObjectPath;

use crate::actor::{Mailbox, Msg};
use crate::alias::{self, AliasBackend};
use crate::service::Props;
use crate::settings::{self, SettingsBackend};

pub const DEVICE_INTERFACE: &str = "com.agenceapi.AppleKbMonitor1.Device";
pub const DEVICES_PATH: &str = "/com/agenceapi/AppleKbMonitor1/devices";
/// Version of the D-Bus API (`InterfaceVersion` property): 1 = root object
/// only, 2 = per-device objects + write methods + event signals.
pub const INTERFACE_VERSION: u32 = 2;

/// How long `RereadName()` waits for the actor's answer before saying the
/// request is queued (the actor may be in the middle of a read burst).
pub const NAME_REPLY_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Object path of a keyboard, `None` if `mac` is not `XX:XX:XX:XX:XX:XX`.
pub fn device_path(mac: &str) -> Option<OwnedObjectPath> {
    let parts: Vec<&str> = mac.split(':').collect();
    let ok = parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()));
    if !ok {
        return None;
    }
    let tail = mac.to_ascii_uppercase().replace(':', "_");
    OwnedObjectPath::try_from(format!("{DEVICES_PATH}/{tail}")).ok()
}

/// Unix uid of the D-Bus caller; refused unless it is the daemon's own uid
/// (#203: the daemon opens a polkit dialog on behalf of its caller).
pub async fn caller_uid(
    conn: &zbus::Connection,
    hdr: &zbus::message::Header<'_>,
) -> zbus::fdo::Result<u32> {
    let sender = hdr
        .sender()
        .ok_or_else(|| zbus::fdo::Error::AccessDenied("anonymous caller".into()))?;
    let uid = zbus::fdo::DBusProxy::new(conn)
        .await?
        .get_connection_unix_user(sender.clone().into())
        .await?;
    // SAFETY: getuid(2) has no preconditions.
    if uid != unsafe { libc::getuid() } {
        tracing::warn!(uid, "D-Bus caller with another uid refused");
        return Err(zbus::fdo::Error::AccessDenied(
            "caller uid differs from the daemon's".into(),
        ));
    }
    Ok(uid)
}

/// Who sent this D-Bus call, for the logs: unique name, pid and program
/// (`:1.42 pid 1234 (plasmashell)`). Best effort, never fails: the alias has
/// been reset behind the user's back before and the culprit must be findable
/// in the journal.
pub async fn describe_caller(conn: &zbus::Connection, hdr: &zbus::message::Header<'_>) -> String {
    let Some(sender) = hdr.sender() else {
        return "anonymous D-Bus caller".into();
    };
    let pid = match zbus::fdo::DBusProxy::new(conn).await {
        Ok(p) => p
            .get_connection_unix_process_id(sender.clone().into())
            .await
            .ok(),
        Err(_) => None,
    };
    let comm = pid.and_then(|p| std::fs::read_to_string(format!("/proc/{p}/comm")).ok());
    match (pid, comm) {
        (Some(p), Some(c)) => format!("{sender} pid {p} ({})", c.trim()),
        (Some(p), None) => format!("{sender} pid {p}"),
        _ => sender.to_string(),
    }
}

/// What every exported object shares.
pub struct Shared {
    pub watch: Arc<Watch>,
    pub mailbox: Arc<Mailbox>,
    pub history: Option<Arc<History>>,
    pub settings: Arc<dyn SettingsBackend>,
    /// Alias (name on this computer) of the keyboards.
    pub alias: Arc<dyn AliasBackend>,
    /// What `Diagnose()` looks at.
    pub diag: Arc<dyn crate::diagnose::Probe>,
    /// Settings remembered per keyboard (#103); `None`: nothing remembered.
    pub reapply: Option<Arc<crate::reapply::Reapplier>>,
    /// MACs with an exported device object, in creation order.
    pub devices: Mutex<Vec<String>>,
}

impl Shared {
    pub fn device_paths(&self) -> Vec<OwnedObjectPath> {
        self.devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter_map(|m| device_path(m))
            .collect()
    }

    pub fn refresh(&self) -> zbus::fdo::Result<()> {
        if self.mailbox.send(Msg::Refresh) {
            Ok(())
        } else {
            Err(zbus::fdo::Error::Failed(
                "acquisition thread not running".into(),
            ))
        }
    }

    /// `RereadName()`: handed to the actor, which forgets `0x51`-`0x54` and
    /// reads them again now or at the end of its 30 s floor. Returns
    /// `(accepted, what will happen)`; the actor is given
    /// [`NAME_REPLY_WAIT`] to answer (it may be in the middle of a read).
    pub fn reread_name(&self) -> zbus::fdo::Result<(bool, String)> {
        let (reply, rx) = crate::actor::NameReply::channel();
        if !self.mailbox.send(Msg::RereadName(reply)) {
            return Err(zbus::fdo::Error::Failed(
                "acquisition thread not running".into(),
            ));
        }
        Ok(match rx.recv_timeout(NAME_REPLY_WAIT) {
            Ok(r) => (r.accepted(), crate::actor::name_reread_text(r)),
            Err(_) => (
                true,
                "request queued: the daemon is reading the keyboard, the name is read again after that"
                    .into(),
            ),
        })
    }

    /// `History(since)`: at most [`DEFAULT_MAX_POINTS`] points (#96).
    pub fn history_json(&self, since: u64) -> zbus::fdo::Result<String> {
        self.history_json_max(since, 0)
    }

    /// Entries with `ts >= since`, thinned by the daemon to at most `max`
    /// points (0 = [`DEFAULT_MAX_POINTS`], never above
    /// [`akm_core::history_limits::MAX_POINTS_LIMIT`]): the first, the last,
    /// every battery replacement and evenly spaced samples (#96).
    pub fn history_json_max(&self, since: u64, max: u32) -> zbus::fdo::Result<String> {
        let max = usize::try_from(max).unwrap_or(usize::MAX);
        let entries = self
            .history
            .as_ref()
            .map(|h| h.read_since_bounded(since, max))
            .unwrap_or_default();
        serde_json::to_string(&entries).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Rename a keyboard on this computer; `""` restores its own name.
    /// Returns the alias now in effect (empty = unknown).
    /// `caller` names who asked ([`describe_caller`]), logged and remembered.
    pub async fn rename(&self, mac: &str, name: &str, caller: String) -> zbus::fdo::Result<String> {
        let (backend, mailbox) = (self.alias.clone(), self.mailbox.clone());
        let (mac, name) = (mac.to_string(), name.to_string());
        let r = unblock(move || alias::rename(backend.as_ref(), &mailbox, &mac, &name, &caller))
            .await?;
        Ok(r.unwrap_or_default())
    }

    pub fn battery_sets_json(&self) -> zbus::fdo::Result<String> {
        let entries = self.history.as_ref().map(|h| h.read()).unwrap_or_default();
        serde_json::to_string(&batteries::battery_sets(&entries))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }
}

/// D-Bus view of one device.
#[derive(Debug, Clone, PartialEq)]
pub struct DevProps {
    pub base: Props,
    pub rate_pct_per_day: f64,
    pub empty_at: u64,
    pub installed_at: u64,
}

impl DevProps {
    /// Values for `mac` in `s`; a snapshot of another (or no) keyboard
    /// shows this one as disconnected, keeping its last known model.
    pub fn for_mac(s: &Snapshot, mac: &str, last: (&str, &str)) -> Self {
        let (last_model, last_name) = last;
        let mine = s.mac().is_some_and(|m| m.eq_ignore_ascii_case(mac));
        if !mine {
            let mut base = Props::from_snapshot(&Snapshot::default());
            base.mac = mac.to_ascii_uppercase();
            base.model = last_model.to_string();
            base.name = last_name.to_string();
            // Another keyboard of the roster (#94): what BlueZ knows of it.
            if let Some(d) = s.devices.iter().find(|d| d.mac.eq_ignore_ascii_case(mac)) {
                base.connected = d.connected;
                base.battery = d.battery.map_or(-1, |p| p.round().clamp(0.0, 100.0) as i32);
                if !d.name.is_empty() {
                    base.name = d.name.clone();
                }
            }
            return Self {
                base,
                rate_pct_per_day: 0.0,
                empty_at: 0,
                installed_at: 0,
            };
        }
        Self {
            base: Props::from_snapshot(s),
            rate_pct_per_day: s.forecast.as_ref().map_or(0.0, |f| f.rate_pct_per_day),
            empty_at: s.forecast.as_ref().map_or(0, |f| f.empty_at),
            installed_at: s.batteries_installed_at.unwrap_or(0),
        }
    }

    /// `RemainingSeconds` at unix time `now`.
    pub fn remaining_s(&self, now: u64) -> i64 {
        if self.empty_at == 0 {
            -1
        } else {
            i64::try_from(self.empty_at.saturating_sub(now)).unwrap_or(i64::MAX)
        }
    }
}

/// `RemainingSeconds` of a snapshot at `now` (-1 = unknown).
pub fn remaining_seconds(s: &Snapshot, now: u64) -> i64 {
    s.remaining_s(now)
        .map_or(-1, |r| i64::try_from(r).unwrap_or(i64::MAX))
}

pub struct Device {
    mac: String,
    shared: Arc<Shared>,
    /// Last known (model, name) while another keyboard / none is published.
    last: Mutex<(String, String)>,
}

impl Device {
    pub fn new(mac: &str, model: &str, name: &str, shared: Arc<Shared>) -> Self {
        Self {
            mac: mac.to_ascii_uppercase(),
            shared,
            last: Mutex::new((model.to_string(), name.to_string())),
        }
    }

    pub fn props(&self) -> DevProps {
        let s = self.shared.watch.get();
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if s.mac().is_some_and(|x| x.eq_ignore_ascii_case(&self.mac)) {
            if let Some(m) = s.model().filter(|m| *m != last.0) {
                last.0 = m.to_string();
            }
            if let Some(n) = s.display_name().filter(|n| *n != last.1) {
                last.1 = n.to_string();
            }
        }
        DevProps::for_mac(&s, &self.mac, (&last.0, &last.1))
    }

    async fn set(&self, p: Param, v: i32) -> zbus::fdo::Result<()> {
        settings::validate(p, v)?;
        let backend = self.shared.settings.clone();
        unblock(move || backend.apply(p, v)).await?;
        Ok(())
    }
}

#[interface(name = "com.agenceapi.AppleKbMonitor1.Device")]
impl Device {
    #[zbus(property)]
    fn mac(&self) -> String {
        self.mac.clone()
    }
    #[zbus(property)]
    fn model(&self) -> String {
        self.props().base.model
    }
    /// Name to show: the alias set on this computer, else the keyboard's
    /// own name, else empty (#141).
    #[zbus(property)]
    fn name(&self) -> String {
        self.props().base.name
    }
    #[zbus(property)]
    fn connected(&self) -> bool {
        self.props().base.connected
    }
    #[zbus(property)]
    fn battery(&self) -> i32 {
        self.props().base.battery
    }
    #[zbus(property)]
    fn voltage(&self) -> f64 {
        self.props().base.voltage
    }
    #[zbus(property)]
    fn rssi(&self) -> i32 {
        self.props().base.rssi
    }
    #[zbus(property)]
    fn last_update(&self) -> u64 {
        self.props().base.last_update
    }
    /// Firmware version (report 0x4F, once per connection), "" = not read.
    #[zbus(property)]
    fn firmware_version(&self) -> String {
        self.props().base.firmware_version
    }
    /// Latest public firmware known for this model, "" = not in the table.
    #[zbus(property)]
    fn firmware_latest_known(&self) -> String {
        self.props().base.firmware_latest_known
    }
    /// `up_to_date` / `update_available` / `unknown`.
    #[zbus(property)]
    fn firmware_status(&self) -> String {
        self.props().base.firmware_status
    }
    #[zbus(property)]
    fn remaining_seconds(&self) -> i64 {
        self.props().remaining_s(SystemClock.now())
    }
    #[zbus(property)]
    fn discharge_rate(&self) -> f64 {
        self.props().rate_pct_per_day
    }
    #[zbus(property)]
    fn empty_at(&self) -> u64 {
        self.props().empty_at
    }
    #[zbus(property)]
    fn batteries_installed_at(&self) -> u64 {
        self.props().installed_at
    }
    #[zbus(property)]
    fn fn_mode(&self) -> i32 {
        self.shared.settings.get(Param::FnMode)
    }
    #[zbus(property)]
    fn swap_opt_cmd(&self) -> i32 {
        self.shared.settings.get(Param::SwapOptCmd)
    }
    #[zbus(property)]
    fn iso_layout(&self) -> i32 {
        self.shared.settings.get(Param::IsoLayout)
    }

    fn refresh(&self) -> zbus::fdo::Result<()> {
        self.shared.refresh()
    }

    fn history(&self, since: u64) -> zbus::fdo::Result<String> {
        self.shared.history_json(since)
    }

    /// [`Self::history`] with the number of points chosen by the client.
    fn history_max(&self, since: u64, max: u32) -> zbus::fdo::Result<String> {
        self.shared.history_json_max(since, max)
    }

    /// Battery sets (JSON array of `akm_core::batteries::BatterySet`).
    fn battery_sets(&self) -> zbus::fdo::Result<String> {
        self.shared.battery_sets_json()
    }

    /// Rename this keyboard on this computer (BlueZ alias, nothing is written
    /// into the keyboard). `""` restores its own name. Returns the new name.
    async fn set_alias(
        &self,
        name: &str,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<String> {
        let caller = describe_caller(conn, &hdr).await;
        self.shared.rename(&self.mac, name, caller).await
    }

    async fn set_fn_mode(
        &self,
        mode: i32,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
    ) -> zbus::fdo::Result<()> {
        let uid = caller_uid(conn, &hdr).await?;
        tracing::info!(uid, mode, sender = ?hdr.sender(), "SetFnMode requested");
        self.set(Param::FnMode, mode).await?;
        let _ = self.fn_mode_changed(&ctxt).await;
        // The new mode on screen (Plasma OSD, #100).
        crate::osd::poke();
        // Chosen while THIS keyboard was addressed: remembered for it (#103).
        if let Some(r) = self.shared.reapply.as_ref() {
            r.remember(&self.mac, mode);
        }
        Ok(())
    }

    #[zbus(signal)]
    pub async fn battery_level_crossed(
        ctxt: &SignalContext<'_>,
        threshold: u32,
        battery: i32,
        urgency: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn connection_changed(
        ctxt: &SignalContext<'_>,
        connected: bool,
        battery: i32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn battery_replaced(
        ctxt: &SignalContext<'_>,
        ts: u64,
        battery_before: i32,
        battery_after: i32,
        voltage_before: f64,
        voltage_after: f64,
    ) -> zbus::Result<()>;
}

/// Export the device object of `mac` if not done yet. Returns its path.
pub fn ensure_device(
    conn: &zbus::blocking::Connection,
    shared: &Arc<Shared>,
    mac: &str,
    model: &str,
    name: &str,
) -> zbus::Result<Option<OwnedObjectPath>> {
    let Some(path) = device_path(mac) else {
        return Ok(None);
    };
    let mac = mac.to_ascii_uppercase();
    {
        let mut d = shared.devices.lock().unwrap_or_else(|e| e.into_inner());
        if d.contains(&mac) {
            return Ok(Some(path));
        }
        d.push(mac.clone());
    }
    conn.object_server()
        .at(&path, Device::new(&mac, model, name, shared.clone()))?;
    tracing::info!("D-Bus device object {path}");
    Ok(Some(path))
}

/// Run a blocking closure on its own thread and await its result without
/// blocking the D-Bus executor (polkit can take a minute).
pub fn unblock<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = T> {
    struct Slot<T> {
        val: Option<T>,
        waker: Option<Waker>,
    }
    let slot = Arc::new(Mutex::new(Slot {
        val: None,
        waker: None,
    }));
    let s2 = slot.clone();
    std::thread::spawn(move || {
        let v = f();
        let mut g = s2.lock().unwrap_or_else(|e| e.into_inner());
        g.val = Some(v);
        if let Some(w) = g.waker.take() {
            w.wake();
        }
    });
    std::future::poll_fn(move |cx| {
        let mut g = slot.lock().unwrap_or_else(|e| e.into_inner());
        match g.val.take() {
            Some(v) => Poll::Ready(v),
            None => {
                g.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use akm_core::forecast::Forecast;
    use akm_core::KbReport;

    #[test]
    fn device_paths() {
        assert_eq!(
            device_path("AA:BB:CC:DD:EE:F1").unwrap().as_str(),
            "/com/agenceapi/AppleKbMonitor1/devices/AA_BB_CC_DD_EE_F1"
        );
        for bad in [
            "",
            "AA:BB:CC:DD:EE",
            "AA:BB:CC:DD:EE:EG",
            "AA-BB-CC-DD-EE-F1",
        ] {
            assert!(device_path(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn dev_props_for_this_and_other_keyboards() {
        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(70.0);
        k.device.mac = Some("AA:BB:CC:DD:EE:F1".into());
        k.device.model = Some("A1314".into());
        k.device.alias = Some("Bureau".into());
        let s = Snapshot {
            connected: true,
            keyboard: Some(k),
            forecast: Some(Forecast {
                rate_pct_per_day: 1.0,
                empty_at: 5_000,
                fitted_pct: 70.0,
                span_s: 0,
                buckets: 0,
            }),
            batteries_installed_at: Some(42),
            ..Default::default()
        };
        let p = DevProps::for_mac(&s, "aa:bb:cc:dd:ee:f1", ("", ""));
        assert_eq!((p.base.battery, p.base.connected), (70, true));
        assert_eq!((p.empty_at, p.installed_at), (5_000, 42));
        assert_eq!(p.remaining_s(1_000), 4_000);
        assert_eq!(p.remaining_s(9_000), 0);
        assert_eq!(remaining_seconds(&s, 1_000), 4_000);
        let o = DevProps::for_mac(&s, "AA:BB:CC:DD:EE:FF", ("old", "old name"));
        assert_eq!((o.base.battery, o.base.connected), (-1, false));
        assert_eq!(
            (o.base.model.as_str(), o.base.mac.as_str()),
            ("old", "AA:BB:CC:DD:EE:FF")
        );
        assert_eq!(o.base.name, "old name");
        assert_eq!(p.base.name, "Bureau");
        assert_eq!(
            (o.base.model.as_str(), o.base.mac.as_str()),
            ("old", "AA:BB:CC:DD:EE:FF")
        );
        assert_eq!(o.remaining_s(0), -1);
        assert_eq!(remaining_seconds(&Snapshot::default(), 0), -1);
        // #168: asleep keyboard (stale report kept): v2 agrees with v1.
        let off = Snapshot {
            connected: false,
            last_update: 777,
            ..s.clone()
        };
        let q = DevProps::for_mac(&off, "AA:BB:CC:DD:EE:F1", ("", ""));
        assert_eq!((q.base.battery, q.base.connected), (70, false));
        assert_eq!((q.empty_at, q.installed_at, q.base.last_update), (5_000, 42, 777));
        assert_eq!(q.remaining_s(1_000), remaining_seconds(&off, 1_000));
    }

    #[test]
    fn unblock_returns_the_value() {
        assert_eq!(zbus::block_on(unblock(|| 6 * 7)), 42);
    }
}
