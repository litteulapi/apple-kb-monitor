#!/usr/bin/env python3
"""End-to-end test of the System Settings module "Apple Keyboard".

    tests/e2e/kcm.py [--out DIR] [--prefix DIR] [--only a,b] [--max-latency-ms 100]

Builds kcm/ (cmake, into DIR/build, installed into DIR/prefix unless
--prefix gives an existing install), then loads it with `kcmshell6
kcm_applekeyboard` under Xvfb with:

  * a PRIVATE session bus without any activation directory (the real
    apple-kb-monitord can never be started by D-Bus activation),
  * a fake apple-kb-monitord (this file, `daemon` sub-command, python-gobject)
    exposing the real interfaces (root, .Keymap, .Link, .Settings) with canned
    data; its Settings.RunAkmctl runs the fake akmctl below,
  * fake `akmctl` and `systemctl` first in $PATH (they only print fixtures and
    record their arguments): no pkexec, no hardware, no user service touched,
  * HOME / XDG_* in the work directory (the user's config.toml is never read).

Scenarios: screens (every tab, French; State in English, dark and at 2x),
absent (no daemon: banner and start button), slow (every daemon call answers
after 40 s), error (every call fails), garbage (invalid JSON), akmctl_slow
(akmctl answers after 40 s), actions (keyboard and pointer use: link check,
self-check, refused rename, notification settings applied) and
systemsettings (the module inside System Settings, found by its search). In each one the pointer moves over the window
and the module's GUI heartbeat (a 16 ms timer of the GUI thread, module
argument "heartbeat", logged as `akm-heartbeat`) must never be late by more than --max-latency-ms (100):
that lateness is how long the window could neither paint nor answer the
compositor's ping ("Not Responding"). Qt does not answer _NET_WM_PING without
a window manager (measured under Xvfb), so that ping cannot be used here.
Any QML error in the log fails, so does any call that could change something
(spy files) during a passive look.

Exit 0 = all passed, 1 = a failure, 77 = a tool is missing.
"""

import argparse
import importlib.util
import json
import os
import select
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
TOP = HERE.parent.parent
FIX = HERE / "fixtures" / "kcm"
WARMUP_S = 1.0  # after the 3 s settle of open_kcm()
BUS = "com.agenceapi.AppleKbMonitor1"
ROOT = "/com/agenceapi/AppleKbMonitor1"
MAC = "AA:BB:CC:DD:EE:F1"  # anonymised
# Calls a passive look at the module must never make.
WRITES = {"SetAlias", "Refresh", "SetKey", "SetPreset", "UseProfile", "Apply", "Reset", "Reconnect",
          "SetFnMode", "NotifyShutdown", "RereadName", "SetConfig"}


class Conflict(ValueError):
    """SetConfig over a newer file: the daemon's named error."""


class Fail(Exception):
    pass


# ── fake daemon (sub-command) ───────────────────────────────────────────────

DAEMON_XML = """
<node>
 <interface name="com.agenceapi.AppleKbMonitor1">
  <method name="GetState"><arg type="s" direction="out"/></method>
  <method name="Refresh"/>
  <method name="RereadName"/>
  <method name="SetAlias"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="GetDevices"><arg type="ao" direction="out"/></method>
  <signal name="StateChanged"><arg type="t"/><arg type="s"/></signal>
  <property name="DaemonVersion" type="s" access="read"/>
  <property name="InterfaceVersion" type="u" access="read"/>
 </interface>
 <interface name="com.agenceapi.AppleKbMonitor1.Keymap">
  <method name="KeyTable"><arg type="b" direction="in"/><arg type="s" direction="out"/></method>
  <method name="Keymap"><arg type="s" direction="out"/></method>
  <method name="SetKey"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="SetPreset"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="UseProfile"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="Apply"><arg type="s" direction="out"/></method>
  <method name="Reset"><arg type="s" direction="out"/></method>
 </interface>
 <interface name="com.agenceapi.AppleKbMonitor1.Settings">
  <method name="GetConfig"><arg type="s" direction="out"/></method>
  <method name="SetConfig"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="RunAkmctl"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="out"/></method>
 </interface>
 <interface name="com.agenceapi.AppleKbMonitor1.Link">
  <method name="Status"><arg type="s" direction="out"/></method>
  <method name="Reconnect"><arg type="b" direction="out"/></method>
 </interface>
</node>"""


