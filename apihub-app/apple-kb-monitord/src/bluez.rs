//! `BlueZ` Battery Provider — exposes Apple keyboard battery to `BlueZ` clients.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

use crate::origin::Origin;

const BLUEZ_SERVICE: &str = "org.bluez";
const PROVIDER_MANAGER_IFACE: &str = "org.bluez.BatteryProviderManager1";
const PROVIDER_ROOT: &str = "/com/agenceapi/AppleKbMonitor";
/// Who provides the level; not where it was read (kernel `power_supply`, HID `0x47` or an estimate).
const SOURCE: &str = "apple-kb-monitord";
const RETRY_BASE: Duration = Duration::from_secs(5);

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

fn mac_segment(norm_mac: &str) -> String {
    norm_mac.replace(':', "_")
}

fn child_path(norm_mac: &str) -> String {
    format!("{}/dev_{}", PROVIDER_ROOT, mac_segment(norm_mac))
}

fn device_path(adapter: &str, norm_mac: &str) -> String {
    format!(
        "{}/dev_{}",
        adapter.trim_end_matches('/'),
        mac_segment(norm_mac)
    )
}

/// The adapter a wanted keyboard is known on, else the first one that takes battery providers.
fn pick_adapter(objects: &[(String, Vec<String>)], macs: &[String]) -> Option<String> {
    let mut managers: Vec<&String> = objects
        .iter()
        .filter(|(_, ifaces)| ifaces.iter().any(|i| i == PROVIDER_MANAGER_IFACE))
        .map(|(p, _)| p)
        .collect();
    managers.sort();
    let known = |a: &str| {
        macs.iter()
            .any(|m| objects.iter().any(|(p, _)| *p == device_path(a, m)))
    };
    managers
        .iter()
        .find(|a| known(a))
        .or_else(|| managers.first())
        .map(|a| (*a).clone())
}

fn retry_delay(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(6);
    RETRY_BASE
        .saturating_mul(1u32 << shift)
        .min(Duration::from_mins(5))
}

