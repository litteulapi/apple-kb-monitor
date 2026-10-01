//! BlueZ Battery Provider — exposes Apple keyboard battery to BlueZ clients.
//!
//! The process exports one `org.bluez.BatteryProvider1` object per connected
//! keyboard under `/com/agenceapi/AppleKbMonitor/dev_AA_BB_CC_DD_EE_FF`, plus a
//! `org.freedesktop.DBus.ObjectManager` on `/com/agenceapi/AppleKbMonitor`
//! (zbus' own implementation, which emits `InterfacesAdded/Removed`), and
//! registers that root with `org.bluez.BatteryProviderManager1` on the adapter.
//! BlueZ then creates `org.bluez.Battery1` on the device object.
//!
//! Design points (issue #67, docs/REVUE-ARCHITECTURE-CLAVIER.md §2.5, §4.4):
//! * No well-known bus name: BlueZ tracks the unique name of the caller of
//!   `RegisterBatteryProvider`, and the system bus policy denies `own` to
//!   unprivileged users anyway.
//! * The adapter path is resolved with `GetManagedObjects` on `org.bluez`
//!   (first object exposing `BatteryProviderManager1`), never hard-coded.
//! * Registration is maintained by a small state machine ([`decide`]): it is
//!   redone when bluetoothd restarts (new owner) or when the adapter changes,
//!   and exported device paths are rebuilt for the new adapter.
//! * MAC addresses are normalised (upper-case, colon separated).
//!
//! # Expected behaviour with UPower / KDE
//! For HID keyboards whose kernel driver already publishes a `power_supply`
//! (A1314, A1255, 2009 models), UPower hides the BlueZ battery with the same
//! serial (the MAC): `up-backend.c::update_added_duplicate_device()`. The value
//! pushed here therefore never reaches UPower, PowerDevil or the Plasma battery
//! applet for those keyboards (the kernel one is displayed). It remains visible
//! to clients reading `org.bluez.Battery1` directly (Plasma Bluetooth applet /
//! bluez-qt, GNOME Bluetooth panel). For a keyboard without kernel battery the
//! provider is the only source and UPower shows it. Hence: keep the provider as
//! a fallback/complement, not as the source of truth.
//!
//! Uses the zbus 4 blocking API on one dedicated thread owning the connection.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const BLUEZ_SERVICE: &str = "org.bluez";
const PROVIDER_MANAGER_IFACE: &str = "org.bluez.BatteryProviderManager1";
const PROVIDER_ROOT: &str = "/com/agenceapi/AppleKbMonitor";
const SOURCE: &str = "apple-kb-monitord (kernel power_supply)";
/// How often BlueZ presence / adapter are re-checked.
const WATCH_PERIOD: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Pure logic (unit-tested)
// ---------------------------------------------------------------------------

/// Normalise a MAC to `AA:BB:CC:DD:EE:FF`. `None` unless exactly six
/// colon-separated hex octets (also guarantees a valid object-path component).
fn normalize_mac(mac: &str) -> Option<String> {
    let parts: Vec<&str> = mac.trim().split(':').collect();
    if parts.len() != 6
        || !parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(parts.join(":").to_ascii_uppercase())
}

/// Path segment `AA_BB_CC_DD_EE_FF` of a normalised MAC.
fn mac_segment(norm_mac: &str) -> String {
    norm_mac.replace(':', "_")
}

/// Child object exported for a keyboard.
fn child_path(norm_mac: &str) -> String {
    format!("{}/dev_{}", PROVIDER_ROOT, mac_segment(norm_mac))
}

/// BlueZ device object of a keyboard on a given adapter.
fn device_path(adapter: &str, norm_mac: &str) -> String {
    format!(
        "{}/dev_{}",
        adapter.trim_end_matches('/'),
        mac_segment(norm_mac)
    )
}

/// Pick the adapter among BlueZ managed objects `(path, interfaces)`:
/// the lexicographically first object exposing `BatteryProviderManager1`.
fn pick_adapter(objects: &[(String, Vec<String>)]) -> Option<String> {
    objects
        .iter()
        .filter(|(_, ifaces)| ifaces.iter().any(|i| i == PROVIDER_MANAGER_IFACE))
        .map(|(p, _)| p.clone())
        .min()
}

