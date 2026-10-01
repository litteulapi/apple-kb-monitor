#!/usr/bin/env python3
"""Fake D-Bus services for the end-to-end tests (python3-gobject / Gio only).

    fakes.py system --spy FILE                   fake BlueZ + UPower on a private "system" bus
    fakes.py daemon --spy FILE --mode MODE [--history FILE] [--die-after S] [--delay S]
    fakes.py desktop --spy FILE [--delay S]      settings portal + StatusNotifierWatcher

MODE of the fake apple-kb-monitord (com.agenceapi.AppleKbMonitor1):
  normal   answers at once, StateChanged every second (varying values)
  static   answers at once, never changes (reference screenshots)
  slow     every call answers after --delay seconds (default 3): a daemon
           that is alive but busy must never freeze a client
  die      behaves normally, then exits abruptly after --die-after seconds
  garbage  Json/GetState return invalid JSON, History a non-array

Every method call received is appended to the spy file as one JSON line
({"t":..., "bus":..., "dest":..., "path":..., "iface":..., "method":...}):
the tests fail on any call that could change the keyboard or the pairing.
"""

import argparse
import json
import os
import sys
import time

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

MAC = "04:DB:56:00:E2:E0"  # anonymised (Apple OUI kept for model detection)
DEV_PATH = "/org/bluez/hci0/dev_" + MAC.replace(":", "_")

SPY = None


def spy(bus, dest, path, iface, method, args=None):
    rec = {"t": round(time.time(), 3), "bus": bus, "dest": dest, "path": path, "iface": iface, "method": method}
    if args is not None:
        rec["args"] = str(args)[:200]
    with open(SPY, "a") as f:
        f.write(json.dumps(rec) + "\n")


def own(conn, name):
    r = conn.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName",
                       GLib.Variant("(su)", (name, 4)), GLib.VariantType("(u)"), 0, -1, None)
    if r.unpack()[0] != 1:
        sys.exit(f"cannot own {name}")


# ── fake BlueZ / UPower ─────────────────────────────────────────────────────

BLUEZ_XML = """
<node>
 <interface name="org.freedesktop.DBus.ObjectManager">
  <method name="GetManagedObjects"><arg type="a{oa{sa{sv}}}" direction="out"/></method>
 </interface>
 <interface name="org.freedesktop.DBus.Properties">
  <method name="Get"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
  <method name="GetAll"><arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method>
  <method name="Set"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/></method>
 </interface>
 <interface name="org.bluez.Device1">
  <method name="Connect"/><method name="Disconnect"/><method name="Pair"/><method name="CancelPairing"/>
  <method name="ConnectProfile"><arg type="s" direction="in"/></method>
  <method name="DisconnectProfile"><arg type="s" direction="in"/></method>
 </interface>
 <interface name="org.bluez.Adapter1">
  <method name="RemoveDevice"><arg type="o" direction="in"/></method>
  <method name="StartDiscovery"/><method name="StopDiscovery"/>
 </interface>
 <interface name="org.bluez.BatteryProviderManager1">
  <method name="RegisterBatteryProvider"><arg type="o" direction="in"/></method>
  <method name="UnregisterBatteryProvider"><arg type="o" direction="in"/></method>
 </interface>
 <interface name="org.freedesktop.UPower">
  <method name="EnumerateDevices"><arg type="ao" direction="out"/></method>
 </interface>
</node>"""


