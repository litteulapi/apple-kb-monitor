//! BlueZ Battery Provider — exposes Apple keyboard battery to the desktop.
//!
//! Registers as a BatteryProvider with BlueZ via D-Bus, exporting per-device
//! `org.bluez.BatteryProvider1` objects. BlueZ reads these and creates
//! `org.bluez.Battery1` on the device path, which UPower / KDE / GNOME
//! pick up natively in their battery widgets.
//!
//! Uses zbus 4 blocking API on a dedicated thread.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::thread;

use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Str};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const BLUEZ_SERVICE: &str = "org.bluez";
const PROVIDER_MANAGER_IFACE: &str = "org.bluez.BatteryProviderManager1";
const PROVIDER_ROOT: &str = "/com/agenceapi/AppleKbMonitor";

// ---------------------------------------------------------------------------
// org.bluez.BatteryProvider1 — per-device battery object
// ---------------------------------------------------------------------------

/// Per-device battery interface exported on the provider's child path.
/// BlueZ reads `Percentage`, `Device`, and `Source` to create a Battery1
/// proxy on the real device object path.
struct BatteryObject {
    device: String,
    source: String,
    pct: Arc<AtomicU8>,
}

#[interface(name = "org.bluez.BatteryProvider1")]
impl BatteryObject {
    #[zbus(property)]
    fn percentage(&self) -> u8 {
        self.pct.load(Ordering::Relaxed)
    }

    #[zbus(property)]
    fn device(&self) -> &str {
        &self.device
    }

    #[zbus(property)]
    fn source(&self) -> &str {
        &self.source
    }
}

// ---------------------------------------------------------------------------
// org.freedesktop.DBus.ObjectManager
// ---------------------------------------------------------------------------

/// ObjectManager that BlueZ calls to discover our battery objects.
/// Returns managed objects keyed by child path, each carrying the
/// BatteryProvider1 property dict.
struct ProviderObjectManager {
    child_path: String,
    device: String,
    source: String,
    pct: Arc<AtomicU8>,
}

/// Build the GetManagedObjects reply. Invalid paths yield an empty reply
/// instead of panicking inside the D-Bus handler.
fn build_managed_objects(child_path: &str, device: &str, source: &str, pct: u8) -> ManagedObjects {
    let mut result = ManagedObjects::new();
    let (Ok(path), Ok(dev_path)) = (
        OwnedObjectPath::try_from(child_path.to_string()),
        ObjectPath::try_from(device),
    ) else {
        eprintln!("[bluez] invalid object path, returning empty managed objects");
        return result;
    };

    let mut props: HashMap<String, OwnedValue> = HashMap::new();
    props.insert("Percentage".into(), pct.into());
    props.insert("Device".into(), dev_path.into());
    props.insert("Source".into(), Str::from(source).into());

    let mut ifaces = HashMap::new();
    ifaces.insert("org.bluez.BatteryProvider1".to_string(), props);
    result.insert(path, ifaces);
    result
}

/// Convert `aa:bb:cc:dd:ee:ff` into the BlueZ path segment `AA_BB_CC_DD_EE_FF`.
/// Returns `None` unless the MAC is exactly six colon-separated hex octets, so
/// the result is always a valid D-Bus object path component.
fn mac_to_path_segment(mac: &str) -> Option<String> {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6
        || !parts.iter().all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(parts.join("_").to_uppercase())
}

/// Return type for GetManagedObjects: path -> interface -> property -> value.
type ManagedObjects =
    HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

#[interface(name = "org.freedesktop.DBus.ObjectManager")]
impl ProviderObjectManager {
    fn get_managed_objects(&self) -> ManagedObjects {
        build_managed_objects(&self.child_path, &self.device, &self.source, self.pct.load(Ordering::Relaxed))
    }

    #[zbus(signal)]
    async fn interfaces_added(
        signal_ctxt: &zbus::object_server::SignalContext<'_>,
        object_path: ObjectPath<'_>,
        interfaces: HashMap<String, HashMap<String, zbus::zvariant::Value<'_>>>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn interfaces_removed(
        signal_ctxt: &zbus::object_server::SignalContext<'_>,
        object_path: ObjectPath<'_>,
        interfaces: Vec<String>,
    ) -> zbus::Result<()>;
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Handle to a running BlueZ BatteryProvider.
///
/// The D-Bus connection lives on a dedicated thread.
/// `update_percentage` is lock-free (atomic store) and safe to call
/// from any thread.
pub struct BatteryProvider {
    pct: Arc<AtomicU8>,
    // Keep the thread handle alive so the connection doesn't get dropped.
    _handle: thread::JoinHandle<()>,
}

impl BatteryProvider {
    /// Spawn a D-Bus thread that registers with BlueZ as a battery provider
    /// for the given MAC address. Returns `None` if the MAC is invalid.
    ///
    /// `initial_pct` should be the current battery reading (not 0).
    /// BlueZ caches the initial value; PropertiesChanged is not emitted.
    pub fn start(mac: &str, initial_pct: u8) -> Option<Self> {
        let mac_path = mac_to_path_segment(mac)?;

        let child_path = format!("{}/dev_{}", PROVIDER_ROOT, mac_path);
        let bluez_dev_path = format!("/org/bluez/hci0/dev_{}", mac_path);
        let source = "apihub-app (HID 0xEA)".to_string();

        let pct = Arc::new(AtomicU8::new(initial_pct));
        let pct_thread = pct.clone();

        let handle = match thread::Builder::new()
            .name("bluez-provider".into())
            .spawn(move || {
                run_provider(child_path, bluez_dev_path, source, pct_thread);
            }) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[bluez] cannot spawn provider thread: {}", e);
                return None;
            }
        };

