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
        onTriggered: monSource.connectSource("ddcutil getvcp 0x10 0x12 0x16 0x18 0x1A 0x62 0x15 0x60 0x8D 0xC0 --brief --noverify 2>/dev/null")
    }

    function fetchData() {
        dataSource.connectSource("apple-kb-monitor --json 2>/dev/null")
    }

    function fetchMonitor() {
        monSource.connectSource("ddcutil getvcp 0x10 0x12 0x16 0x18 0x1A 0x62 0x15 0x60 0x8D 0xC0 --brief --noverify 2>/dev/null")
    }

    property int monRedGain: 50
    property int monGreenGain: 50
    property int monBlueGain: 50
    property int monVolume: 50
    property int monPictureMode: 0
    property int monInput: 15
    property int monMute: 2
    property int monUsageHours: 0

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
                    var vcp = parts[1]
                    var val = parseInt(parts[3])
                    if (vcp === "0x10") root.monBrightness = val
                    else if (vcp === "0x12") root.monContrast = val
                    else if (vcp === "0x16") root.monRedGain = val
                    else if (vcp === "0x18") root.monGreenGain = val
                    else if (vcp === "0x1A" || vcp === "0x1a") root.monBlueGain = val
                    else if (vcp === "0x62") root.monVolume = val
                    else if (vcp === "0x15") root.monPictureMode = val
                    else if (vcp === "0x60") root.monInput = val
                    else if (vcp === "0x8D" || vcp === "0x8d") root.monMute = val
                    else if (vcp === "0xC0" || vcp === "0xc0") root.monUsageHours = val
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
