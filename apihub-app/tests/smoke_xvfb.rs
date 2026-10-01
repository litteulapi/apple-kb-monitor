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
    let status = Command::new("unshare")
        .args(["-rm", "bash", "-c", SANDBOX, "sandbox", "dbus-run-session"])
        .arg(format!("--config-file={}", conf.display()))
        .args(["--", "bash", "-c", &format!("exec 2>\"$OUT/script.err\"\n{script}")])
        .env("DISPLAY", &display)
        .env_remove("WAYLAND_DISPLAY")
        .env("APP", env!("CARGO_BIN_EXE_apihub-app"))
        .env("OUT", &dir)
        .env("XCLOSE", &xclose)
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