/// Delay before retrying a failed registration: 5 s, 10 s, 20 s ... capped at 5 min.
fn retry_delay(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(6);
    WATCH_PERIOD
        .saturating_mul(1u32 << shift)
        .min(Duration::from_secs(300))
}

/// A failure is logged only when the counter is a power of two (1, 2, 4, 8...),
/// so a permanently refusing BlueZ does not flood the journal.
fn should_log_failure(failures: u32) -> bool {
    failures > 0 && failures.is_power_of_two()
}

/// What we know about the registration with BlueZ.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reg {
    Unregistered,
    Registered { owner: String, adapter: String },
}

/// What was just observed on the bus (`None` = bluetoothd absent or no adapter).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Observed {
    owner: String,
    adapter: String,
}

/// Action required to converge to a registered state.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    Nothing,
    /// BlueZ gone: forget the registration (objects stay exported).
    Forget,
    /// (Re-)register on `adapter`; `unregister_from` is set when the old BlueZ
    /// instance is still alive and must drop its registration first.
    Register {
        adapter: String,
        unregister_from: Option<String>,
    },
}

fn decide(reg: &Reg, obs: Option<&Observed>) -> Action {
    match (reg, obs) {
        (Reg::Unregistered, None) => Action::Nothing,
        (Reg::Registered { .. }, None) => Action::Forget,
        (Reg::Unregistered, Some(o)) => Action::Register {
            adapter: o.adapter.clone(),
            unregister_from: None,
        },
        (Reg::Registered { owner, adapter }, Some(o)) => {
            if *owner != o.owner {
                // bluetoothd restarted: it forgot us, the old owner is gone.
                Action::Register {
                    adapter: o.adapter.clone(),
                    unregister_from: None,
                }
            } else if *adapter != o.adapter {
                Action::Register {
                    adapter: o.adapter.clone(),
                    unregister_from: Some(adapter.clone()),
                }
            } else {
                Action::Nothing
            }
        }
    }
}

// ---------------------------------------------------------------------------
// org.bluez.BatteryProvider1 — per-device battery object
// ---------------------------------------------------------------------------

struct Bat {
    device: OwnedObjectPath,
    pct: u8,
}

#[interface(name = "org.bluez.BatteryProvider1")]
impl Bat {
    #[zbus(property)]
    fn percentage(&self) -> u8 {
        self.pct
    }

    #[zbus(property)]
    fn device(&self) -> OwnedObjectPath {
        self.device.clone()
    }

    #[zbus(property)]
    fn source(&self) -> &str {
        SOURCE
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

enum Cmd {
    Set(String, u8),
    #[allow(dead_code)]
    Remove(String),
    Stop,
}

/// Handle to the BlueZ battery provider thread. Dropping it unregisters from
/// BlueZ and removes every exported object.
pub struct BatteryProvider {
    tx: Sender<Cmd>,
    handle: Option<thread::JoinHandle<()>>,
    /// MAC used by the legacy single-keyboard API ([`start`] / [`update_percentage`]).
    legacy_mac: Option<String>,
}

impl BatteryProvider {
    /// Spawn the provider thread with no keyboard yet.
    #[allow(dead_code)]
    pub fn spawn() -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let handle = match thread::Builder::new()
            .name("bluez-provider".into())
            .spawn(move || run_provider(rx))
        {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("cannot spawn provider thread: {}", e);
                return None;
            }
        };
        Some(Self {
            tx,
            handle: Some(handle),
            legacy_mac: None,
        })
    }

    /// Spawn the provider and publish `initial_pct` for `mac`. Returns `None`
    /// if the MAC is invalid. Kept for the single-keyboard caller in `main.rs`.
    pub fn start(mac: &str, initial_pct: u8) -> Option<Self> {
        let norm = normalize_mac(mac)?;
        let mut p = Self::spawn()?;
        p.set_battery(&norm, initial_pct);
        p.legacy_mac = Some(norm);
        Some(p)
    }

    /// Publish / update the battery percentage (0-100) of a keyboard. The
    /// object is exported on first call. Invalid MACs are ignored.
    pub fn set_battery(&self, mac: &str, percent: u8) {
        if let Some(m) = normalize_mac(mac) {
            let _ = self.tx.send(Cmd::Set(m, percent.min(100)));
        }
    }