def snapshot(rev):
    now = int(time.time())
    return {
        "schema": 1, "version": rev, "connected": True,
        "keyboard": {
            "wake": {"last_age_s": None, "count": 0},
            "device": {"model": "Apple Wireless Keyboard (A1314, aluminium, ISO)", "name": "Test keyboard",
                       "alias": "Desk keyboard", "mac": MAC, "chip": "BCM2042", "driver": "hid-apple",
                       "name_on_keyboard": "Test keyboard"},
            "battery": {"percentage": 87.0, "percentage_fine": 87.0, "voltage": 2.91, "voltage_mv": 2910,
                        "voltage_filtered_mv": 2905, "voltage_doubtful": False,
                        "charge_estimate": {"pct": 64.0, "low": 55.0, "high": 72.0, "chemistry": "alkaline",
                                            "basis_mv": 2905},
                        "new_batteries": False, "apple_display_pct": 90.0, "threshold_level": "ok"},
            "bluetooth": {"connected": True, "paired": True},
            "radio": {"rssi_dbm": -2, "rssi_rel_db": -2, "rssi_kind": "bredr-golden-range",
                      "rssi_quality": "good", "tx_power_dbm": 8},
            "firmware": {"version": "0x0050", "version_hex": "0x0050", "latest_known": "0x0050",
                         "status": "up_to_date"},
            "raw": {}, "incomplete": False,
        },
        "kb_error": None, "caps_lock": False, "num_lock": False, "forecast": {"rate_pct_per_day": 0.8, "empty_at": int(time.time()) + 41 * 86400, "fitted_pct": 86.0, "span_s": 604800, "buckets": 168},
        "rssi_at": now - 5, "last_update": now - 42, "last_error": None, "forecast": None,
        "batteries_installed_at": now - 40 * 86400,
    }


def run_daemon(a):
    import gi
    gi.require_version("Gio", "2.0")
    from gi.repository import Gio, GLib

    conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    info = Gio.DBusNodeInfo.new_for_xml(DAEMON_XML)
    rev = {"n": 1}
    keys = (FIX / "keys.json").read_text()
    keymap = (FIX / "keymap.json").read_text()
    cfg = {"values": {}, "rev": 1}
    cfg_path = Path(os.environ["XDG_CONFIG_HOME"]) / "apple-kb-monitor/config.toml"

    def get_config():
        return json.dumps({"values": cfg["values"], "revision": str(cfg["rev"])})

    def set_config(key, value, rev):
        # Same contract as the daemon: one key, refused over a newer file.
        if rev != str(cfg["rev"]):
            raise Conflict("config.toml changed since it was read")
        cfg["values"][key] = json.loads(value)
        cfg["rev"] += 1
        sections = {}
        for k, v in cfg["values"].items():
            sec, name = k.split(".", 1)
            sections.setdefault(sec, []).append(f"{name} = {json.dumps(v)}")
        cfg_path.parent.mkdir(parents=True, exist_ok=True)
        cfg_path.write_text("".join(f"[{sec}]\n" + "\n".join(ls) + "\n" for sec, ls in sections.items()))
        return str(cfg["rev"])

    def run_akmctl(inv, args_json):
        # The fake akmctl, without blocking the loop (as the daemon's worker thread).
        p = Gio.Subprocess.new(["akmctl"] + json.loads(args_json),
                               Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE)

        def done(proc, res):
            _, out, err = proc.communicate_utf8_finish(res)
            r = {"code": proc.get_exit_status(), "out": out or "", "err": err or "", "timed_out": False}
            inv.return_value(GLib.Variant("(s)", (json.dumps(r),)))
        p.communicate_utf8_async(None, None, done)

    def spy(iface, name, args):
        with open(a.spy, "a") as f:
            f.write(json.dumps({"t": round(time.time(), 3), "iface": iface, "method": name,
                                "args": str(args)[:200]}) + "\n")

    def answer(inv, fn):
        # A slow daemon answers later WITHOUT blocking its own loop (signals go on).
        if a.mode == "slow":
            GLib.timeout_add(int(a.delay * 1000), lambda: (fn(), False)[1])
        else:
            fn()

    def method(c, sender, path, iface, name, params, inv):
        args = params.unpack() if params else None
        spy(iface, name, args)
        if a.mode == "error":
            inv.return_dbus_error("org.freedesktop.DBus.Error.Failed", "e2e fake: simulated failure")
            return
        if name == "SetConfig":
            try:
                new_rev = set_config(*args)
                answer(inv, lambda: inv.return_value(GLib.Variant("(s)", (new_rev,))))
            except ValueError as e:
                err = "com.agenceapi.AppleKbMonitor1.Error.Conflict" if isinstance(e, Conflict) else "org.freedesktop.DBus.Error.Failed"
                inv.return_dbus_error(err, str(e))
            return
        if name == "RunAkmctl":
            answer(inv, lambda: run_akmctl(inv, args[0]))
            return
        if name in WRITES:
            # The fake never changes anything; a write is recorded and refused.
            inv.return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "e2e fake: write recorded")
            return
        garbage = a.mode == "garbage"
        replies = {
            "GetState": lambda: GLib.Variant("(s)", ('{"schema":1,"keyb' if garbage else json.dumps(snapshot(rev["n"])),)),
            "GetDevices": lambda: GLib.Variant("(ao)", ([ROOT + "/dev_" + MAC.replace(":", "_")],)),
            "KeyTable": lambda: GLib.Variant("(s)", ("[" if garbage else keys,)),
            "Keymap": lambda: GLib.Variant("(s)", ("{" if garbage else keymap,)),
            "Status": lambda: GLib.Variant("(s)", (json.dumps([{"mac": MAC, "health": "connected"}]),)),
            "GetConfig": lambda: GLib.Variant("(s)", ("{" if garbage else get_config(),)),
        }
        if name in replies:
            v = replies[name]()
            answer(inv, lambda: inv.return_value(v))
        else:
            inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", name)

    def get_prop(c, sender, path, iface, prop):
        spy("org.freedesktop.DBus.Properties", "Get:" + prop, None)
        return {"DaemonVersion": GLib.Variant("s", "3.1.0-e2e"), "InterfaceVersion": GLib.Variant("u", 2)}.get(prop)

    conn.register_object(ROOT, info.interfaces[0], method, get_prop, None)
    conn.register_object(ROOT, info.interfaces[1], method, None, None)
    conn.register_object(ROOT + "/Settings", info.interfaces[2], method, None, None)
    conn.register_object(ROOT + "/Link", info.interfaces[3], method, None, None)
    r = conn.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName",
                       GLib.Variant("(su)", (BUS, 4)), GLib.VariantType("(u)"), 0, -1, None)
    if r.unpack()[0] != 1:
        sys.exit("cannot own " + BUS)
    print("ready", flush=True)

    def tick():
        rev["n"] += 1
        conn.emit_signal(None, ROOT, BUS, "StateChanged", GLib.Variant("(ts)", (rev["n"], "{}")))
        return True

    GLib.timeout_add(2000, tick)
    GLib.MainLoop().run()


