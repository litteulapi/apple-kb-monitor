// SPDX-License-Identifier: GPL-2.0-or-later
//
// Data of the module: every read and write goes through apple-kb-monitord on the session bus.
import QtQuick
import org.kde.plasma.workspace.dbus as DBus

QtObject {
    id: store

    readonly property string bus: "com.agenceapi.AppleKbMonitor1"
    readonly property string root: "/com/agenceapi/AppleKbMonitor1"
    readonly property string keymapIface: bus + ".Keymap"
    readonly property string settingsIface: bus + ".Settings"
    readonly property string settingsPath: root + "/Settings"
    readonly property string linkIface: bus + ".Link"
    readonly property string linkPath: root + "/Link"
    readonly property int readTimeout: 5000
    readonly property int cmdTimeout: 20000
    readonly property int longCmdTimeout: 60000
    readonly property int authTimeout: 120000
    readonly property bool present: _watch.registered
    // BusCall of the module (kcm.busCall): calls with their own deadline. Without it
    // (QML tests), the bus gives up after 25 s whatever the deadline.
    property var caller: null

    property var state: null
    property string stateError: ""
    property bool stateBusy: false
    property bool stateAgain: false
    property string daemonVersion: ""
    property var keyTable: null
    property string keyTableError: ""
    property bool keyTableBusy: false
    property var keymap: null
    property string keymapError: ""  // a failureText() token or a D-Bus error
    property bool configLoaded: false
    property string configRevision: ""
    property string configPath: ""  // given by a daemon that reports it
    property string configError: ""
    // GetConfig not answered or refused by the bus: not a file error
    property string configBusError: ""
    property bool keyTableTimeout: false

    readonly property DBus.DBusServiceWatcher _watch: DBus.DBusServiceWatcher {
        busType: DBus.BusType.Session
        watchedService: store.bus
        onRegisteredChanged: {
            if (registered) {
                store.refreshAll();
            } else {
                store.state = null;
                store.stateError = "";
                store.daemonVersion = "";
            }
        }
    }
    readonly property DBus.SignalWatcher _signals: DBus.SignalWatcher {
        busType: DBus.BusType.Session
        service: store.bus
        path: store.root
        iface: store.bus
        enabled: store.present
        function dbusStateChanged() {
            store.fetchState();
        }
    }
    // Fn mode set from the widget, the global shortcut or the menu: PropertiesChanged on the followed keyboard.
    readonly property string devicePath: {
        const d = state && state.keyboard && state.keyboard.device ? state.keyboard.device : null;
        const mac = d && typeof d.mac === "string" ? d.mac : "";
        return /^([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}$/.test(mac) ? root + "/devices/" + mac.toUpperCase().replace(/:/g, "_") : "";
    }
    readonly property DBus.SignalWatcher _deviceProps: DBus.SignalWatcher {
        busType: DBus.BusType.Session
        service: store.bus
        path: store.devicePath
        iface: "org.freedesktop.DBus.Properties"
        enabled: store.present && store.devicePath !== ""
        function dbusPropertiesChanged(iface) {
            if (String(iface) === store.bus + ".Device") store._keysSoon.restart();
        }
    }
    readonly property Timer _keysSoon: Timer {
        interval: 300
        onTriggered: store.fetchKeys()
    }
    readonly property TextEdit _clip: TextEdit { visible: false; textFormat: TextEdit.PlainText }
    readonly property Component _timer: Component { Timer {} }

    function errorText(e) {
        return e && e.error ? String(e.error.message) : String(e);
    }
    function errorName(e) {
        return e && e.error ? String(e.error.name) : "";
    }
    function wrap(sig, a) {
        switch (sig) {
        case "s": return new DBus.string(String(a));
        case "b": return new DBus.bool(!!a);
        case "u": return new DBus.uint32(Math.max(0, Math.floor(a)));
        case "i": return new DBus.int32(Math.floor(a));
        }
        return a;
    }
    function call(service, path, iface, method, sig, args, cb) {
        const typed = [];
        for (let i = 0; i < args.length; ++i) typed.push(wrap(sig[i], args[i]));
        DBus.SessionBus.asyncCall({ service: service, path: path, iface: iface, member: method, arguments: typed },
            function (reply) { cb(true, reply.value, ""); },
            function (error) { cb(false, null, store.errorText(error), store.noReply(error), store.errorName(error)); });
    }
    function noReply(e) {
        return !!(e && e.error && String(e.error.name) === "org.freedesktop.DBus.Error.NoReply");
    }
    // cb runs once: the reply, or (false, null, "timeout") at the deadline. A write that may
    // still complete passes probe(done), polled until it sees the effect.
    function dbus(path, iface, method, sig, args, timeout, cb, probe) {
        // Never D-Bus-activate a daemon the user stopped: the header offers "Start the service".
        if (!present) { cb(false, null, "absent", ""); return; }
        let settled = false;
        let poller = null;
        const deadline = _timer.createObject(store, { interval: timeout });
        const finish = function (ok, value, error, name) {
            if (settled) return;
            settled = true;
            deadline.destroy();
            if (poller) poller.destroy();
            cb(ok, value, error, name || "");
        };
        deadline.triggered.connect(function () { finish(false, null, "timeout"); });
        deadline.start();
        if (probe) {
            poller = _timer.createObject(store, { interval: 2000, repeat: true });
            poller.triggered.connect(function () {
                probe(function (done) { if (done) finish(true, null, ""); });
            });
            poller.start();
        }
        const done = function (ok, value, error, lost, name) {
            if (ok || !lost) finish(ok, value, error, name);
            else if (!probe) finish(false, null, "timeout");
        };
        if (caller) {
            caller.call(bus, path, iface, method, sig, args, timeout, function (ok, value, name, message) {
                done(ok, ok ? value : null, message, name === "org.freedesktop.DBus.Error.NoReply", name);
            });
        } else {
            call(bus, path, iface, method, sig, args, done);
        }
    }
    function prop(path, iface, name, cb) {
        if (!present) { cb(false, null, "absent"); return; }
        call(bus, path, "org.freedesktop.DBus.Properties", "Get", "ss", [iface, name], cb);
    }
    // An error of this store in words, for display ("absent", "forbidden", "timeout", "invalid: …").
    function failureText(e) {
        const s = String(e || "");
        if (s === "absent") return i18n("The keyboard service (apple-kb-monitord) is not running");
        if (s === "forbidden") return i18n("command not allowed");
        if (s === "timeout") return i18n("no answer in time");
        if (s.startsWith("invalid: ")) return i18nc("%1 is a JSON parser error", "unreadable reply: %1", s.slice(9));
        return s;
    }
    // akmctl through the daemon (Settings.RunAkmctl, a closed list of commands).
    function cmd(program, args, timeout, cb, probe) {
        if (program !== "akmctl" || !present) {
            cb(-1, "", present ? "forbidden" : "absent", false, false);
            return;
        }
        // The daemon's own limit ends first, so its timed-out result and partial output still arrive.
        dbus(settingsPath, settingsIface, "RunAkmctl", "su", [JSON.stringify(args), Math.max(1000, timeout - 3000)], timeout, function (ok, value, error) {
            if (!ok) { cb(-1, "", error, error === "timeout", false, error === "timeout" ? "timeout" : ""); return; }
            if (value === null) { cb(0, "", "", false, false, ""); return; }
            let r;
            try { r = JSON.parse(String(value)); } catch (e) { cb(-1, "", "invalid: " + e.message, false, false, ""); return; }
            cb(r.code, r.out, r.err, r.timed_out, r.signal !== undefined && r.signal !== null, r.verdict || "");
        }, probe);
    }
    function deviceName(name, checkOnly, cb) {
        cmd("akmctl", ["rename", "--device-name=" + name, checkOnly ? "--check" : "--yes"], longCmdTimeout, cb);
    }
    function shellQuote(t) {
        return "'" + String(t).replace(/'/g, "'\\''") + "'";
    }
    function deviceNameCommand(name, flag) {
        return "akmctl rename --device-name=" + shellQuote(name) + (flag ? " " + flag : "");
    }
    function maskMac(v) {
        if (v === undefined || v === null || v === "") return "";
        const p = String(v).split(":");
        if (p.length !== 6 || !p.every(function (b) { return /^[0-9A-Fa-f]{2}$/.test(b); }))
            return String(v);
        return (p[0] + ":" + p[1] + ":XX:XX:XX:" + p[5]).toUpperCase();
    }
    // Every Bluetooth address of a text, "AA:BB:CC:DD:EE:F1" or "AA_BB_CC_DD_EE_F1", masked as on screen.
    function maskMacs(text) {
        return String(text).replace(/(^|[^0-9A-Fa-f])([0-9A-Fa-f]{2})([:_])([0-9A-Fa-f]{2})\3(?:[0-9A-Fa-f]{2}\3){3}([0-9A-Fa-f]{2})(?![0-9A-Fa-f])/g,
            function (m, pre, a, sep, b, f) { return pre + [a, b, "XX", "XX", "XX", f].join(sep).toUpperCase(); });
    }
    function copyText(text) {
        _clip.text = text;
        _clip.selectAll();
        _clip.copy();
        _clip.text = "";
    }

    function fetchState() {
        if (!present) return;
        if (stateBusy) { stateAgain = true; return; }
        stateBusy = true;
        dbus(root, bus, "GetState", "", [], readTimeout, function (ok, value, error) {
            stateBusy = false;
            if (ok) {
                try {
                    const s = JSON.parse(String(value));
                    if (s === null || typeof s !== "object") throw new Error("not an object");
                    state = s;
                    stateError = "";
                } catch (e) {
                    stateError = "invalid: " + e.message;
                }
            } else {
                stateError = error;
            }
            if (stateAgain) { stateAgain = false; fetchState(); }
        });
    }

    function fetchVersion() {
        if (!present) return;
        prop(root, bus, "DaemonVersion", function (ok, value) {
            daemonVersion = ok ? String(value) : "";
        });
    }

    property int _keysGen: 0
    function fetchKeys() {
        const gen = ++_keysGen;
        keyTableBusy = true;
        let pending = present ? 2 : 1;
        let errors = [];
        let timeouts = 0;
        let gotAkmctl = false;
        const finish = function () {
            if (gen !== _keysGen) return;
            pending -= 1;
            if (pending > 0) return;
            keyTableBusy = false;
            keyTableError = present && (keyTable === null || errors.length === 2) ? errors.join(" / ") : "";
            keyTableTimeout = keyTableError !== "" && timeouts === errors.length;
        };
        cmd("akmctl", ["keys", "--json"], cmdTimeout / 2, function (code, out, err, timedOut) {
            if (gen !== _keysGen) return;
            if (code === 0) {
                try {
                    keyTable = JSON.parse(out);
                    gotAkmctl = true;
                } catch (e) {
                    errors.push("akmctl: " + e.message);
                }
            } else {
                if (timedOut) timeouts += 1;
                errors.push(timedOut ? "akmctl: timeout" : ("akmctl: " + (err || code)).trim());
            }
            finish();
        });
        if (present) {
            dbus(root, keymapIface, "KeyTable", "b", [false], readTimeout, function (ok, value, error) {
                if (gen !== _keysGen) return;
                if (!ok) {
                    if (error === "timeout") timeouts += 1;
                    errors.push("D-Bus: " + error);
                } else if (!gotAkmctl) {
                    try {
                        keyTable = JSON.parse(String(value));
                    } catch (e) {
                        errors.push("D-Bus: " + e.message);
                    }
                }
                finish();
            });
        }
    }

    function fetchKeymap() {
        if (!present) return;
        dbus(root, keymapIface, "Keymap", "", [], readTimeout, function (ok, value, error) {
            if (!ok) { keymapError = error || "error"; return; }
            try {
                keymap = JSON.parse(String(value));
                keymapError = "";
            } catch (e) {
                keymapError = "invalid: " + e.message;
            }
        });
    }

    function getConfig(cb) {
        // An absent daemon is not an unreadable file: the module's header says it.
        if (!present) {
            configLoaded = false;
            configError = "";
            configBusError = "";
            cb(false, {}, "");
            return;
        }
        dbus(settingsPath, settingsIface, "GetConfig", "", [], readTimeout, function (ok, value, error, name) {
            let values = {};
            if (ok) {
                try {
                    const r = JSON.parse(String(value));
                    values = r.values || {};
                    configRevision = String(r.revision || "");
                    configPath = typeof r.path === "string" ? r.path : "";
                } catch (e) {
                    ok = false;
                    error = e.message;
                }
            }
            // The daemon reports a file it cannot read as Failed ("config.toml: …").
            const fileError = !ok && name === "org.freedesktop.DBus.Error.Failed";
            configLoaded = ok;
            configError = fileError ? error : "";
            configBusError = ok || fileError ? "" : error;
            cb(ok, values, error);
        });
    }
    // pairs = [[key, value], ...], written one by one by the daemon (one line each).
    function setConfig(pairs, cb) {
        if (pairs.length === 0) { cb(true, ""); return; }
        const p = pairs[0];
        dbus(settingsPath, settingsIface, "SetConfig", "sss", [p[0], JSON.stringify(p[1]), configRevision], cmdTimeout, function (ok, value, error, name) {
            if (!ok) { cb(false, name.endsWith(".Conflict") ? "changed" : error); return; }
            configRevision = String(value);
            setConfig(pairs.slice(1), cb);
        });
    }
    function refreshAll() {
        fetchState();
        fetchVersion();
        fetchKeys();
        fetchKeymap();
    }

    function setParam(name, value, cb) {
        cmd("akmctl", ["set", "param", name, String(value), "--persist"], authTimeout, function (code, out, err, timedOut) {
            // Already in effect, so no password: the daemon remembers it for reconnections and announces it (widget).
            if (code === 0 && name === "fnmode" && devicePath !== "")
                dbus(devicePath, bus + ".Device", "SetFnMode", "i", [value], readTimeout, function (ok, v, error) {
                    if (!ok) console.warn("apple-kb-monitor: SetFnMode after akmctl failed:", error);
                });
            cb(code === 0, timedOut ? "timeout" : (code === 0 ? out : (err || out)).trim());
            fetchKeys();
        }, function (done) {
            dbus(root, keymapIface, "KeyTable", "b", [false], readTimeout, function (ok, json) {
                let params = null;
                try { params = ok ? JSON.parse(String(json)).params : null; } catch (e) {}
                done(!!params && params[name] === value);
            });
        });
    }

    // Keymap editor: the daemon validates and writes keymap.toml.
    function keymapCall(method, sig, args, timeout, cb) {
        dbus(root, keymapIface, method, sig, args, timeout, function (ok, value, error) {
            cb(ok, ok ? String(value) : error);
            fetchKeymap();
            fetchKeys();
        });
    }
    function setKey(key, code, cb) { keymapCall("SetKey", "sss", ["", key, code], cmdTimeout, cb); }
    function setPreset(preset, cb) { keymapCall("SetPreset", "ss", ["", preset], cmdTimeout, cb); }
    function applyKeymap(cb) { keymapCall("Apply", "", [], authTimeout, cb); }
    function resetKeymap(cb) { keymapCall("Reset", "", [], authTimeout, cb); }
    function kdeApply(cb) {
        cmd("akmctl", ["keymap", "kde-apply"], cmdTimeout, function (code, out, err, timedOut) {
            cb(code === 0, timedOut ? "timeout" : (code === 0 ? out : (err || out)).trim());
            fetchKeys();
        });
    }

    function setAlias(mac, name, cb) {
        dbus(root, bus, "SetAlias", "ss", [mac, name], cmdTimeout, function (ok, value, error) {
            cb(ok, ok ? String(value) : error);
            fetchState();
        });
    }

    function reconnect(cb) {
        dbus(linkPath, linkIface, "Reconnect", "", [], readTimeout, function (ok, value, error) {
            cb(ok && value === true, ok ? "" : error);
        });
    }

    function systemd(method, cb) {
        call("org.freedesktop.systemd1", "/org/freedesktop/systemd1", "org.freedesktop.systemd1.Manager", method, "ss",
             ["apple-kb-monitord.service", "replace"], function (ok, value, error) { cb(ok, error); });
    }
    function startDaemon(cb) {
        systemd("StartUnit", cb);
    }
    function restartDaemon(cb) {
        systemd("TryRestartUnit", cb);
    }
    readonly property Timer _poll: Timer {
        interval: 60000
        repeat: true
        running: store.present
        onTriggered: store.fetchState()
    }
}