    /// Withdraw a keyboard (e.g. on disconnection): its object is unexported
    /// so BlueZ drops `Battery1` instead of showing a frozen value.
    #[allow(dead_code)] // used once main.rs withdraws the provider on disconnection
    pub fn remove(&self, mac: &str) {
        if let Some(m) = normalize_mac(mac) {
            let _ = self.tx.send(Cmd::Remove(m));
        }
    }

    /// Legacy: update the keyboard given to [`start`].
    pub fn update_percentage(&self, pct: u8) {
        if let Some(m) = &self.legacy_mac {
            self.set_battery(m, pct);
        }
    }
}

impl Drop for BatteryProvider {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Provider thread
// ---------------------------------------------------------------------------

struct Worker {
    conn: Connection,
    /// Desired state: normalised MAC -> percentage.
    desired: BTreeMap<String, u8>,
    /// MACs currently exported (child path derived from the MAC).
    exported: BTreeMap<String, ()>,
    reg: Reg,
    /// Consecutive registration failures and earliest next attempt.
    failures: u32,
    retry_at: Instant,
}

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

fn run_provider(rx: mpsc::Receiver<Cmd>) {
    let conn = match zbus::blocking::connection::Builder::system()
        .and_then(|b| b.serve_at(PROVIDER_ROOT, zbus::fdo::ObjectManager))
        .and_then(|b| b.build())
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("D-Bus setup failed: {}", e);
            return;
        }
    };
    let mut w = Worker {
        conn,
        desired: BTreeMap::new(),
        exported: BTreeMap::new(),
        reg: Reg::Unregistered,
        failures: 0,
        retry_at: Instant::now(),
    };

    let mut next_watch = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Cmd::Set(mac, pct)) => w.apply_set(&mac, pct),
            Ok(Cmd::Remove(mac)) => w.apply_remove(&mac),
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= next_watch {
            w.watch();
            next_watch = Instant::now() + WATCH_PERIOD;
        }
    }
    w.shutdown();
}

impl Worker {
    fn adapter(&self) -> Option<&str> {
        match &self.reg {
            Reg::Registered { adapter, .. } => Some(adapter),
            Reg::Unregistered => None,
        }
    }

    fn apply_set(&mut self, mac: &str, pct: u8) {
        self.desired.insert(mac.to_string(), pct);
        let Some(adapter) = self.adapter().map(str::to_string) else {
            return; // exported at registration time
        };
        if self.exported.contains_key(mac) {
            self.update_object(mac, pct);
        } else {
            self.export(&adapter, mac, pct);
        }
    }

    fn apply_remove(&mut self, mac: &str) {
        self.desired.remove(mac);
        self.unexport(mac);
    }

    fn export(&mut self, adapter: &str, mac: &str, pct: u8) {
        let Ok(dev) = OwnedObjectPath::try_from(device_path(adapter, mac)) else {
            return;
        };
        let path = child_path(mac);
        match self
            .conn
            .object_server()
            .at(path.as_str(), Bat { device: dev, pct })
        {
            Ok(_) => {
                self.exported.insert(mac.to_string(), ());
            }
            Err(e) => tracing::warn!("export {} failed: {}", path, e),
        }
    }

    fn unexport(&mut self, mac: &str) {
        if self.exported.remove(mac).is_some() {
            let path = child_path(mac);
            if let Err(e) = self.conn.object_server().remove::<Bat, _>(path.as_str()) {
                tracing::warn!("unexport {} failed: {}", path, e);
            }
        }
    }

    fn update_object(&self, mac: &str, pct: u8) {
        let path = child_path(mac);
        let Ok(iref) = self.conn.object_server().interface::<_, Bat>(path.as_str()) else {
            return;
        };
        let changed = {
            let mut b = iref.get_mut();
            let c = b.pct != pct;
            b.pct = pct;
            c
        };
        if changed {
            if let Err(e) = zbus::block_on(iref.get().percentage_changed(iref.signal_context())) {
                tracing::warn!("PropertiesChanged failed: {}", e);
            }
        }
    }

