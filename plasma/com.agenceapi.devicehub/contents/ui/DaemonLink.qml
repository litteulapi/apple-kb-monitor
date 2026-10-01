import QtQuick
import org.kde.plasma.workspace.dbus as DBus

// D-Bus link to the daemon apple-kb-monitord, in-process only (#150): the
// StateChanged signal is received through a native SignalWatcher and the
// window is opened with org.freedesktop.Application.Activate. No
// subprocess is ever started, so nothing can be left orphaned when the
// widget is removed, reloaded or plasmashell restarts.
// Depends on QtQuick + org.kde.plasma.workspace.dbus only (testable headless
// with plasma/tests/run-widget-tests.sh).
Item {
    id: link

    property string busName: "com.agenceapi.AppleKbMonitor1"
    property string objectPath: "/com/agenceapi/AppleKbMonitor1"
    // Name and path claimed by the window (apihub-app, DBusActivatable=true).
    property string windowName: "com.agenceapi.AppleKbMonitor"
    property string windowPath: "/com/agenceapi/AppleKbMonitor"
    // Safety net if a signal is ever missed; the daemon emits about one
    // StateChanged every 30 s.
    property int safetyPollMs: 120000

    // Tray arbitration (#253): while the widget lives it holds a claim on the
    // daemon's tray icon, which then leaves the notification area (one icon,
    // not two). One claim per widget instance; plasmashell hosts them all
    // under one bus name, so each instance names itself. The claim is renewed
    // every safetyPollMs and re-sent when the daemon (re)appears; it dies with
    // plasmashell, and releaseTray() hands the icon back when the widget goes.
    property string trayName: "com.agenceapi.AppleKbMonitor1"
    property string trayPath: "/com/agenceapi/AppleKbMonitor1/Tray"
    property string trayIface: "com.agenceapi.AppleKbMonitor1.Tray"
    property string claimId: ""

    readonly property bool registered: watcher.registered

    signal stateReceived(string json)
    signal failed(string message)
    signal windowFailed(string message)
    signal windowActivated()
    signal aliasSet(string name)
    signal aliasFailed(string message)

    function fetch() {
        if (!watcher.registered) return;
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: link.objectPath,
            iface: link.busName,
            member: "GetState",
            arguments: []
        }, function (reply) {
            link.stateReceived(String(reply.value));
        }, function (error) {
            link.failed(error && error.error ? String(error.error.message) : String(error));
        });
    }

    function claimTray() {
        if (!watcher.registered || link.claimId === "") return;
        DBus.SessionBus.asyncCall({
            service: link.trayName,
            path: link.trayPath,
            iface: link.trayIface,
            member: "ClaimTrayFor",
            arguments: [new DBus.string(link.claimId)]
        }, function () {}, function (error) {
            console.warn("apple-kb-monitor: ClaimTrayFor failed", error && error.error ? error.error.message : error);
        });
    }

    function releaseTray() {
        if (link.claimId === "") return;
        DBus.SessionBus.asyncCall({
            service: link.trayName,
            path: link.trayPath,
            iface: link.trayIface,
            member: "ReleaseTrayFor",
            arguments: [new DBus.string(link.claimId)]
        }, function () {}, function () {});
    }

    onRegisteredChanged: if (registered) claimTray()
    onClaimIdChanged: claimTray()

    // Rename on this computer (BlueZ alias). Do NOT pass `signature` to
    // asyncCall: with it the typed arguments are dropped from the message
    // (measured, Plasma 6.7), the callee then sees a call without arguments.
    function setAlias(mac, name) {
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: link.objectPath,
            iface: link.busName,
            member: "SetAlias",
            arguments: [new DBus.string(mac), new DBus.string(name)]
        }, function (reply) {
            link.aliasSet(String(reply.value));
        }, function (error) {
            link.aliasFailed(error && error.error ? String(error.error.message) : String(error));
        });
    }

    // Raise the window, or start it by D-Bus activation when it is closed.
    function activateWindow() {
        DBus.SessionBus.asyncCall({
            service: link.windowName,
            path: link.windowPath,
            iface: "org.freedesktop.Application",
            member: "Activate",
            arguments: [new DBus.dict({})]
        }, function () {
            link.windowActivated();
        }, function (error) {
            link.windowFailed(error && error.error ? String(error.error.message) : String(error));
        });
    }

    DBus.DBusServiceWatcher {
        id: watcher
        busType: DBus.BusType.Session
        watchedService: link.busName
    }

    DBus.SignalWatcher {
        busType: DBus.BusType.Session
        service: link.busName
        path: link.objectPath
        iface: link.busName
        enabled: watcher.registered
        // Called by the watcher for each StateChanged(t revision, s json).
        function dbusStateChanged() {
            link.fetch();
        }
    }

    Timer {
        interval: link.safetyPollMs
        repeat: true
        running: watcher.registered
        onTriggered: {
            link.fetch();
            link.claimTray();
        }
    }
}
