"""Fake daemon (GetState, History, keyboard object, link object, StateChanged)
and fake window (Application.Activate)
on the private session bus of run-widget-tests.sh. Never touches a keyboard."""
import json, sys, time, dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib

DAEMON = "com.agenceapi.AppleKbMonitor1"
WINDOW = "com.agenceapi.AppleKbMonitor"
DEVICE = "com.agenceapi.AppleKbMonitor1.Device"
LINK = "com.agenceapi.AppleKbMonitor1.Link"
MAC = "AA:BB:CC:DD:EE:F1"
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
out = sys.argv[1]


class Daemon(dbus.service.Object):
    n = 0

    @dbus.service.method(DAEMON, out_signature="s")
    def GetState(self):
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
        return json.dumps([
            {"ts": int(since) - 3600, "pct": 99.0},
            {"ts": now - 7200, "pct": 90.0, "voltage": 2.95, "schema": 2},
            {"ts": now - 3600, "pct": 89.0, "voltage": 2.5, "voltage_valid": False},
            {"ts": now - 60, "pct": 88.0, "voltage": 2.93, "schema": 2}])

    @dbus.service.method(DAEMON)
    def Refresh(self):
        with open(out, "a") as f:
            f.write("refresh\n")

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="ss", out_signature="v")
    def Get(self, iface, name):
        if iface == DAEMON and name == "DaemonVersion":
            return "9.8.7"
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
        if mode not in (1, 2):
            raise dbus.exceptions.DBusException("fnmode %d refused" % mode)
        Device.mode = int(mode)
        with open(out, "a") as f:
            f.write("fnmode %d\n" % mode)


class Link(dbus.service.Object):
    @dbus.service.method(LINK, out_signature="s")
    def Status(self):
        return json.dumps([{"mac": MAC, "name": "Clavier de alice", "health": "unreachable",
                            "since": 1, "attempts": 4, "failures": 3,
                            "last_error": "page timeout", "last_reason": "", "updated": 2}])

    @dbus.service.method(LINK, out_signature="b")
    def Reconnect(self):
        with open(out, "a") as f:
            f.write("reconnect\n")
        return True


class Window(dbus.service.Object):
    @dbus.service.method("org.freedesktop.Application", in_signature="a{sv}")
    def Activate(self, platform_data):
        with open(out, "a") as f:
            f.write("activate %d\n" % len(platform_data))



class Tray(dbus.service.Object):
    @dbus.service.method("com.agenceapi.AppleKbMonitor1.Tray", in_signature="s")
    def ClaimTrayFor(self, instance):
        with open(out, "a") as f:
            f.write("claim %s\n" % instance)

    @dbus.service.method("com.agenceapi.AppleKbMonitor1.Tray", in_signature="s")
    def ReleaseTrayFor(self, instance):
        with open(out, "a") as f:
            f.write("release %s\n" % instance)


dn = dbus.service.BusName(DAEMON, bus)
wn = dbus.service.BusName(WINDOW, bus)
d = Daemon(bus, "/com/agenceapi/AppleKbMonitor1")
w = Window(bus, "/com/agenceapi/AppleKbMonitor")
t = Tray(bus, "/com/agenceapi/AppleKbMonitor1/Tray")
k = Device(bus, "/com/agenceapi/AppleKbMonitor1/devices/AA_BB_CC_DD_EE_F1")
l = Link(bus, "/com/agenceapi/AppleKbMonitor1/Link")


def tick():
    d.n += 1
    d.StateChanged(d.n, "{}")
    return True


GLib.timeout_add(700, tick)
GLib.MainLoop().run()