    /// Observe BlueZ and converge the registration state.
    fn watch(&mut self) {
        let obs = self.observe();
        match decide(&self.reg, obs.as_ref()) {
            Action::Nothing => {}
            Action::Forget => {
                tracing::warn!("bluetoothd gone, will re-register when it returns");
                self.reg = Reg::Unregistered;
            }
            Action::Register { .. } if self.failures > 0 && Instant::now() < self.retry_at => {}
            Action::Register {
                adapter,
                unregister_from,
            } => {
                let owner = obs.map(|o| o.owner).unwrap_or_default();
                if let Some(old) = unregister_from {
                    let _ = self.call_manager(&old, "UnregisterBatteryProvider");
                }
                // Rebuild exported objects so Device points to the new adapter.
                let macs: Vec<String> = self.exported.keys().cloned().collect();
                for m in macs {
                    self.unexport(&m);
                }
                let desired: Vec<(String, u8)> =
                    self.desired.iter().map(|(m, p)| (m.clone(), *p)).collect();
                for (m, p) in desired {
                    self.export(&adapter, &m, p);
                }
                match self.call_manager(&adapter, "RegisterBatteryProvider") {
                    Ok(()) => {
                        tracing::info!("registered battery provider on {}", adapter);
                        self.failures = 0;
                        self.reg = Reg::Registered { owner, adapter };
                    }
                    Err(e) => {
                        self.failures = self.failures.saturating_add(1);
                        self.retry_at = Instant::now() + retry_delay(self.failures);
                        if should_log_failure(self.failures) {
                            tracing::warn!(
                                "registration on {} failed (attempt {}), retry in {}s: {}",
                                adapter,
                                self.failures,
                                retry_delay(self.failures).as_secs(),
                                e
                            );
                        }
                    }
                }
            }
        }
    }

