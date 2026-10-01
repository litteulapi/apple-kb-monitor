#!/usr/bin/env python3
"""Session: fake StatusNotifierWatcher (logs registrations). System: fake UPower
(UPOWER=hang never answers EnumerateDevices, else returns no device)."""
import os, sys, dbus, dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib
DBusGMainLoop(set_as_default=True)
held = []
def log(m): sys.stderr.write(m + "\n"); sys.stderr.flush()
class Watcher(dbus.service.Object):
    @dbus.service.method("org.kde.StatusNotifierWatcher", in_signature="s", out_signature="")
    def RegisterStatusNotifierItem(self, name):
        log(f"[fake] RegisterStatusNotifierItem({name})")
class UPower(dbus.service.Object):
    @dbus.service.method("org.freedesktop.UPower", in_signature="", out_signature="ao",
                         async_callbacks=("ok", "err"))
    def EnumerateDevices(self, ok, err):
        log("[fake] UPower.EnumerateDevices")
        if os.environ.get("UPOWER") == "hang":
            held.append(ok)
        else:
            ok(dbus.Array([], signature="o"))
s = dbus.SessionBus(); y = dbus.SystemBus()
a = dbus.service.BusName("org.kde.StatusNotifierWatcher", s); w = Watcher(s, "/StatusNotifierWatcher")
b = dbus.service.BusName("org.freedesktop.UPower", y); u = UPower(y, "/org/freedesktop/UPower")
log("[fake] ready"); GLib.MainLoop().run()
