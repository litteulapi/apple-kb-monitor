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

            // Helper function to decode input name from VCP value
            function inputName(v) {
                var map = {15: "DP-1", 16: "DP-2", 17: "HDMI-1", 18: "HDMI-2"};
                return map[v] || "??";
            }

            // Helper function to decode picture mode name from VCP value
            function picModeName(v) {
                var map = {1: "Gamer 1", 6: "Gamer 2", 17: "FPS", 19: "RTS", 20: "Vivid", 21: "Reader", 22: "HDR", 24: "sRGB", 45: "Custom"};
                return map[v] || "Mode " + v;
            }

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
                        text: root.monVolume + "%"
                        color: root.monMute === 1 ? "#FF3333" : "#FFF"
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
                        width: parent.width * root.monVolume / 100
                        height: 20
                        radius: 2
                        color: root.monMute === 1 ? "#FF3333" : "#8888CC"
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
                                root.monVolume = Math.max(0, Math.min(100, Math.round(m.x / parent.width * 100)));
                                root.setMonitorValue(98, root.monVolume);
                            }
                        }

                        onClicked: function(m) {
                            root.monVolume = Math.round(m.x / parent.width * 100);
                            root.setMonitorValue(98, root.monVolume);
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
                        required property var modelData
                        property bool active: modelData.v === root.monInput
                        Layout.fillWidth: true
                        height: 34
                        color: inputArea.pressed ? "#1A1A2A" : "#0A0A14"
                        radius: 3
                        border.width: active ? 2 : 0
                        border.color: active ? "#00B4D8" : "transparent"

                        Text {
                            anchors.centerIn: parent
                            text: parent.modelData.t
                            color: parent.active ? "#FFFFFF" : "#99AABB"
                            font.pixelSize: 13
                            font.bold: true
                            font.family: "monospace"
                        }

                        MouseArea {
                            id: inputArea
                            anchors.fill: parent
                            onClicked: root.setMonitorValue(96, parent.modelData.v)
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
            // TODO: Color preset active state needs root.monColorPreset added to main.qml
            //       (VCP 0x14). Once available, highlight 6500K/9300K/USER based on match.
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
                        required property var modelData
                        property bool isMuteBtn: modelData.t === "MUTE"
                        property bool isUnmuteBtn: modelData.t === "UNMUTE"
                        property bool muteActive: isMuteBtn && root.monMute === 1
                        property bool unmuteActive: isUnmuteBtn && root.monMute === 2
                        Layout.fillWidth: true
                        height: 34
                        color: presetArea.pressed ? "#1A1A2A" : "#0A0A14"
                        radius: 3
                        border.width: (muteActive || unmuteActive) ? 2 : 0
                        border.color: muteActive ? "#FF3333" : unmuteActive ? "#00B4D8" : "transparent"

                        Text {
                            anchors.centerIn: parent
                            text: parent.modelData.t
                            color: parent.muteActive ? "#FF3333"
                                : parent.modelData.t === "PWR OFF" ? "#FF3333"
                                : parent.unmuteActive ? "#FFFFFF"
                                : "#99AABB"
                            font.pixelSize: 13
                            font.bold: parent.muteActive
                            font.family: "monospace"
                        }

                        MouseArea {
                            id: presetArea
                            anchors.fill: parent
                            onClicked: root.setMonitorValue(parent.modelData.c, parent.modelData.v)
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
                        text: root.monRedGain + "%"
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
                        width: parent.width * root.monRedGain / 100
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
                                root.monRedGain = Math.max(0, Math.min(100, Math.round(m.x / parent.width * 100)));
                                root.setMonitorValue(22, root.monRedGain);
                            }
                        }

                        onClicked: function(m) {
                            root.monRedGain = Math.round(m.x / parent.width * 100);
                            root.setMonitorValue(22, root.monRedGain);
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
                        text: root.monGreenGain + "%"
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
                        width: parent.width * root.monGreenGain / 100
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
                                root.monGreenGain = Math.max(0, Math.min(100, Math.round(m.x / parent.width * 100)));
                                root.setMonitorValue(24, root.monGreenGain);
                            }
                        }

                        onClicked: function(m) {
                            root.monGreenGain = Math.round(m.x / parent.width * 100);
                            root.setMonitorValue(24, root.monGreenGain);
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
                        text: root.monBlueGain + "%"
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
                        width: parent.width * root.monBlueGain / 100
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
                                root.monBlueGain = Math.max(0, Math.min(100, Math.round(m.x / parent.width * 100)));
                                root.setMonitorValue(26, root.monBlueGain);
                            }
                        }

                        onClicked: function(m) {
                            root.monBlueGain = Math.round(m.x / parent.width * 100);
                            root.setMonitorValue(26, root.monBlueGain);
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
                        {"t": "VIVID", "v": 20},
                        {"t": "READER", "v": 21},
                        {"t": "HDR", "v": 22},
                        {"t": "sRGB", "v": 24},
                        {"t": "CUSTOM", "v": 45}
                    ]

                    Rectangle {
                        required property var modelData
                        property bool active: modelData.v === root.monPictureMode
                        Layout.fillWidth: true
                        height: 34
                        color: picModeArea.pressed ? "#1A1A2A" : "#0A0A14"
                        radius: 3
                        border.width: active ? 2 : 0
                        border.color: active ? "#00B4D8" : "transparent"

                        Text {
                            anchors.centerIn: parent
                            text: parent.modelData.t
                            color: parent.active ? "#FFFFFF" : "#99AABB"
                            font.pixelSize: 13
                            font.bold: true
                            font.family: "monospace"
                        }

                        MouseArea {
                            id: picModeArea
                            anchors.fill: parent
                            onClicked: root.setMonitorValue(21, parent.modelData.v)
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

            // ===== LG ADVANCED =====
            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: "#1A1A2A"
                Layout.leftMargin: 16
                Layout.rightMargin: 16
            }

            Text {
                text: "LG ADVANCED"
                color: "#334"
                font.pixelSize: 11
                font.family: "monospace"
                font.letterSpacing: 3
                Layout.leftMargin: 16
            }

            GridLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                columns: 4
                columnSpacing: 4
                rowSpacing: 4

                Repeater {
                    model: [
                        {"t": "FreeSync", "v": root.monFreeSync, "on": 2},
                        {"t": "HDR", "v": root.monHDR, "on": 1},
                        {"t": "DAS", "v": root.monDAS, "on": 1},
                        {"t": "Response", "v": root.monResponseTime, "on": -1}
                    ]

                    Rectangle {
                        Layout.fillWidth: true
                        height: 34
                        radius: 3
                        color: "#0A0A14"
                        border.color: modelData.on > 0 && modelData.v === modelData.on ? "#00B4D8" : "transparent"
                        border.width: modelData.on > 0 && modelData.v === modelData.on ? 2 : 0

                        Column {
                            anchors.centerIn: parent
                            spacing: 0

                            Text {
                                text: modelData.t
                                color: modelData.on > 0 && modelData.v === modelData.on ? "#00FF88" : "#99AABB"
                                font.pixelSize: 10
                                font.family: "monospace"
                                anchors.horizontalCenter: parent.horizontalCenter
                            }

                            Text {
                                text: modelData.on > 0 ? (modelData.v === modelData.on ? "ON" : "OFF") : modelData.v.toString()
                                color: modelData.on > 0 && modelData.v === modelData.on ? "#00FF88" : "#556"
                                font.pixelSize: 9
                                font.bold: true
                                font.family: "monospace"
                                anchors.horizontalCenter: parent.horizontalCenter
                            }
                        }
                    }
                }
            }

            // Gamma + Black Level
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                spacing: 2

                RowLayout {
                    Layout.fillWidth: true

                    Text {
                        text: "GAMMA"
                        color: "#667"
                        font.pixelSize: 11
                        font.family: "monospace"
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Text {
                        text: root.monGamma.toString()
                        color: "#FFF"
                        font.pixelSize: 14
                        font.bold: true
                        font.family: "monospace"
                    }
                }

                RowLayout {
                    Layout.fillWidth: true

                    Text {
                        text: "BLACK LVL"
                        color: "#667"
                        font.pixelSize: 11
                        font.family: "monospace"
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Text {
                        text: root.monBlackLevel.toString()
                        color: "#FFF"
                        font.pixelSize: 14
                        font.bold: true
                        font.family: "monospace"
                    }
                }
            }

            // Monitor info bar
            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                spacing: 1

                Rectangle {
                    Layout.fillWidth: true
                    height: 1
                    color: "#1A1A2A"
                }

                Text {
                    text: "MONITOR INFO"
                    color: "#334"
                    font.pixelSize: 9
                    font.family: "monospace"
                    font.letterSpacing: 2
                }

                GridLayout {
                    Layout.fillWidth: true
                    columns: 2
                    rowSpacing: 1
                    columnSpacing: 12

                    Text { text: "MODEL"; color: "#445"; font.pixelSize: 9; font.family: "monospace" }
                    Text { text: "LG 34GN850"; color: "#778"; font.pixelSize: 9; font.family: "monospace" }

                    Text { text: "PANEL"; color: "#445"; font.pixelSize: 9; font.family: "monospace" }
                    Text { text: "IPS 3440x1440 10-bit"; color: "#778"; font.pixelSize: 9; font.family: "monospace" }

                    Text { text: "FIRMWARE"; color: "#445"; font.pixelSize: 9; font.family: "monospace" }
                    Text { text: root.monFirmware || "3.0"; color: "#778"; font.pixelSize: 9; font.family: "monospace" }

                    Text { text: "REFRESH"; color: "#445"; font.pixelSize: 9; font.family: "monospace" }
                    Text { text: (root.monVFreq / 100).toFixed(1) + " Hz"; color: "#778"; font.pixelSize: 9; font.family: "monospace" }

                    Text { text: "USAGE"; color: "#445"; font.pixelSize: 9; font.family: "monospace" }
                    Text { text: root.monUsageHours + "h (" + Math.round(root.monUsageHours / 8760 * 10) / 10 + " yrs)"; color: "#778"; font.pixelSize: 9; font.family: "monospace" }

                    Text { text: "GPU"; color: "#445"; font.pixelSize: 9; font.family: "monospace" }
                    Text { text: "RTX 4070 SUPER"; color: "#778"; font.pixelSize: 9; font.family: "monospace" }
                }
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

            // ===== STATUS BAR =====
            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: "#1A1A2A"
                Layout.leftMargin: 16
                Layout.rightMargin: 16
            }

            Text {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                text: col.inputName(root.monInput)
                    + " \u00B7 " + col.picModeName(root.monPictureMode)
                    + (root.monMute === 1 ? " \u00B7 MUTED" : "")
                    + " \u00B7 " + root.monUsageHours + "h"
                color: "#445"
                font.pixelSize: 10
                font.family: "monospace"
                elide: Text.ElideRight
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
