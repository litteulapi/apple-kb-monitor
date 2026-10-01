"""Fake daemon (GetState + StateChanged) and fake window (Application.Activate)
on the private session bus of run-widget-tests.sh. Never touches a keyboard."""
import json, sys, dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib

DAEMON = "com.agenceapi.AppleKbMonitor1"
WINDOW = "com.agenceapi.AppleKbMonitor"
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

    @dbus.service.signal(DAEMON, signature="ts")
    def StateChanged(self, rev, js):
        pass


class Window(dbus.service.Object):
    @dbus.service.method("org.freedesktop.Application", in_signature="a{sv}")
    def Activate(self, platform_data):
        with open(out, "a") as f:
            f.write("activate %d\n" % len(platform_data))



dn = dbus.service.BusName(DAEMON, bus)
wn = dbus.service.BusName(WINDOW, bus)
d = Daemon(bus, "/com/agenceapi/AppleKbMonitor1")
w = Window(bus, "/com/agenceapi/AppleKbMonitor")


def tick():
    d.n += 1
    d.StateChanged(d.n, "{}")
    return True


GLib.timeout_add(700, tick)
GLib.MainLoop().run()