def bluez_objects():
    s, b, o, u = (lambda v: GLib.Variant("s", v)), (lambda v: GLib.Variant("b", v)), (lambda v: GLib.Variant("o", v)), (lambda v: GLib.Variant("u", v))
    adapter = {"Address": s("00:1A:7D:00:00:01"), "Name": s("e2e"), "Powered": b(True), "Discoverable": b(False),
               "Pairable": b(True), "Class": u(0x6c010c), "Alias": s("e2e")}
    dev = {"Address": s(MAC), "AddressType": s("public"), "Name": s("Clavier de test"), "Alias": s("Clavier de test"),
           "Class": u(0x2540), "Icon": s("input-keyboard"), "Paired": b(True), "Bonded": b(True), "Trusted": b(True),
           "Blocked": b(False), "Connected": b(True), "LegacyPairing": b(False), "Adapter": o("/org/bluez/hci0"),
           "Modalias": s("usb:v05ACp0256d0050"), "WakeAllowed": b(True), "ServicesResolved": b(True),
           "UUIDs": GLib.Variant("as", ["00001124-0000-1000-8000-00805f9b34fb"])}
    return {
        "/org/bluez/hci0": {"org.bluez.Adapter1": adapter, "org.bluez.BatteryProviderManager1": {}},
        DEV_PATH: {"org.bluez.Device1": dev, "org.bluez.Input1": {"ReconnectMode": s("any")}},
    }


def run_system():
    conn = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
    info = Gio.DBusNodeInfo.new_for_xml(BLUEZ_XML)
    objs = bluez_objects()

    def handler(c, sender, path, iface, method, params, inv):
        spy("system", "org.bluez", path, iface, method, params.unpack() if params else None)
        if iface == "org.freedesktop.DBus.ObjectManager":
            inv.return_value(GLib.Variant("(a{oa{sa{sv}}})", (objs,)))
        elif method == "GetAll":
            inv.return_value(GLib.Variant("(a{sv})", (objs.get(path, {}).get(params.unpack()[0], {}),)))
        elif method == "Get":
            i, p = params.unpack()
            v = objs.get(path, {}).get(i, {}).get(p)
            if v is None:
                inv.return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", f"no property {p}")
            else:
                inv.return_value(GLib.Variant("(v)", (v,)))
        elif method in ("RegisterBatteryProvider", "UnregisterBatteryProvider"):
            inv.return_value(None)
        elif method == "EnumerateDevices":
            inv.return_value(GLib.Variant("(ao)", ([],)))
        else:
            # Connect/Pair/RemoveDevice/Set...: recorded, refused (never needed by a monitor UI).
            inv.return_dbus_error("org.bluez.Error.NotPermitted", "e2e fake: refused and recorded")

    for path in ["/", "/org/bluez", "/org/bluez/hci0", DEV_PATH, "/org/freedesktop/UPower"]:
        for i in info.interfaces:
            conn.register_object(path, i, handler, None, None)
    own(conn, "org.bluez")
    own(conn, "org.freedesktop.UPower")
    print("ready", flush=True)
    GLib.MainLoop().run()


# ── fake apple-kb-monitord ──────────────────────────────────────────────────

DAEMON_XML = """
<node>
 <interface name="com.agenceapi.AppleKbMonitor1">
  <method name="GetState"><arg type="s" direction="out"/></method>
  <method name="Refresh"/>
  <method name="SetAlias"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
  <method name="History"><arg type="t" direction="in"/><arg type="s" direction="out"/></method>
  <method name="GetDevices"><arg type="ao" direction="out"/></method>
  <method name="BatterySets"><arg type="s" direction="out"/></method>
  <signal name="StateChanged"><arg type="t"/><arg type="s"/></signal>
  <property name="Json" type="s" access="read"/>
  <property name="Battery" type="i" access="read"/>
  <property name="Connected" type="b" access="read"/>
  <property name="Revision" type="t" access="read"/>
  <property name="InterfaceVersion" type="u" access="read"/>
  <property name="DaemonVersion" type="s" access="read"/>
  <property name="LastUpdate" type="t" access="read"/>
 </interface>
 <interface name="com.agenceapi.AppleKbMonitor1.Link">
  <method name="Status"><arg type="s" direction="out"/></method>
 </interface>
</node>"""


