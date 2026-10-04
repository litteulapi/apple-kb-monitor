import QtQuick
import org.kde.plasma.workspace.dbus as DBus
import "FnMode.js" as Fn

Item {
    id: link

    property string busName: "com.agenceapi.AppleKbMonitor1"
    property string objectPath: "/com/agenceapi/AppleKbMonitor1"
    // The followed keyboard: its Fn mode changed elsewhere (shortcut, menu, System Settings) is followed.
    property string deviceMac: ""
    property int safetyPollMs: 120000
    // SetFnMode waits for polkit up to the helper's 2 min; the bus gives up after 25 s.
    property int fnModeDeadlineMs: 130000

    property string trayPath: "/com/agenceapi/AppleKbMonitor1/Tray"
    property string trayIface: "com.agenceapi.AppleKbMonitor1.Tray"
    property string deviceIface: "com.agenceapi.AppleKbMonitor1.Device"
    property string linkPath: "/com/agenceapi/AppleKbMonitor1/Link"
    property string linkIface: "com.agenceapi.AppleKbMonitor1.Link"

    readonly property bool registered: watcher.registered

    signal stateReceived(string json)
    signal failed(string message)
    signal aliasSet(string name)
    signal aliasFailed(string message)
    signal historyReceived(string tag, string json)
    signal historyFailed(string tag, string message)
    signal fnModeReceived(int mode)
    signal fnModeSet(int mode)
    signal fnModeFailed(string message)
    signal versionReceived(string version)
    signal linkStatusReceived(string json)
    signal menuReceived(string json)
    signal panelRequested()
    signal menuItemFailed(string message)
    signal refreshDone(bool ok, string message)
    signal diagnoseReceived(string json)
    signal diagnoseFailed(string message)
    signal reconnectDone(bool ok, string message)

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
            link.failed(link.errorText(error));
        });
    }

    function fetchMenu() {
        if (!watcher.registered) return;
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: link.trayPath,
            iface: link.trayIface,
            member: "MenuItems",
            arguments: []
        }, function (reply) {
            link.menuReceived(String(reply.value));
        }, function (error) {
            console.warn("apple-kb-monitor: MenuItems failed", link.errorText(error));
        });
    }

    function activateMenuItem(id) {
        if (!watcher.registered) return;  // a call to a free name would auto-start the daemon
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: link.trayPath,
            iface: link.trayIface,
            member: "ActivateMenuItem",
            arguments: [new DBus.int32(id)]
        }, function () {}, function (error) {
            link.menuItemFailed(link.errorText(error));
        });
    }

    function errorText(error) {
        return error && error.error ? String(error.error.message) : String(error);
    }

    function fetchHistory(since, tag) {
        if (!watcher.registered) return;
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: link.objectPath,
            iface: link.busName,
            member: "History",
            arguments: [new DBus.uint64(Math.max(0, Math.floor(since)))]
        }, function (reply) {
            link.historyReceived(tag, String(reply.value));
        }, function (error) {
            link.historyFailed(tag, link.errorText(error));
        });
    }

    function fetchFnMode(mac) {
        var path = Fn.devicePath(link.objectPath, mac);
        if (!watcher.registered || path === "") return;
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: path,
            iface: "org.freedesktop.DBus.Properties",
            member: "Get",
            arguments: [new DBus.string(link.deviceIface), new DBus.string("FnMode")]
        }, function (reply) {
            link.fnModeReceived(Number(reply.value));
        }, function (error) {
            link.fnModeFailed(link.errorText(error));
        });
    }

    function setFnMode(mac, mode) {
        var path = Fn.devicePath(link.objectPath, mac);
        if (!watcher.registered || path === "") return;
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: path,
            iface: link.deviceIface,
            member: "SetFnMode",
            arguments: [new DBus.int32(mode)]
        }, function () {
            link.fnModeSet(mode);
        }, function (error) {
            if (error && error.error && String(error.error.name) === "org.freedesktop.DBus.Error.NoReply")
                link.awaitFnMode(path, mode, link.errorText(error));
            else
                link.fnModeFailed(link.errorText(error));
        });
    }

    // No reply is not a failure: the change may still be applied, so watch FnMode until the deadline.
    function awaitFnMode(path, mode, message) {
        var left = Math.ceil((link.fnModeDeadlineMs - 25000) / 2000);
        var done = false;
        var poll = pollTimer.createObject(link);
        var finish = function (ok) {
            if (done) return;
            done = true;
            poll.destroy();
            if (ok) link.fnModeSet(mode);
            else link.fnModeFailed(message);
        };
        poll.triggered.connect(function () {
            if (--left < 0 || !watcher.registered) { finish(false); return; }
            DBus.SessionBus.asyncCall({
                service: link.busName,
                path: path,
                iface: "org.freedesktop.DBus.Properties",
                member: "Get",
                arguments: [new DBus.string(link.deviceIface), new DBus.string("FnMode")]
            }, function (reply) {
                if (Number(reply.value) === mode) finish(true);
            }, function () {});
        });
        poll.start();
    }

    Component {
        id: pollTimer
        Timer { interval: 2000; repeat: true }
    }

    function fetchVersion() {
        if (!watcher.registered) return;
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: link.objectPath,
            iface: "org.freedesktop.DBus.Properties",
            member: "Get",
            arguments: [new DBus.string(link.busName), new DBus.string("DaemonVersion")]
        }, function (reply) {
            link.versionReceived(String(reply.value));
        }, function () {});
    }

    function fetchLinkStatus() {
        if (!watcher.registered) return;
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: link.linkPath,
            iface: link.linkIface,
            member: "Status",
            arguments: []
        }, function (reply) {
            link.linkStatusReceived(String(reply.value));
        }, function () {});
    }

    function refresh() {
        if (!watcher.registered) return;
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: link.objectPath,
            iface: link.busName,
            member: "Refresh",
            arguments: []
        }, function () {
            link.refreshDone(true, "");
        }, function (error) {
            link.refreshDone(false, link.errorText(error));
        });
    }

    function diagnose() {
        if (!watcher.registered) return;
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: link.objectPath,
            iface: link.busName,
            member: "Diagnose",
            arguments: []
        }, function (reply) {
            link.diagnoseReceived(String(reply.value));
        }, function (error) {
            link.diagnoseFailed(link.errorText(error));
        });
    }

    function reconnect() {
        if (!watcher.registered) return;
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: link.linkPath,
            iface: link.linkIface,
            member: "Reconnect",
            arguments: []
        }, function (reply) {
            link.reconnectDone(!!reply.value, "");
        }, function (error) {
            link.reconnectDone(false, link.errorText(error));
        });
    }


    function setAlias(mac, name) {
        if (!watcher.registered) return;
        DBus.SessionBus.asyncCall({
            service: link.busName,
            path: link.objectPath,
            iface: link.busName,
            member: "SetAlias",
            arguments: [new DBus.string(mac), new DBus.string(name)]
        }, function (reply) {
            link.aliasSet(String(reply.value));
        }, function (error) {
            link.aliasFailed(link.errorText(error));
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
        function dbusStateChanged() {
            link.fetch();
            link.fetchMenu();  // the menu carries state too (battery, enabled entries, Fn radio)
        }
    }

    DBus.SignalWatcher {
        busType: DBus.BusType.Session
        service: link.busName
        path: Fn.devicePath(link.objectPath, link.deviceMac)
        iface: "org.freedesktop.DBus.Properties"
        enabled: watcher.registered && path !== ""
        function dbusPropertiesChanged(iface) {
            if (String(iface) === link.deviceIface) link.fetchFnMode(link.deviceMac);
        }
    }

    DBus.SignalWatcher {
        busType: DBus.BusType.Session
        service: link.busName
        path: link.trayPath
        iface: link.trayIface
        enabled: watcher.registered
        function dbusPanelRequested() {
            link.panelRequested();
        }
    }

    Timer {
        interval: link.safetyPollMs
        repeat: true
        running: watcher.registered
        onTriggered: {
            link.fetch();
            link.fetchMenu();
        }
    }
}
