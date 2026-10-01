import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.plasma5support as P5
import org.kde.kirigami as Kirigami

PlasmoidItem {
    id: root

    // ── Keyboard properties ──
    property bool connected: false
    property int batteryPercent: 0
    property real voltage: 0
    property int rssi: 0
    property string kbModel: ""
    property string fwVersion: ""
    property string batteryType: ""
    property string dischargeRate: ""

    // ── Representations ──
    preferredRepresentation: compactRepresentation
    compactRepresentation: CompactRepresentation {}
    fullRepresentation: FullRepresentation {}

    // ── Tooltip ──
    toolTipMainText: connected ? kbModel : "No device"
    toolTipSubText: connected
        ? batteryPercent + "% \u00B7 " + voltage.toFixed(3) + "V \u00B7 RSSI " + rssi + "dBm"
        : "Waiting for Apple Keyboard..."

    // ── Polling timers ──
    Timer {
        interval: 30000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: dataSource.connectSource("apple-kb-monitor --json 2>/dev/null")
    }

    // ── Public functions ──
    function fetchData() {
        dataSource.connectSource("apple-kb-monitor --json 2>/dev/null");
    }

    function openSettings() {
        settingsLauncher.connectSource("apihub-app")
    }

    // ── Keyboard data source ──
    P5.DataSource {
        id: dataSource
        engine: "executable"
        onNewData: function(source, data) {
            var stdout = data["stdout"];
            if (!stdout) { disconnectSource(source); return; }
            try {
                var d = JSON.parse(stdout);
                root.connected = true;
                root.batteryPercent = d.battery.percentage_fine || d.battery.percentage || 0;
                root.voltage = d.battery.voltage || 0;
                root.kbModel = d.device.model || "Apple Keyboard";
                if (d.radio && d.radio.rssi_dbm !== undefined)
                    root.rssi = d.radio.rssi_dbm;
                root.fwVersion = (d.firmware && d.firmware.version) ? d.firmware.version : "";
                root.batteryType = (d.analysis && d.analysis.battery_type) ? d.analysis.battery_type.type : "";
                root.dischargeRate = (d.analysis && d.analysis.discharge) ? d.analysis.discharge.remaining_display : "";
            } catch(e) {
                root.connected = false;
            }
            disconnectSource(source);
        }
    }

    // ── Settings launcher ──
    P5.DataSource {
        id: settingsLauncher
        engine: "executable"
        onNewData: function(source, data) { disconnectSource(source); }
    }
}