    fn observe(&self) -> Option<Observed> {
        let reply = self
            .conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &BLUEZ_SERVICE,
            )
            .ok()?;
        let owner: String = reply.body().deserialize().ok()?;
        let reply = self
            .conn
            .call_method(
                Some(BLUEZ_SERVICE),
                "/",
                Some("org.freedesktop.DBus.ObjectManager"),
                "GetManagedObjects",
                &(),
            )
            .ok()?;
        let objs: ManagedObjects = reply.body().deserialize().ok()?;
        let flat: Vec<(String, Vec<String>)> = objs
            .iter()
            .map(|(p, i)| (p.as_str().to_string(), i.keys().cloned().collect()))
            .collect();
        Some(Observed {
            owner,
            adapter: pick_adapter(&flat)?,
        })
    }

    fn call_manager(&self, adapter: &str, method: &str) -> zbus::Result<()> {
        let root = ObjectPath::try_from(PROVIDER_ROOT)?;
        match self.conn.call_method(
            Some(BLUEZ_SERVICE),
            adapter,
            Some(PROVIDER_MANAGER_IFACE),
            method,
            &root,
        ) {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("AlreadyExists") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Unregister from BlueZ and remove every exported object.
    fn shutdown(&mut self) {
        if let Reg::Registered { adapter, .. } = self.reg.clone() {
            let _ = self.call_manager(&adapter, "UnregisterBatteryProvider");
        }
        let macs: Vec<String> = self.exported.keys().cloned().collect();
        for m in macs {
            self.unexport(&m);
        }
        let _ = self
            .conn
            .object_server()
            .remove::<zbus::fdo::ObjectManager, _>(PROVIDER_ROOT);
        self.reg = Reg::Unregistered;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_normalised_to_upper() {
        assert_eq!(
            normalize_mac("aa:bb:0c:dd:ee:01").as_deref(),
            Some("AA:BB:0C:DD:EE:01")
        );
        assert_eq!(
            normalize_mac(" 04:db:56:CA:42:ee ").as_deref(),
            Some("04:DB:56:CA:42:EE")
        );
    }

    #[test]
    fn mac_rejects_non_macs() {
        for bad in [
            "",
            "unknown",
            "aa:bb:cc:dd:ee",
            "aa:bb:cc:dd:ee:ff:00",
            "aa:bb:cc:dd:ee:g1",
            "a:bb:cc:dd:ee:ff",
            "aa:bb:cc:dd:ee:ff/../x",
            "aa bb cc dd ee ff",
            "+a:bb:cc:dd:ee:ff",
        ] {
            assert_eq!(normalize_mac(bad), None, "{:?}", bad);
        }
    }

    #[test]
    fn paths_are_derived_from_normalised_mac() {
        let m = normalize_mac("04:db:56:ca:42:ee").unwrap();
        assert_eq!(
            child_path(&m),
            "/com/agenceapi/AppleKbMonitor/dev_04_DB_56_CA_42_EE"
        );
        assert_eq!(
            device_path("/org/bluez/hci1", &m),
            "/org/bluez/hci1/dev_04_DB_56_CA_42_EE"
        );
        assert_eq!(
            device_path("/org/bluez/hci0/", &m),
            "/org/bluez/hci0/dev_04_DB_56_CA_42_EE"
        );
        assert!(ObjectPath::try_from(child_path(&m)).is_ok());
    }

    fn obj(p: &str, i: &[&str]) -> (String, Vec<String>) {
        (p.to_string(), i.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn adapter_resolved_from_managed_objects() {
        let objs = vec![
            obj("/org/bluez", &["org.bluez.AgentManager1"]),
            obj("/org/bluez/hci1/dev_X", &["org.bluez.Device1"]),
            obj(
                "/org/bluez/hci1",
                &[PROVIDER_MANAGER_IFACE, "org.bluez.Adapter1"],
            ),
            obj("/org/bluez/hci0", &[PROVIDER_MANAGER_IFACE]),
        ];
        assert_eq!(pick_adapter(&objs).as_deref(), Some("/org/bluez/hci0"));
        assert_eq!(pick_adapter(&objs[..2]), None);
        assert_eq!(pick_adapter(&[]), None);
    }

    fn obs(o: &str, a: &str) -> Observed {
        Observed {
            owner: o.into(),
            adapter: a.into(),
        }
    }
    fn reg(o: &str, a: &str) -> Reg {
        Reg::Registered {
            owner: o.into(),
            adapter: a.into(),
        }
    }

    #[test]
    fn state_machine_registration() {
        // BlueZ absent: nothing to do.
        assert_eq!(decide(&Reg::Unregistered, None), Action::Nothing);
        // BlueZ appears: register.
        assert_eq!(
            decide(&Reg::Unregistered, Some(&obs(":1.5", "/org/bluez/hci0"))),
            Action::Register {
                adapter: "/org/bluez/hci0".into(),
                unregister_from: None
            }
        );
        // Stable: nothing.
        assert_eq!(
            decide(
                &reg(":1.5", "/org/bluez/hci0"),
                Some(&obs(":1.5", "/org/bluez/hci0"))
            ),
            Action::Nothing
        );
        // bluetoothd vanished.
        assert_eq!(
            decide(&reg(":1.5", "/org/bluez/hci0"), None),
            Action::Forget
        );
        // bluetoothd restarted (new owner): re-register, no unregister (old is dead).
        assert_eq!(
            decide(
                &reg(":1.5", "/org/bluez/hci0"),
                Some(&obs(":1.9", "/org/bluez/hci0"))
            ),
            Action::Register {
                adapter: "/org/bluez/hci0".into(),
                unregister_from: None
            }
        );
        // Adapter changed under the same bluetoothd: unregister from old one.
        assert_eq!(
            decide(
                &reg(":1.5", "/org/bluez/hci0"),
                Some(&obs(":1.5", "/org/bluez/hci1"))
            ),
            Action::Register {
                adapter: "/org/bluez/hci1".into(),
                unregister_from: Some("/org/bluez/hci0".into())
            }
        );
    }

    #[test]
    fn retry_backoff_and_log_throttle() {
        let d: Vec<u64> = (1..=9).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(d, vec![5, 10, 20, 40, 80, 160, 300, 300, 300]);
        let logged: Vec<u32> = (0..=9).filter(|n| should_log_failure(*n)).collect();
        assert_eq!(logged, vec![1, 2, 4, 8]);
    }

    #[test]
    fn start_rejects_invalid_mac_without_spawning() {
        assert!(BatteryProvider::start("unknown", 50).is_none());
    }

    /// Live test against the system bus (needs a running bluetoothd and a
    /// connected keyboard): `cargo test -- --ignored live_register --nocapture`.
    #[test]
    #[ignore]
    fn live_register() {
        let mac = std::env::var("KB_MAC").unwrap_or_else(|_| "04:DB:56:CA:42:EE".into());
        let p = BatteryProvider::start(&mac, 42).expect("start");
        thread::sleep(Duration::from_secs(8));
        p.set_battery(&mac, 43);
        thread::sleep(Duration::from_secs(2));
        drop(p);
    }
}
