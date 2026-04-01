import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasma5support as P5

Rectangle {
    Layout.preferredWidth: 440
    Layout.preferredHeight: 680
    Layout.minimumWidth: 400
    Layout.minimumHeight: 600
    color: "#000000"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 20
        spacing: 12

        // ═══ HEADER ═══
        RowLayout {
            Layout.fillWidth: true
            Text { text: "APIHUB"; color: "#00B4D8"; font.pixelSize: 22; font.bold: true; font.family: "monospace"; font.letterSpacing: 6 }
            Item { Layout.fillWidth: true }
            Rectangle { width: 10; height: 10; radius: 5; color: root.connected ? "#00FF88" : "#FF3333"
                SequentialAnimation on opacity { loops: Animation.Infinite; NumberAnimation { to: 0.3; duration: 1000 }; NumberAnimation { to: 1.0; duration: 1000 } }
            }
            Text { text: root.connected ? "NOMINAL" : "OFFLINE"; color: root.connected ? "#00FF88" : "#FF3333"; font.pixelSize: 13; font.family: "monospace"; font.letterSpacing: 2 }
        }
        Rectangle { Layout.fillWidth: true; height: 1; color: "#1A1A2A" }

        // ═══ KEYBOARD SECTION ═══
        Text { text: "KEYBOARD"; color: "#445"; font.pixelSize: 13; font.family: "monospace"; font.letterSpacing: 4 }

        // Big data row
        RowLayout {
            Layout.fillWidth: true
            spacing: 0

            // Battery — HUGE number
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                Text { text: "BATTERY"; color: "#667"; font.pixelSize: 12; font.family: "monospace" }
                RowLayout {
                    spacing: 2
                    Text { text: root.batteryPercent.toString(); color: root.batteryPercent <= 15 ? "#FF3333" : root.batteryPercent <= 50 ? "#FFAA00" : "#FFFFFF"; font.pixelSize: 52; font.bold: true; font.family: "monospace" }
                    Text { text: "%"; color: "#445"; font.pixelSize: 22; font.family: "monospace"; Layout.alignment: Qt.AlignBottom; Layout.bottomMargin: 10 }
                }
            }

            // Voltage
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                Text { text: "VOLTAGE"; color: "#667"; font.pixelSize: 12; font.family: "monospace" }
                RowLayout {
                    spacing: 2
                    Text { text: root.voltage.toFixed(2); color: "#FFFFFF"; font.pixelSize: 36; font.bold: true; font.family: "monospace" }
                    Text { text: "V"; color: "#445"; font.pixelSize: 16; font.family: "monospace"; Layout.alignment: Qt.AlignBottom; Layout.bottomMargin: 6 }
                }
            }

            // RSSI
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                Text { text: "RSSI"; color: "#667"; font.pixelSize: 12; font.family: "monospace" }
                RowLayout {
                    spacing: 2
                    Text { text: root.rssi.toString(); color: "#FFFFFF"; font.pixelSize: 36; font.bold: true; font.family: "monospace" }
                    Text { text: "dBm"; color: "#445"; font.pixelSize: 12; font.family: "monospace"; Layout.alignment: Qt.AlignBottom; Layout.bottomMargin: 6 }
                }
            }
        }

        // Battery bar
        Rectangle { Layout.fillWidth: true; height: 3; color: "#111"
            Rectangle { width: parent.width * root.batteryPercent / 100; height: parent.height; color: root.batteryPercent <= 15 ? "#FF3333" : root.batteryPercent <= 50 ? "#FFAA00" : "#00B4D8"; Behavior on width { NumberAnimation { duration: 500 } } }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: "#1A1A2A" }

        // ═══ MONITOR SECTION ═══
        Text { text: "MONITOR · LG 34GN850"; color: "#445"; font.pixelSize: 13; font.family: "monospace"; font.letterSpacing: 4 }

        // Brightness
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4
            RowLayout {
                Layout.fillWidth: true
                Text { text: "BRIGHTNESS"; color: "#667"; font.pixelSize: 12; font.family: "monospace" }
                Item { Layout.fillWidth: true }
                Text { text: root.monBrightness + "%"; color: "#FFFFFF"; font.pixelSize: 20; font.bold: true; font.family: "monospace" }
            }
            Rectangle {
                Layout.fillWidth: true; height: 24; color: "#111"; radius: 3
                Rectangle { width: parent.width * root.monBrightness / 100; height: parent.height; radius: 3; color: "#00B4D8"; opacity: 0.7; Behavior on width { NumberAnimation { duration: 200 } } }
                MouseArea { anchors.fill: parent; onClicked: function(m) { root.monBrightness = Math.round(m.x/parent.width*100); root.setMonitorValue(16, root.monBrightness) } }
            }
        }

        // Contrast
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4
            RowLayout {
                Layout.fillWidth: true
                Text { text: "CONTRAST"; color: "#667"; font.pixelSize: 12; font.family: "monospace" }
                Item { Layout.fillWidth: true }
                Text { text: root.monContrast + "%"; color: "#FFFFFF"; font.pixelSize: 20; font.bold: true; font.family: "monospace" }
            }
            Rectangle {
                Layout.fillWidth: true; height: 24; color: "#111"; radius: 3
                Rectangle { width: parent.width * root.monContrast / 100; height: parent.height; radius: 3; color: "#FF6B35"; opacity: 0.7; Behavior on width { NumberAnimation { duration: 200 } } }
                MouseArea { anchors.fill: parent; onClicked: function(m) { root.monContrast = Math.round(m.x/parent.width*100); root.setMonitorValue(18, root.monContrast) } }
            }
        }

        // Volume
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4
            RowLayout {
                Layout.fillWidth: true
                Text { text: "VOLUME"; color: "#667"; font.pixelSize: 12; font.family: "monospace" }
                Item { Layout.fillWidth: true }
                Text { id: volText; text: "50%"; color: "#FFFFFF"; font.pixelSize: 20; font.bold: true; font.family: "monospace" }
            }
            Rectangle {
                Layout.fillWidth: true; height: 24; color: "#111"; radius: 3
                Rectangle { id: volBar; width: parent.width * 0.5; height: parent.height; radius: 3; color: "#8888CC"; opacity: 0.7 }
                MouseArea { anchors.fill: parent; onClicked: function(m) { var v = Math.round(m.x/parent.width*100); volBar.width = parent.width*v/100; volText.text = v+"%"; root.setMonitorValue(98, v) } }
            }
        }

        // INPUT SOURCE
        Text { text: "INPUT"; color: "#667"; font.pixelSize: 12; font.family: "monospace" }
        RowLayout {
            Layout.fillWidth: true; spacing: 6
            Repeater {
                model: [{"t":"DP-1","v":15},{"t":"DP-2","v":16},{"t":"HDMI-1","v":17},{"t":"HDMI-2","v":18}]
                Rectangle { Layout.fillWidth: true; height: 36; color: ima.pressed ? "#1A1A2A" : "#0A0A14"; radius: 4
                    Text { anchors.centerIn: parent; text: modelData.t; color: "#AAB"; font.pixelSize: 14; font.bold: true; font.family: "monospace" }
                    MouseArea { id: ima; anchors.fill: parent; onClicked: root.setMonitorValue(96, modelData.v) }
                }
            }
        }

        // COLOR TEMP + POWER
        RowLayout {
            Layout.fillWidth: true; spacing: 6
            Repeater {
                model: [{"t":"6500K","c":20,"v":5},{"t":"9300K","c":20,"v":8},{"t":"USER","c":20,"v":11},{"t":"MUTE","c":141,"v":1},{"t":"PWR OFF","c":214,"v":4}]
                Rectangle { Layout.fillWidth: true; height: 36; color: cma.pressed ? "#1A1A2A" : "#0A0A14"; radius: 4
                    Text { anchors.centerIn: parent; text: modelData.t; color: modelData.t === "PWR OFF" ? "#FF3333" : "#AAB"; font.pixelSize: 13; font.family: "monospace" }
                    MouseArea { id: cma; anchors.fill: parent; onClicked: root.setMonitorValue(modelData.c, modelData.v) }
                }
            }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: "#1A1A2A" }

        // ACTIONS
        RowLayout {
            Layout.fillWidth: true; spacing: 6
            Repeater {
                model: ["REFRESH","STATUS","GRAPH"]
                Rectangle { Layout.fillWidth: true; height: 34; color: ama.pressed ? "#1A1A2A" : "#0A0A14"; radius: 4
                    Text { anchors.centerIn: parent; text: modelData; color: "#556"; font.pixelSize: 13; font.family: "monospace"; font.letterSpacing: 2 }
                    MouseArea { id: ama; anchors.fill: parent; onClicked: { if(modelData==="REFRESH"){root.fetchData();root.fetchMonitor()}else{actS.connectSource("konsole -e 'apple-kb-monitor --"+(modelData==="STATUS"?"status":"graph")+"; read'")} } }
                }
            }
        }

        Item { Layout.fillHeight: true }
    }

    P5.DataSource { id: actS; engine: "executable"; onNewData: function(s, d) { disconnectSource(s) } }
}