def snapshot(rev):
    now = int(time.time())
    pct = 87.0 - (rev % 7)
    return {
        "schema": 1, "version": rev, "connected": True,
        "keyboard": {
            "wake": {"last_age_s": None, "count": 0},
            "device": {"model": "Apple Wireless Keyboard (A1314, aluminum, ISO)", "name": "Clavier de test",
                       "alias": "Clavier de test avec un nom tres long pour verifier les debordements",
                       "mac": MAC, "chip": "BCM2042", "driver": "hid-apple"},
            "battery": {"percentage": pct, "percentage_fine": pct + 0.4, "percentage_estimate": None,
                        "percentage_interpolated": None, "voltage": 2.91, "voltage_mv": 2910,
                        "voltage_filtered_mv": 2905, "voltage_doubtful": False, "adc_raw": None,
                        "charge_estimate": None, "new_batteries": False},
            "bluetooth": {"connected": True, "paired": True, "rssi_dbus": None, "tx_power_dbus": None,
                          "paired_host_addr": None},
            "radio": {"rssi_dbm": -(rev % 4), "rssi_rel_db": -(rev % 4), "rssi_kind": "bredr-golden-range",
                      "rssi_quality": "excellent", "tx_power_dbm": 8},
            "firmware": {"version": None}, "raw": {}, "incomplete": False,
        },
        "kb_error": None, "caps_lock": rev % 2 == 1, "num_lock": False,
        "remaining_display": "~41 jours", "rssi_at": now - 5, "last_update": now - 60,
        "last_error": None, "forecast": None, "batteries_installed_at": now - 40 * 86400,
    }


def run_daemon(a):
    conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    info = Gio.DBusNodeInfo.new_for_xml(DAEMON_XML)
    state = {"rev": 1}
    history = "[]"
    if a.history:
        entries = []
        with open(a.history, "rb") as f:
            for line in f:
                try:
                    entries.append(json.loads(line))
                except ValueError:
                    pass
        history = json.dumps(entries)
    garbage = a.mode == "garbage"

    def slow():
        if a.mode == "slow":
            time.sleep(a.delay)

    def js():
        return '{"schema":1,"version":' if garbage else json.dumps(snapshot(state["rev"]))

    def method(c, sender, path, iface, name, params, inv):
        spy("session", "com.agenceapi.AppleKbMonitor1", path, iface, name, params.unpack() if params else None)
        slow()
        if name == "GetState":
            inv.return_value(GLib.Variant("(s)", (js(),)))
        elif name == "History":
            inv.return_value(GLib.Variant("(s)", ('{"not":"an array"}' if garbage else history,)))
        elif name == "Status":
            inv.return_value(GLib.Variant("(s)", (json.dumps([{"mac": MAC, "health": "connected"}]),)))
        elif name == "GetDevices":
            inv.return_value(GLib.Variant("(ao)", ([],)))
        elif name == "BatterySets":
            inv.return_value(GLib.Variant("(s)", ("[]",)))
        elif name == "Refresh":
            inv.return_value(None)
        else:  # SetAlias: a monitor must never rename on its own
            inv.return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "e2e fake: recorded")

    def get_prop(c, sender, path, iface, prop):
        spy("session", "com.agenceapi.AppleKbMonitor1", path, "org.freedesktop.DBus.Properties", "Get:" + prop)
        slow()
        return {
            "Json": GLib.Variant("s", js()), "Battery": GLib.Variant("i", 87), "Connected": GLib.Variant("b", True),
            "Revision": GLib.Variant("t", state["rev"]), "InterfaceVersion": GLib.Variant("u", 1),
            "DaemonVersion": GLib.Variant("s", "e2e"), "LastUpdate": GLib.Variant("t", int(time.time()) - 60),
        }.get(prop)

    conn.register_object("/com/agenceapi/AppleKbMonitor1", info.interfaces[0], method, get_prop, None)
    conn.register_object("/com/agenceapi/AppleKbMonitor1/Link", info.interfaces[1], method, None, None)
    own(conn, "com.agenceapi.AppleKbMonitor1")
    print("ready", flush=True)

    def tick():
        state["rev"] += 1
        conn.emit_signal(None, "/com/agenceapi/AppleKbMonitor1", "com.agenceapi.AppleKbMonitor1", "StateChanged",
                         GLib.Variant("(ts)", (state["rev"], js())))
        return True

    if a.mode != "static":
        GLib.timeout_add(1000, tick)
    if a.mode == "die":
        GLib.timeout_add(int(a.die_after * 1000), lambda: os._exit(3))
    GLib.MainLoop().run()