fn should_log_failure(failures: u32) -> bool {
    failures > 0 && failures.is_power_of_two()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Reg {
    Unregistered,
    Registered { owner: String, adapter: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Observed {
    owner: String,
    adapter: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    Nothing,
    /// `BlueZ` gone: forget the registration (objects stay exported).
    Forget,
    /// (Re-)register on `adapter`.
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

struct Bat {
    device: OwnedObjectPath,
    pct: u8,
}

#[interface(name = "org.bluez.BatteryProvider1")]
#[allow(clippy::unused_self)] // zbus interface: D-Bus methods take `&self` and owned arguments
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

enum Cmd {
    Set(String, u8),
    Remove(String),
    Stop,
    /// `BlueZ` appeared, left or changed its objects (connection generation).
    Recheck(u64),
    /// The bus connection of that generation ended.
    BusLost(u64),
}

/// Handle to the `BlueZ` battery provider thread.
pub struct BatteryProvider {
    tx: Sender<Cmd>,
    handle: Option<thread::JoinHandle<()>>,
}

impl BatteryProvider {
    /// Spawn the provider thread with no keyboard yet.
    pub fn spawn() -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let events = tx.clone();
        let handle = match thread::Builder::new()
            .name("bluez-provider".into())
            .spawn(move || run_provider(&rx, &events))
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
        })
    }

    #[cfg(test)]
    /// Spawn the provider and publish `initial_pct` for `mac`.
    #[must_use]
    pub fn start(mac: &str, initial_pct: u8) -> Option<Self> {
        let norm = normalize_mac(mac)?;
        let p = Self::spawn()?;
        p.set_battery(&norm, initial_pct);
        Some(p)
    }

    /// Publish / update the battery percentage (0-100) of a keyboard.
    pub fn set_battery(&self, mac: &str, percent: u8) {
        if let Some(m) = normalize_mac(mac) {
            let _ = self.tx.send(Cmd::Set(m, percent.min(100)));
        }
    }

    /// Withdraw a keyboard (e.g. on disconnection).
    pub fn remove(&self, mac: &str) {
        if let Some(m) = normalize_mac(mac) {
            let _ = self.tx.send(Cmd::Remove(m));
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

struct Worker {
    conn: Connection,
    desired: BTreeMap<String, u8>,
    exported: BTreeSet<String>,
    reg: Reg,
    failures: u32,
    retry_at: Instant,
}

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

fn is_already_exists(e: &zbus::Error) -> bool {
    matches!(e, zbus::Error::MethodError(name, _, _) if name.as_str() == "org.bluez.Error.AlreadyExists")
}

fn connect_system() -> zbus::Result<Connection> {
    zbus::blocking::connection::Builder::system()
        .and_then(|b| b.serve_at(PROVIDER_ROOT, zbus::fdo::ObjectManager))
        .and_then(zbus::blocking::ConnectionBuilder::build)
}

enum Exit {
    Stop,
    /// The bus connection died (dbus-daemon / dbus-broker restarted).
    ConnectionLost,
}

fn run_provider(rx: &mpsc::Receiver<Cmd>, events: &Sender<Cmd>) {
    run_provider_with(rx, events, connect_system, retry_delay);
}

/// The provider thread never ends before `Stop`.
fn run_provider_with(
    rx: &mpsc::Receiver<Cmd>,
    events: &Sender<Cmd>,
    mut connect: impl FnMut() -> zbus::Result<Connection>,
    backoff: impl Fn(u32) -> Duration,
) {
    let mut desired: BTreeMap<String, u8> = BTreeMap::new();
    let mut failures = 0u32;
    let mut generation = 0u64;
    loop {
        generation += 1;
        match connect().and_then(|c| watch_bluez(&c, events, generation).map(|()| c)) {
            Ok(conn) => {
                failures = 0;
                let (exit, d) = serve(conn, std::mem::take(&mut desired), rx, generation);
                desired = d;
                match exit {
                    Exit::Stop => return,
                    Exit::ConnectionLost => {
                        tracing::warn!("system bus connection lost, reconnecting");
                    }
                }
            }
            Err(e) => {
                failures = failures.saturating_add(1);
                if should_log_failure(failures) {
                    tracing::warn!("D-Bus setup failed (attempt {failures}): {e}");
                }
            }
        }
        // Wait before the next attempt, still taking commands.
        let until = Instant::now() + backoff(failures.max(1));
        while Instant::now() < until {
            match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                Ok(Cmd::Set(mac, pct)) => {
                    desired.insert(mac, pct);
                }
                Ok(Cmd::Remove(mac)) => {
                    desired.remove(&mac);
                }
                Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(Cmd::Recheck(_) | Cmd::BusLost(_)) | Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

/// Forward the bus events that can change the registration (`BlueZ` owner, its adapters) as
/// [`Cmd::Recheck`], and the end of the connection as [`Cmd::BusLost`]: no polling.
fn watch_bluez(conn: &Connection, events: &Sender<Cmd>, generation: u64) -> zbus::Result<()> {
    use zbus::message::Type;
    use zbus::MatchRule;
    let owner = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, BLUEZ_SERVICE)?
        .build();
    // BlueZ's ObjectManager is at `/`: filter on the sender; the object path is checked below.
    let objects = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(BLUEZ_SERVICE)?
        .interface("org.freedesktop.DBus.ObjectManager")?
        .build();
    for rule in [owner, objects] {
        let it = zbus::blocking::MessageIterator::for_match_rule(rule, conn, None)?;
        let tx = events.clone();
        let mut origin = Origin::new(conn, &[BLUEZ_SERVICE]);
        thread::Builder::new()
            .name("bluez-watch".into())
            .spawn(move || {
                for m in it {
                    let Ok(m) = m else { break };
                    if origin.accept(&m)
                        && is_bluez_event(&m)
                        && tx.send(Cmd::Recheck(generation)).is_err()
                    {
                        return;
                    }
                }
                let _ = tx.send(Cmd::BusLost(generation));
            })
            .map_err(|e| zbus::Error::Failure(format!("cannot spawn the BlueZ watcher: {e}")))?;
    }
    Ok(())
}

/// `NameOwnerChanged(org.bluez)` (already filtered by the rule), or an object of `BlueZ` added or
/// removed (not our own provider objects).
fn is_bluez_event(m: &zbus::Message) -> bool {
    let (h, body) = (m.header(), m.body());
    let path = match h.member().map(zbus::names::MemberName::as_str) {
        Some("NameOwnerChanged") => return true,
        Some("InterfacesAdded") => body
            .deserialize::<(
                OwnedObjectPath,
                HashMap<String, HashMap<String, OwnedValue>>,
            )>()
            .map(|(p, _)| p),
        _ => body
            .deserialize::<(OwnedObjectPath, Vec<String>)>()
            .map(|(p, _)| p),
    };
    path.is_ok_and(|p| p.as_str().starts_with("/org/bluez"))
}

fn serve(
    conn: Connection,
    desired: BTreeMap<String, u8>,
    rx: &mpsc::Receiver<Cmd>,
    generation: u64,
) -> (Exit, BTreeMap<String, u8>) {
    let mut w = Worker {
        conn,
        desired,
        exported: BTreeSet::new(),
        reg: Reg::Unregistered,
        failures: 0,
        retry_at: Instant::now(),
    };

    w.watch();
    let mut pending = None;
    let exit = loop {
        // Blocks until a command or a BlueZ event; a failed registration retries at `retry_at`.
        let cmd = if let Some(c) = pending.take() {
            Ok(c)
        } else if w.failures > 0 {
            rx.recv_timeout(w.retry_at.saturating_duration_since(Instant::now()))
        } else {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match cmd {
            Ok(Cmd::Set(mac, pct)) => w.apply_set(&mac, pct),
            Ok(Cmd::Remove(mac)) => w.apply_remove(&mac),
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break Exit::Stop,
            Ok(Cmd::BusLost(g)) if g == generation => break Exit::ConnectionLost,
            Ok(Cmd::Recheck(g)) if g == generation => {
                // One re-read for a burst of BlueZ events; the first other command waits its turn.
                pending = rx.try_iter().find(|c| !matches!(c, Cmd::Recheck(_)));
                w.watch();
            }
            Ok(Cmd::Recheck(_) | Cmd::BusLost(_)) => {}
            Err(RecvTimeoutError::Timeout) => w.watch(),
        }
    };
    if matches!(exit, Exit::Stop) {
        w.shutdown();
    }
    // Ends the watcher threads of this connection.
    let _ = w.conn.clone().close();
    (exit, std::mem::take(&mut w.desired))
}

impl Worker {
    fn adapter(&self) -> Option<&str> {
        match &self.reg {
            Reg::Registered { adapter, .. } => Some(adapter),
            Reg::Unregistered => None,
        }
    }

    fn apply_set(&mut self, mac: &str, pct: u8) {
        if self.desired.insert(mac.to_string(), pct).is_none() {
            self.watch(); // the keyboard may be on another adapter
        }
        let Some(adapter) = self.adapter().map(str::to_string) else {
            return; // exported at registration time
        };
        if self.exported.contains(mac) {
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
                self.exported.insert(mac.to_string());
            }
            Err(e) => tracing::warn!("export {} failed: {}", path, e),
        }
    }

    fn unexport(&mut self, mac: &str) {
        if self.exported.remove(mac) {
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
                let macs: Vec<String> = self.exported.iter().cloned().collect();
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
            adapter: pick_adapter(&flat, &self.desired.keys().cloned().collect::<Vec<_>>())?,
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
            Err(e) if is_already_exists(&e) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn shutdown(&mut self) {
        if let Reg::Registered { adapter, .. } = self.reg.clone() {
            let _ = self.call_manager(&adapter, "UnregisterBatteryProvider");
        }
        let macs: Vec<String> = self.exported.iter().cloned().collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn connect_to(addr: &str) -> zbus::Result<Connection> {
        zbus::blocking::connection::Builder::address(addr)
            .and_then(|b| b.serve_at(PROVIDER_ROOT, zbus::fdo::ObjectManager))
            .and_then(zbus::blocking::ConnectionBuilder::build)
    }

    #[test]
    fn already_exists_is_matched_by_error_name() {
        let msg = zbus::message::Message::method("/", "x")
            .unwrap()
            .build(&())
            .unwrap();
        let err = |name: &str, text: &str| {
            zbus::Error::MethodError(
                zbus::names::OwnedErrorName::try_from(name).unwrap(),
                Some(text.into()),
                msg.clone(),
            )
        };
        assert!(is_already_exists(&err("org.bluez.Error.AlreadyExists", "")));
        assert!(!is_already_exists(&err(
            "org.bluez.Error.Failed",
            "AlreadyExists"
        )));
    }

    #[test]
    fn provider_thread_retries_after_failed_setup() {
        // a failed initialisation must not end the thread.
        let (tx, rx) = mpsc::channel();
        let events = tx.clone();
        let tries = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let t = tries.clone();
        let h = thread::spawn(move || {
            run_provider_with(
                &rx,
                &events,
                move || {
                    t.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Err(zbus::Error::Failure("no bus".into()))
                },
                |_| Duration::from_millis(50),
            );
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while tries.load(std::sync::atomic::Ordering::SeqCst) < 3 {
            assert!(
                Instant::now() < deadline,
                "fewer than 3 set-up attempts in 10 s"
            );
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!h.is_finished(), "thread ended after a failed set-up");
        tx.send(Cmd::Stop).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn provider_reconnects_after_the_bus_restarts() {
        let Some(mut a) = crate::testbus::private_bus() else {
            eprintln!("SKIP: dbus-daemon not installed");
            return;
        };
        let Some(b) = crate::testbus::private_bus() else {
            return;
        };
        let (addr_a, addr_b) = (a.addr.clone(), b.addr.clone());
        let (tx, rx) = mpsc::channel();
        let events = tx.clone();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = calls.clone();
        let h = thread::spawn(move || {
            run_provider_with(
                &rx,
                &events,
                move || {
                    let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    connect_to(if n == 0 { &addr_a } else { &addr_b })
                },
                |_| Duration::from_millis(50),
            );
        });
        tx.send(Cmd::Set("AA:BB:CC:DD:EE:F1".into(), 50)).unwrap();
        thread::sleep(Duration::from_millis(800));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        a.kill();
        let t0 = Instant::now();
        while calls.load(std::sync::atomic::Ordering::SeqCst) < 2
            && t0.elapsed() < Duration::from_secs(20)
        {
            thread::sleep(Duration::from_millis(100));
        }
        assert!(
            calls.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "provider never reconnected"
        );
        assert!(!h.is_finished());
        tx.send(Cmd::Stop).unwrap();
        h.join().unwrap();
        drop(b);
    }

    struct FakeObjects(std::sync::Arc<std::sync::atomic::AtomicU32>);

    #[interface(name = "org.freedesktop.DBus.ObjectManager")]
    impl FakeObjects {
        fn get_managed_objects(&self) -> ManagedObjects {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let adapter = OwnedObjectPath::try_from("/org/bluez/hci0").unwrap();
            HashMap::from([(
                adapter,
                HashMap::from([(PROVIDER_MANAGER_IFACE.into(), HashMap::new())]),
            )])
        }
    }

    struct FakeManager(std::sync::Arc<std::sync::atomic::AtomicU32>);

    #[interface(name = "org.bluez.BatteryProviderManager1")]
    #[allow(clippy::unused_self)] // zbus interface: D-Bus methods take `&self`
    impl FakeManager {
        fn register_battery_provider(&self, root: OwnedObjectPath) {
            assert_eq!(root.into_inner().as_str(), PROVIDER_ROOT);
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        fn unregister_battery_provider(&self, root: OwnedObjectPath) {
            assert_eq!(root.into_inner().as_str(), PROVIDER_ROOT);
        }
    }

    #[test]
    fn bluez_arrival_is_an_event_not_a_poll() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        use std::sync::Arc;
        let Some(bus) = crate::testbus::private_bus() else {
            eprintln!("SKIP: dbus-daemon not installed");
            return;
        };
        let addr = bus.addr.clone();
        let (tx, rx) = mpsc::channel();
        let events = tx.clone();
        let h = thread::spawn(move || {
            run_provider_with(
                &rx,
                &events,
                move || connect_to(&addr),
                |_| Duration::from_secs(1),
            );
        });
        thread::sleep(Duration::from_millis(300));
        let (lookups, regs) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
        let bluez = zbus::blocking::connection::Builder::address(bus.addr.as_str())
            .and_then(|b| b.name(BLUEZ_SERVICE))
            .and_then(|b| b.serve_at("/", FakeObjects(lookups.clone())))
            .and_then(|b| b.serve_at("/org/bluez/hci0", FakeManager(regs.clone())))
            .and_then(zbus::blocking::ConnectionBuilder::build)
            .unwrap();
        let t0 = Instant::now();
        while regs.load(SeqCst) == 0 {
            assert!(
                t0.elapsed() < Duration::from_secs(3),
                "BlueZ arrival not noticed"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let seen = lookups.load(SeqCst);
        // Another peer's ObjectManager signal about a /org/bluez path is not BlueZ's.
        let stranger = connect_to(&bus.addr).unwrap();
        let added = |c: &Connection| {
            let path = OwnedObjectPath::try_from("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").unwrap();
            let ifaces: HashMap<String, HashMap<String, OwnedValue>> = HashMap::new();
            c.emit_signal(
                None::<()>,
                "/",
                "org.freedesktop.DBus.ObjectManager",
                "InterfacesAdded",
                &(path, ifaces),
            )
            .unwrap();
        };
        added(&stranger);
        thread::sleep(Duration::from_millis(300));
        assert_eq!(lookups.load(SeqCst), seen, "foreign signal re-read BlueZ");
        added(&bluez);
        let t0 = Instant::now();
        while lookups.load(SeqCst) == seen {
            assert!(
                t0.elapsed() < Duration::from_secs(3),
                "BlueZ signal ignored"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let seen = lookups.load(SeqCst);
        // The former loop re-read every BlueZ object each 5 s.
        thread::sleep(Duration::from_millis(5500));
        assert_eq!(lookups.load(SeqCst), seen, "BlueZ objects polled");
        assert_eq!(regs.load(SeqCst), 1);
        tx.send(Cmd::Stop).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn mac_normalised_to_upper() {
        assert_eq!(
            normalize_mac("aa:bb:0c:dd:ee:01").as_deref(),
            Some("AA:BB:0C:DD:EE:01")
        );
        assert_eq!(
            normalize_mac(" AA:BB:CC:DD:EE:F1 ").as_deref(),
            Some("AA:BB:CC:DD:EE:F1")
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
            assert_eq!(normalize_mac(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn paths_are_derived_from_normalised_mac() {
        let m = normalize_mac("aa:bb:cc:dd:ee:f1").unwrap();
        assert_eq!(
            child_path(&m),
            "/com/agenceapi/AppleKbMonitor/dev_AA_BB_CC_DD_EE_F1"
        );
        assert_eq!(
            device_path("/org/bluez/hci1", &m),
            "/org/bluez/hci1/dev_AA_BB_CC_DD_EE_F1"
        );
        assert_eq!(
            device_path("/org/bluez/hci0/", &m),
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_F1"
        );
        assert!(ObjectPath::try_from(child_path(&m)).is_ok());
    }

    fn obj(p: &str, i: &[&str]) -> (String, Vec<String>) {
        (
            p.to_string(),
            i.iter().map(std::string::ToString::to_string).collect(),
        )
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
        assert_eq!(pick_adapter(&objs, &[]).as_deref(), Some("/org/bluez/hci0"));
        assert!(
            !SOURCE.contains("power_supply"),
            "the level may come from HID 0x47"
        );
        assert_eq!(pick_adapter(&objs[..2], &[]), None);
        assert_eq!(pick_adapter(&[], &[]), None);
        let kb = normalize_mac("aa:bb:cc:dd:ee:f1").unwrap();
        let mut objs = objs;
        objs.push(obj(
            "/org/bluez/hci1/dev_AA_BB_CC_DD_EE_F1",
            &["org.bluez.Device1"],
        ));
        assert_eq!(
            pick_adapter(&objs, &[kb]).as_deref(),
            Some("/org/bluez/hci1"),
            "the keyboard's adapter"
        );
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
        assert_eq!(decide(&Reg::Unregistered, None), Action::Nothing);
        assert_eq!(
            decide(&Reg::Unregistered, Some(&obs(":1.5", "/org/bluez/hci0"))),
            Action::Register {
                adapter: "/org/bluez/hci0".into(),
                unregister_from: None
            }
        );
        assert_eq!(
            decide(
                &reg(":1.5", "/org/bluez/hci0"),
                Some(&obs(":1.5", "/org/bluez/hci0"))
            ),
            Action::Nothing
        );
        assert_eq!(
            decide(&reg(":1.5", "/org/bluez/hci0"), None),
            Action::Forget
        );
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

    #[test]
    #[ignore = "needs a real BlueZ adapter (KB_MAC)"]
    fn live_register() {
        let mac = std::env::var("KB_MAC").unwrap_or_else(|_| "AA:BB:CC:DD:EE:F1".into());
        let p = BatteryProvider::start(&mac, 42).expect("start");
        thread::sleep(Duration::from_secs(8));
        p.set_battery(&mac, 43);
        thread::sleep(Duration::from_secs(2));
        drop(p);
    }
}
