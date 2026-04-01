import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.plasma5support as P5

Rectangle {
    id: full
    Layout.preferredWidth: 380
    Layout.preferredHeight: 500
    Layout.minimumWidth: 340
    Layout.minimumHeight: 440
    color: "#05080F"

    // Scanlines
    Canvas {
        anchors.fill: parent
        opacity: 0.02
        onPaint: {
            var ctx = getContext("2d")
            ctx.strokeStyle = "#00D4FF"
            ctx.lineWidth = 0.5
            for (var y = 0; y < height; y += 2) {
                ctx.beginPath()
                ctx.moveTo(0, y)
                ctx.lineTo(width, y)
                ctx.stroke()
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 12
        spacing: 8

        // ═══ HEADER ═══
        RowLayout {
            Layout.fillWidth: true
            Canvas {
                width: 18
                height: 18
                property real p: 0
                NumberAnimation on p { from: 0; to: 1; duration: 2000; loops: Animation.Infinite }
                onPChanged: requestPaint()
                onPaint: {
                    var ctx = getContext("2d")
                    ctx.clearRect(0, 0, 18, 18)
                    var g = ctx.createRadialGradient(9, 9, 0, 9, 9, 9)
                    g.addColorStop(0, "rgba(0,212,255," + (0.4 + p * 0.5) + ")")
                    g.addColorStop(1, "rgba(0,0,0,0)")
                    ctx.fillStyle = g
                    ctx.fillRect(0, 0, 18, 18)
                    ctx.fillStyle = "#00D4FF"
                    ctx.fillRect(7, 3, 4, 3)
                    ctx.fillRect(5, 6, 8, 4)
                    ctx.fillRect(7, 10, 4, 4)
                }
            }
            Text {
                text: "APIHUB"
                color: "#00D4FF"
                font.pixelSize: 16
                font.bold: true
                font.family: "monospace"
                font.letterSpacing: 4
            }
            Item { Layout.fillWidth: true }
            Rectangle {
                width: 8
                height: 8
                radius: 4
                color: root.connected ? "#00FF88" : "#FF4444"
                SequentialAnimation on opacity {
                    loops: Animation.Infinite
                    NumberAnimation { to: 0.2; duration: 800 }
                    NumberAnimation { to: 1.0; duration: 800 }
                }
            }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: "#12182A"; opacity: 0.6 }

        // ═══ TELEMETRY GRID (SpaceX style: big values, tiny labels) ═══
        GridLayout {
            Layout.fillWidth: true
            columns: 3
            rowSpacing: 2
            columnSpacing: 2

            // Battery %
            TelemetryTile {
                label: "BATTERY"
                value: root.batteryPercent.toString()
                unit: "%"
                accent: root.batteryPercent <= 15 ? "#FF4444"
                      : root.batteryPercent <= 50 ? "#FFaa00"
                      : "#00D4FF"
                Layout.fillWidth: true
                Layout.columnSpan: 1
            }

            // Voltage
            TelemetryTile {
                label: "VOLTAGE"
                value: root.voltage.toFixed(2)
                unit: "V"
                accent: "#FFFFFF"
                Layout.fillWidth: true
            }

            // RSSI
            TelemetryTile {
                label: "RSSI"
                value: root.rssi.toString()
                unit: "dBm"
                accent: "#FFFFFF"
                Layout.fillWidth: true
            }
        }

        // Battery bar
        Rectangle {
            Layout.fillWidth: true
            height: 4
            color: "#0E1420"
            radius: 2
            Rectangle {
                width: parent.width * root.batteryPercent / 100
                height: parent.height
                radius: 2
                color: root.batteryPercent <= 15 ? "#FF4444"
                     : root.batteryPercent <= 50 ? "#FFaa00"
                     : "#00D4FF"
                opacity: 0.7
                Behavior on width { NumberAnimation { duration: 400 } }
            }
        }

        // Status row
        RowLayout {
            Layout.fillWidth: true
            Text {
                text: root.kbModel
                color: "#2A3040"
                font.pixelSize: 10
                font.family: "monospace"
            }
            Item { Layout.fillWidth: true }
            Text {
                text: root.connected ? "ALL SYSTEMS NOMINAL" : "NO SIGNAL"
                color: root.connected ? "#00FF88" : "#FF4444"
                font.pixelSize: 9
                font.family: "monospace"
                font.letterSpacing: 1
            }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: "#12182A"; opacity: 0.6 }

        // ═══ MONITOR ═══
        Text {
            text: "MONITOR · DDC/CI"
            color: "#2A3040"
            font.pixelSize: 9
            font.family: "monospace"
            font.letterSpacing: 2
        }

        // Brightness + Contrast tiles
        GridLayout {
            Layout.fillWidth: true
            columns: 2
            rowSpacing: 2
            columnSpacing: 2

            TelemetryTile {
                label: "BRIGHTNESS"
                value: root.monBrightness.toString()
                unit: "%"
                accent: "#00D4FF"
                Layout.fillWidth: true
                clickable: true
                onTileClicked: function(ratio) {
                    root.monBrightness = Math.round(ratio * 100)
                    root.setMonitorValue(16, root.monBrightness)
                }
            }

            TelemetryTile {
                label: "CONTRAST"
                value: root.monContrast.toString()
                unit: "%"
                accent: "#FF6B35"
                Layout.fillWidth: true
                clickable: true
                onTileClicked: function(ratio) {
                    root.monContrast = Math.round(ratio * 100)
                    root.setMonitorValue(18, root.monContrast)
                }
            }
        }

        // Input grid
        RowLayout {
            Layout.fillWidth: true
            spacing: 2

            Repeater {
                model: [
                    {"label": "DP-1", "val": 15},
                    {"label": "DP-2", "val": 16},
                    {"label": "HDMI-1", "val": 17},
                    {"label": "HDMI-2", "val": 18}
                ]

                Rectangle {
                    Layout.fillWidth: true
                    height: 28
                    color: inputMa.pressed ? "#12182A" : "#0A0F1A"
                    radius: 2

                    Text {
                        anchors.centerIn: parent
                        text: modelData.label
                        color: "#445566"
                        font.pixelSize: 11
                        font.family: "monospace"
                        font.letterSpacing: 1
                    }

                    MouseArea {
                        id: inputMa
                        anchors.fill: parent
                        onClicked: root.setMonitorValue(96, modelData.val)
                    }
                }
            }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: "#12182A"; opacity: 0.6 }

        // ═══ ACTIONS ═══
        RowLayout {
            Layout.fillWidth: true
            spacing: 2

            Repeater {
                model: ["REFRESH", "STATUS", "GRAPH"]

                Rectangle {
                    Layout.fillWidth: true
                    height: 26
                    color: actMa.pressed ? "#12182A" : "#0A0F1A"
                    radius: 2

                    Text {
                        anchors.centerIn: parent
                        text: modelData
                        color: "#334455"
                        font.pixelSize: 10
                        font.family: "monospace"
                        font.letterSpacing: 2
                    }

                    MouseArea {
                        id: actMa
                        anchors.fill: parent
                        onClicked: {
                            if (modelData === "REFRESH") {
                                root.fetchData()
                                root.fetchMonitor()
                            } else if (modelData === "STATUS") {
                                actSource.connectSource("konsole -e 'apple-kb-monitor --status; read'")
                            } else {
                                actSource.connectSource("konsole -e 'apple-kb-monitor --graph; read'")
                            }
                        }
                    }
                }
            }
        }

        Item { Layout.fillHeight: true }
    }

    // ═══ TELEMETRY TILE COMPONENT ═══
    component TelemetryTile: Rectangle {
        property string label: ""
        property string value: ""
        property string unit: ""
        property color accent: "#FFFFFF"
        property bool clickable: false
        signal tileClicked(real ratio)

        height: 64
        color: "#0A0F1A"
        radius: 2

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 6
            spacing: 0

            Text {
                text: label
                color: "#2A3545"
                font.pixelSize: 9
                font.family: "monospace"
                font.letterSpacing: 1
            }

            Item { Layout.fillHeight: true }

            RowLayout {
                spacing: 2
                Layout.alignment: Qt.AlignBottom

                Text {
                    text: value
                    color: accent
                    font.pixelSize: 28
                    font.bold: true
                    font.family: "monospace"
                }
                Text {
                    text: unit
                    color: "#2A3545"
                    font.pixelSize: 12
                    font.family: "monospace"
                    Layout.alignment: Qt.AlignBottom
                    Layout.bottomMargin: 4
                }
            }
        }

        MouseArea {
            anchors.fill: parent
            visible: clickable
            onClicked: function(mouse) {
                tileClicked(mouse.x / parent.width)
            }
        }
    }

    P5.DataSource {
        id: actSource
        engine: "executable"
        onNewData: function(source, data) { disconnectSource(source) }
    }
}
