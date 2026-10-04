// Store.qml with a module caller (kcm.busCall): every call goes through it with
// its own deadline, and its NoReply is the page's "timeout" (store_timeouts.sh,
// with fake_slow_daemon.py owning the name: an absent daemon is never called).
import QtQuick
import "../ui"

Item {
    id: test
    property var seen: []
    property var limits: []

    function fail(why) {
        console.log("FAIL store-caller: " + why);
        Qt.exit(1);
    }

    Store {
        id: store
        caller: ({
            call: function (service, path, iface, method, sig, args, timeout, cb) {
                test.seen.push(method + "/" + timeout);
                if (method === "RunAkmctl") test.limits.push(args[1] + "<" + timeout);
                if (method === "Apply") cb(false, null, "org.freedesktop.DBus.Error.NoReply", "no reply");
                else if (method === "RunAkmctl" && String(args[0]).indexOf("badjson") >= 0) cb(true, "not json", "", "");
                else if (method === "RunAkmctl") cb(true, '{"code":0,"out":"{}","err":""}', "", "");
                else cb(true, "rev2", "", "");
            }
        })
    }

    Timer {
        interval: 50; repeat: true; running: true
        property int tries: 0
        onTriggered: {
            if (store.present) { stop(); test.run(); }
            else if (++tries > 100) test.fail("the fake daemon never appeared");
        }
    }

    function run() {
        let applied = "";
        store.applyKeymap(function (ok, msg) { applied = ok ? "ok" : msg; });
        store.dbus(store.settingsPath, store.settingsIface, "SetConfig", "sss", ["a", "1", ""], store.cmdTimeout, function (ok, value) {
            if (!ok || value !== "rev2") test.fail("SetConfig reply " + ok + " " + value);
        });
        if (test.seen.indexOf("Apply/" + store.authTimeout) < 0) test.fail("Apply not called with its deadline: " + test.seen);
        if (test.seen.indexOf("SetConfig/" + store.cmdTimeout) < 0) test.fail("SetConfig not called with its deadline: " + test.seen);
        if (applied !== "timeout") test.fail("NoReply gave " + applied);
        let bad = "";
        store.cmd("akmctl", ["badjson"], 1000, function (code, out, err) { bad = code + " " + err; });
        if (!bad.startsWith("-1 invalid: ")) test.fail("unparsable RunAkmctl reply gave " + bad);
        store.cmd("akmctl", ["doctor", "--json"], 60000, function () {});
        if (test.limits.indexOf("57000<60000") < 0) test.fail("RunAkmctl limit not below the deadline: " + test.limits);
        console.log("PASS store-caller");
        Qt.exit(0);
    }
}
