import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.plasma5support as P5
import org.kde.plasma.workspace.dbus as DBus
import org.kde.kirigami as Kirigami

// State comes from the daemon apple-kb-monitord over the session bus
// (com.agenceapi.AppleKbMonitor1.GetState): no process is spawned and the
// keyboard is never touched by the widget (#62).
PlasmoidItem {
    id: root

    readonly property string busName: "com.agenceapi.AppleKbMonitor1"
    readonly property string objectPath: "/com/agenceapi/AppleKbMonitor1"

    // ── Keyboard properties (battery -1, voltage 0, rssi 127, "" = unknown) ──
    property bool daemonRunning: watcher.registered
    property bool connected: false
    property int batteryPercent: -1
    property real voltage: 0
    property int rssi: 127
    property string kbModel: ""
    property string fwVersion: ""
    property string batteryType: ""
    property string dischargeRate: ""
    property string lastError: ""

    // ── Representations ──
    preferredRepresentation: compactRepresentation
    compactRepresentation: CompactRepresentation {}
    fullRepresentation: FullRepresentation {}

    // ── Tooltip ──
    toolTipMainText: connected ? kbModel : (daemonRunning ? "No keyboard" : "Monitor not running")
    toolTipSubText: connected
        ? (batteryPercent >= 0 ? batteryPercent + "%" : "n/a")
          + (voltage > 0 ? " · " + voltage.toFixed(3) + "V" : "")
          + (rssi !== 0 ? " · RSSI " + rssi + "dBm" : "")
        : (daemonRunning ? "Waiting for Apple Keyboard..." : "apple-kb-monitord is not on the session bus")

    DBus.DBusServiceWatcher {
        id: watcher
        busType: DBus.BusType.Session
        watchedService: root.busName
        onRegisteredChanged: root.fetchData()
    }

    // The daemon publishes changes; polling its in-memory state is cheap
    // (no hardware access), 15 s is plenty for a panel badge.
    Timer {
        interval: 15000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.fetchData()
    }

    function batteryTypeOf(v) {
        if (v <= 0) return "";
        if (v >= 3.1) return "Lithium (fresh)";
        if (v >= 2.85) return "Alkaline (fresh)";
        if (v >= 2.5) return "Alkaline or NiMH";
        if (v >= 2.3) return "NiMH (likely)";
        if (v >= 2.0) return "Depleted";
        return "Critical — replace";
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
        root.rssi = (kb && kb.radio && kb.radio.rssi_dbm !== null && kb.radio.rssi_dbm !== undefined) ? kb.radio.rssi_dbm : 127;
        root.kbModel = (kb && kb.device && kb.device.model) ? kb.device.model : "Apple Keyboard";
        root.fwVersion = (kb && kb.firmware && kb.firmware.version) ? kb.firmware.version : "";
        root.batteryType = batteryTypeOf(root.voltage);
        root.dischargeRate = d.remaining_display || "";
        root.lastError = d.last_error || "";
    }

    function clear() {
        root.connected = false;
        root.batteryPercent = -1;
        root.voltage = 0;
        root.rssi = 127;
    }

    // ── Public functions ──
    function fetchData() {
        if (!watcher.registered) {
            clear();
            return;
        }
        DBus.SessionBus.asyncCall({
            service: root.busName,
            path: root.objectPath,
            iface: "com.agenceapi.AppleKbMonitor1",
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

    function openSettings() {
        settingsLauncher.connectSource("apihub-app --show")
    }

    // ── Settings launcher (opens the GUI client; it does not read the keyboard
    //    while the daemon runs) ──
    P5.DataSource {
        id: settingsLauncher
        engine: "executable"
        onNewData: function(source, data) { disconnectSource(source); }
    }
}
