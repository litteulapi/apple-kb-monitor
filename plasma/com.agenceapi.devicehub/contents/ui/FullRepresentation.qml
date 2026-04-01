import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: full
    Layout.preferredWidth: Kirigami.Units.gridUnit * 22
    Layout.preferredHeight: Kirigami.Units.gridUnit * 28
    spacing: 0

    // ═══ HEADER ═══
    Rectangle {
        Layout.fillWidth: true
        height: Kirigami.Units.gridUnit * 3
        color: "#0A0E17"
        radius: 4

        RowLayout {
            anchors.fill: parent
            anchors.margins: Kirigami.Units.smallSpacing * 2

            // Logo
            Rectangle {
                width: Kirigami.Units.gridUnit * 2
                height: width
                radius: width / 2
                color: root.connected ? "#00D4FF" : "#444"

                Text {
                    anchors.centerIn: parent
                    text: "⌨"
                    font.pixelSize: parent.height * 0.5
                }
            }

            ColumnLayout {
                spacing: 0
                Text {
                    text: "APIHUB"
                    color: "#00D4FF"
                    font { pixelSize: 14; bold: true; family: "monospace"; letterSpacing: 2 }
                }
                Text {
                    text: "v2.4.0 · AgenceAPI"
                    color: "#555"
                    font { pixelSize: 10; family: "monospace" }
                }
            }

            Item { Layout.fillWidth: true }

            // Status indicator
            Rectangle {
                width: 8; height: 8; radius: 4
                color: root.connected ? "#00FF88" : "#FF4444"

                SequentialAnimation on opacity {
                    loops: Animation.Infinite
                    NumberAnimation { to: 0.3; duration: 1000 }
                    NumberAnimation { to: 1.0; duration: 1000 }
                }
            }
        }
    }

    // ═══ KEYBOARD SECTION ═══
    Rectangle {
        Layout.fillWidth: true
        Layout.topMargin: 2
        height: childrenRect.height + Kirigami.Units.gridUnit
        color: "#0D1117"
        radius: 4

        ColumnLayout {
            anchors { left: parent.left; right: parent.right; top: parent.top; margins: Kirigami.Units.smallSpacing * 2 }
            spacing: Kirigami.Units.smallSpacing

            // Section title
            Text {
                text: "⌨  APPLE WIRELESS KEYBOARD"
                color: "#00D4FF"
                font { pixelSize: 11; bold: true; family: "monospace"; letterSpacing: 1 }
            }

            Text {
                text: root.connected ? root.model : "Not connected"
                color: root.connected ? "#888" : "#FF4444"
                font { pixelSize: 10; family: "monospace" }
            }

            // Battery bar
            Rectangle {
                Layout.fillWidth: true
                height: 24
                color: "#1A1F2E"
                radius: 4

                Rectangle {
                    width: parent.width * (root.batteryPercent / 100)
                    height: parent.height
                    radius: 4
                    color: root.batteryPercent <= 15 ? "#FF4444"
                         : root.batteryPercent <= 50 ? "#FFaa00"
                         : "#00D4FF"
                    opacity: 0.8

                    Behavior on width { NumberAnimation { duration: 300 } }
                }

                Text {
                    anchors.centerIn: parent
                    text: root.batteryPercent + "%"
                    color: "#FFF"
                    font { pixelSize: 12; bold: true; family: "monospace" }
                }
            }

            // Telemetry grid
            GridLayout {
                Layout.fillWidth: true
                columns: 2
                rowSpacing: 2
                columnSpacing: Kirigami.Units.gridUnit

                DataRow { label: "VOLTAGE"; value: root.voltage.toFixed(3) + " V" }
                DataRow { label: "RSSI"; value: root.rssi + " dBm" }
                DataRow { label: "BATTERY"; value: root.batteryPercent + "% (fine)" }
                DataRow { label: "STATUS"; value: root.connected ? "ONLINE" : "OFFLINE"; valueColor: root.connected ? "#00FF88" : "#FF4444" }
            }
        }
    }

    // ═══ MONITOR SECTION ═══
    Rectangle {
        Layout.fillWidth: true
        Layout.topMargin: 2
        height: childrenRect.height + Kirigami.Units.gridUnit
        color: "#0D1117"
        radius: 4

        ColumnLayout {
            anchors { left: parent.left; right: parent.right; top: parent.top; margins: Kirigami.Units.smallSpacing * 2 }
            spacing: Kirigami.Units.smallSpacing

            Text {
                text: "🖥  LG 34GN850 · DDC/CI"
                color: "#FF6B35"
                font { pixelSize: 11; bold: true; family: "monospace"; letterSpacing: 1 }
            }

            // Brightness slider
            RowLayout {
                Layout.fillWidth: true
                Text { text: "BRI"; color: "#888"; font { pixelSize: 10; family: "monospace" }; Layout.preferredWidth: 30 }
                QQC2.Slider {
                    id: briSlider
                    Layout.fillWidth: true
                    from: 0; to: 100; stepSize: 1
                    value: root.monBrightness
                    onMoved: {
                        root.monBrightness = value
                        root.setMonitorValue(16, Math.round(value))
                    }
                }
                Text { text: root.monBrightness + "%"; color: "#00D4FF"; font { pixelSize: 10; bold: true; family: "monospace" }; Layout.preferredWidth: 35 }
            }

            // Contrast slider
            RowLayout {
                Layout.fillWidth: true
                Text { text: "CON"; color: "#888"; font { pixelSize: 10; family: "monospace" }; Layout.preferredWidth: 30 }
                QQC2.Slider {
                    id: conSlider
                    Layout.fillWidth: true
                    from: 0; to: 100; stepSize: 1
                    value: root.monContrast
                    onMoved: {
                        root.monContrast = value
                        root.setMonitorValue(18, Math.round(value))
                    }
                }
                Text { text: root.monContrast + "%"; color: "#FF6B35"; font { pixelSize: 10; bold: true; family: "monospace" }; Layout.preferredWidth: 35 }
            }

            // Input source buttons
            Text {
                text: "INPUT"
                color: "#888"
                font { pixelSize: 10; family: "monospace" }
                Layout.topMargin: Kirigami.Units.smallSpacing
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Repeater {
                    model: [
                        { label: "DP-1", vcp: 96, val: 15 },
                        { label: "DP-2", vcp: 96, val: 16 },
                        { label: "HDMI-1", vcp: 96, val: 17 },
                        { label: "HDMI-2", vcp: 96, val: 18 }
                    ]

                    QQC2.Button {
                        text: modelData.label
                        Layout.fillWidth: true
                        font { pixelSize: 9; family: "monospace" }
                        onClicked: root.setMonitorValue(modelData.vcp, modelData.val)
                    }
                }
            }
        }
    }

    // ═══ ACTIONS ═══
    Rectangle {
        Layout.fillWidth: true
        Layout.topMargin: 2
        height: childrenRect.height + Kirigami.Units.gridUnit
        color: "#0D1117"
        radius: 4

        RowLayout {
            anchors { left: parent.left; right: parent.right; top: parent.top; margins: Kirigami.Units.smallSpacing * 2 }
            spacing: Kirigami.Units.smallSpacing

            QQC2.Button {
                text: "⟳ Refresh"
                font { pixelSize: 10; family: "monospace" }
                onClicked: { root.fetchData(); root.fetchMonitor() }
            }

            QQC2.Button {
                text: "📊 Status"
                font { pixelSize: 10; family: "monospace" }
                onClicked: statusSource.exec("konsole -e 'apple-kb-monitor --status; read'")
            }

            QQC2.Button {
                text: "📈 Graph"
                font { pixelSize: 10; family: "monospace" }
                onClicked: statusSource.exec("konsole -e 'apple-kb-monitor --graph; read'")
            }

            Item { Layout.fillWidth: true }
        }
    }

    Item { Layout.fillHeight: true }

    // Status source for action buttons
    PlasmaCore.DataSource {
        id: statusSource
        engine: "executable"
        function exec(cmd) { connectSource(cmd) }
    }

    // DataRow component
    component DataRow: RowLayout {
        property string label: ""
        property string value: ""
        property color valueColor: "#00D4FF"

        Text { text: label; color: "#555"; font { pixelSize: 10; family: "monospace" }; Layout.preferredWidth: 70 }
        Text { text: value; color: valueColor; font { pixelSize: 10; bold: true; family: "monospace" } }
    }
}
