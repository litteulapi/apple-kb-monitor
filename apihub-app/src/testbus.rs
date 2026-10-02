//! Test support: a private session bus and a fake `apple-kb-monitord` (root
//! object + one keyboard object), so that the thin clients of this crate are
//! tested without the real daemon, sysfs or a keyboard.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use apple_kb_monitord::devices::device_path;
use apple_kb_monitord::service::{BUS_NAME, OBJECT_PATH};
use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::OwnedObjectPath;

/// Anonymised keyboard address used by every fixture of the repository.
pub const MAC: &str = "AA:BB:CC:DD:EE:F1";

/// Bus configuration WITHOUT any service directory: the stock session
/// configuration would D-Bus-activate the installed `apple-kb-monitord` (a
/// real daemon, reading the real keyboard) as soon as a test calls its name.
const BUS_CONFIG: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=@TMP@</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;

/// A `dbus-daemon` nobody else talks to and that can start nothing; killed
/// on drop, also when the test panics.
pub struct PrivateBus {
    child: Child,
    pub addr: String,
    config: std::path::PathBuf,
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.config);
    }
}

/// `None` when `dbus-daemon` is not installed (the test is skipped).
pub fn private_bus() -> Option<PrivateBus> {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let tmp = std::env::temp_dir();
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let config = tmp.join(format!("apihub-testbus-{}-{n}.conf", std::process::id()));
    std::fs::write(&config, BUS_CONFIG.replace("@TMP@", &tmp.to_string_lossy())).ok()?;
    let spawned = Command::new("dbus-daemon")
        .arg(format!("--config-file={}", config.display()))
        .args(["--nofork", "--print-address"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else {
        let _ = std::fs::remove_file(&config);
        return None;
    };
    let mut addr = String::new();
    let read = child
        .stdout
        .take()
        .map(|o| BufReader::new(o).read_line(&mut addr));
    let bus = PrivateBus {
        child,
        addr: addr.trim().to_string(),
        config,
    };
    matches!(read, Some(Ok(n)) if n > 0).then_some(bus)
}

pub fn connect(addr: &str) -> zbus::Result<Connection> {
    zbus::blocking::connection::Builder::address(addr).and_then(|b| b.build())
}

#[derive(Default)]
struct State {
    mode: i32,
    set_calls: Vec<i32>,
    refuse: bool,
    connected: bool,
    json: String,
    reconnects: u32,
}

type Shared = Arc<Mutex<State>>;

struct Root(Shared);

#[interface(name = "com.agenceapi.AppleKbMonitor1")]
impl Root {
    fn get_devices(&self) -> Vec<OwnedObjectPath> {
        device_path(MAC).into_iter().collect()
    }

    fn get_state(&self) -> String {
        self.0.lock().unwrap().json.clone()
    }

    #[zbus(property)]
    fn json(&self) -> String {
        self.0.lock().unwrap().json.clone()
    }
}

struct Device(Shared);

#[interface(name = "com.agenceapi.AppleKbMonitor1.Device")]
impl Device {
    #[zbus(property)]
    fn fn_mode(&self) -> i32 {
        self.0.lock().unwrap().mode
    }

    #[zbus(property)]
    fn connected(&self) -> bool {
        self.0.lock().unwrap().connected
    }

    fn set_fn_mode(&self, mode: i32) -> zbus::fdo::Result<()> {
        let mut s = self.0.lock().unwrap();
        if s.refuse {
            return Err(zbus::fdo::Error::AccessDenied(
                "authentication dismissed".into(),
            ));
        }
        s.set_calls.push(mode);
        s.mode = mode;
        Ok(())
    }
}

struct Link(Shared);

#[interface(name = "com.agenceapi.AppleKbMonitor1.Link")]
impl Link {
    fn reconnect(&self) -> bool {
        let mut s = self.0.lock().unwrap();
        s.reconnects += 1;
        !s.refuse
    }
}

/// Fake daemon on its own private bus; everything is torn down on drop.
pub struct FakeDaemon {
    bus: PrivateBus,
    state: Shared,
    _server: Connection,
}

impl FakeDaemon {
    /// `None` (test skipped) when `dbus-daemon` is missing.
    pub fn start(fn_mode: i32) -> Option<Self> {
        let bus = private_bus()?;
        let state: Shared = Arc::new(Mutex::new(State {
            mode: fn_mode,
            connected: true,
            json: "{}".into(),
            ..State::default()
        }));
        let device = device_path(MAC)?;
        let server = zbus::blocking::connection::Builder::address(bus.addr.as_str())
            .and_then(|b| b.serve_at(OBJECT_PATH, Root(state.clone())))
            .and_then(|b| b.serve_at(device, Device(state.clone())))
            .and_then(|b| b.serve_at(apple_kb_monitord::repair::LINK_PATH, Link(state.clone())))
            .and_then(|b| b.name(BUS_NAME))
            .and_then(|b| b.build())
            .expect("fake daemon on the private bus");
        Some(Self {
            bus,
            state,
            _server: server,
        })
    }

    pub fn client(&self) -> Connection {
        connect(&self.bus.addr).expect("client connection to the private bus")
    }

    pub fn set_calls(&self) -> Vec<i32> {
        self.state.lock().unwrap().set_calls.clone()
    }

    /// Calls of `Link.Reconnect` received so far.
    pub fn reconnects(&self) -> u32 {
        self.state.lock().unwrap().reconnects
    }

    pub fn force_mode(&self, mode: i32) {
        self.state.lock().unwrap().mode = mode;
    }

    pub fn refuse(&self, refuse: bool) {
        self.state.lock().unwrap().refuse = refuse;
    }

    /// The JSON served by `GetState` and the `Json` property.
    pub fn set_json(&self, json: &str) {
        self.state.lock().unwrap().json = json.to_string();
    }
}
