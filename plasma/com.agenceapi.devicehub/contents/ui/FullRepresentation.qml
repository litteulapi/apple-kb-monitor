import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.plasma.plasma5support as P5
import org.kde.kirigami as Kirigami

QQC2.ScrollView {
    Layout.preferredWidth: 340
    Layout.preferredHeight: 580
    Layout.minimumWidth: 300
    Layout.minimumHeight: 400

    background: Rectangle {
        color: "#000000"
    }

    Flickable {
        contentWidth: availableWidth
        contentHeight: col.implicitHeight + 40

        Rectangle {
            anchors.fill: parent
            color: "#000000"
        }

        ColumnLayout {
            id: col
            width: parent.width
            spacing: 8

            property int redGain: 50
            property int greenGain: 50
            property int blueGain: 50

            // --- Top spacer ---
            Item {
                height: 8
            }

            // ===== HEADER =====
            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16

                Text {
                    text: "✦ APIHUB"
                    color: "#00B4D8"
                    font.pixelSize: 18
                    font.bold: true
                    font.family: "monospace"
                    font.letterSpacing: 4
                }

                Item {
                    Layout.fillWidth: true
                }

                Rectangle {
                    width: 8
                    height: 8
                    radius: 4
                    color: root.connected ? "#00FF88" : "#FF3333"

                    SequentialAnimation on opacity {
                        loops: Animation.Infinite

                        NumberAnimation {
                            to: 0.3
                            duration: 1000
                        }

                        NumberAnimation {
                            to: 1
                            duration: 1000
                        }
                    }
                }

                Text {
                    text: root.connected ? "OK" : "--"
                    color: root.connected ? "#00FF88" : "#FF3333"
                    font.pixelSize: 14
                    font.bold: true
                    font.family: "monospace"
                }
            }

            // ===== SEPARATOR =====
            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: "#1A1A2A"
                Layout.leftMargin: 16
                Layout.rightMargin: 16
            }

            // ===== BATTERY =====
            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                spacing: 4

                Text {
                    text: root.batteryPercent.toString()
                    color: root.batteryPercent <= 15 ? "#FF3333" : root.batteryPercent <= 50 ? "#FFAA00" : "#FFFFFF"
                    font.pixelSize: 56
                    font.bold: true
                    font.family: "monospace"
                }

                ColumnLayout {
                    spacing: 0
                    Layout.alignment: Qt.AlignBottom
                    Layout.bottomMargin: 12

                    Text {
                        text: "%"
                        color: "#556"
                        font.pixelSize: 18
                        font.family: "monospace"
                    }

                    Text {
                        text: "BATTERY"
                        color: "#334"
                        font.pixelSize: 9
                        font.family: "monospace"
                    }
                }

                Item {
                    Layout.fillWidth: true
                }

                ColumnLayout {
                    spacing: 6
                    Layout.alignment: Qt.AlignBottom
                    Layout.bottomMargin: 12

                    Row {
                        spacing: 6

                        Text {
                            text: root.voltage.toFixed(2)
                            color: "#FFF"
                            font.pixelSize: 18
                            font.bold: true
                            font.family: "monospace"
                        }

                        Text {
                            text: "V"
                            color: "#556"
                            font.pixelSize: 14
                            font.family: "monospace"
                        }
                    }

                    Row {
                        spacing: 6

                        Text {
                            text: root.rssi.toString()
                            color: "#FFF"
                            font.pixelSize: 18
                            font.bold: true
                            font.family: "monospace"
                        }

                        Text {
                            text: "dBm"
                            color: "#556"
                            font.pixelSize: 11
                            font.family: "monospace"
                        }
                    }
                }
            }

            // ===== BATTERY PROGRESS BAR =====
            Rectangle {
                Layout.fillWidth: true
                height: 3
                color: "#111"
                Layout.leftMargin: 16
                Layout.rightMargin: 16

                Rectangle {
                    width: parent.width * root.batteryPercent / 100
                    height: 3
                    color: root.batteryPercent <= 15 ? "#FF3333" : root.batteryPercent <= 50 ? "#FFAA00" : "#00B4D8"

                    Behavior on width {
                        NumberAnimation {
                            duration: 400
                        }
                    }
                }
            }

            // ===== KEYBOARD INFO =====
            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                spacing: 12

                Text {
                    text: "MODEL"
                    color: "#667"
                    font.pixelSize: 13
                    font.family: "monospace"
                }

                Text {
                    text: root.kbModel || "--"
                    color: "#FFF"
                    font.pixelSize: 18
                    font.bold: true
                    font.family: "monospace"
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                }

                Text {
                    text: "FW"
                    color: "#667"
                    font.pixelSize: 13
                    font.family: "monospace"
                }

                Text {
                    text: root.fwVersion || "--"
                    color: "#FFF"
                    font.pixelSize: 18
                    font.bold: true
                    font.family: "monospace"
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                spacing: 12

                Text {
                    text: "TYPE"
                    color: "#667"
                    font.pixelSize: 13
                    font.family: "monospace"
                }

                Text {
                    text: root.batteryType || "--"
                    color: "#FFF"
                    font.pixelSize: 18
                    font.bold: true
                    font.family: "monospace"
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                }

                Text {
                    text: "DRAIN"
                    color: "#667"
                    font.pixelSize: 13
                    font.family: "monospace"
                }

                Text {
                    text: root.dischargeRate || "--"
                    color: "#FFF"
                    font.pixelSize: 18
                    font.bold: true
                    font.family: "monospace"
                }
            }

            // ===== SEPARATOR =====
            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: "#1A1A2A"
                Layout.leftMargin: 16
                Layout.rightMargin: 16
            }

            // ===== MONITOR LABEL =====
            Text {
                text: "MONITOR"
                color: "#334"
                font.pixelSize: 11
                font.family: "monospace"
                font.letterSpacing: 3
                Layout.leftMargin: 16
            }

            // ===== BRIGHTNESS =====
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                spacing: 2

                RowLayout {
                    Layout.fillWidth: true

                    Text {
                        text: "BRIGHTNESS"
                        color: "#667"
                        font.pixelSize: 13
                        font.family: "monospace"
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Text {
                        text: root.monBrightness + "%"
                        color: "#FFF"
                        font.pixelSize: 18
                        font.bold: true
                        font.family: "monospace"
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    height: 20
                    color: "#111"
                    radius: 2

                    Rectangle {
                        width: parent.width * root.monBrightness / 100
                        height: 20
                        radius: 2
                        color: "#00B4D8"
                        opacity: 0.7

                        Behavior on width {
                            NumberAnimation {
                                duration: 150
                            }
                        }
                    }

                    MouseArea {
                        anchors.fill: parent

                        onPositionChanged: function(m) {
                            if (pressed) {
                                root.monBrightness = Math.max(0, Math.min(100, Math.round(m.x / parent.width * 100)));
                                root.setMonitorValue(16, root.monBrightness);
                            }
                        }

                        onClicked: function(m) {
                            root.monBrightness = Math.round(m.x / parent.width * 100);
                            root.setMonitorValue(16, root.monBrightness);
                        }
                    }
                }
            }

            // ===== CONTRAST =====
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                spacing: 2

                RowLayout {
                    Layout.fillWidth: true

                    Text {
                        text: "CONTRAST"
                        color: "#667"
                        font.pixelSize: 13
                        font.family: "monospace"
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Text {
                        text: root.monContrast + "%"
                        color: "#FFF"
                        font.pixelSize: 18
                        font.bold: true
                        font.family: "monospace"
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    height: 20
                    color: "#111"
                    radius: 2

                    Rectangle {
                        width: parent.width * root.monContrast / 100
                        height: 20
                        radius: 2
                        color: "#FF6B35"
                        opacity: 0.7

                        Behavior on width {
                            NumberAnimation {
                                duration: 150
                            }
                        }
                    }

                    MouseArea {
                        anchors.fill: parent

                        onPositionChanged: function(m) {
                            if (pressed) {
                                root.monContrast = Math.max(0, Math.min(100, Math.round(m.x / parent.width * 100)));
                                root.setMonitorValue(18, root.monContrast);
                            }
                        }

                        onClicked: function(m) {
                            root.monContrast = Math.round(m.x / parent.width * 100);
                            root.setMonitorValue(18, root.monContrast);
                        }
                    }
                }
            }

            // ===== VOLUME =====
            ColumnLayout {
                id: volumeSection
                property int vol: 50
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                spacing: 2

                RowLayout {
                    Layout.fillWidth: true

                    Text {
                        text: "VOLUME"
                        color: "#667"
                        font.pixelSize: 13
                        font.family: "monospace"
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Text {
                        text: volumeSection.vol + "%"
                        color: "#FFF"
                        font.pixelSize: 18
                        font.bold: true
                        font.family: "monospace"
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    height: 20
                    color: "#111"
                    radius: 2

                    Rectangle {
                        width: parent.width * volumeSection.vol / 100
                        height: 20
                        radius: 2
                        color: "#8888CC"
                        opacity: 0.7
                    }

                    MouseArea {
                        anchors.fill: parent

                        onPositionChanged: function(m) {
                            if (pressed) {
                                volumeSection.vol = Math.max(0, Math.min(100, Math.round(m.x / parent.width * 100)));
                                root.setMonitorValue(98, volumeSection.vol);
                            }
                        }

                        onClicked: function(m) {
                            volumeSection.vol = Math.round(m.x / parent.width * 100);
                            root.setMonitorValue(98, volumeSection.vol);
                        }
                    }
                }
            }

            // ===== SEPARATOR =====
            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: "#1A1A2A"
                Layout.leftMargin: 16
                Layout.rightMargin: 16
            }

            // ===== INPUT LABEL =====
            Text {
                text: "INPUT"
                color: "#334"
                font.pixelSize: 11
                font.family: "monospace"
                font.letterSpacing: 3
                Layout.leftMargin: 16
            }

            // ===== INPUT BUTTONS =====
            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                columns: 4
                columnSpacing: 4

                Repeater {
                    model: [
                        {"t": "DP-1", "v": 15},
                        {"t": "DP-2", "v": 16},
                        {"t": "HDMI-1", "v": 17},
                        {"t": "HDMI-2", "v": 18}
                    ]

                    Rectangle {
                        Layout.fillWidth: true
                        height: 34
                        color: inputArea.pressed ? "#1A1A2A" : "#0A0A14"
                        radius: 3

                        Text {
                            anchors.centerIn: parent
                            text: modelData.t
                            color: "#99AABB"
                            font.pixelSize: 13
                            font.bold: true
                            font.family: "monospace"
                        }

                        MouseArea {
                            id: inputArea
                            anchors.fill: parent
                            onClicked: root.setMonitorValue(96, modelData.v)
                        }
                    }
                }
            }

            // ===== PRESETS LABEL =====
            Text {
                text: "PRESETS"
                color: "#334"
                font.pixelSize: 11
                font.family: "monospace"
                font.letterSpacing: 3
                Layout.leftMargin: 16
            }

            // ===== PRESETS BUTTONS =====
            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                columns: 3
                columnSpacing: 4
                rowSpacing: 4

                Repeater {
                    model: [
                        {"t": "6500K", "c": 20, "v": 5},
                        {"t": "9300K", "c": 20, "v": 8},
                        {"t": "USER", "c": 20, "v": 11},
                        {"t": "MUTE", "c": 141, "v": 1},
                        {"t": "UNMUTE", "c": 141, "v": 2},
                        {"t": "PWR OFF", "c": 214, "v": 4}
                    ]

                    Rectangle {
                        Layout.fillWidth: true
                        height: 34
                        color: presetArea.pressed ? "#1A1A2A" : "#0A0A14"
                        radius: 3

                        Text {
                            anchors.centerIn: parent
                            text: modelData.t
                            color: modelData.t === "PWR OFF" ? "#FF3333" : "#99AABB"
                            font.pixelSize: 13
                            font.family: "monospace"
                        }

                        MouseArea {
                            id: presetArea
                            anchors.fill: parent
                            onClicked: root.setMonitorValue(modelData.c, modelData.v)
                        }
                    }
                }
            }

            // ===== SEPARATOR =====
            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: "#1A1A2A"
                Layout.leftMargin: 16
                Layout.rightMargin: 16
            }

            // ===== RGB GAINS LABEL =====
            Text {
                text: "RGB"
                color: "#334"
                font.pixelSize: 11
                font.family: "monospace"
                font.letterSpacing: 3
                Layout.leftMargin: 16
            }

            // ===== RED GAIN =====
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                spacing: 2

                RowLayout {
                    Layout.fillWidth: true

                    Text {
                        text: "RED"
                        color: "#667"
                        font.pixelSize: 13
                        font.family: "monospace"
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Text {
                        text: col.redGain + "%"
                        color: "#FFF"
                        font.pixelSize: 18
                        font.bold: true
                        font.family: "monospace"
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    height: 20
                    color: "#111"
                    radius: 2

                    Rectangle {
                        width: parent.width * col.redGain / 100
                        height: 20
                        radius: 2
                        color: "#FF4444"
                        opacity: 0.7

                        Behavior on width {
                            NumberAnimation {
                                duration: 150
                            }
                        }
                    }

                    MouseArea {
                        anchors.fill: parent

                        onPositionChanged: function(m) {
                            if (pressed) {
                                col.redGain = Math.max(0, Math.min(100, Math.round(m.x / parent.width * 100)));
                                root.setMonitorValue(22, col.redGain);
                            }
                        }

                        onClicked: function(m) {
                            col.redGain = Math.round(m.x / parent.width * 100);
                            root.setMonitorValue(22, col.redGain);
                        }
                    }
                }
            }

            // ===== GREEN GAIN =====
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                spacing: 2

                RowLayout {
                    Layout.fillWidth: true

                    Text {
                        text: "GREEN"
                        color: "#667"
                        font.pixelSize: 13
                        font.family: "monospace"
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Text {
                        text: col.greenGain + "%"
                        color: "#FFF"
                        font.pixelSize: 18
                        font.bold: true
                        font.family: "monospace"
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    height: 20
                    color: "#111"
                    radius: 2

                    Rectangle {
                        width: parent.width * col.greenGain / 100
                        height: 20
                        radius: 2
                        color: "#44FF44"
                        opacity: 0.7

                        Behavior on width {
                            NumberAnimation {
                                duration: 150
                            }
                        }
                    }

                    MouseArea {
                        anchors.fill: parent

                        onPositionChanged: function(m) {
                            if (pressed) {
                                col.greenGain = Math.max(0, Math.min(100, Math.round(m.x / parent.width * 100)));
                                root.setMonitorValue(24, col.greenGain);
                            }
                        }

                        onClicked: function(m) {
                            col.greenGain = Math.round(m.x / parent.width * 100);
                            root.setMonitorValue(24, col.greenGain);
                        }
                    }
                }
            }

            // ===== BLUE GAIN =====
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                spacing: 2

                RowLayout {
                    Layout.fillWidth: true

                    Text {
                        text: "BLUE"
                        color: "#667"
                        font.pixelSize: 13
                        font.family: "monospace"
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Text {
                        text: col.blueGain + "%"
                        color: "#FFF"
                        font.pixelSize: 18
                        font.bold: true
                        font.family: "monospace"
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    height: 20
                    color: "#111"
                    radius: 2

                    Rectangle {
                        width: parent.width * col.blueGain / 100
                        height: 20
                        radius: 2
                        color: "#4488FF"
                        opacity: 0.7

                        Behavior on width {
                            NumberAnimation {
                                duration: 150
                            }
                        }
                    }

                    MouseArea {
                        anchors.fill: parent

                        onPositionChanged: function(m) {
                            if (pressed) {
                                col.blueGain = Math.max(0, Math.min(100, Math.round(m.x / parent.width * 100)));
                                root.setMonitorValue(26, col.blueGain);
                            }
                        }

                        onClicked: function(m) {
                            col.blueGain = Math.round(m.x / parent.width * 100);
                            root.setMonitorValue(26, col.blueGain);
                        }
                    }
                }
            }

            // ===== SEPARATOR =====
            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: "#1A1A2A"
                Layout.leftMargin: 16
                Layout.rightMargin: 16
            }

            // ===== LG PICTURE MODE LABEL =====
            Text {
                text: "PICTURE MODE"
                color: "#334"
                font.pixelSize: 11
                font.family: "monospace"
                font.letterSpacing: 3
                Layout.leftMargin: 16
            }

            // ===== LG PICTURE MODE BUTTONS =====
            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                columns: 4
                columnSpacing: 4
                rowSpacing: 4

                Repeater {
                    model: [
                        {"t": "GAMER 1", "v": 1},
                        {"t": "GAMER 2", "v": 6},
                        {"t": "FPS", "v": 17},
                        {"t": "RTS", "v": 19},
                        {"t": "VIVID", "v": 21},
                        {"t": "READER", "v": 22},
                        {"t": "HDR", "v": 24},
                        {"t": "sRGB", "v": 32}
                    ]

                    Rectangle {
                        Layout.fillWidth: true
                        height: 34
                        color: picModeArea.pressed ? "#1A1A2A" : "#0A0A14"
                        radius: 3

                        Text {
                            anchors.centerIn: parent
                            text: modelData.t
                            color: "#99AABB"
                            font.pixelSize: 13
                            font.bold: true
                            font.family: "monospace"
                        }

                        MouseArea {
                            id: picModeArea
                            anchors.fill: parent
                            onClicked: root.setMonitorValue(21, modelData.v)
                        }
                    }
                }
            }

            // ===== SEPARATOR =====
            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: "#1A1A2A"
                Layout.leftMargin: 16
                Layout.rightMargin: 16
            }

            // ===== ACTIONS =====
            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                columns: 3
                columnSpacing: 4

                Repeater {
                    model: ["REFRESH", "STATUS", "GRAPH"]

                    Rectangle {
                        Layout.fillWidth: true
                        height: 32
                        color: actionArea.pressed ? "#1A1A2A" : "#0A0A14"
                        radius: 3

                        Text {
                            anchors.centerIn: parent
                            text: modelData
                            color: "#667"
                            font.pixelSize: 12
                            font.family: "monospace"
                            font.letterSpacing: 1
                        }

                        MouseArea {
                            id: actionArea
                            anchors.fill: parent

                            onClicked: {
                                if (modelData === "REFRESH") {
                                    root.fetchData();
                                    root.fetchMonitor();
                                } else {
                                    var cmd = modelData === "STATUS" ? "status" : "graph";
                                    actS.connectSource("konsole -e 'apple-kb-monitor --" + cmd + "; read'");
                                }
                            }
                        }
                    }
                }
            }

            // --- Bottom spacer ---
            Item {
                height: 8
            }
        }
    }

    P5.DataSource {
        id: actS
        engine: "executable"

        onNewData: function(s, d) {
            disconnectSource(s);
        }
    }
}
