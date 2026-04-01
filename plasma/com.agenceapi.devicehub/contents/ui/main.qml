import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.plasma5support as P5
import org.kde.kirigami as Kirigami

PlasmoidItem {
    id: root

    property int batteryPercent: 0
    property real voltage: 0
    property int rssi: 0
    property string kbModel: ""
    property bool connected: false
    property int monBrightness: 50
    property int monContrast: 50
    property string fwVersion: ""
    property string batteryType: ""
    property string dischargeRate: ""

    preferredRepresentation: compactRepresentation
    compactRepresentation: CompactRepresentation {}
    fullRepresentation: FullRepresentation {}

    toolTipMainText: connected ? kbModel : "No device"
    toolTipSubText: connected
        ? batteryPercent + "% · " + voltage.toFixed(3) + "V · RSSI " + rssi + "dBm"
        : "Waiting for Apple Keyboard..."

    Timer {
        interval: 30000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: dataSource.connectSource("apple-kb-monitor --json 2>/dev/null")
    }

    Timer {
        interval: 60000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: monSource.connectSource("ddcutil getvcp 10 12 --brief --noverify 2>/dev/null")
    }

    function fetchData() {
        dataSource.connectSource("apple-kb-monitor --json 2>/dev/null")
    }

    function fetchMonitor() {
        monSource.connectSource("ddcutil getvcp 10 12 --brief --noverify 2>/dev/null")
    }

    function setMonitorValue(vcp, value) {
        ddcWrite.connectSource("python3 -c \"import os,fcntl;fd=os.open('/dev/i2c-6',os.O_RDWR);fcntl.ioctl(fd,0x0703,0x37);p=bytes([0x51,0x84,0x03," + vcp + ",0x00," + value + "]);c=0x6E\nfor b in p:c^=b\nos.write(fd,p+bytes([c&0xFF]));os.close(fd)\"")
    }

    P5.DataSource {
        id: dataSource
        engine: "executable"
        onNewData: function(source, data) {
            var stdout = data["stdout"]
            if (!stdout) return
            try {
                var d = JSON.parse(stdout)
                root.connected = true
                root.batteryPercent = d.battery.percentage_fine || d.battery.percentage || 0
                root.voltage = d.battery.voltage || 0
                root.kbModel = d.device.model || "Apple Keyboard"
                if (d.radio && d.radio.rssi_dbm !== undefined)
                    root.rssi = d.radio.rssi_dbm
                root.fwVersion = (d.firmware && d.firmware.version) ? d.firmware.version : ""
                root.batteryType = (d.analysis && d.analysis.battery_type) ? d.analysis.battery_type.type : ""
                root.dischargeRate = (d.analysis && d.analysis.discharge) ? d.analysis.discharge.remaining_display : ""
            } catch(e) {
                root.connected = false
            }
            disconnectSource(source)
        }
    }

    P5.DataSource {
        id: monSource
        engine: "executable"
        onNewData: function(source, data) {
            var stdout = data["stdout"]
            if (!stdout) { disconnectSource(source); return }
            var lines = stdout.split("\n")
            for (var i = 0; i < lines.length; i++) {
                var parts = lines[i].trim().split(/\s+/)
                if (parts.length >= 4) {
                    if (parts[1] === "0x10") root.monBrightness = parseInt(parts[3])
                    if (parts[1] === "0x12") root.monContrast = parseInt(parts[3])
                }
            }
            disconnectSource(source)
        }
    }

    P5.DataSource {
        id: ddcWrite
        engine: "executable"
        onNewData: function(source, data) { disconnectSource(source) }
    }
}