# ── fake programs ───────────────────────────────────────────────────────────

FAKE_AKMCTL = r'''#!/usr/bin/env python3
# Fake akmctl for tests/e2e/kcm.py: prints fixtures, records its arguments.
import json, os, sys, time
fix, spy = {fix!r}, {spy!r}
with open(spy, "a") as f:
    f.write(json.dumps({{"t": round(time.time(), 3), "prog": "akmctl", "args": sys.argv[1:]}}) + "\n")
time.sleep(float(os.environ.get("AKM_FAKE_DELAY", "0")))
a = sys.argv[1:]
files = {{("keys", "--json"): "keys.json", ("keymap", "show", "--json"): "keymap.json",
          ("doctor", "--json"): "doctor.json", ("selftest", "--json", "--no-save"): "selftest.json"}}
name = files.get(tuple(a))
if name:
    sys.stdout.write(open(os.path.join(fix, name), encoding="utf-8").read())
    sys.exit(0)
sys.stderr.write("e2e fake akmctl: refused (recorded)\n")
sys.exit(1)
'''

FAKE_SYSTEMCTL = r'''#!/bin/sh
# Fake systemctl for tests/e2e/kcm.py: records, never touches a unit.
printf '{{"prog":"systemctl","args":"%s"}}\n' "$*" >> {spy!r}
echo "e2e fake systemctl: refused (recorded)" >&2
exit 1
'''


# ── harness ────────────────────────────────────────────────────────────────