# ── fake desktop: XDG settings portal + StatusNotifierWatcher ───────────────

DESKTOP_XML = """
<node>
 <interface name="org.freedesktop.portal.Settings">
  <method name="Read"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
  <method name="ReadOne"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
  <method name="ReadAll"><arg type="as" direction="in"/><arg type="a{sa{sv}}" direction="out"/></method>
  <signal name="SettingChanged"><arg type="s"/><arg type="s"/><arg type="v"/></signal>
 </interface>
 <interface name="org.kde.StatusNotifierWatcher">
  <method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
  <method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
  <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
  <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
  <property name="ProtocolVersion" type="i" access="read"/>
 </interface>
</node>"""


def run_desktop(a):
    """A desktop whose services answer after --delay s (0 = at once): a slow
    portal or tray host must never freeze the window."""
    conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    info = Gio.DBusNodeInfo.new_for_xml(DESKTOP_XML)

    def later(fn):
        # Answer after the delay WITHOUT blocking the loop: every call is slow,
        # none waits for another (as a busy but alive service).
        if a.delay > 0:
            GLib.timeout_add(int(a.delay * 1000), lambda: fn() and False)
        else:
            fn()

    def method(c, sender, path, iface, name, params, inv):
        spy("session", "desktop", path, iface, name, params.unpack() if params else None)

        def reply():
            if name in ("Read", "ReadOne"):
                ns, key = params.unpack()
                if key == "color-scheme":
                    v = GLib.Variant("u", 1)
                elif key == "accent-color":
                    v = GLib.Variant("(ddd)", (0.2, 0.4, 0.8))
                else:
                    inv.return_dbus_error("org.freedesktop.portal.Error.NotFound", "no such key")
                    return
                inv.return_value(GLib.Variant("(v)", (GLib.Variant("v", v) if name == "Read" else v,)))
            elif name == "ReadAll":
                inv.return_value(GLib.Variant("(a{sa{sv}})", ({},)))
            else:
                inv.return_value(None)

        later(reply)

    def get_prop(c, sender, path, iface, prop):
        spy("session", "desktop", path, iface, "Get:" + prop)
        time.sleep(a.delay)
        return {"IsStatusNotifierHostRegistered": GLib.Variant("b", True),
                "RegisteredStatusNotifierItems": GLib.Variant("as", []),
                "ProtocolVersion": GLib.Variant("i", 0)}.get(prop)

    conn.register_object("/org/freedesktop/portal/desktop", info.interfaces[0], method, None, None)
    conn.register_object("/StatusNotifierWatcher", info.interfaces[1], method, get_prop, None)
    own(conn, "org.freedesktop.portal.Desktop")
    own(conn, "org.kde.StatusNotifierWatcher")
    print("ready", flush=True)
    GLib.MainLoop().run()


def main():
    global SPY
    p = argparse.ArgumentParser()
    p.add_argument("what", choices=["system", "daemon", "desktop"])
    p.add_argument("--spy", required=True)
    p.add_argument("--mode", default="normal", choices=["normal", "static", "slow", "die", "garbage"])
    p.add_argument("--history")
    p.add_argument("--die-after", type=float, default=10.0)
    p.add_argument("--delay", type=float, default=3.0)
    a = p.parse_args()
    SPY = a.spy
    {"system": run_system, "daemon": lambda: run_daemon(a), "desktop": lambda: run_desktop(a)}[a.what]()


if __name__ == "__main__":
    main()
