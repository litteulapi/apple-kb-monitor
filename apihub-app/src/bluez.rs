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

/// Return type for GetManagedObjects: path -> interface -> property -> value.
type ManagedObjects =
    HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

#[interface(name = "org.freedesktop.DBus.ObjectManager")]
impl ProviderObjectManager {
    fn get_managed_objects(&self) -> ManagedObjects {
        let path = OwnedObjectPath::try_from(self.child_path.clone())
            .expect("invalid child object path");

        let dev_path = ObjectPath::try_from(self.device.as_str())
            .expect("invalid device path");

        let mut props: HashMap<String, OwnedValue> = HashMap::new();
        props.insert("Percentage".into(), self.pct.load(Ordering::Relaxed).into());
        props.insert("Device".into(), dev_path.into());
        props.insert("Source".into(), Str::from(self.source.as_str()).into());

        let mut ifaces = HashMap::new();
        ifaces.insert("org.bluez.BatteryProvider1".to_string(), props);

        let mut result = ManagedObjects::new();
        result.insert(path, ifaces);
        result
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
    /// The initial percentage is 0; call `update_percentage` once you have
    /// a real reading.
    pub fn start(mac: &str) -> Option<Self> {
        let mac_path = mac.to_uppercase().replace(':', "_");
        if mac_path.is_empty() {
            return None;
        }

        let child_path = format!("{}/dev_{}", PROVIDER_ROOT, mac_path);
        let bluez_dev_path = format!("/org/bluez/hci0/dev_{}", mac_path);
        let source = "apihub-app (HID 0xEA)".to_string();

        let pct = Arc::new(AtomicU8::new(0));
        let pct_thread = pct.clone();

        let handle = thread::Builder::new()
            .name("bluez-provider".into())
            .spawn(move || {
                run_provider(child_path, bluez_dev_path, source, pct_thread);
            })
            .expect("failed to spawn bluez-provider thread");

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

    // Register with BlueZ BatteryProviderManager1.
    match register_provider(&conn) {
        Ok(()) => eprintln!(
            "[bluez] registered battery provider at {}",
            PROVIDER_ROOT
        ),
        Err(e) => eprintln!("[bluez] registration failed (BlueZ may lack BatteryProvider support): {}", e),
    }

    // Park the thread — the connection must stay alive for BlueZ to
    // query our objects via D-Bus.
    loop {
        thread::park();
    }
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
