"""Fake daemon (GetState, History, Diagnose, keyboard object, link object,
tray menu, StateChanged) on the private session
bus of run-widget-tests.sh. Never touches a keyboard.

Optional second argument, for the screenshots of the Pip-Boy popup
(docs/captures/readme): "demo" = a complete state and 90 days of
history; "nosignal" = the same without any RSSI measurement; "offline" = the
keyboard is not connected."""
import json, sys, time, dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib

DAEMON = "com.agenceapi.AppleKbMonitor1"
DEVICE = "com.agenceapi.AppleKbMonitor1.Device"
LINK = "com.agenceapi.AppleKbMonitor1.Link"
MAC = "AA:BB:CC:DD:EE:F1"
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
out = sys.argv[1]
SCENARIO = sys.argv[2] if len(sys.argv) > 2 else ""
NOW = int(time.time())


def demo_state():
    if SCENARIO == "offline":
        return {"schema": 1, "version": 3, "connected": False, "keyboard": None,
                "kb_error": "Keyboard: not found", "last_update": NOW - 7200}
    radio = {} if SCENARIO == "nosignal" else {
        "rssi_rel_db": -3, "rssi_kind": "relative", "rssi_quality": "good", "tx_power_dbm": 4}
    return {
        "schema": 1, "version": 12, "connected": True, "caps_lock": True, "num_lock": False,
        "last_update": NOW - 40, "rssi_at": None if SCENARIO == "nosignal" else NOW - 28,
        "forecast": {"rate_pct_per_day": 0.8, "empty_at": NOW + 41 * 86400, "fitted_pct": 86.0,
                     "span_s": 20 * 86400, "buckets": 300},
        "batteries_installed_at": NOW - 52 * 86400,
        "link_quality": {"disconnects_last_hour": 0, "disconnects_last_day": 2, "disconnects_7d": 5,
                         "unexpected_last_hour": 0, "disconnects_by_hour": [0] * 24,
                         "disconnects_by_day": [0, 1, 0, 2, 0, 0, 2], "unstable": False,
                         "unstable_since": None, "signal_7d": None, "signal_by_day": []},
        "keyboard": {
            "device": {"model": "Apple Wireless Keyboard (A1314)", "name": "Apple Wireless Keyboard",
                       "alias": "alex", "mac": MAC, "chip": "BCM2042", "driver": "hid-apple"},
            "battery": {"percentage": 86.0, "voltage": 2.91, "percentage_interpolated": 71.0,
                        "charge_estimate": {"pct": 70, "low": 60, "high": 80, "chemistry": "alkaline"},
                        "thresholds": {"full_mv": 2954, "low_mv": 2506, "critical_mv": 2404, "empty_mv": 2054},
                        "threshold_level": "ok", "apple_display_pct": 90.0},
            "bluetooth": {"connected": True, "paired": True, "paired_host_addr": "AA:BB:CC:DD:EE:F2"},
            "radio": radio,
            "firmware": {"version": "0x0050", "status": "up_to_date", "latest_known": "0x0050"},
            "wake": {"last_age_s": 300, "count": 4},
        }}


def demo_history(since):
    pts = []
    t = NOW - 90 * 86400
    while t <= NOW:
        age = (NOW - t) / 86400.0
        pct = min(100.0, 86.0 + age * 0.15 + (2.0 if int(t / 3600) % 37 == 0 else 0.0))
        if t >= since:
            pts.append({"ts": t, "pct": round(pct, 1), "voltage": round(2.91 + age * 0.0012, 3), "schema": 2})
        t += 3600 if age < 2 else 4 * 3600
    return json.dumps(pts)


DIAG = {"schema": 1, "daemon_version": "3.1.0", "passed": 8, "total": 9, "checks": [
    {"id": "binary", "label": "Monitoring daemon binary", "ok": True, "detail": "apple-kb-monitord 3.1.0"},
    {"id": "bluez", "label": "BlueZ CLI", "ok": True, "detail": "bluetoothctl: 5.87"},
    {"id": "service", "label": "apple-kb-monitord.service", "ok": True, "detail": "active"},
    {"id": "dbus", "label": "D-Bus com.agenceapi.AppleKbMonitor1", "ok": True, "detail": "daemon reachable"},
    {"id": "hidraw", "label": "hidraw readable", "ok": True, "detail": "/dev/hidraw7: readable"},
    {"id": "mapping", "label": "Key mapping", "ok": False,
     "detail": "no hwdb installed (optional): akmctl keymap install"},
    {"id": "fnmode", "label": "hid_apple fnmode", "ok": True, "detail": "1 (persistent)"},
    {"id": "udev", "label": "udev rules", "ok": True, "detail": "installed"},
    {"id": "rssi_helper", "label": "RSSI helper", "ok": True,
     "detail": "rssi-helper installed (needs CAP_NET_ADMIN)"}]}