        Some(Self {
            pct,
            _handle: handle,
        })
    }

    /// Update the exported battery percentage (0-100). Lock-free.
    pub fn update_percentage(&self, pct: u8) {
        self.pct.store(pct.min(100), Ordering::Relaxed);
    }
}

/// D-Bus event loop — runs on the dedicated thread, blocks forever.
fn run_provider(
    child_path: String,
    bluez_dev_path: String,
    source: String,
    pct: Arc<AtomicU8>,
) {
    // Build the battery object and ObjectManager.
    let pct_watch = pct.clone();
    let battery_obj = BatteryObject {
        device: bluez_dev_path.clone(),
        source: source.clone(),
        pct: pct.clone(),
    };

    let om = ProviderObjectManager {
        child_path: child_path.clone(),
        device: bluez_dev_path,
        source,
        pct,
    };

    let child_path_sig = child_path.clone();

    // Connect to system bus, register well-known name, export objects.
    let conn = match zbus::blocking::connection::Builder::system()
        .and_then(|b| b.name("com.agenceapi.AppleKbMonitor"))
        .and_then(|b| b.serve_at(&*child_path, battery_obj))
        .and_then(|b| b.serve_at(PROVIDER_ROOT, om))
        .and_then(|b| b.build())
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[bluez] D-Bus setup failed: {}", e);
            return;
        }
    };

    // Register with BlueZ BatteryProviderManager1 and keep it registered:
    // BlueZ may start after us or be restarted (it forgets providers then).
    // The same loop pushes percentage changes to BlueZ as PropertiesChanged —
    // BlueZ only caches the initial value otherwise.
    let mut registered_owner: Option<String> = None;
    let mut last_pct = pct_watch.load(Ordering::Relaxed);
    let mut tick: u32 = 0;
    loop {
        if tick % 5 == 0 {
            let owner = bluez_owner(&conn);
            if owner.is_none() {
                registered_owner = None;
            } else if owner != registered_owner {
                match register_provider(&conn) {
                    Ok(()) => {
                        eprintln!("[bluez] registered battery provider at {}", PROVIDER_ROOT);
                        registered_owner = owner;
                    }
                    Err(e) => eprintln!(
                        "[bluez] registration failed (BlueZ may lack BatteryProvider support), will retry: {}", e
                    ),
                }
            }
        }
        tick = tick.wrapping_add(1);

        let cur = pct_watch.load(Ordering::Relaxed);
        if cur != last_pct {
            match emit_percentage_changed(&conn, &child_path_sig) {
                Ok(()) => last_pct = cur,
                Err(e) => eprintln!("[bluez] PropertiesChanged failed: {}", e),
            }
        }
        thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// Unique bus name currently owning `org.bluez`, if BlueZ is running.
fn bluez_owner(conn: &Connection) -> Option<String> {
    let reply = conn
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetNameOwner",
            &BLUEZ_SERVICE,
        )
        .ok()?;
    reply.body().deserialize::<String>().ok()
}

/// Emit org.freedesktop.DBus.Properties.PropertiesChanged for `Percentage`.
fn emit_percentage_changed(conn: &Connection, child_path: &str) -> zbus::Result<()> {
    let iface_ref = conn
        .object_server()
        .interface::<_, BatteryObject>(child_path.to_string())?;
    let iface = iface_ref.get();
    zbus::block_on(iface.percentage_changed(iface_ref.signal_context()))
}

/// Call RegisterBatteryProvider on BlueZ.
fn register_provider(conn: &Connection) -> zbus::Result<()> {
    let provider_path = ObjectPath::try_from(PROVIDER_ROOT)?;
    conn.call_method(
        Some(BLUEZ_SERVICE),
        "/org/bluez/hci0",
        Some(PROVIDER_MANAGER_IFACE),
        "RegisterBatteryProvider",
        &provider_path,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_segment_valid() {
        assert_eq!(mac_to_path_segment("aa:bb:0c:dd:ee:01").as_deref(), Some("AA_BB_0C_DD_EE_01"));
    }

    #[test]
    fn mac_segment_rejects_non_macs() {
        for bad in ["", "unknown", "aa:bb:cc:dd:ee", "aa:bb:cc:dd:ee:ff:00", "aa:bb:cc:dd:ee:g1",
                    "a:bb:cc:dd:ee:ff", "aa:bb:cc:dd:ee:ff/../x", "aa bb cc dd ee ff", "+a:bb:cc:dd:ee:ff"] {
            assert_eq!(mac_to_path_segment(bad), None, "{:?}", bad);
        }
    }

    #[test]
    fn managed_objects_shape() {
        let m = build_managed_objects(
            "/com/agenceapi/AppleKbMonitor/dev_AA_BB_CC_DD_EE_FF",
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF",
            "src", 77,
        );
        assert_eq!(m.len(), 1);
        let ifaces = m.values().next().unwrap();
        let props = &ifaces["org.bluez.BatteryProvider1"];
        assert!(props.contains_key("Percentage") && props.contains_key("Device") && props.contains_key("Source"));
    }

    #[test]
    fn managed_objects_invalid_path_does_not_panic() {
        assert!(build_managed_objects("not a path", "/org/bluez/hci0", "s", 1).is_empty());
        assert!(build_managed_objects("/ok", "bad device", "s", 1).is_empty());
    }

    #[test]
    fn start_rejects_invalid_mac_without_spawning() {
        assert!(BatteryProvider::start("unknown", 50).is_none());
    }
}
