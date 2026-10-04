// The key table follows a Fn mode changed elsewhere (widget, shortcut, menu): Device PropertiesChanged.
// A GetConfig past its deadline is a bus error, not an unreadable settings file.
// A Fn mode set by the module is handed to Device.SetFnMode, which remembers it.
import QtQuick
import org.kde.plasma.workspace.dbus as DBus
import "../ui"

Item {
    id: test

    function fail(why) {
        console.log("FAIL store-props: " + why);
        Qt.exit(1);
    }

    Store { id: store }

    Timer {
        interval: 1500; running: true
        onTriggered: {
            if (!store.keyTable) return test.fail("no key table at start");
            // GetState of the fake daemon is too slow: the followed keyboard is given here
            store.getConfig(function () {});
            store.state = { keyboard: { device: { mac: "aa:bb:cc:dd:ee:f1" } } };
            if (store.devicePath !== store.root + "/devices/AA_BB_CC_DD_EE_F1") return test.fail("devicePath = " + store.devicePath);
            DBus.SessionBus.asyncCall({ service: store.bus, path: store.root + "/devices/AA_BB_CC_DD_EE_F1", iface: store.bus + ".Device",
                                        member: "SetFnMode", arguments: [new DBus.int32(3)] });
            check.start();
        }
    }
    Timer {
        id: check
        interval: 6000
        onTriggered: {
            const p = store.keyTable && store.keyTable.params;
            if (!p || p.fnmode !== 3) return test.fail("key table not reloaded: fnmode = " + (p ? p.fnmode : "?"));
            if (store.configBusError !== "timeout" || store.configError !== "" || store.configLoaded)
                return test.fail("GetConfig timeout: configBusError=" + store.configBusError + " configError=" + store.configError);
            // A Fn mode set from the module reaches the daemon too, which remembers it.
            store.setParam("fnmode", 0, function (ok, msg) {
                if (!ok) return test.fail("setParam: " + msg);
                remembered.start();
            });
        }
    }
    Timer {
        id: remembered
        interval: 1000
        onTriggered: {
            DBus.SessionBus.asyncCall({ service: store.bus, path: store.root, iface: store.bus, member: "FnModeCalls", arguments: [] },
                function (reply) {
                    const calls = JSON.parse(String(reply.value));
                    if (calls[calls.length - 1] !== 0) return test.fail("SetFnMode not called after akmctl: " + reply.value);
                    console.log("PASS store-props");
                    Qt.exit(0);
                }, function (e) { test.fail("FnModeCalls: " + e); });
        }
    }
}