class Daemon(dbus.service.Object):
    n = 0

    @dbus.service.method(DAEMON, out_signature="s")
    def GetState(self):
        if SCENARIO:
            return json.dumps(demo_state())
        return json.dumps({"connected": True, "keyboard": {"battery": {
            "percentage": 87.0, "percentage_fine": 12.0, "voltage": 2.987}}})

    @dbus.service.method(DAEMON, in_signature="ss", out_signature="s")
    def SetAlias(self, mac, name):
        with open(out, "a") as f:
            f.write("alias %s %s\n" % (mac, name))
        return name

    @dbus.service.method(DAEMON, in_signature="t", out_signature="s")
    def History(self, since):
        # 3 readings inside the period asked for, 1 before it (must be ignored
        # by the widget whatever the daemon sends), 1 with a legacy voltage.
        now = int(time.time())
        with open(out, "a") as f:
            f.write("history %d\n" % (now - int(since)))
        if SCENARIO:
            return demo_history(int(since))
        return json.dumps([
            {"ts": int(since) - 3600, "pct": 99.0},
            {"ts": now - 7200, "pct": 90.0, "voltage": 2.95, "schema": 2},
            {"ts": now - 3600, "pct": 89.0, "voltage": 2.5, "voltage_valid": False},
            {"ts": now - 60, "pct": 88.0, "voltage": 2.93, "schema": 2}])

    @dbus.service.method(DAEMON, out_signature="s")
    def Diagnose(self):
        with open(out, "a") as f:
            f.write("diagnose\n")
        return json.dumps(DIAG)

    @dbus.service.method(DAEMON)
    def Refresh(self):
        with open(out, "a") as f:
            f.write("refresh\n")

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="ss", out_signature="v")
    def Get(self, iface, name):
        if iface == DAEMON and name == "DaemonVersion":
            return "3.1.0" if SCENARIO else "9.8.7"
        raise dbus.exceptions.DBusException("no such property %s.%s" % (iface, name))

    @dbus.service.signal(DAEMON, signature="ts")
    def StateChanged(self, rev, js):
        pass


class Device(dbus.service.Object):
    """Keyboard object: FnMode property and SetFnMode (polkit is the daemon's)."""
    mode = 1

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="ss", out_signature="v")
    def Get(self, iface, name):
        if iface == DEVICE and name == "FnMode":
            return dbus.Int32(self.mode)
        raise dbus.exceptions.DBusException("no such property %s.%s" % (iface, name))

    @dbus.service.method(DEVICE, in_signature="i")
    def SetFnMode(self, mode):
        if mode == 3:  # polkit answered after the bus gave up: NoReply, applied 2 s later
            with open(out, "a") as f:
                f.write("late-fnmode 3\n")
            GLib.timeout_add(2000, lambda: self.announce(3) or False)
            raise dbus.exceptions.DBusException("no reply in time", name="org.freedesktop.DBus.Error.NoReply")
        if mode not in (1, 2):
            raise dbus.exceptions.DBusException("fnmode %d refused" % mode)
        self.announce(int(mode))
        with open(out, "a") as f:
            f.write("fnmode %d\n" % mode)

    def announce(self, mode):
        """As the daemon: the new mode, then PropertiesChanged on the keyboard object."""
        Device.mode = mode
        self.PropertiesChanged(DEVICE, {"FnMode": dbus.Int32(mode)}, [])

    @dbus.service.signal(dbus.PROPERTIES_IFACE, signature="sa{sv}as")
    def PropertiesChanged(self, iface, changed, invalidated):
        pass


class Link(dbus.service.Object):
    @dbus.service.method(LINK, out_signature="s")
    def Status(self):
        if SCENARIO:
            return json.dumps([{"mac": MAC, "name": "alex", "health": "connected",
                                "since": 1, "attempts": 1, "failures": 0, "last_error": "",
                                "last_reason": "", "updated": 2}])
        return json.dumps([{"mac": MAC, "name": "alex", "health": "unreachable",
                            "since": 1, "attempts": 4, "failures": 3,
                            "last_error": "page timeout", "last_reason": "", "updated": 2}])

    @dbus.service.method(LINK, out_signature="b")
    def Reconnect(self):
        with open(out, "a") as f:
            f.write("reconnect\n")
        return True


class Tray(dbus.service.Object):
    @dbus.service.signal("com.agenceapi.AppleKbMonitor1.Tray")
    def PanelRequested(self):
        pass

    # Same entries and labels as View::menu_json (English), for the screenshots.
    @dbus.service.method("com.agenceapi.AppleKbMonitor1.Tray", out_signature="s")
    def MenuItems(self):
        with open(out, "a") as f:
            f.write("menuitems\n")
        on = SCENARIO != "offline"
        rows = [(12, "Refresh", on, None),
                (13, "Copy information", True, None), (14, "Bluetooth settings\u2026", True, None),
                (15, "Rename keyboard\u2026", True, None), (16, "Repair the link\u2026", True, None),
                (19, "Media keys first (Fn + F1 = F1)", True, False),
                (22, "F1\u2013F12 first (Fn + F1 = media)", True, True),
                (23, "Reconnect", not on, None), (24, "Disconnect", on, None),
                (25, "Forget this keyboard\u2026", True, None)]
        return json.dumps([{"id": i, "label": lb, "enabled": en, "visible": True, "separator": False,
                            "checked": ck} for i, lb, en, ck in rows])

    @dbus.service.method("com.agenceapi.AppleKbMonitor1.Tray", in_signature="i")
    def ActivateMenuItem(self, item):
        with open(out, "a") as f:
            f.write("menu %d\n" % item)


dn = dbus.service.BusName(DAEMON, bus)
d = Daemon(bus, "/com/agenceapi/AppleKbMonitor1")
t = Tray(bus, "/com/agenceapi/AppleKbMonitor1/Tray")
k = Device(bus, "/com/agenceapi/AppleKbMonitor1/devices/AA_BB_CC_DD_EE_F1")
l = Link(bus, "/com/agenceapi/AppleKbMonitor1/Link")


def tick():
    d.n += 1
    d.StateChanged(d.n, "{}")
    return True


GLib.timeout_add(700, tick)
GLib.timeout_add(2500, lambda: t.PanelRequested() and False)
GLib.MainLoop().run()
