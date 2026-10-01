import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.plasma5support as P5
import org.kde.plasma.workspace.dbus as DBus
import org.kde.kirigami as Kirigami

// Event-driven view of the daemon apple-kb-monitord (session bus,
// com.agenceapi.AppleKbMonitor1). No periodic timer: the applet re-reads
// GetState (in-memory snapshot, read-only, never touches the keyboard) when
// the daemon appears and each time it emits StateChanged.
//
// Plasma's QML D-Bus module has no signal API, so StateChanged is awaited by
// a bounded one-shot dbus-monitor (killed at the first match) run through the
// "executable" engine; its exit re-arms the listener (see armListener).
PlasmoidItem {
    id: root

    readonly property string busName: "com.agenceapi.AppleKbMonitor1"
    readonly property string objectPath: "/com/agenceapi/AppleKbMonitor1"

    // ── State (battery -1 / voltage 0 / rssi NaN / "" = unknown) ──
    readonly property bool daemonRunning: watcher.registered
    property bool connected: false
    property int batteryPercent: -1
    property real voltage: 0
    property real rssi: NaN
    property string kbModel: ""
    property string fwVersion: ""
    property string batteryType: ""
    property string remaining: ""
    property string lastError: ""
    property string windowHint: ""

    readonly property bool hasBattery: connected && batteryPercent >= 0
    readonly property bool hasRssi: connected && !isNaN(rssi)

    // Short human text used by tooltip, compact label and screen readers.
    readonly property string batteryText: batteryPercent >= 0 ? i18n("%1%", batteryPercent) : "—"
    readonly property string rssiText: hasRssi ? i18n("%1 dBm", rssi) : "—"
    readonly property string stateText: connected
        ? i18n("Connected")
        : (daemonRunning ? i18n("Keyboard not connected") : i18n("Monitor stopped"))
    readonly property string accessibleSummary: connected
        ? i18n("Apple keyboard, battery %1 percent", batteryPercent >= 0 ? batteryPercent : i18n("unknown"))
        : i18n("Apple keyboard, %1", stateText)

    // Visible in the notification area while the daemon is on the bus.
    Plasmoid.status: daemonRunning ? PlasmaCore.Types.ActiveStatus : PlasmaCore.Types.PassiveStatus
    Plasmoid.title: i18n("ApiHub")
    Plasmoid.icon: "apihub-scarab"

    preferredRepresentation: compactRepresentation
    compactRepresentation: CompactRepresentation {}
    fullRepresentation: FullRepresentation {}

    toolTipMainText: connected ? kbModel : stateText
    toolTipSubText: connected
        ? [hasBattery ? i18n("Battery %1", batteryText) : "",
           voltage > 0 ? i18n("%1 V", voltage.toFixed(2)) : "",
           hasRssi ? i18n("Signal %1", rssiText) : "",
           remaining].filter(function (s) { return s !== ""; }).join("\n")
        : (daemonRunning ? i18n("Waiting for the Apple keyboard…")
                         : i18n("apple-kb-monitord is not on the session bus"))

    DBus.DBusServiceWatcher {
        id: watcher
        busType: DBus.BusType.Session
        watchedService: root.busName
        onRegisteredChanged: {
            if (registered) {
                root.fetchData();
                root.armListener();
            } else {
                root.clear();
            }
        }
    }

    Component.onCompleted: {
        if (watcher.registered) {
            fetchData();
            armListener();
        }
    }
    Component.onDestruction: listener.disconnectSource(listener.command)

    function batteryTypeOf(v) {
        if (v <= 0) return "";
        if (v >= 3.1) return i18n("Lithium (fresh)");
        if (v >= 2.85) return i18n("Alkaline (fresh)");
        if (v >= 2.5) return i18n("Alkaline or NiMH");
        if (v >= 2.3) return i18n("NiMH (likely)");
        if (v >= 2.0) return i18n("Depleted");
        return i18n("Critical, replace");
    }

    // RSSI is unknown when absent, null, the 127 sentinel or >= 0 dBm (the
    // daemon reports 0 when it has no reading; a real link never reads 0 dBm).
    function rssiOf(radio) {
        if (!radio || radio.rssi_dbm === null || radio.rssi_dbm === undefined) return NaN;
        var v = Number(radio.rssi_dbm);
        return (isNaN(v) || v >= 0) ? NaN : v;
    }

    function apply(json) {
        var d = JSON.parse(json);
        var kb = d.keyboard || null;
        var b = kb ? (kb.battery || {}) : {};
        var pct = b.percentage_fine;
        if (pct === null || pct === undefined) pct = b.percentage_interpolated;
        if (pct === null || pct === undefined) pct = b.percentage;
        root.connected = !!d.connected && kb !== null;
        root.batteryPercent = (pct === null || pct === undefined) ? -1 : Math.round(pct);
        root.voltage = b.voltage || 0;
        root.rssi = rssiOf(kb ? kb.radio : null);
        root.kbModel = (kb && kb.device && kb.device.model) ? kb.device.model : i18n("Apple Keyboard");
        root.fwVersion = (kb && kb.firmware && kb.firmware.version) ? kb.firmware.version : "";
        root.batteryType = batteryTypeOf(root.voltage);
        root.remaining = d.remaining_display || "";
        root.lastError = d.last_error || "";
    }

    function clear() {
        root.connected = false;
        root.batteryPercent = -1;
        root.voltage = 0;
        root.rssi = NaN;
        root.remaining = "";
    }

    function fetchData() {
        if (!watcher.registered) {
            clear();
            return;
        }
        DBus.SessionBus.asyncCall({
            service: root.busName,
            path: root.objectPath,
            iface: root.busName,
            member: "GetState",
            arguments: [],
            signature: ""
        }, function (reply) {
            try {
                root.apply(String(reply.value));
            } catch (e) {
                console.warn("apple-kb-monitor: bad GetState reply", e);
                root.clear();
            }
        }, function (error) {
            console.warn("apple-kb-monitor: GetState failed", error && error.error ? error.error.message : error);
            root.clear();
        });
    }

    // ── StateChanged listener ──
    // Waits (max 10 min, then re-armed) for one StateChanged, then exits. A
    // failing listener (no dbus-monitor) is throttled to one retry per 5 s.
    function armListener() {
        if (!watcher.registered) return;
        listener.disconnectSource(listener.command);
        listener.connectSource(listener.command);
    }

    P5.DataSource {
        id: listener
        engine: "executable"
        readonly property string command: "s=$(date +%s); timeout 600 sh -c '"
            + "stdbuf -oL dbus-monitor --session type=signal,sender=com.agenceapi.AppleKbMonitor1,member=StateChanged 2>/dev/null"
            + " | { grep -m1 member=StateChanged >/dev/null; kill 0; }'; "
            + "[ $(( $(date +%s) - s )) -ge 2 ] || sleep 5"
        onNewData: function (source, data) {
            disconnectSource(source);
            if (root.daemonRunning) {
                root.fetchData();
                root.armListener();
            }
        }
    }

    // ── Open the existing apihub-app window (#121) ──
    // apihub-app owns one tray item (org.kde.StatusNotifierItem-<pid>-N,
    // Id "apihub-app"); its Activate() opens the window. Nothing is spawned,
    // so no second tray icon can appear.
    function openWindow() {
        root.windowHint = "";
        DBus.SessionBus.asyncCall({
            service: "org.freedesktop.DBus",
            path: "/org/freedesktop/DBus",
            iface: "org.freedesktop.DBus",
            member: "ListNames",
            arguments: [],
            signature: ""
        }, function (reply) {
            var names = (reply.value || []).filter(function (n) {
                return String(n).indexOf("org.kde.StatusNotifierItem-") === 0;
            });
            root.probeNext(names.map(String));
        }, function () { root.windowHint = i18n("Cannot reach the session bus."); });
    }

    function probeNext(candidates) {
        if (candidates.length === 0) {
            windowHint = i18n("The ApiHub window is not running (apihub-app).");
            return;
        }
        var svc = candidates[0];
        var rest = candidates.slice(1);
        DBus.SessionBus.asyncCall({
            service: svc,
            path: "/StatusNotifierItem",
            iface: "org.freedesktop.DBus.Properties",
            member: "Get",
            arguments: [new DBus.string("org.kde.StatusNotifierItem"), new DBus.string("Id")],
        }, function (reply) {
            if (String(reply.value) === "apihub-app") {
                DBus.SessionBus.asyncCall({
                    service: svc,
                    path: "/StatusNotifierItem",
                    iface: "org.kde.StatusNotifierItem",
                    member: "Activate",
                    arguments: [new DBus.int32(0), new DBus.int32(0)],
                }, function () {},
                   function () { root.windowHint = i18n("Could not open the ApiHub window."); });
            } else {
                root.probeNext(rest);
            }
        }, function () { root.probeNext(rest); });
    }
}
