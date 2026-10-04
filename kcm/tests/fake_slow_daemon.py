"""Slow stand-in for apple-kb-monitord on a private session bus (store_timeouts.sh):
GetState answers after 8 s, Slow(ms) after ms, RunAkmctl never answers (a polkit
prompt left open) but the parameter it sets shows up in KeyTable 2 s later;
GetConfig answers after 8 s; Device.SetFnMode changes it at once and emits PropertiesChanged, as the daemon does
(FnModeCalls lists the modes it was given)."""
import json, sys, dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib

BUS = "com.agenceapi.AppleKbMonitor1"
ROOT = "/com/agenceapi/AppleKbMonitor1"
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
params = {"fnmode": 1}
pending = []
fn_calls = []


class Daemon(dbus.service.Object):
    @dbus.service.method(BUS, out_signature="s", async_callbacks=("ok", "err"))
    def GetState(self, ok, err):
        GLib.timeout_add(8000, lambda: ok(json.dumps({"schema": 1, "late": True})) and False)

    @dbus.service.method(BUS, in_signature="u", out_signature="s", async_callbacks=("ok", "err"))
    def Slow(self, ms, ok, err):
        GLib.timeout_add(int(ms), lambda: ok("slow %d" % ms) and False)

    @dbus.service.method(BUS, out_signature="s")
    def FnModeCalls(self):
        return json.dumps(fn_calls)

    @dbus.service.method(BUS + ".Keymap", in_signature="b", out_signature="s")
    def KeyTable(self, _check):
        return json.dumps({"params": params, "keys": []})


class Settings(dbus.service.Object):
    @dbus.service.method(BUS + ".Settings", out_signature="s", async_callbacks=("ok", "err"))
    def GetConfig(self, ok, err):
        GLib.timeout_add(8000, lambda: ok(json.dumps({"values": {}, "revision": "1"})) and False)

    @dbus.service.method(BUS + ".Settings", in_signature="su", out_signature="s", async_callbacks=("ok", "err"))
    def RunAkmctl(self, args, _timeout, ok, err):
        a = json.loads(args)
        pending.append(ok)
        if a[:2] == ["set", "param"]:
            GLib.timeout_add(2000, lambda: params.__setitem__(a[2], int(a[3])) or False)


class Device(dbus.service.Object):
    @dbus.service.method(BUS + ".Device", in_signature="i")
    def SetFnMode(self, mode):
        fn_calls.append(int(mode))
        params["fnmode"] = int(mode)
        self.PropertiesChanged(BUS + ".Device", {"FnMode": dbus.Int32(mode)}, [])

    @dbus.service.signal(dbus.PROPERTIES_IFACE, signature="sa{sv}as")
    def PropertiesChanged(self, iface, changed, invalidated):
        pass


name = dbus.service.BusName(BUS, bus)
Daemon(bus, ROOT)
Settings(bus, ROOT + "/Settings")
Device(bus, ROOT + "/devices/AA_BB_CC_DD_EE_F1")
GLib.MainLoop().run()
