//! D-Bus API v2: one object per keyboard under `…/devices/<MAC>`, via the `ObjectManager`.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use akm_core::batteries;
use akm_core::hid_params::Param;
use akm_core::history::{Clock, History, SystemClock};
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
/// Version of the D-Bus API (`InterfaceVersion` property).
pub const INTERFACE_VERSION: u32 = 2;

/// How long `RereadName()` waits for the actor's answer before saying the request is queued.
pub const NAME_REPLY_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Object path of a keyboard, `None` if `mac` is not `XX:XX:XX:XX:XX:XX`.
#[must_use]
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

/// Unix uid of the D-Bus caller; refused unless it is the daemon's own uid.
///
/// # Errors
/// `AccessDenied` when the caller is another user or its uid cannot be known.
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

/// Who sent this D-Bus call, for the logs: unique name, pid and program.
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
    /// Settings remembered per keyboard; `None`: nothing remembered.
    pub reapply: Option<Arc<crate::reapply::Reapplier>>,
    /// MACs with an exported device object, in creation order.
    pub devices: Mutex<Vec<String>>,
}

impl Shared {
    /// Run `f` off the bus executor: it may wait for the keyboard or parse the history file.
    ///
    /// # Errors
    /// `f`'s error, or [`unblock`]'s.
    pub async fn off_bus<T: Send + 'static>(
        self: &Arc<Self>,
        f: impl FnOnce(&Self) -> zbus::fdo::Result<T> + Send + 'static,
    ) -> zbus::fdo::Result<T> {
        let s = self.clone();
        unblock(move || f(&s)).await?
    }

    pub fn device_paths(&self) -> Vec<OwnedObjectPath> {
        self.devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|m| device_path(m))
            .collect()
    }

    /// `Refresh()`: an error says honestly when nothing will be read.
    ///
    /// # Errors
    /// `LimitsExceeded` when asked too soon, `Failed` when no keyboard can be read.
    pub fn refresh(&self) -> zbus::fdo::Result<()> {
        let (reply, rx) = crate::actor::RefreshReply::channel();
        if !self.mailbox.send(Msg::Refresh(reply)) {
            return Err(zbus::fdo::Error::Failed(
                "acquisition thread not running".into(),
            ));
        }
        refresh_reply(rx.recv_timeout(NAME_REPLY_WAIT))
    }

    /// `RereadName()`: handed to the actor, which forgets `0x51`-`0x54` and reads them again now or
    /// at the end of its 30 s floor.
    ///
    /// # Errors
    /// `Failed` when the actor is gone.
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
                tr!("request queued: the daemon is reading the keyboard, the name is read again after that"),
            ),
        })
    }

    /// The keyboard the daemon follows now, whose history the manager object shows.
    #[must_use]
    pub fn followed_mac(&self) -> Option<String> {
        self.watch.get().mac().map(str::to_string)
    }

    /// `History(since)` of `mac` (`None`: every keyboard): at most
    /// [`DEFAULT_MAX_POINTS`](akm_core::history_limits::DEFAULT_MAX_POINTS) points.
    ///
    /// # Errors
    /// `Failed` when the history cannot be serialized.
    pub fn history_json(&self, since: u64, mac: Option<&str>) -> zbus::fdo::Result<String> {
        self.history_json_max(since, 0, mac)
    }

    /// Entries of `mac` with `ts >= since`, thinned to at most `max` points keeping the first, the
    /// last and every battery replacement.
    ///
    /// # Errors
    /// `Failed` when the history cannot be serialized.
    pub fn history_json_max(
        &self,
        since: u64,
        max: u32,
        mac: Option<&str>,
    ) -> zbus::fdo::Result<String> {
        let max = usize::try_from(max).unwrap_or(usize::MAX);
        let entries = self
            .history
            .as_ref()
            .map(|h| {
                let mine = akm_core::history::of_keyboard(h.read_since(since), mac);
                akm_core::history_limits::downsample(mine, max)
            })
            .unwrap_or_default();
        serde_json::to_string(&entries).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Rename a keyboard on this computer; `""` restores its own name.
    ///
    /// # Errors
    /// The refusal or failure of the `BlueZ` write.
    pub async fn rename(&self, mac: &str, name: &str, caller: String) -> zbus::fdo::Result<String> {
        let (backend, mailbox) = (self.alias.clone(), self.mailbox.clone());
        let (mac, name) = (mac.to_string(), name.to_string());
        let r = unblock(move || alias::rename(backend.as_ref(), &mailbox, &mac, &name, &caller))
            .await??;
        Ok(r.unwrap_or_default())
    }

    /// # Errors
    /// `Failed` when the sets cannot be serialized.
    pub fn battery_sets_json(&self, mac: Option<&str>) -> zbus::fdo::Result<String> {
        let entries = self.history.as_ref().map(|h| h.read()).unwrap_or_default();
        let entries = akm_core::history::of_keyboard(entries, mac);
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
    /// Values for `mac` in `s`; a snapshot of another (or no) keyboard shows this one as
    /// disconnected, keeping its last known model.
    #[must_use]
    pub fn for_mac(s: &Snapshot, mac: &str, last: (&str, &str)) -> Self {
        let (last_model, last_name) = last;
        let mine = s.mac().is_some_and(|m| m.eq_ignore_ascii_case(mac));
        if !mine {
            let mut base = Props::from_snapshot(&Snapshot::default());
            base.mac = mac.to_ascii_uppercase();
            base.model = last_model.to_string();
            base.name = last_name.to_string();
            // Another keyboard of the roster: what BlueZ knows of it.
            if let Some(d) = s.devices.iter().find(|d| d.mac.eq_ignore_ascii_case(mac)) {
                base.connected = d.connected;
                base.battery = d.battery.map_or(-1, akm_core::conv::pct_i32);
                if !d.name.is_empty() {
                    base.name.clone_from(&d.name);
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
    #[must_use]
    pub fn remaining_s(&self, now: u64) -> i64 {
        if self.empty_at == 0 {
            -1
        } else {
            i64::try_from(self.empty_at.saturating_sub(now)).unwrap_or(i64::MAX)
        }
    }
}

#[must_use]
pub fn remaining_seconds(s: &Snapshot, now: u64) -> i64 {
    s.remaining_s(now)
        .map_or(-1, |r| i64::try_from(r).unwrap_or(i64::MAX))
}

pub struct Device {
    mac: String,
    shared: Arc<Shared>,
    last: Mutex<(String, String)>,
}

impl Device {
    /// `SetFnMode` once the caller is checked; the daemon's own menu calls it directly.
    ///
    /// # Errors
    /// The refusal or failure of the settings backend.
    pub async fn apply_fn_mode(&self, mode: i32, conn: &zbus::Connection) -> zbus::fdo::Result<()> {
        self.set(Param::FnMode, mode).await?;
        crate::service::emit_keymap_params(conn).await;
        // The new mode on screen (Plasma OSD).
        crate::osd::poke();
        // Chosen while THIS keyboard was addressed: remembered for it.
        if let Some(r) = self.shared.reapply.as_ref() {
            r.remember(&self.mac, mode);
        }
        Ok(())
    }

    pub fn new(mac: &str, model: &str, name: &str, shared: Arc<Shared>) -> Self {
        Self {
            mac: mac.to_ascii_uppercase(),
            shared,
            last: Mutex::new((model.to_string(), name.to_string())),
        }
    }

    pub fn props(&self) -> DevProps {
        let s = self.shared.watch.get();
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        unblock(move || backend.apply(p, v)).await??;
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
    /// own name, else empty.
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
    #[zbus(property(emits_changed_signal = "false"))]
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

    async fn refresh(&self) -> zbus::fdo::Result<()> {
        self.shared.off_bus(Shared::refresh).await
    }

    async fn history(&self, since: u64) -> zbus::fdo::Result<String> {
        let mac = self.mac.clone();
        self.shared
            .off_bus(move |s| s.history_json(since, Some(&mac)))
            .await
    }

    /// [`Self::history`] with the number of points chosen by the client.
    async fn history_max(&self, since: u64, max: u32) -> zbus::fdo::Result<String> {
        let mac = self.mac.clone();
        self.shared
            .off_bus(move |s| s.history_json_max(since, max, Some(&mac)))
            .await
    }

    /// Battery sets (JSON array of `akm_core::batteries::BatterySet`).
    async fn battery_sets(&self) -> zbus::fdo::Result<String> {
        let mac = self.mac.clone();
        self.shared
            .off_bus(move |s| s.battery_sets_json(Some(&mac)))
            .await
    }

    /// Rename this keyboard on this computer (`BlueZ` alias, nothing is written
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
    ) -> zbus::fdo::Result<()> {
        let uid = caller_uid(conn, &hdr).await?;
        tracing::info!(uid, mode, sender = ?hdr.sender(), "SetFnMode requested");
        self.apply_fn_mode(mode, conn).await
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

/// Export the device object of `mac` if not done yet.
///
/// # Errors
/// The D-Bus error of the export.
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
        let mut d = shared
            .devices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

/// Withdraw the object of a keyboard no longer known (forgotten).
pub fn remove_device(conn: &zbus::blocking::Connection, shared: &Arc<Shared>, mac: &str) {
    let Some(path) = device_path(mac) else { return };
    shared
        .devices
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|m| !m.eq_ignore_ascii_case(mac));
    match conn.object_server().remove::<Device, _>(&path) {
        Ok(_) => tracing::info!("D-Bus device object {path} withdrawn"),
        Err(e) => tracing::warn!("cannot withdraw {path}: {e}"),
    }
    let _ = conn
        .object_server()
        .remove::<crate::passive::Input, _>(&path);
}

/// Run a blocking closure on its own thread and await its result without blocking the D-Bus
/// executor (polkit can take a minute). No thread available (`TasksMax`): `LimitsExceeded`;
/// the closure panics: `Failed`, so the caller gets an answer instead of its timeout.
pub fn unblock<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = zbus::fdo::Result<T>> {
    unblock_with(|job| std::thread::Builder::new().spawn(job).map(drop), f)
}

type Job = Box<dyn FnOnce() + Send>;

fn unblock_with<T: Send + 'static>(
    spawn: impl FnOnce(Job) -> std::io::Result<()>,
    f: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = zbus::fdo::Result<T>> {
    struct Slot<T> {
        val: Option<std::thread::Result<T>>,
        waker: Option<Waker>,
    }
    let slot = Arc::new(Mutex::new(Slot {
        val: None,
        waker: None,
    }));
    let s2 = slot.clone();
    let mut refused = spawn(Box::new(move || {
        let v = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let mut g = s2.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.val = Some(v);
        if let Some(w) = g.waker.take() {
            w.wake();
        }
    }))
    .err()
    .map(|e| zbus::fdo::Error::LimitsExceeded(tr!("no thread for the request: {e}", e = e)));
    std::future::poll_fn(move |cx| {
        if let Some(e) = refused.take() {
            return Poll::Ready(Err(e));
        }
        let mut g = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(v) = g.val.take() {
            Poll::Ready(v.map_err(|_| {
                zbus::fdo::Error::Failed("internal error: the request handler panicked".into())
            }))
        } else {
            g.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    })
}

/// `Refresh()` reply. A timeout means a busy actor with the refresh queued: `Ok`; a dropped
/// reply means the actor died: `Failed`.
fn refresh_reply(
    r: Result<akm_core::machine::RefreshOutcome, std::sync::mpsc::RecvTimeoutError>,
) -> zbus::fdo::Result<()> {
    use akm_core::machine::RefreshOutcome as R;
    use std::sync::mpsc::RecvTimeoutError;
    match r {
        Ok(o @ R::TooSoon { .. }) => Err(zbus::fdo::Error::LimitsExceeded(
            crate::actor::refresh_text(o).unwrap_or_default(),
        )),
        Ok(o @ R::Disconnected) => Err(zbus::fdo::Error::Failed(
            crate::actor::refresh_text(o).unwrap_or_default(),
        )),
        Err(RecvTimeoutError::Disconnected) => Err(zbus::fdo::Error::Failed(
            "acquisition thread not running".into(),
        )),
        Ok(R::Accepted) | Err(RecvTimeoutError::Timeout) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refresh_whose_actor_died_is_a_failure() {
        use std::sync::mpsc::RecvTimeoutError;
        assert!(matches!(
            refresh_reply(Err(RecvTimeoutError::Disconnected)),
            Err(zbus::fdo::Error::Failed(_))
        ));
        assert!(
            refresh_reply(Err(RecvTimeoutError::Timeout)).is_ok(),
            "queued"
        );
        assert!(refresh_reply(Ok(akm_core::machine::RefreshOutcome::Accepted)).is_ok());
    }
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

    #[allow(clippy::many_single_char_names)] // test fixtures with short local names
    #[test]
    fn dev_props_for_this_and_other_keyboards() {
        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(70.0);
        k.device.mac = Some("AA:BB:CC:DD:EE:F1".into());
        k.device.model = Some("A1314".into());
        k.device.alias = Some("Desk".into());
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
        assert_eq!(p.base.name, "Desk");
        assert_eq!(
            (o.base.model.as_str(), o.base.mac.as_str()),
            ("old", "AA:BB:CC:DD:EE:FF")
        );
        assert_eq!(o.remaining_s(0), -1);
        assert_eq!(remaining_seconds(&Snapshot::default(), 0), -1);
        let off = Snapshot {
            connected: false,
            last_update: 777,
            ..s.clone()
        };
        let q = DevProps::for_mac(&off, "AA:BB:CC:DD:EE:F1", ("", ""));
        assert_eq!((q.base.battery, q.base.connected), (70, false));
        assert_eq!(
            (q.empty_at, q.installed_at, q.base.last_update),
            (5_000, 42, 777)
        );
        assert_eq!(q.remaining_s(1_000), remaining_seconds(&off, 1_000));
    }

    #[test]
    fn unblock_returns_the_value() {
        assert_eq!(zbus::block_on(unblock(|| 6 * 7)).unwrap(), 42);
        let refused = unblock_with(|_| Err(std::io::ErrorKind::WouldBlock.into()), || 1);
        assert!(matches!(
            zbus::block_on(refused),
            Err(zbus::fdo::Error::LimitsExceeded(_))
        ));
    }

    #[test]
    fn unblock_answers_when_the_job_panics() {
        let r = zbus::block_on(unblock(|| -> u8 { panic!("boom") }));
        assert!(matches!(r, Err(zbus::fdo::Error::Failed(_))), "{r:?}");
    }
}
