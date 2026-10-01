#!/usr/bin/env python3
"""Fake apple-kb-monitord + fake XDG portal on a PRIVATE session bus.

Never touches hardware. Knobs (env):
  FAKE_DAEMON=0           do not export the daemon
  HIST_MODE=normal|const|single|huge|nan|future|dup|empty|zero_span
  HIST_N=<n>              number of history points
  HIST_DELAY=<s>          sleep before answering History()
  JSON_DELAY=<s>          sleep before answering Get(Json)
  ALIAS_DELAY=<s>         sleep before answering SetAlias
  RAW_N=<n>               number of raw vendor entries in the snapshot
  LONG=<n>                length of model/name strings
  PCT=<float>             battery percentage (may be 'nan' -> null)
  PORTAL=0                do not export the portal
  PORTAL_DELAY=<s>        sleep before answering portal Read()
  SIGNAL_EVERY=<s>        emit StateChanged every s seconds
"""
import json, os, sys, time, math
import dbus, dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
E = os.environ.get
NAME = "com.agenceapi.AppleKbMonitor1"
PATH = "/com/agenceapi/AppleKbMonitor1"
now = int(time.time())
CALLS = [0]


def history():
    mode = E("HIST_MODE", "normal")
    n = int(E("HIST_N", "200"))
    out = []
    if mode == "empty":
        return out
    for i in range(n):
        ts = now - 23 * 3600 + int(i * (23 * 3600) / max(n, 1))
        pct = 100 - 30 * i / max(n, 1)
        v = 3.0 - 0.4 * i / max(n, 1)
        if mode == "const":
            pct, v = 96.0, 2.9
        elif mode == "single":
            if i > 0:
                break
        elif mode == "zero_span":
            ts = now - 100
        elif mode == "future":
            ts = now + 10**9 + i
        elif mode == "dup":
            ts = now - 10
        elif mode == "tiny":
            v = 2.9 + i * 1e-12
        out.append({"ts": ts, "pct": pct, "voltage": v, "schema": 2})
    if mode == "mixfuture":  # one line written while the clock was 30 days ahead (#166)
        out.append({"ts": now + 30 * 86400, "pct": 50.0, "voltage": 2.7, "schema": 2})
    return out


def snapshot():
    long = int(E("LONG", "0"))
    raw_n = int(E("RAW_N", "2"))
    pct = E("PCT", "96")
    p = None if pct == "nan" else float(pct)
    model = "Apple Wireless Keyboard (A1314) [FAKE]" + ("W" * long)
    kb = {
        "device": {"model": model, "mac": "00:00:00:00:00:00",
                   "name": "Fake" + ("é" * long), "driver": "hid-apple",
                   "chip": "BCM2042"},
        "battery": {"percentage": p, "voltage": 2.9},
        "radio": {"rssi_dbm": 0, "tx_power_dbm": 8},
        "bluetooth": {"connected": True},
        "firmware": {"version": "0x0069"},
        "raw": {f"0x{i:04X}": "AB" * 32 for i in range(raw_n)},
    }
    return {"connected": True, "keyboard": kb, "last_update": now - 60}


class Daemon(dbus.service.Object):
    @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="ss", out_signature="v",
                         async_callbacks=("ok", "err"))
    def Get(self, iface, prop, ok, err):
        # asynchronous: a slow Json does not delay the other methods
        val = dbus.String(json.dumps(snapshot()) if prop == "Json" else "")
        GLib.timeout_add(int(float(E("JSON_DELAY", "0")) * 1000), lambda: (ok(val), False)[1])

    @dbus.service.method(NAME, in_signature="", out_signature="s")
    def GetState(self):
        return json.dumps(snapshot())

    @dbus.service.method(NAME, in_signature="t", out_signature="s")
    def History(self, since):
        CALLS[0] += 1
        d = float(E("HIST_DELAY", "0")) if CALLS[0] == 1 else float(E("HIST_DELAY2", E("HIST_DELAY", "0")))
        sys.stderr.write(f"[fake] History({since}) delay={d}\n"); sys.stderr.flush()
        time.sleep(d)
        return json.dumps(history())

    @dbus.service.method(NAME, in_signature="ss", out_signature="s")
    def SetAlias(self, mac, name):
        time.sleep(float(E("ALIAS_DELAY", "0")))
        return name

    @dbus.service.signal(NAME, signature="ts")
    def StateChanged(self, rev, js):
        pass


class Portal(dbus.service.Object):
    @dbus.service.method("org.freedesktop.portal.Settings", in_signature="ss", out_signature="v")
    def Read(self, ns, key):
        d = float(E("PORTAL_DELAY", "0"))
        sys.stderr.write(f"[fake] portal Read({key}) delay={d}\n"); sys.stderr.flush()
        time.sleep(d)
        if key == "color-scheme":
            return dbus.UInt32(int(E("SCHEME", "1")), variant_level=1)
        return dbus.Struct((0.2, 0.4, 0.9), signature="ddd", variant_level=1)

    @dbus.service.signal("org.freedesktop.portal.Settings", signature="ssv")
    def SettingChanged(self, ns, key, v):
        pass


objs = []
if E("PORTAL", "1") == "1":
    pn = dbus.service.BusName("org.freedesktop.portal.Desktop", bus)
    objs.append((pn, Portal(bus, "/org/freedesktop/portal/desktop")))
if E("FAKE_DAEMON", "1") == "1":
    dn = dbus.service.BusName(NAME, bus)
    d = Daemon(bus, PATH)
    objs.append((dn, d))
    every = float(E("SIGNAL_EVERY", "0"))
    if every > 0:
        rev = [0]
        def tick():
            rev[0] += 1
            d.StateChanged(dbus.UInt64(rev[0]), json.dumps(snapshot()))
            return True
        GLib.timeout_add(int(every * 1000), tick)
class Watcher(dbus.service.Object):
    """StatusNotifierWatcher whose Properties.Get never answers (hung plasmashell)."""
    @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="ss", out_signature="v",
                         async_callbacks=("ok", "err"))
    def Get(self, iface, prop, ok, err):
        sys.stderr.write("[fake] watcher Get (never answered)\n"); sys.stderr.flush()
        HELD.append((ok, err))


HELD = []
if E("WATCHER", "") == "hang":
    wn = dbus.service.BusName("org.kde.StatusNotifierWatcher", bus)
    objs.append((wn, Watcher(bus, "/StatusNotifierWatcher")))

sys.stderr.write("[fake] ready\n"); sys.stderr.flush()
GLib.MainLoop().run()
