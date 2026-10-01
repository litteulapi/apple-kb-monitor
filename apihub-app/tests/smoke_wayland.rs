//! #231: under Wayland, a minimized window must not freeze the main thread.
//! With `vsync: true`, Mesa's eglSwapBuffers waited for a frame callback that
//! a minimized surface never gets: main thread parked in ppoll for ever, no
//! pong ("Not responding"), Activate/close ignored.
//!
//! Runs a nested `kwin_wayland --virtual` on a private session bus, in a
//! user+mount namespace without `/dev/hidraw*` (same sandbox as
//! `smoke_xvfb.rs`). Minimizes the window with a KWin script, samples the main
//! thread for 10 s, then closes the (still minimized) window: the process must
//! exit. Skipped with a message when kwin_wayland, dbus-run-session or user
//! namespaces are missing.

use std::process::{Command, Stdio};

const SANDBOX: &str = r#"
mount -t tmpfs none /sys/class/hidraw || exit 90
for f in /dev/hidraw*; do [ -e "$f" ] && { mount --bind /dev/null "$f" || exit 91; }; done
exec "$@""#;

const BUS_CONF: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy>
</busconfig>"#;

const SCRIPT: &str = r#"
kwin_script() {  # $1 = file with the script body
  local id
  id=$(dbus-send --session --print-reply --dest=org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript string:"$1" string:"akm$RANDOM" | awk '/int32/{print $2}')
  dbus-send --session --print-reply --dest=org.kde.KWin /Scripting/Script$id org.kde.kwin.Script.run >/dev/null 2>&1 \
    || dbus-send --session --print-reply --dest=org.kde.KWin /$id org.kde.kwin.Script.run >/dev/null 2>&1
}
kwin_wayland --virtual --socket akm-test --width 1024 --height 768 --no-lockscreen --no-global-shortcuts > "$OUT/kwin.log" 2>&1 & K=$!
for i in $(seq 100); do [ -S "$XDG_RUNTIME_DIR/akm-test" ] && break; sleep 0.1; done
[ -S "$XDG_RUNTIME_DIR/akm-test" ] || { echo nokwin > "$OUT/skip"; kill $K; exit 0; }
export WAYLAND_DISPLAY=akm-test; unset DISPLAY
"$APP" > "$OUT/app.log" 2>&1 & A=$!
sleep 4
kill -0 $A || echo "died before minimize" >> "$OUT/app.log"
echo 'for (const w of workspace.windowList()) if (w.caption.indexOf("Apple Keyboard") >= 0) w.minimized = true;' > "$OUT/min.js"
kwin_script "$OUT/min.js"
for t in $(seq 10); do sleep 1; echo "$(cat /proc/$A/wchan 2>/dev/null)" >> "$OUT/wchan"; done
echo 'for (const w of workspace.windowList()) if (w.caption.indexOf("Apple Keyboard") >= 0) w.closeWindow();' > "$OUT/close.js"
kwin_script "$OUT/close.js"
rc=timeout
for k in $(seq 50); do kill -0 $A 2>/dev/null || { wait $A; rc=$?; break; }; sleep 0.1; done
echo "rc=$rc" > "$OUT/rc"
kill -KILL $A 2>/dev/null; kill $K; wait
"#;

fn have(bin: &str) -> bool {
    Command::new("sh").args(["-c", &format!("command -v {bin}")]).stdout(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

#[test]
fn minimized_window_keeps_the_main_thread_alive_under_wayland() {
    for bin in ["kwin_wayland", "dbus-run-session", "dbus-send", "unshare"] {
        if !have(bin) {
            eprintln!("skipped: {bin} not installed");
            return;
        }
    }
    let dir = std::env::temp_dir().join(format!("apihub-wl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("run")).expect("temp dir");
    let conf = dir.join("bus.conf");
    std::fs::write(&conf, BUS_CONF).expect("bus config");
    let status = Command::new("unshare")
        .args(["-rm", "bash", "-c", SANDBOX, "sandbox", "dbus-run-session"])
        .arg(format!("--config-file={}", conf.display()))
        .args(["--", "bash", "-c", &format!("exec 2>\"$OUT/script.err\"\n{SCRIPT}")])
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .env("APP", env!("CARGO_BIN_EXE_apihub-app"))
        .env("OUT", &dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_STATE_HOME", dir.join("state"))
        // The scripts find the window by its English title (#114).
        .env("LC_ALL", "C")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run the sandbox");
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).unwrap_or_default();
    let (skip, log, wchan, rc, err) = (read("skip"), read("app.log"), read("wchan"), read("rc"), read("script.err"));
    let _ = std::fs::remove_dir_all(&dir);
    if matches!(status.code(), Some(90 | 91)) {
        eprintln!("skipped: unprivileged user namespaces unavailable");
        return;
    }
    if !skip.is_empty() {
        eprintln!("skipped: kwin_wayland --virtual did not start");
        return;
    }
    assert!(!log.contains("panicked") && !log.contains("died"), "{log}\n{err}");
    let samples: Vec<&str> = wchan.lines().collect();
    assert_eq!(samples.len(), 10, "main thread not sampled: {wchan:?}\n{log}\n{err}");
    // Stuck in eglSwapBuffers = libwayland ppoll = poll_schedule_timeout for
    // 10 s; the event loop itself waits in epoll.
    assert!(samples.iter().any(|w| w.contains("epoll")), "main thread never back in its event loop while minimized: {samples:?}");
    assert_eq!(rc.trim(), "rc=0", "closing the minimized window did not end the process\nwchan: {samples:?}\n{log}\n{err}");
}
