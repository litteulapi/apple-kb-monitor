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

    // ── Monitor properties ──
    property int monBrightness: 50
    property int monContrast: 50
    property int monRedGain: 50
    property int monGreenGain: 50
    property int monBlueGain: 50
    property int monVolume: 50
    property int monPictureMode: 0
    property int monInput: 15
    property int monMute: 2
    property int monUsageHours: 0
    property int monColorPreset: 0
    property int monSharpness: 50
    property int monAspect: 1
    property int monSmartEnergy: 0
    property int monResponseTime: 0
    property int monFreeSync: 0
    property int monBlackStabilizer: 50
    property int monGamma: 0
    property int monPowerLed: 0
    property string monFirmware: ""
    property int monHFreq: 0
    property int monVFreq: 0
    property int monOsdLock: 2
    property int monLanguage: 2
    property int monSplitMode: 1

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

    Timer {
        interval: 60000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: monSource.connectSource("ddc-tool json 6 2>/dev/null")
    }

    // ── Public functions ──
    function fetchData() {
        dataSource.connectSource("apple-kb-monitor --json 2>/dev/null");
    }

    function fetchMonitor() {
        monSource.connectSource("ddc-tool json 6 2>/dev/null");
    }

    function setMonitorValue(vcp, value) {
        ddcWrite.connectSource("ddc-tool write 6 " + vcp + " " + value);
        if (vcp == 16) {
            ddcWrite.connectSource("qdbus6 org.kde.plasmashell /org/kde/osdService org.kde.osdService.brightnessChanged " + value);
        }
    }

    function openSettings() {
        settingsLauncher.connectSource("/usr/bin/apihub-settings")
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

    // ── Monitor data source ──
    P5.DataSource {
        id: monSource
        engine: "executable"
        onNewData: function(source, data) {
            var stdout = data["stdout"];
            if (!stdout) { disconnectSource(source); return; }
            try {
                var m = JSON.parse(stdout);
                if (m.brightness) root.monBrightness = m.brightness.current;
                if (m.contrast) root.monContrast = m.contrast.current;
                if (m.red_gain) root.monRedGain = m.red_gain.current;
                if (m.green_gain) root.monGreenGain = m.green_gain.current;
                if (m.blue_gain) root.monBlueGain = m.blue_gain.current;
                if (m.volume) root.monVolume = m.volume.current;
                if (m.picture_mode) root.monPictureMode = m.picture_mode.current;
                if (m.input_source) root.monInput = m.input_source.current;
                if (m.audio_mute) root.monMute = m.audio_mute.current;
                if (m.usage_hours) root.monUsageHours = m.usage_hours.current;
                if (m.color_preset) root.monColorPreset = m.color_preset.current;
                if (m.sharpness) root.monSharpness = m.sharpness.current;
                if (m.aspect_ratio) root.monAspect = m.aspect_ratio.current;
                if (m.smart_energy) root.monSmartEnergy = m.smart_energy.current;
                if (m.response_time) root.monResponseTime = m.response_time.current;
                if (m.freesync) root.monFreeSync = m.freesync.current;
                if (m.black_stabilizer) root.monBlackStabilizer = m.black_stabilizer.current;
                if (m.gamma) root.monGamma = m.gamma.current;
                if (m.power_led) root.monPowerLed = m.power_led.current;
                if (m.firmware) root.monFirmware = (m.firmware.current >> 8) + "." + (m.firmware.current & 0xFF);
                if (m.h_freq) root.monHFreq = m.h_freq.current;
                if (m.v_freq) root.monVFreq = m.v_freq.current;
                if (m.osd_lock) root.monOsdLock = m.osd_lock.current;
                if (m.language) root.monLanguage = m.language.current;
                if (m.split_mode) root.monSplitMode = m.split_mode.current;
            } catch(e) {}
            disconnectSource(source);
        }
    }

    // ── DDC write data source ──
    P5.DataSource {
        id: ddcWrite
        engine: "executable"
        onNewData: function(source, data) { disconnectSource(source); }
    }

    // ── Settings launcher ──
    P5.DataSource {
        id: settingsLauncher
        engine: "executable"
        onNewData: function(source, data) { disconnectSource(source); }
    }
}
