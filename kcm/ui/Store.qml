// SPDX-License-Identifier: GPL-2.0-or-later
// Data of the module: every read and write goes through kcm.bridge
// (asynchronous D-Bus with a deadline, QProcess, worker-thread file I/O).
// Nothing here ever waits: each request takes a callback.
import QtQuick

QtObject {
    id: store

    // kcm.bridge (AkmBridge, C++): call/getProperty/run/readConfig/writeConfig/copyText
    required property var bridge

    readonly property string bus: bridge ? bridge.busName : "com.agenceapi.AppleKbMonitor1"
    readonly property string root: bridge ? bridge.objectPath : "/com/agenceapi/AppleKbMonitor1"
    readonly property string keymapIface: bus + ".Keymap"
    readonly property string linkIface: bus + ".Link"
    readonly property string linkPath: root + "/Link"

    // Deadlines (ms). Reads are short; anything that may show a polkit
    // dialog gets two minutes (the dialog is modal for the user, not for us).
    readonly property int readTimeout: 5000
    readonly property int cmdTimeout: 20000
    readonly property int longCmdTimeout: 60000
    readonly property int authTimeout: 120000

    readonly property bool present: bridge ? bridge.daemonPresent : false
    readonly property bool known: bridge ? bridge.daemonKnown : false

    // ── daemon state (GetState) ──
    property var state: null
    property string stateError: ""
    property bool stateBusy: false
    property bool stateAgain: false
    property double stateAt: 0          // ms, when the last answer arrived
    property string daemonVersion: ""

    // ── keys ──
    property var keyTable: null
    property string keyTableError: ""
    property string keyTableSource: ""   // "akmctl" or "dbus"
    property bool keyTableBusy: false
    property var keymap: null
    property string keymapError: ""

    // ── config.toml ──
    property string configText: ""
    property bool configLoaded: false
    property string configError: ""

    property var _calls: ({})
    property var _runs: ({})
    property var _files: ({})

    signal stateArrived()

    // ── low level ──
    function dbus(path, iface, method, sig, args, timeout, cb) {
        if (!bridge) { cb(false, null, "no bridge"); return; }
        const id = bridge.call(bus, path, iface, method, sig, args, timeout);
        _calls[id] = cb;
    }
    function prop(path, iface, name, cb) {
        if (!bridge) { cb(false, null, "no bridge"); return; }
        const id = bridge.getProperty(bus, path, iface, name, readTimeout);
        _calls[id] = cb;
    }
    function cmd(program, args, timeout, cb) {
        if (!bridge) { cb(-1, "", "no bridge", false); return; }
        const id = bridge.run(program, args, timeout);
        _runs[id] = cb;
    }

    // Name stored inside the keyboard (#248): `akmctl rename
    // --device-name=<name> --yes` (or `--check`), built and bounded (90 s) by
    // the bridge; cb(exitCode, stdout, stderr, timedOut, crashed).
    function deviceName(name, checkOnly, cb) {
        if (!bridge) { cb(-1, "", "no bridge", false, false); return; }
        const id = bridge.runDeviceName(name, checkOnly);
        _runs[id] = cb;
    }

    // Meaning of akmctl's exit code and the command to copy (C++, unit
    // tested: AkmDeviceName::verdict / copyCommand).
    function deviceNameVerdict(code, checkOnly, timedOut, crashed) {
        return bridge ? bridge.deviceNameVerdict(code, checkOnly, timedOut, crashed) : (checkOnly ? "check-failed" : "uncertain");
    }
    function deviceNameCommand(name, flag) {
        return bridge ? bridge.deviceNameCommand(name, flag) : "";
    }

    readonly property Connections _c: Connections {
        target: store.bridge
        function onCallFinished(id, ok, value, error) {
            const cb = store._calls[id];
            delete store._calls[id];
            if (cb) cb(ok, value, error);
        }
        function onRunFinished(id, code, out, err, timedOut, crashed) {
            const cb = store._runs[id];
            delete store._runs[id];
            if (cb) cb(code, out, err, timedOut, crashed);
        }
        function onFileFinished(id, ok, text, error) {
            const cb = store._files[id];
            delete store._files[id];
            if (cb) cb(ok, text, error);
        }
        function onDaemonStateChanged(revision) { store.fetchState(); }
        function onDaemonPresentChanged() {
            if (store.present) {
                store.refreshAll();
            } else {
                store.state = null;
                store.stateError = "";
                store.daemonVersion = "";
            }
        }
    }

    // ── reads ──
    function fetchState() {
        if (!present) return;
        if (stateBusy) { stateAgain = true; return; }
        stateBusy = true;
        dbus(root, bus, "GetState", "", [], readTimeout, function (ok, value, error) {
            stateBusy = false;
            stateAt = Date.now();
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
            stateArrived();
            if (stateAgain) { stateAgain = false; fetchState(); }
        });
    }

    function fetchVersion() {
        prop(root, bus, "DaemonVersion", function (ok, value) {
            daemonVersion = ok ? String(value) : "";
        });
    }

    // Effective key table. Both sources are asked at once: the daemon's
    // KeyTable answers fast; akmctl adds the KDE action of each key and
    // replaces it when it arrives. An error is shown only if both fail.
    property int _keysGen: 0
    function fetchKeys() {
        const gen = ++_keysGen;
        keyTableBusy = true;
        let pending = present ? 2 : 1;
        let errors = [];
        let gotAkmctl = false;
        const finish = function () {
            if (gen !== _keysGen) return;
            pending -= 1;
            if (pending > 0) return;
            keyTableBusy = false;
            keyTableError = (keyTable === null || errors.length === 2 || (!present && errors.length === 1)) ? errors.join(" / ") : "";
        };
        cmd("akmctl", ["keys", "--json"], cmdTimeout / 2, function (code, out, err, timedOut) {
            if (gen !== _keysGen) return;
            if (code === 0) {
                try {
                    keyTable = JSON.parse(out);
                    keyTableSource = "akmctl";
                    gotAkmctl = true;
                } catch (e) {
                    errors.push("akmctl: " + e.message);
                }
            } else {
                errors.push(timedOut ? "akmctl: timeout" : ("akmctl: " + (err || code)).trim());
            }
            finish();
        });
        if (present) {
            dbus(root, keymapIface, "KeyTable", "b", [false], readTimeout, function (ok, value, error) {
                if (gen !== _keysGen) return;
                if (!ok) {
                    errors.push("D-Bus: " + error);
                } else if (!gotAkmctl) {
                    try {
                        keyTable = JSON.parse(String(value));
                        keyTableSource = "dbus";
                    } catch (e) {
                        errors.push("D-Bus: " + e.message);
                    }
                }
                finish();
            });
        }
    }

    function fetchKeymap() {
        const parse = function (text, where) {
            try {
                keymap = JSON.parse(text);
                keymapError = "";
            } catch (e) {
                keymapError = where + ": " + e.message;
            }
        };
        if (present) {
            dbus(root, keymapIface, "Keymap", "", [], readTimeout, function (ok, value, error) {
                if (ok) parse(String(value), "D-Bus");
                else keymapError = "D-Bus: " + error;
            });
        } else {
            cmd("akmctl", ["keymap", "show", "--json"], cmdTimeout, function (code, out, err, timedOut) {
                if (code === 0) parse(out, "akmctl");
                else keymapError = timedOut ? "akmctl: timeout" : ("akmctl: " + (err || code)).trim();
            });
        }
    }

    // configText is what the file holds, exactly; configLoaded is false when
    // it could not be read whole (#284): nothing may then be written.
    function readConfig(cb) {
        const id = bridge.readConfig();
        _files[id] = function (ok, text, error) {
            configLoaded = ok;
            configError = ok ? "" : error;
            configText = ok ? text : "";
            if (cb) cb(ok, text, error);
        };
    }
    // Written only over the very text that was read (the bridge compares it
    // with the file first); error "changed" = the file changed since.
    function writeConfig(text, cb) {
        if (!configLoaded) { cb(false, "unread"); return; }
        const id = bridge.writeConfig(text, configText);
        _files[id] = function (ok, t, error) {
            if (ok) configText = text;
            if (cb) cb(ok, error);
        };
    }

    function refreshAll() {
        fetchState();
        fetchVersion();
        fetchKeys();
        fetchKeymap();
    }

    // ── writes (each one is an explicit user action) ──

    // hid_apple parameter, persistent, through the existing polkit action
    // (akmctl -> pkexec akm-helper). Global to every Apple keyboard.
    function setParam(name, value, cb) {
        cmd("akmctl", ["set", "param", name, String(value), "--persist"], authTimeout, function (code, out, err, timedOut) {
            cb(code === 0, timedOut ? "timeout" : (code === 0 ? out : (err || out)).trim());
            fetchKeys();
        });
    }

    // keymap.toml edits: the daemon's Keymap interface, else akmctl.
    function setKey(key, code, cb) {
        const done = function (ok, msg) { cb(ok, msg); fetchKeymap(); fetchKeys(); };
        if (present) {
            dbus(root, keymapIface, "SetKey", "sss", ["", key, code], cmdTimeout, function (ok, value, error) {
                done(ok, ok ? "" : error);
            });
        } else {
            const args = code === "" ? ["keymap", "unset", key] : ["keymap", "set", key, code];
            cmd("akmctl", args, cmdTimeout, function (c, out, err, to) { done(c === 0, to ? "timeout" : (err || out).trim()); });
        }
    }
    function setPreset(preset, cb) {
        const done = function (ok, msg) { cb(ok, msg); fetchKeymap(); fetchKeys(); };
        if (present) {
            dbus(root, keymapIface, "SetPreset", "ss", ["", preset], cmdTimeout, function (ok, value, error) {
                done(ok, ok ? "" : error);
            });
        } else {
            cmd("akmctl", ["keymap", "preset", preset], cmdTimeout, function (c, out, err, to) { done(c === 0, to ? "timeout" : (err || out).trim()); });
        }
    }
    function applyKeymap(cb) {
        const done = function (ok, msg) { cb(ok, msg); fetchKeymap(); fetchKeys(); };
        if (present) {
            dbus(root, keymapIface, "Apply", "", [], authTimeout, function (ok, value, error) {
                done(ok, ok ? String(value) : error);
            });
        } else {
            cmd("akmctl", ["keymap", "apply"], authTimeout, function (c, out, err, to) { done(c === 0, to ? "timeout" : (c === 0 ? out : (err || out)).trim()); });
        }
    }
    function resetKeymap(cb) {
        const done = function (ok, msg) { cb(ok, msg); fetchKeymap(); fetchKeys(); };
        if (present) {
            dbus(root, keymapIface, "Reset", "", [], authTimeout, function (ok, value, error) {
                done(ok, ok ? String(value) : error);
            });
        } else {
            cmd("akmctl", ["keymap", "reset"], authTimeout, function (c, out, err, to) { done(c === 0, to ? "timeout" : (c === 0 ? out : (err || out)).trim()); });
        }
    }
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

    function startDaemon(cb) {
        cmd("systemctl", ["--user", "start", "apple-kb-monitord.service"], cmdTimeout, function (code, out, err, timedOut) {
            cb(code === 0, timedOut ? "timeout" : (err || out).trim());
            if (bridge) bridge.checkDaemon();
        });
    }
    function restartDaemon(cb) {
        cmd("systemctl", ["--user", "try-restart", "apple-kb-monitord.service"], cmdTimeout, function (code, out, err, timedOut) {
            cb(code === 0, timedOut ? "timeout" : (err || out).trim());
        });
    }

    // Safety net if a StateChanged signal is ever missed.
    readonly property Timer _poll: Timer {
        interval: 60000
        repeat: true
        running: store.present
        onTriggered: store.fetchState()
    }
}