class Env:
    def __init__(self, a):
        self.a = a
        self.out = Path(a.out).resolve()
        self.out.mkdir(parents=True, exist_ok=True)
        self.work = self.out / "work"
        shutil.rmtree(self.work, ignore_errors=True)
        self.work.mkdir()
        self.procs = []
        self.display = None
        self.results = {}

    def spawn(self, argv, log, env=None, **kw):
        f = open(log, "ab")
        kw.setdefault("stdout", f)
        p = subprocess.Popen(argv, stderr=f, env=env, **kw)
        self.procs.append(p)
        return p

    def build(self):
        if self.a.prefix:
            self.prefix = Path(self.a.prefix).resolve()
            return
        b, self.prefix = self.out / "build", self.out / "prefix"
        for cmd in (["cmake", "-S", str(TOP / "kcm"), "-B", str(b), "-DCMAKE_BUILD_TYPE=Release",
                     f"-DCMAKE_INSTALL_PREFIX={self.prefix}"],
                    ["cmake", "--build", str(b), "-j4"], ["cmake", "--install", str(b)]):
            r = subprocess.run(cmd, capture_output=True, text=True)
            if r.returncode:
                raise Fail(f"{' '.join(cmd[:2])} failed:\n{(r.stdout + r.stderr)[-2000:]}")

    def start_xvfb(self):
        r, w = os.pipe()
        p = subprocess.Popen(["Xvfb", "-displayfd", str(w), "-screen", "0", "1600x1100x24", "-nolisten", "tcp",
                              "-noreset"], pass_fds=(w,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(w)
        self.procs.append(p)
        buf = b""
        deadline = time.time() + 15
        while time.time() < deadline and not buf.endswith(b"\n"):
            if not select.select([r], [], [], max(0.0, deadline - time.time()))[0]:
                break
            chunk = os.read(r, 16)
            if not chunk:
                break
            buf += chunk
        os.close(r)
        if not buf.strip():
            raise Fail("Xvfb did not start")
        self.display = ":" + buf.decode().strip()

    def start_bus(self):
        # No <servicedir> and no <standard_session_servicedirs/>: nothing can be
        # started by D-Bus activation, least of all the real daemon.
        conf = self.work / "session-bus.conf"
        conf.write_text("""<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:dir=/tmp</listen><auth>EXTERNAL</auth>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy>
</busconfig>""")
        p = subprocess.Popen(["dbus-daemon", f"--config-file={conf}", "--nofork", "--print-address=1"],
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.procs.append(p)
        return p.stdout.readline().decode().strip(), p

    def scenario_env(self, sc, lang="fr", dark=False, scale=None, akmctl_delay=0.0):
        d = self.out / sc
        shutil.rmtree(d, ignore_errors=True)
        d.mkdir(parents=True)
        home = d / "home"
        bindir = d / "bin"
        for x in (home / ".config", home / ".local/share", home / ".cache", home / ".local/state", bindir):
            x.mkdir(parents=True, exist_ok=True)
        run = d / "runtime"
        run.mkdir(mode=0o700)
        spy = d / "programs-spy.jsonl"
        (bindir / "akmctl").write_text(FAKE_AKMCTL.format(fix=str(FIX), spy=str(spy)))
        (bindir / "systemctl").write_text(FAKE_SYSTEMCTL.format(spy=str(spy)))
        for f in ("akmctl", "systemctl"):
            (bindir / f).chmod(0o755)
        if dark:
            scheme = Path("/usr/share/color-schemes/BreezeDark.colors")
            if scheme.exists():
                shutil.copy(scheme, home / ".config/kdeglobals")
        bus, busp = self.start_bus()
        env = {k: v for k, v in os.environ.items()
               if k not in ("WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "DBUS_SYSTEM_BUS_ADDRESS",
                            "QT_SCALE_FACTOR", "LANGUAGE", "LC_ALL", "LANG")}
        env.update({
            "DISPLAY": self.display, "DBUS_SESSION_BUS_ADDRESS": bus,
            # an unreachable system bus: nothing system-wide is ever asked
            "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=" + str(d / "no-system-bus"),
            "XDG_RUNTIME_DIR": str(run), "XDG_SESSION_TYPE": "x11", "HOME": str(home),
            "XDG_CONFIG_HOME": str(home / ".config"), "XDG_DATA_HOME": str(home / ".local/share"),
            "XDG_CACHE_HOME": str(home / ".cache"), "XDG_STATE_HOME": str(home / ".local/state"),
            "XDG_CURRENT_DESKTOP": "KDE", "KDE_FULL_SESSION": "true", "QT_QPA_PLATFORM": "xcb",
            "QT_QPA_PLATFORMTHEME": "kde", "QT_QUICK_CONTROLS_STYLE": "org.kde.desktop",
            "QT_PLUGIN_PATH": str(self.prefix / "lib/plugins") + ":" + str(self.prefix / "lib/qt6/plugins"),
            "XDG_DATA_DIRS": str(self.prefix / "share") + ":/usr/share",
            "PATH": str(bindir) + ":/usr/bin:/bin", "LIBGL_ALWAYS_SOFTWARE": "1", "NO_AT_BRIDGE": "1",
            "LANG": "fr_FR.UTF-8" if lang == "fr" else "en_US.UTF-8", "LANGUAGE": lang,
            "AKM_FAKE_DELAY": str(akmctl_delay),
            # Qt logs to the journal when stderr is not a terminal: the heartbeat and QML errors go to kcmshell.log
            "QT_FORCE_STDERR_LOGGING": "1",
        })
        if scale:
            env["QT_SCALE_FACTOR"] = str(scale)
        return d, env, busp

    def fake_daemon(self, d, env, mode="normal", delay=40.0):
        p = self.spawn([sys.executable, str(Path(__file__).resolve()), "daemon", "--spy", str(d / "daemon-spy.jsonl"),
                        "--mode", mode, "--delay", str(delay)], d / "fake-daemon.log", env=env,
                       stdout=subprocess.PIPE)
        if p.stdout.readline().strip() != b"ready":
            raise Fail(f"fake daemon did not start, log: {d / 'fake-daemon.log'}")
        return p

    def launch(self, d, env, tab=None):
        argv = ["kcmshell6", "kcm_applekeyboard", "--args", ((tab or "") + " heartbeat").strip()]
        return self.spawn(argv, d / "kcmshell.log", env=env)

    def wait_window(self, p, env, d, timeout=30.0):
        t0 = time.time()
        while time.time() - t0 < timeout:
            r = subprocess.run(["xdotool", "search", "--onlyvisible", "--class", "kcmshell6"], env=env,
                               capture_output=True, text=True)
            ws = [int(w) for w in r.stdout.split()]
            if ws:
                return ws[-1]
            if p.poll() is not None:
                raise Fail(f"kcmshell6 exited (rc={p.returncode}) before showing its window, log: {d / 'kcmshell.log'}")
            time.sleep(0.1)
        raise Fail(f"no kcmshell6 window after {timeout:.0f} s, log: {d / 'kcmshell.log'}")

    def screenshot(self, win, path):
        r = subprocess.run(["import", "-display", self.display, "-window", str(win), str(path)],
                           capture_output=True, text=True, timeout=30)
        return str(path) if r.returncode == 0 else None

    def measure(self, d, p, win, seconds, env):
        """Move the pointer over the window (repaints) for `seconds` and read the
        module's GUI heartbeat (`akm-heartbeat` lines, kcm/ui/main.qml): the
        worst lateness of a 16 ms timer of the GUI thread = the longest time
        the window could not paint nor answer the compositor's ping."""
        log = d / "kcmshell.log"
        t0 = time.time()
        n = 0
        while time.time() - t0 < seconds:
            if p.poll() is not None:
                raise Fail(f"kcmshell6 died during the measure (rc={p.returncode})")
            subprocess.run(["xdotool", "mousemove", "--window", str(win), str(60 + (n * 37) % 700),
                            str(80 + (n * 53) % 500)], env=env, capture_output=True)
            n += 1
            time.sleep(0.1)
        t1 = time.time()
        rows = []
        for ln in (log.read_text(errors="replace").splitlines() if log.exists() else []):
            if "akm-heartbeat " not in ln:
                continue
            try:
                r = json.loads(ln.split("akm-heartbeat ", 1)[1])
            except ValueError:
                continue
            if (t0 + WARMUP_S) * 1000 <= r["ts_ms"] <= t1 * 1000 + 600:
                rows.append(r)
        gaps = [b["ts_ms"] - a["ts_ms"] for a, b in zip(rows, rows[1:])]
        late = [r["max_late_ms"] for r in rows]
        # a blocked loop also shows as a missing heartbeat line (one per 500 ms)
        worst_gap = max(gaps) if gaps else None
        worst = max(late + ([worst_gap - 500] if worst_gap else [])) if rows else None
        return {"seconds": round(t1 - t0, 1), "samples": len(rows), "max_ms": worst,
                "max_late_ms": max(late) if late else None, "max_gap_ms": worst_gap,
                "threshold_ms": self.a.max_latency_ms, "pointer_moves": n}

    def assert_responsive(self, stats, what):
        expected = (stats["seconds"] - WARMUP_S) * 2 * 0.6
        if stats["samples"] < expected:
            raise Fail(f"{what}: GUI heartbeat missing ({stats['samples']} samples, expected ~{expected:.0f}): "
                       f"the event loop was blocked - this is the 'Not Responding' of the desktop")
        if stats["max_ms"] is not None and stats["max_ms"] > stats["threshold_ms"]:
            raise Fail(f"{what}: GUI thread blocked {stats['max_ms']} ms (> {stats['threshold_ms']} ms)")

    def check_log(self, d):
        """Any QML warning from the module's own files, or a JS error, fails."""
        log = (d / "kcmshell.log").read_text(errors="replace")
        bad = [ln for ln in log.splitlines()
               if "akm-heartbeat " not in ln
               and ("qrc:/kcm/kcm_applekeyboard" in ln or "TypeError" in ln or "ReferenceError" in ln)]
        if bad:
            raise Fail("QML errors in the log:\n  " + "\n  ".join(bad[:10]) + f"\n(log: {d / 'kcmshell.log'})")

    def check_spy(self, d):
        """A passive look reads only: no daemon write, no systemctl, akmctl only for the key table."""
        reads = (["keys", "--json"], ["keymap", "show", "--json"])
        bad = []
        for name in ("daemon-spy.jsonl", "programs-spy.jsonl"):
            f = d / name
            if not f.exists():
                continue
            for ln in f.read_text().splitlines():
                rec = json.loads(ln)
                if rec.get("method") in WRITES or rec.get("prog") == "systemctl":
                    bad.append(rec)
                elif rec.get("prog") == "akmctl" and rec.get("args") not in reads:
                    bad.append(rec)
        if bad:
            raise Fail(f"calls that could change something during a passive look: {bad[:5]}")

    def stop(self, *ps):
        for p in ps:
            if p and p.poll() is None:
                p.terminate()
                try:
                    p.wait(5)
                except subprocess.TimeoutExpired:
                    p.kill()
                    p.wait()

    def teardown(self):
        self.stop(*reversed(self.procs))


# ── scenarios ──────────────────────────────────────────────────────────────

TABS = ["state", "keys", "notifications", "name", "diagnostics"]


def open_kcm(env, sc, tab=None, mode="normal", seconds=8.0, settle=3.0, scroll=0, **kw):
    d, e, busp = env.scenario_env(sc, **kw)
    dp = env.fake_daemon(d, e, mode=mode) if mode != "absent" else None
    p = env.launch(d, e, tab)
    try:
        win = env.wait_window(p, e, d)
        subprocess.run(["xdotool", "windowsize", str(win), "900", "780"], env=e, capture_output=True)
        time.sleep(settle)
        stats = env.measure(d, p, win, seconds, e)
        shot = env.screenshot(win, d / f"{sc}.png")
        if scroll:
            # mouse wheel down over the content: the lower part of a long section
            subprocess.run(["xdotool", "mousemove", "--window", str(win), "450", "400"] + ["click", "5"] * scroll,
                           env=e, capture_output=True)
            time.sleep(1)
            env.screenshot(win, d / f"{sc}-bottom.png")
        return d, e, p, win, stats, shot
    finally:
        env.stop(p, dp, busp)


def check_doctor_fixture():
    """The Diagnostics tab must render the fixture by id (its i18nc texts), never by the default branch."""
    qml = (HERE.parent.parent / "kcm" / "ui" / "DiagPage.qml").read_text()
    doc = json.loads((FIX / "doctor.json").read_text())
    ids = [doc["verdict"].get("id")] + [x.get("id") for x in doc["findings"]]
    missing = [i for i in ids if not i or f'case "{i}":' not in qml]
    if missing:
        raise Fail(f"doctor.json ids not handled by DiagPage.qml: {missing}")


def sc_screens(env):
    check_doctor_fixture()
    res = {}
    for tab in TABS:
        d, e, p, win, stats, shot = open_kcm(env, "screens-" + tab, tab=tab, seconds=6.0,
                                             scroll=25 if tab in ("keys", "notifications") else 0)
        env.check_log(d)
        env.check_spy(d)
        env.assert_responsive(stats, "tab " + tab)
        res[tab] = {"capture": shot, "max_ms": stats["max_ms"]}
    for name, kw in (("screens-state-en", {"lang": "en"}), ("screens-state-dark", {"dark": True}),
                     ("screens-keys-2x", {"scale": 2, "tab": "keys"})):
        d, e, p, win, stats, shot = open_kcm(env, name, seconds=4.0, **kw)
        env.check_log(d)
        env.check_spy(d)
        env.assert_responsive(stats, name)
        res[name] = {"capture": shot, "max_ms": stats["max_ms"]}
    return res


def sc_absent(env):
    d, e, p, win, stats, shot = open_kcm(env, "absent", mode="absent", seconds=10.0)
    env.check_log(d)
    env.check_spy(d)
    env.assert_responsive(stats, "daemon absent")
    return {"capture": shot, **stats}


def sc_slow(env):
    # Every daemon call answers after 40 s: the module must stay fluid and say
    # "did not answer within 5 seconds" (its own deadline), not wait 40 s.
    d, e, p, win, stats, shot = open_kcm(env, "slow", mode="slow", seconds=15.0)
    env.check_log(d)
    env.check_spy(d)
    env.assert_responsive(stats, "daemon slow (40 s)")
    return {"capture": shot, **stats}


def sc_error(env):
    d, e, p, win, stats, shot = open_kcm(env, "error", mode="error", seconds=8.0)
    env.check_log(d)
    env.check_spy(d)
    env.assert_responsive(stats, "daemon error")
    return {"capture": shot, **stats}


def sc_garbage(env):
    d, e, p, win, stats, shot = open_kcm(env, "garbage", mode="garbage", seconds=8.0)
    env.check_log(d)
    env.check_spy(d)
    env.assert_responsive(stats, "daemon garbage")
    return {"capture": shot, **stats}


def sc_akmctl_slow(env):
    d, e, p, win, stats, shot = open_kcm(env, "akmctl-slow", tab="keys", seconds=12.0, akmctl_delay=40.0)
    env.check_log(d)
    env.check_spy(d)
    env.assert_responsive(stats, "akmctl slow (40 s)")
    return {"capture": shot, **stats}


def sc_actions(env):
    """Keyboard and pointer use against the fake daemon: link check and
    self-check (keyboard only), rename (refused by the fake: the error must be
    shown), notification settings saved by Apply into config.toml."""
    d, e, busp = env.scenario_env("actions")
    dp = env.fake_daemon(d, e)
    p = env.launch(d, e, "diagnostics")
    out = {}
    # XTEST key presses to the focused window, as a real keyboard
    key = lambda *k: subprocess.run(["xdotool", "key", "--delay", "60"] + list(k), env=e, capture_output=True)
    click = lambda x, y: subprocess.run(["xdotool", "mousemove", "--window", str(win), str(x), str(y), "click", "1"],
                                        env=e, capture_output=True)
    try:
        win = env.wait_window(p, e, d)
        subprocess.run(["xdotool", "windowsize", str(win), "900", "780"], env=e, capture_output=True)
        subprocess.run(["xdotool", "windowfocus", "--sync", str(win)], env=e, capture_output=True)
        time.sleep(3)
        key("Tab", "space")     # first focusable item of the section: "Check the link"
        time.sleep(1.5)
        key("Tab", "space")     # "Self-check"
        time.sleep(1.5)
        out["diagnostics"] = env.screenshot(win, d / "actions-diagnostics.png")
        progs = (d / "programs-spy.jsonl").read_text()
        if '"doctor"' not in progs or '"selftest"' not in progs:
            raise Fail(f"keyboard: doctor/selftest not run from the keyboard, see {out['diagnostics']}")
        # Ctrl+Tab from Diagnostics wraps to State, Ctrl+PgUp x2 back to Name
        key("ctrl+Tab")
        time.sleep(0.5)
        out["ctrl_tab"] = env.screenshot(win, d / "actions-ctrl-tab.png")
        key("ctrl+Prior", "ctrl+Prior")
        time.sleep(1)
        click(480, 189)
        subprocess.run(["xdotool", "type", "--delay", "20", "E2E keyboard"], env=e, capture_output=True)
        key("Return")
        time.sleep(1.5)
        out["name"] = env.screenshot(win, d / "actions-name.png")
        spy = (d / "daemon-spy.jsonl").read_text()
        if "SetAlias" not in spy or "E2E keyboard" not in spy:
            raise Fail(f"rename: SetAlias not sent, see {out['name']}")
        # Notifications: switch the alerts off, then Apply
        click(216, 61)
        time.sleep(1)
        click(325, 139)
        time.sleep(0.5)
        stats = env.measure(d, p, win, 3.0, e)
        out["notifications_before"] = env.screenshot(win, d / "actions-notifications.png")
        # the "Apply" button of the kcmshell6 dialog: second from the right
        g = subprocess.run(["xdotool", "getwindowgeometry", "--shell", str(win)], env=e, capture_output=True,
                           text=True).stdout
        geo = dict(ln.split("=", 1) for ln in g.split() if "=" in ln)
        click(int(geo["WIDTH"]) - 145, int(geo["HEIGHT"]) - 23)
        time.sleep(1.5)
        cfg = Path(e["XDG_CONFIG_HOME"]) / "apple-kb-monitor/config.toml"
        out["config"] = cfg.read_text() if cfg.exists() else None
        out["notifications_after"] = env.screenshot(win, d / "actions-notifications-saved.png")
        if not out["config"] or "enabled = false" not in out["config"]:
            raise Fail(f"Apply did not write enabled = false into {cfg}: {out['config']!r}, see "
                       f"{out['notifications_after']}")
        env.check_log(d)
        env.assert_responsive(stats, "actions")
        return out
    finally:
        env.stop(p, dp, busp)


def sc_systemsettings(env):
    """The module inside System Settings (Input & Output > Keyboard), and found
    by the search field with a French keyword."""
    out = {}
    if not shutil.which("systemsettings"):
        return {"skipped": "systemsettings not installed"}
    d, e, busp = env.scenario_env("systemsettings")
    dp = env.fake_daemon(d, e)
    listed = subprocess.run(["kcmshell6", "--list"], env=e, capture_output=True, text=True).stdout
    if "kcm_applekeyboard" not in listed:
        env.stop(dp, busp)
        raise Fail("kcm_applekeyboard missing from kcmshell6 --list")
    for name, argv in (("module", ["systemsettings", "kcm_applekeyboard"]), ("search", ["systemsettings"])):
        p = env.spawn(argv, d / f"systemsettings-{name}.log", env=e)
        try:
            t0 = time.time()
            win = None
            while time.time() - t0 < 30 and win is None:
                r = subprocess.run(["xdotool", "search", "--onlyvisible", "--class", "systemsettings"], env=e,
                                   capture_output=True, text=True)
                win = r.stdout.split()[-1] if r.stdout.split() else None
                time.sleep(0.3)
            if win is None:
                raise Fail(f"no System Settings window, log: {d / f'systemsettings-{name}.log'}")
            subprocess.run(["xdotool", "windowsize", win, "1300", "850"], env=e, capture_output=True)
            subprocess.run(["xdotool", "windowfocus", "--sync", win], env=e, capture_output=True)
            time.sleep(6)
            if name == "search":
                subprocess.run(["xdotool", "type", "--delay", "40", "batterie"], env=e, capture_output=True)
                time.sleep(2)
            out[name] = env.screenshot(win, d / f"systemsettings-{name}.png")
        finally:
            env.stop(p)
    env.stop(dp, busp)
    return out


SCENARIOS = {"screens": sc_screens, "actions": sc_actions, "systemsettings": sc_systemsettings, "absent": sc_absent, "slow": sc_slow, "error": sc_error,
             "garbage": sc_garbage, "akmctl_slow": sc_akmctl_slow}


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd")
    dm = sub.add_parser("daemon")
    dm.add_argument("--spy", required=True)
    dm.add_argument("--mode", default="normal", choices=["normal", "slow", "error", "garbage"])
    dm.add_argument("--delay", type=float, default=40.0)
    ap.add_argument("--out", default=str(TOP / "scripts/out" / ("kcm-" + time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()))))
    ap.add_argument("--prefix", help="existing install prefix of kcm/ (skip the build)")
    ap.add_argument("--only", default="")
    ap.add_argument("--max-latency-ms", type=float, default=100.0)
    a = ap.parse_args()
    if a.cmd == "daemon":
        run_daemon(a)
        return 0

    names = [n for n in (a.only.split(",") if a.only else SCENARIOS) if n]
    unknown = [n for n in names if n not in SCENARIOS]
    if unknown:
        ap.error(f"--only: unknown scenario(s) {', '.join(unknown)}; known: {', '.join(SCENARIOS)}")
    missing = [t for t in ("Xvfb", "xdotool", "import", "dbus-daemon", "kcmshell6", "cmake") if not shutil.which(t)]
    if importlib.util.find_spec("gi") is None:
        missing.append("python-gobject")
    if missing:
        print("kcm e2e skipped: missing " + " ".join(missing))
        return 77
    sys.path.insert(0, str(HERE))
    env = Env(a)
    failed = []
    try:
        env.build()
        env.start_xvfb()
        for n in names:
            t0 = time.time()
            try:
                r = SCENARIOS[n](env)
                env.results[n] = {"ok": True, "seconds": round(time.time() - t0, 1), **({"detail": r} if r else {})}
                print(f"[ok]   {n} ({time.time() - t0:.0f} s)")
            except Fail as ex:
                env.results[n] = {"ok": False, "error": str(ex)}
                failed.append(n)
                print(f"[FAIL] {n}: {ex}")
    except Fail as ex:
        print(f"[FAIL] setup: {ex}")
        failed.append("setup")
    finally:
        env.teardown()
        (env.out / "kcm-results.json").write_text(json.dumps(env.results, indent=1, ensure_ascii=False))
    print(f"results: {env.out / 'kcm-results.json'}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
