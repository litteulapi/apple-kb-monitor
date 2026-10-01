//! Smoke test: the window really opens under a virtual X server, survives a
//! few seconds without panicking, and a second launch only raises it (#195,
//! #197).
//!
//! Never touches a keyboard: the app runs in a user+mount namespace where
//! `/sys/class/hidraw` is empty and every `/dev/hidraw*` is `/dev/null`, on a
//! private session bus without activatable services. Skipped (with a
//! message) when Xvfb, xdotool, dbus-run-session or unprivileged user
//! namespaces are missing.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const SANDBOX: &str = r#"
mount -t tmpfs none /sys/class/hidraw || exit 90
for f in /dev/hidraw*; do [ -e "$f" ] && { mount --bind /dev/null "$f" || exit 91; }; done
exec "$@""#;

fn have(bin: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {bin}")])
        .stdout(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn userns_ok() -> bool {
    Command::new("unshare")
        .args(["-rm", "bash", "-c", SANDBOX, "sandbox", "true"])
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_xvfb() -> Option<(Kill, String)> {
    let mut child = Command::new("Xvfb")
        .args(["-displayfd", "1", "-screen", "0", "1024x768x24", "-nolisten", "tcp"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        match out.read(&mut byte) {
            Ok(1) if byte[0] == b'\n' => break,
            Ok(1) => buf.push(byte[0]),
            _ => break,
        }
    }
    let n = String::from_utf8(buf).ok()?.trim().to_string();
    if n.is_empty() {
        let _ = child.kill();
        return None;
    }
    Some((Kill(child), format!(":{n}")))
}

/// Private session bus with no activatable service: neither the installed
/// daemon nor a desktop portal can be started behind the test's back.
const BUS_CONF: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy>
</busconfig>"#;

/// Graceful close, as a window manager's close button: WM_DELETE_WINDOW.
const XCLOSE: &str = r#"
import ctypes, sys
x = ctypes.cdll.LoadLibrary("libX11.so.6")
x.XOpenDisplay.restype = ctypes.c_void_p
x.XInternAtom.restype = ctypes.c_ulong
x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
x.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]
x.XFlush.argtypes = [ctypes.c_void_p]
class M(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int),
                ("display", ctypes.c_void_p), ("window", ctypes.c_ulong), ("message_type", ctypes.c_ulong),
                ("format", ctypes.c_int), ("l", ctypes.c_long * 5)]
class E(ctypes.Union):
    _fields_ = [("xclient", M), ("pad", ctypes.c_long * 24)]
d = x.XOpenDisplay(None); w = int(sys.argv[1]); e = E()
e.xclient.type = 33; e.xclient.window = w; e.xclient.format = 32
e.xclient.message_type = x.XInternAtom(d, b"WM_PROTOCOLS", 0)
e.xclient.l[0] = x.XInternAtom(d, b"WM_DELETE_WINDOW", 0)
x.XSendEvent(d, w, 0, 0, ctypes.byref(e)); x.XFlush(d)
"#;

/// Runs `script` (bash) in the hidraw-less sandbox, on a private bus, with
/// `$APP`, `$OUT` (a fresh directory) and `$XCLOSE` set. Returns `$OUT`
/// files as a lookup closure, or `None` when the environment cannot run it.
fn run_sandboxed(tag: &str, script: &str) -> Option<impl Fn(&str) -> String> {
    for bin in ["Xvfb", "xdotool", "dbus-run-session", "unshare", "python3"] {
        if !have(bin) {
            eprintln!("skipped: {bin} not installed");
            return None;
        }
    }
    if !userns_ok() {
        eprintln!("skipped: unprivileged user namespaces unavailable");
        return None;
    }
    let Some((_xvfb, display)) = start_xvfb() else {
        eprintln!("skipped: Xvfb did not start");
        return None;
    };
    let dir = std::env::temp_dir().join(format!("apihub-smoke-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let conf = dir.join("bus.conf");
    let xclose = dir.join("xclose.py");
    std::fs::write(&conf, BUS_CONF).expect("bus config");
    std::fs::write(&xclose, XCLOSE).expect("xclose helper");
    std::fs::write(dir.join("daemon.py"), SLOW_DAEMON).expect("fake daemon");
    std::fs::write(dir.join("xping.py"), XPING).expect("ping helper");
    let status = Command::new("unshare")
        .args(["-rm", "bash", "-c", SANDBOX, "sandbox", "dbus-run-session"])
        .arg(format!("--config-file={}", conf.display()))
        .args(["--", "bash", "-c", &format!("exec 2>\"$OUT/script.err\"\n{script}")])
        .env("DISPLAY", &display)
        .env_remove("WAYLAND_DISPLAY")
        .env("APP", env!("CARGO_BIN_EXE_apihub-app"))
        .env("OUT", &dir)
        .env("XCLOSE", &xclose)
        .env("FIX", concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("RUST_BACKTRACE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run the sandbox");
    assert!(status.code() != Some(90) && status.code() != Some(91), "sandbox failed");
    let files: Vec<(String, String)> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read_to_string(e.path()).unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    Some(move |name: &str| files.iter().find(|(n, _)| n == name).map(|(_, c)| c.clone()).unwrap_or_default())
}

const WAIT_WINDOW: &str = r#"wait_window() { local j; for j in $(seq 100); do xdotool search --name "Apple Keyboard Monitor" >/dev/null 2>&1 && return 0; sleep 0.1; done; return 1; }
nwin() { xdotool search --name "Apple Keyboard Monitor" 2>/dev/null | wc -l; }
"#;

#[test]
fn window_opens_and_stays_single_under_xvfb() {
    let script = format!(
        r#"{WAIT_WINDOW}
"$APP" > "$OUT/first.log" 2>&1 & A=$!
wait_window; sleep 2
timeout 15 "$APP" > "$OUT/second.log" 2>&1; echo "second=$?" > "$OUT/second.rc"
sleep 1
kill -0 $A && echo alive > "$OUT/alive"
nwin > "$OUT/windows"
kill -TERM $A; wait $A
"#
    );
    let Some(read) = run_sandboxed("single", &script) else { return };
    let (first, second) = (read("first.log"), read("second.log"));
    assert!(!first.contains("panicked"), "first instance panicked:\n{first}");
    assert!(!second.contains("panicked"), "second launch panicked:\n{second}");
    assert!(!first.contains("cannot open window"), "window not opened:\n{first}");
    assert_eq!(read("alive").trim(), "alive", "first instance died:\n{first}");
    assert_eq!(read("windows").trim(), "1", "exactly one window expected:\n{first}\n{second}");
    assert_eq!(read("second.rc").trim(), "second=0", "second launch:\n{second}");
    assert!(second.contains("already running"), "second launch did not raise the first:\n{second}");
}

/// #226: closing the window must end the process and never reopen a window,
/// even when the tray icon was clicked while the window was open. 5 times.
#[test]
fn closing_the_window_never_reopens_nor_crashes() {
    let script = format!(
        r#"{WAIT_WINDOW}
for i in 1 2 3 4 5; do
  "$APP" > "$OUT/run$i.log" 2>&1 & A=$!
  wait_window || echo "no window" >> "$OUT/run$i.log"
  sleep 1
  # Legacy tray icon clicked and Activate sent while the window is open.
  dbus-send --session --type=method_call --dest=org.kde.StatusNotifierItem-$A-1 /StatusNotifierItem org.kde.StatusNotifierItem.Activate int32:0 int32:0 2>/dev/null
  dbus-send --session --print-reply --type=method_call --dest=com.agenceapi.AppleKbMonitor /com/agenceapi/AppleKbMonitor org.freedesktop.Application.Activate 'dict:string:variant:' >/dev/null 2>&1
  sleep 0.5
  python3 "$XCLOSE" "$(xdotool search --name 'Apple Keyboard Monitor' | head -1)"
  rc=timeout
  for k in $(seq 100); do kill -0 $A 2>/dev/null || {{ wait $A; rc=$?; break; }}; sleep 0.1; done
  sleep 3
  echo "rc=$rc windows=$(nwin)" > "$OUT/run$i.res"
  kill -KILL $A 2>/dev/null; wait $A 2>/dev/null
done
"#
    );
    let Some(read) = run_sandboxed("close", &script) else { return };
    for i in 1..=5 {
        let log = read(&format!("run{i}.log"));
        assert!(!log.contains("panicked"), "run {i} panicked:\n{log}");
        assert!(!log.contains("no window"), "run {i}: no window:\n{log}");
        assert_eq!(read(&format!("run{i}.res")).trim(), "rc=0 windows=0", "run {i}:\n{log}\nscript: {}", read("script.err"));
    }
}

/// Fake `apple-kb-monitord` and desktop portal on the private bus: serves the
/// anonymised real snapshot, but answers `History()` and the portal's
/// `Settings.Read` only after `$DELAY` seconds, like a
/// daemon busy on a slow keyboard read. Timestamps of the real history
/// fixture are shifted so that its last point is "now".
const SLOW_DAEMON: &str = r#"
import json, sys, time
from gi.repository import Gio, GLib
snap, hist, delay = open(sys.argv[1]).read(), open(sys.argv[2]).read().splitlines(), int(sys.argv[3])
rows = [json.loads(l) for l in hist if l.strip()]
shift = int(time.time()) - max(r["ts"] for r in rows)
for r in rows: r["ts"] += shift
XML = """<node><interface name="com.agenceapi.AppleKbMonitor1">
<method name="GetState"><arg type="s" direction="out"/></method>
<method name="History"><arg type="t" direction="in"/><arg type="s" direction="out"/></method>
<property name="Json" type="s" access="read"/>
<signal name="StateChanged"><arg type="t"/><arg type="s"/></signal></interface></node>"""
iface = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
def call(conn, sender, path, iname, method, params, inv):
    if method == "History":
        GLib.timeout_add_seconds(delay, lambda: (inv.return_value(GLib.Variant("(s)", (json.dumps(rows),))), False)[1])
    else:
        inv.return_value(GLib.Variant("(s)", (snap,)))
def prop(conn, sender, path, iname, name):
    return GLib.Variant("s", snap)
def acquired(conn, name):
    conn.register_object("/com/agenceapi/AppleKbMonitor1", iface, call, prop, None)
def owned(conn, name):
    open(sys.argv[4], "w").write("ready")
Gio.bus_own_name(Gio.BusType.SESSION, "com.agenceapi.AppleKbMonitor1", 0, acquired, owned, None)
# Desktop portal whose Settings.Read answers after the same delay (#232).
PXML = """<node><interface name="org.freedesktop.portal.Settings">
<method name="Read"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
<signal name="SettingChanged"><arg type="s"/><arg type="s"/><arg type="v"/></signal></interface></node>"""
piface = Gio.DBusNodeInfo.new_for_xml(PXML).interfaces[0]
def pcall(conn, sender, path, iname, method, params, inv):
    GLib.timeout_add_seconds(delay, lambda: (inv.return_value(GLib.Variant("(v)", (GLib.Variant("v", GLib.Variant("u", 1)),))), False)[1])
def pacquired(conn, name):
    conn.register_object("/org/freedesktop/portal/desktop", piface, pcall, None, None)
Gio.bus_own_name(Gio.BusType.SESSION, "org.freedesktop.portal.Desktop", 0, pacquired, None, None)
GLib.MainLoop().run()
"#;

/// Like a window manager: `_NET_WM_PING` every 250 ms for `secs` seconds;
/// prints "sent answered max_ms". A WM declares the window "not responding"
/// after ~5 s without a reply.
const XPING: &str = r#"
import ctypes, sys, time, select
x = ctypes.cdll.LoadLibrary("libX11.so.6")
x.XOpenDisplay.restype = ctypes.c_void_p
x.XDefaultRootWindow.restype = ctypes.c_ulong; x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x.XInternAtom.restype = ctypes.c_ulong; x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
x.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]
x.XSelectInput.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_long]
x.XFlush.argtypes = x.XPending.argtypes = x.XConnectionNumber.argtypes = [ctypes.c_void_p]
x.XNextEvent.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
class M(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int),
                ("display", ctypes.c_void_p), ("window", ctypes.c_ulong), ("message_type", ctypes.c_ulong),
                ("format", ctypes.c_int), ("l", ctypes.c_long * 5)]
class E(ctypes.Union):
    _fields_ = [("xclient", M), ("pad", ctypes.c_long * 24)]
d = x.XOpenDisplay(None); w = int(sys.argv[1]); secs = float(sys.argv[2])
x.XSelectInput(d, x.XDefaultRootWindow(d), 1 << 19)  # SubstructureNotifyMask
proto, ping = x.XInternAtom(d, b"WM_PROTOCOLS", 0), x.XInternAtom(d, b"_NET_WM_PING", 0)
fd = x.XConnectionNumber(d); end = time.time() + secs; sent = ok = 0; worst = 0.0; serial = 0
while time.time() < end:
    serial += 1; e = E(); e.xclient.type = 33; e.xclient.window = w; e.xclient.format = 32
    e.xclient.message_type = proto; e.xclient.l[0] = ping; e.xclient.l[1] = serial; e.xclient.l[2] = w
    x.XSendEvent(d, w, 0, 0, ctypes.byref(e)); x.XFlush(d); t0 = time.time(); sent += 1; got = False
    while not got and time.time() - t0 < 5:
        while x.XPending(d):
            r = E(); x.XNextEvent(d, ctypes.byref(r))
            if r.xclient.type == 33 and r.xclient.l[0] == ping and r.xclient.l[1] == serial: got = True
        if not got: select.select([fd], [], [], 0.02)
    lat = (time.time() - t0) * 1000
    worst = max(worst, lat)
    ok += got
    time.sleep(0.25)
print(sent, ok, round(worst))
"#;

/// #230/#232: 3.1.0 froze ("not responding") because the UI thread waited
/// on D-Bus: History() at start and on Refresh (no timeout), and the
/// appearance lock held by the portal thread during its Read calls.
/// With the real (anonymised) history and a daemon that takes 20 s to answer
/// History(), the window must answer every WM ping for 30 s, through tab
/// switches, resizes and Refresh clicks, and no frame may take 100 ms.
#[test]
fn window_answers_pings_with_real_history_and_a_slow_daemon() {
    let script = format!(
        r#"{WAIT_WINDOW}
mkdir -p "$XDG_STATE_HOME/apple-kb-monitor"
python3 "$OUT/daemon.py" "$FIX/ui-gel-snapshot.json" "$FIX/ui-gel-history.jsonl" 20 "$OUT/daemon.ready" & DM=$!
for j in $(seq 50); do [ -f "$OUT/daemon.ready" ] && break; sleep 0.1; done
AKM_FRAME_STATS="$OUT/frames.log" "$APP" > "$OUT/app.log" 2>&1 & A=$!
wait_window || echo "no window" >> "$OUT/app.log"
W=$(xdotool search --name "Apple Keyboard Monitor" | head -1)
python3 "$OUT/xping.py" "$W" 30 > "$OUT/ping" & P=$!
sleep 2
for k in 1 2 3; do
  xdotool mousemove --window "$W" 112 12 click 1; sleep 1
  xdotool mousemove --window "$W" 40 12 click 1; sleep 1
  xdotool windowsize "$W" 900 700; sleep 1; xdotool windowsize "$W" 720 600; sleep 1
  # Refresh of the history tile (720x600 layout), twice in a row.
  xdotool mousemove --window "$W" 219 417 click 1 click 1; sleep 3
done
wait $P
kill -0 $A && echo alive > "$OUT/alive"
kill -TERM $A; wait $A; kill $DM
"#
    );
    let Some(read) = run_sandboxed("gel", &script) else { return };
    let log = read("app.log");
    assert!(!log.contains("panicked") && !log.contains("no window"), "{log}\n{}", read("script.err"));
    assert_eq!(read("alive").trim(), "alive", "{log}");
    let ping: Vec<u64> = read("ping").split_whitespace().filter_map(|v| v.parse().ok()).collect();
    assert_eq!(ping.len(), 3, "ping helper: {:?}\n{}", read("ping"), read("script.err"));
    let (sent, answered, worst_ms) = (ping[0], ping[1], ping[2]);
    eprintln!("pings: sent={sent} answered={answered} worst={worst_ms} ms");
    assert_eq!(answered, sent, "window stopped answering the WM (worst {worst_ms} ms)\n{log}");
    assert!(sent >= 20, "too few pings: {sent}");
    assert!(worst_ms < 1000, "a ping waited {worst_ms} ms\n{log}");
    let frames = read("frames.log");
    let worst_frame = frames
        .lines()
        .filter_map(|l| l.split_whitespace().find_map(|kv| kv.strip_prefix("max_update_ms=")))
        .filter_map(|v| v.parse::<f64>().ok())
        .fold(0.0_f64, f64::max);
    assert!(!frames.is_empty(), "no frame statistics\n{log}");
    assert!(worst_frame < 100.0, "a frame took {worst_frame} ms\n{frames}");
}
