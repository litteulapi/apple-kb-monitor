import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents3
import org.kde.plasma.extras as PlasmaExtras
import org.kde.plasma.plasma5support as P5

ColumnLayout {
    id: fullRep

    Layout.preferredWidth: Kirigami.Units.gridUnit * 24
    Layout.preferredHeight: Kirigami.Units.gridUnit * 36
    Layout.minimumWidth: Kirigami.Units.gridUnit * 20
    Layout.minimumHeight: Kirigami.Units.gridUnit * 24

    spacing: 0

    // ── Helper functions ──
    function inputName(v) {
        var map = {15: "DP-1", 16: "DP-2", 17: "HDMI-1", 18: "HDMI-2"};
        return map[v] || "??";
    }

    function picModeName(v) {
        var map = {
            1: "Reader", 6: "Gamer 2", 17: "Custom", 19: "RTS",
            20: "Vivid", 21: "sRGB", 25: "EBU", 32: "Photo",
            40: "FPS 1", 41: "FPS 2", 45: "Custom 2", 72: "Cinema"
        };
        return map[v] || "Mode " + v;
    }

    // ── Tab bar ──
    PlasmaComponents3.TabBar {
        id: tabBar
        Layout.fillWidth: true

        PlasmaComponents3.TabButton {
            icon.name: "input-keyboard"
            text: "Keyboard"
        }
        PlasmaComponents3.TabButton {
            icon.name: "video-display"
            text: "Monitor"
        }
        PlasmaComponents3.TabButton {
            icon.name: "documentinfo"
            text: "System"
        }
    }

    // ── Tab content ──
    QQC2.SwipeView {
        id: swipeView
        Layout.fillWidth: true
        Layout.fillHeight: true
        currentIndex: tabBar.currentIndex
        clip: true

        onCurrentIndexChanged: tabBar.currentIndex = currentIndex

        // ════════════════════════════════════════
        //  TAB 0 : KEYBOARD
        // ════════════════════════════════════════
        QQC2.ScrollView {
            id: keyboardTab

            Flickable {
                contentWidth: parent.width
                contentHeight: kbCol.implicitHeight + Kirigami.Units.largeSpacing * 2

                ColumnLayout {
                    id: kbCol
                    width: parent.width
                    spacing: Kirigami.Units.smallSpacing

                    // ── Battery section ──
                    PlasmaExtras.Heading {
                        text: "Battery"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.topMargin: Kirigami.Units.largeSpacing
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        PlasmaComponents3.Label {
                            text: root.batteryPercent.toString()
                            font.pixelSize: Kirigami.Units.gridUnit * 3
                            font.bold: true
                            color: root.batteryPercent <= 15
                                ? Kirigami.Theme.negativeTextColor
                                : root.batteryPercent <= 50
                                    ? Kirigami.Theme.neutralTextColor
                                    : Kirigami.Theme.textColor
                        }

                        PlasmaComponents3.Label {
                            text: "%"
                            font.pixelSize: Kirigami.Units.gridUnit * 1.2
                            color: Kirigami.Theme.disabledTextColor
                            Layout.alignment: Qt.AlignBottom
                            Layout.bottomMargin: Kirigami.Units.gridUnit * 0.6
                        }

                        Item { Layout.fillWidth: true }

                        ColumnLayout {
                            Layout.alignment: Qt.AlignBottom
                            Layout.bottomMargin: Kirigami.Units.gridUnit * 0.4
                            spacing: Kirigami.Units.smallSpacing

                            RowLayout {
                                spacing: Kirigami.Units.smallSpacing
                                PlasmaComponents3.Label {
                                    text: root.voltage.toFixed(3)
                                    font.bold: true
                                }
                                PlasmaComponents3.Label {
                                    text: "V"
                                    color: Kirigami.Theme.disabledTextColor
                                }
                            }

                            RowLayout {
                                spacing: Kirigami.Units.smallSpacing
                                PlasmaComponents3.Label {
                                    text: root.rssi.toString()
                                    font.bold: true
                                }
                                PlasmaComponents3.Label {
                                    text: "dBm"
                                    color: Kirigami.Theme.disabledTextColor
                                }
                            }
                        }
                    }

                    PlasmaComponents3.ProgressBar {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        from: 0
                        to: 100
                        value: root.batteryPercent
                    }

                    // ── Device section ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Device"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    Kirigami.FormLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing

                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Model:"
                            text: root.kbModel || "--"
                        }
                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Firmware:"
                            text: root.fwVersion || "--"
                        }
                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Status:"
                            text: root.connected ? "Connected" : "Disconnected"
                            color: root.connected
                                ? Kirigami.Theme.positiveTextColor
                                : Kirigami.Theme.negativeTextColor
                        }
                    }

                    // ── Analysis section ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Analysis"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    Kirigami.FormLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing

                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Battery type:"
                            text: root.batteryType || "--"
                        }
                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Remaining:"
                            text: root.dischargeRate || "--"
                        }
                    }

                    Item { Layout.fillHeight: true }
                }
            }
        }

        // ════════════════════════════════════════
        //  TAB 1 : MONITOR
        // ════════════════════════════════════════
        QQC2.ScrollView {
            id: monitorTab

            Flickable {
                contentWidth: parent.width
                contentHeight: monCol.implicitHeight + Kirigami.Units.largeSpacing * 2

                ColumnLayout {
                    id: monCol
                    width: parent.width
                    spacing: Kirigami.Units.smallSpacing

                    // ── Picture Mode section ──
                    PlasmaExtras.Heading {
                        text: "Picture Mode"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.topMargin: Kirigami.Units.largeSpacing
                    }

                    GridLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        columns: 4
                        columnSpacing: Kirigami.Units.smallSpacing
                        rowSpacing: Kirigami.Units.smallSpacing

                        Repeater {
                            model: [
                                {"t": "READER", "v": 1},
                                {"t": "GAMER 2", "v": 6},
                                {"t": "CUSTOM", "v": 17},
                                {"t": "RTS", "v": 19},
                                {"t": "VIVID", "v": 20},
                                {"t": "sRGB", "v": 21},
                                {"t": "FPS 1", "v": 40},
                                {"t": "FPS 2", "v": 41},
                                {"t": "PHOTO", "v": 32},
                                {"t": "CINEMA", "v": 72},
                                {"t": "EBU", "v": 25},
                                {"t": "CUSTOM 2", "v": 45}
                            ]

                            Rectangle {
                                required property var modelData
                                property bool active: root.monPictureMode === modelData.v

                                Layout.fillWidth: true
                                implicitHeight: Kirigami.Units.gridUnit * 1.8
                                radius: Kirigami.Units.cornerRadius
                                color: active
                                    ? Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)
                                    : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                                border.width: active ? 2 : 0
                                border.color: Kirigami.Theme.highlightColor

                                PlasmaComponents3.Label {
                                    anchors.centerIn: parent
                                    text: modelData.t
                                    font: Kirigami.Theme.smallFont
                                    color: active
                                        ? Kirigami.Theme.highlightedTextColor
                                        : Kirigami.Theme.textColor
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: root.setMonitorValue(21, modelData.v)
                                }
                            }
                        }
                    }

                    // ── Brightness / Contrast section ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Backlight"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    // Brightness
                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: 0

                        RowLayout {
                            Layout.fillWidth: true
                            PlasmaComponents3.Label {
                                text: "Brightness"
                            }
                            Item { Layout.fillWidth: true }
                            PlasmaComponents3.Label {
                                text: root.monBrightness + "%"
                                font.bold: true
                            }
                        }

                        PlasmaComponents3.Slider {
                            Layout.fillWidth: true
                            from: 0
                            to: 100
                            value: root.monBrightness
                            stepSize: 1
                            onMoved: {
                                root.monBrightness = value;
                                root.setMonitorValue(16, value);
                            }
                        }
                    }

                    // Contrast
                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: 0

                        RowLayout {
                            Layout.fillWidth: true
                            PlasmaComponents3.Label {
                                text: "Contrast"
                            }
                            Item { Layout.fillWidth: true }
                            PlasmaComponents3.Label {
                                text: root.monContrast + "%"
                                font.bold: true
                            }
                        }

                        PlasmaComponents3.Slider {
                            Layout.fillWidth: true
                            from: 0
                            to: 100
                            value: root.monContrast
                            stepSize: 1
                            onMoved: {
                                root.monContrast = value;
                                root.setMonitorValue(18, value);
                            }
                        }
                    }

                    // ── Color section ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Color"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    // Red gain
                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: 0

                        RowLayout {
                            Layout.fillWidth: true
                            PlasmaComponents3.Label {
                                text: "Red"
                                color: Kirigami.Theme.negativeTextColor
                            }
                            Item { Layout.fillWidth: true }
                            PlasmaComponents3.Label {
                                text: root.monRedGain + "%"
                                font.bold: true
                            }
                        }

                        PlasmaComponents3.Slider {
                            Layout.fillWidth: true
                            from: 0
                            to: 100
                            value: root.monRedGain
                            stepSize: 1
                            onMoved: {
                                root.monRedGain = value;
                                root.setMonitorValue(22, value);
                            }
                        }
                    }

                    // Green gain
                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: 0

                        RowLayout {
                            Layout.fillWidth: true
                            PlasmaComponents3.Label {
                                text: "Green"
                                color: Kirigami.Theme.positiveTextColor
                            }
                            Item { Layout.fillWidth: true }
                            PlasmaComponents3.Label {
                                text: root.monGreenGain + "%"
                                font.bold: true
                            }
                        }

                        PlasmaComponents3.Slider {
                            Layout.fillWidth: true
                            from: 0
                            to: 100
                            value: root.monGreenGain
                            stepSize: 1
                            onMoved: {
                                root.monGreenGain = value;
                                root.setMonitorValue(24, value);
                            }
                        }
                    }

                    // Blue gain
                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: 0

                        RowLayout {
                            Layout.fillWidth: true
                            PlasmaComponents3.Label {
                                text: "Blue"
                                color: Kirigami.Theme.linkColor
                            }
                            Item { Layout.fillWidth: true }
                            PlasmaComponents3.Label {
                                text: root.monBlueGain + "%"
                                font.bold: true
                            }
                        }

                        PlasmaComponents3.Slider {
                            Layout.fillWidth: true
                            from: 0
                            to: 100
                            value: root.monBlueGain
                            stepSize: 1
                            onMoved: {
                                root.monBlueGain = value;
                                root.setMonitorValue(26, value);
                            }
                        }
                    }

                    // Color presets
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Repeater {
                            model: [
                                {"t": "6500K", "v": 5},
                                {"t": "9300K", "v": 8},
                                {"t": "USER", "v": 11}
                            ]

                            PlasmaComponents3.Button {
                                required property var modelData
                                Layout.fillWidth: true
                                text: modelData.t
                                checked: root.monColorPreset === modelData.v
                                highlighted: root.monColorPreset === modelData.v
                                onClicked: root.setMonitorValue(20, modelData.v)
                            }
                        }
                    }

                    // ── Audio section ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Audio"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: 0

                        RowLayout {
                            Layout.fillWidth: true
                            PlasmaComponents3.Label {
                                text: "Volume"
                            }
                            Item { Layout.fillWidth: true }
                            PlasmaComponents3.Label {
                                text: root.monVolume + "%"
                                font.bold: true
                                color: root.monMute === 1
                                    ? Kirigami.Theme.negativeTextColor
                                    : Kirigami.Theme.textColor
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Kirigami.Units.smallSpacing

                            PlasmaComponents3.Slider {
                                Layout.fillWidth: true
                                from: 0
                                to: 100
                                value: root.monVolume
                                stepSize: 1
                                onMoved: {
                                    root.monVolume = value;
                                    root.setMonitorValue(98, value);
                                }
                            }

                            PlasmaComponents3.ToolButton {
                                icon.name: root.monMute === 1
                                    ? "audio-volume-muted"
                                    : "audio-volume-high"
                                checked: root.monMute === 1
                                onClicked: {
                                    var newVal = root.monMute === 1 ? 2 : 1;
                                    root.monMute = newVal;
                                    root.setMonitorValue(141, newVal);
                                }
                            }
                        }
                    }

                    // ── Input section ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Input"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Repeater {
                            model: [
                                {"t": "DP-1", "v": 15},
                                {"t": "DP-2", "v": 16},
                                {"t": "HDMI-1", "v": 17},
                                {"t": "HDMI-2", "v": 18}
                            ]

                            PlasmaComponents3.Button {
                                required property var modelData
                                Layout.fillWidth: true
                                text: modelData.t
                                checked: root.monInput === modelData.v
                                highlighted: root.monInput === modelData.v
                                onClicked: root.setMonitorValue(96, modelData.v)
                            }
                        }
                    }

                    // ── Display section ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Display"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    // Sharpness slider
                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: 0

                        RowLayout {
                            Layout.fillWidth: true
                            PlasmaComponents3.Label {
                                text: "Sharpness"
                            }
                            Item { Layout.fillWidth: true }
                            PlasmaComponents3.Label {
                                text: root.monSharpness + "%"
                                font.bold: true
                            }
                        }

                        PlasmaComponents3.Slider {
                            Layout.fillWidth: true
                            from: 0
                            to: 100
                            value: root.monSharpness
                            stepSize: 1
                            onMoved: {
                                root.monSharpness = value;
                                root.setMonitorValue(135, value);
                            }
                        }
                    }

                    // Aspect Ratio buttons
                    PlasmaComponents3.Label {
                        text: "Aspect Ratio"
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Repeater {
                            model: [
                                {"t": "WIDE", "v": 1},
                                {"t": "ORIGINAL", "v": 2},
                                {"t": "CINEMA 1", "v": 3},
                                {"t": "CINEMA 2", "v": 4}
                            ]

                            Rectangle {
                                required property var modelData
                                property bool active: root.monAspect === modelData.v

                                Layout.fillWidth: true
                                implicitHeight: Kirigami.Units.gridUnit * 1.8
                                radius: Kirigami.Units.cornerRadius
                                color: active
                                    ? Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)
                                    : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                                border.width: active ? 2 : 0
                                border.color: Kirigami.Theme.highlightColor

                                PlasmaComponents3.Label {
                                    anchors.centerIn: parent
                                    text: modelData.t
                                    font: Kirigami.Theme.smallFont
                                    color: active
                                        ? Kirigami.Theme.highlightedTextColor
                                        : Kirigami.Theme.textColor
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: root.setMonitorValue(245, modelData.v)
                                }
                            }
                        }
                    }

                    // ── Advanced section ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Advanced"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    // Response Time buttons
                    PlasmaComponents3.Label {
                        text: "Response Time"
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Repeater {
                            model: [
                                {"t": "OFF", "v": 0},
                                {"t": "FAST", "v": 1},
                                {"t": "NORMAL", "v": 2},
                                {"t": "SLOW", "v": 3}
                            ]

                            Rectangle {
                                required property var modelData
                                property bool active: root.monResponseTime === modelData.v

                                Layout.fillWidth: true
                                implicitHeight: Kirigami.Units.gridUnit * 1.8
                                radius: Kirigami.Units.cornerRadius
                                color: active
                                    ? Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)
                                    : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                                border.width: active ? 2 : 0
                                border.color: Kirigami.Theme.highlightColor

                                PlasmaComponents3.Label {
                                    anchors.centerIn: parent
                                    text: modelData.t
                                    font: Kirigami.Theme.smallFont
                                    color: active
                                        ? Kirigami.Theme.highlightedTextColor
                                        : Kirigami.Theme.textColor
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: root.setMonitorValue(247, modelData.v)
                                }
                            }
                        }
                    }

                    // FreeSync toggle
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        Layout.topMargin: Kirigami.Units.smallSpacing
                        spacing: Kirigami.Units.smallSpacing

                        PlasmaComponents3.Label {
                            text: "FreeSync"
                        }

                        Item { Layout.fillWidth: true }

                        Repeater {
                            model: [
                                {"t": "OFF", "v": 0},
                                {"t": "ON", "v": 1}
                            ]

                            Rectangle {
                                required property var modelData
                                property bool active: root.monFreeSync === modelData.v

                                implicitWidth: Kirigami.Units.gridUnit * 4
                                implicitHeight: Kirigami.Units.gridUnit * 1.8
                                radius: Kirigami.Units.cornerRadius
                                color: active
                                    ? Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)
                                    : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                                border.width: active ? 2 : 0
                                border.color: Kirigami.Theme.highlightColor

                                PlasmaComponents3.Label {
                                    anchors.centerIn: parent
                                    text: modelData.t
                                    font: Kirigami.Theme.smallFont
                                    color: active
                                        ? Kirigami.Theme.highlightedTextColor
                                        : Kirigami.Theme.textColor
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: root.setMonitorValue(248, modelData.v)
                                }
                            }
                        }
                    }

                    // Smart Energy buttons
                    PlasmaComponents3.Label {
                        text: "Smart Energy"
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Repeater {
                            model: [
                                {"t": "OFF", "v": 0},
                                {"t": "LOW", "v": 1},
                                {"t": "HIGH", "v": 2}
                            ]

                            Rectangle {
                                required property var modelData
                                property bool active: root.monSmartEnergy === modelData.v

                                Layout.fillWidth: true
                                implicitHeight: Kirigami.Units.gridUnit * 1.8
                                radius: Kirigami.Units.cornerRadius
                                color: active
                                    ? Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)
                                    : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                                border.width: active ? 2 : 0
                                border.color: Kirigami.Theme.highlightColor

                                PlasmaComponents3.Label {
                                    anchors.centerIn: parent
                                    text: modelData.t
                                    font: Kirigami.Theme.smallFont
                                    color: active
                                        ? Kirigami.Theme.highlightedTextColor
                                        : Kirigami.Theme.textColor
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: root.setMonitorValue(246, modelData.v)
                                }
                            }
                        }
                    }

                    // Black Stabilizer slider
                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        Layout.topMargin: Kirigami.Units.smallSpacing
                        spacing: 0

                        RowLayout {
                            Layout.fillWidth: true
                            PlasmaComponents3.Label {
                                text: "Black Stabilizer"
                            }
                            Item { Layout.fillWidth: true }
                            PlasmaComponents3.Label {
                                text: root.monBlackStabilizer + "%"
                                font.bold: true
                            }
                        }

                        PlasmaComponents3.Slider {
                            Layout.fillWidth: true
                            from: 0
                            to: 100
                            value: root.monBlackStabilizer
                            stepSize: 1
                            onMoved: {
                                root.monBlackStabilizer = value;
                                root.setMonitorValue(249, value);
                            }
                        }
                    }

                    // Gamma buttons
                    PlasmaComponents3.Label {
                        text: "Gamma"
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Repeater {
                            model: [
                                {"t": "OFF", "v": 0},
                                {"t": "MODE 0", "v": 1},
                                {"t": "MODE 1", "v": 2},
                                {"t": "MODE 2", "v": 3}
                            ]

                            Rectangle {
                                required property var modelData
                                property bool active: root.monGamma === modelData.v

                                Layout.fillWidth: true
                                implicitHeight: Kirigami.Units.gridUnit * 1.8
                                radius: Kirigami.Units.cornerRadius
                                color: active
                                    ? Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)
                                    : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                                border.width: active ? 2 : 0
                                border.color: Kirigami.Theme.highlightColor

                                PlasmaComponents3.Label {
                                    anchors.centerIn: parent
                                    text: modelData.t
                                    font: Kirigami.Theme.smallFont
                                    color: active
                                        ? Kirigami.Theme.highlightedTextColor
                                        : Kirigami.Theme.textColor
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: root.setMonitorValue(254, modelData.v)
                                }
                            }
                        }
                    }

                    // Power LED toggle
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        Layout.topMargin: Kirigami.Units.smallSpacing
                        spacing: Kirigami.Units.smallSpacing

                        PlasmaComponents3.Label {
                            text: "Power LED"
                        }

                        Item { Layout.fillWidth: true }

                        Repeater {
                            model: [
                                {"t": "OFF", "v": 0},
                                {"t": "ON", "v": 1}
                            ]

                            Rectangle {
                                required property var modelData
                                property bool active: root.monPowerLed === modelData.v

                                implicitWidth: Kirigami.Units.gridUnit * 4
                                implicitHeight: Kirigami.Units.gridUnit * 1.8
                                radius: Kirigami.Units.cornerRadius
                                color: active
                                    ? Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)
                                    : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                                border.width: active ? 2 : 0
                                border.color: Kirigami.Theme.highlightColor

                                PlasmaComponents3.Label {
                                    anchors.centerIn: parent
                                    text: modelData.t
                                    font: Kirigami.Theme.smallFont
                                    color: active
                                        ? Kirigami.Theme.highlightedTextColor
                                        : Kirigami.Theme.textColor
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: root.setMonitorValue(253, modelData.v)
                                }
                            }
                        }
                    }

                    // ── Split Screen section ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Split Screen"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Repeater {
                            model: [
                                {"t": "OFF", "v": 1},
                                {"t": "LR 50/50", "v": 5},
                                {"t": "LR 66/33", "v": 3},
                                {"t": "PBP ON", "v": 0}
                            ]

                            Rectangle {
                                required property var modelData

                                Layout.fillWidth: true
                                implicitHeight: Kirigami.Units.gridUnit * 1.8
                                radius: Kirigami.Units.cornerRadius
                                color: Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                                border.width: 0
                                border.color: Kirigami.Theme.highlightColor

                                PlasmaComponents3.Label {
                                    anchors.centerIn: parent
                                    text: modelData.t
                                    font: Kirigami.Theme.smallFont
                                    color: Kirigami.Theme.textColor
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: root.setMonitorValue(215, modelData.v)
                                }
                            }
                        }
                    }

                    // ── OSD section ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "OSD"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    // OSD Lock toggle
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        PlasmaComponents3.Label {
                            text: "OSD Lock"
                        }

                        Item { Layout.fillWidth: true }

                        Repeater {
                            model: [
                                {"t": "LOCK", "v": 1},
                                {"t": "UNLOCK", "v": 2}
                            ]

                            Rectangle {
                                required property var modelData
                                property bool active: root.monOsdLock === modelData.v

                                implicitWidth: Kirigami.Units.gridUnit * 4
                                implicitHeight: Kirigami.Units.gridUnit * 1.8
                                radius: Kirigami.Units.cornerRadius
                                color: active
                                    ? Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)
                                    : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                                border.width: active ? 2 : 0
                                border.color: Kirigami.Theme.highlightColor

                                PlasmaComponents3.Label {
                                    anchors.centerIn: parent
                                    text: modelData.t
                                    font: Kirigami.Theme.smallFont
                                    color: active
                                        ? Kirigami.Theme.highlightedTextColor
                                        : Kirigami.Theme.textColor
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: {
                                        root.monOsdLock = modelData.v;
                                        root.setMonitorValue(202, modelData.v);
                                    }
                                }
                            }
                        }
                    }

                    // Language selector
                    PlasmaComponents3.Label {
                        text: "Language"
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Repeater {
                            model: [
                                {"t": "EN", "v": 0},
                                {"t": "FR", "v": 2},
                                {"t": "DE", "v": 3}
                            ]

                            Rectangle {
                                required property var modelData
                                property bool active: root.monLanguage === modelData.v

                                Layout.fillWidth: true
                                implicitHeight: Kirigami.Units.gridUnit * 1.8
                                radius: Kirigami.Units.cornerRadius
                                color: active
                                    ? Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g, Kirigami.Theme.highlightColor.b, 0.15)
                                    : Qt.rgba(Kirigami.Theme.textColor.r, Kirigami.Theme.textColor.g, Kirigami.Theme.textColor.b, 0.05)
                                border.width: active ? 2 : 0
                                border.color: Kirigami.Theme.highlightColor

                                PlasmaComponents3.Label {
                                    anchors.centerIn: parent
                                    text: modelData.t
                                    font: Kirigami.Theme.smallFont
                                    color: active
                                        ? Kirigami.Theme.highlightedTextColor
                                        : Kirigami.Theme.textColor
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: {
                                        root.monLanguage = modelData.v;
                                        root.setMonitorValue(204, modelData.v);
                                    }
                                }
                            }
                        }
                    }

                    // ── Reset section ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Reset"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Repeater {
                            model: [
                                {"t": "FACTORY", "vcp": 4, "v": 1},
                                {"t": "BRI/CON", "vcp": 5, "v": 1},
                                {"t": "COLOR", "vcp": 8, "v": 1}
                            ]

                            Rectangle {
                                required property var modelData

                                Layout.fillWidth: true
                                implicitHeight: Kirigami.Units.gridUnit * 1.8
                                radius: Kirigami.Units.cornerRadius
                                color: Qt.rgba(Kirigami.Theme.negativeTextColor.r, Kirigami.Theme.negativeTextColor.g, Kirigami.Theme.negativeTextColor.b, 0.08)
                                border.width: 0
                                border.color: Kirigami.Theme.negativeTextColor

                                PlasmaComponents3.Label {
                                    anchors.centerIn: parent
                                    text: modelData.t
                                    font: Kirigami.Theme.smallFont
                                    color: Kirigami.Theme.negativeTextColor
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: root.setMonitorValue(modelData.vcp, modelData.v)
                                }
                            }
                        }
                    }

                    Item { Layout.fillHeight: true }
                }
            }
        }

        // ════════════════════════════════════════
        //  TAB 2 : SYSTEM
        // ════════════════════════════════════════
        QQC2.ScrollView {
            id: systemTab

            Flickable {
                contentWidth: parent.width
                contentHeight: sysCol.implicitHeight + Kirigami.Units.largeSpacing * 2

                ColumnLayout {
                    id: sysCol
                    width: parent.width
                    spacing: Kirigami.Units.smallSpacing

                    // ── Monitor info ──
                    PlasmaExtras.Heading {
                        text: "Monitor"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.topMargin: Kirigami.Units.largeSpacing
                    }

                    Kirigami.FormLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing

                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Model:"
                            text: "LG 34GN850"
                        }
                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Panel:"
                            text: "IPS 3440x1440 10-bit"
                        }
                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Firmware:"
                            text: root.monFirmware || "3.0"
                        }
                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Refresh:"
                            text: (root.monVFreq / 100).toFixed(1) + " Hz"
                        }
                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Usage:"
                            text: root.monUsageHours + "h (" + (Math.round(root.monUsageHours / 8760 * 10) / 10) + " yrs)"
                        }
                    }

                    // ── GPU info ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "GPU"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    Kirigami.FormLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing

                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Card:"
                            text: "RTX 4070 SUPER"
                        }
                    }

                    // ── Keyboard info ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Keyboard"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    Kirigami.FormLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing

                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Model:"
                            text: root.kbModel || "--"
                        }
                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Firmware:"
                            text: root.fwVersion || "--"
                        }
                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "RSSI:"
                            text: root.rssi + " dBm"
                        }
                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Battery:"
                            text: root.batteryType || "--"
                        }
                        PlasmaComponents3.Label {
                            Kirigami.FormData.label: "Status:"
                            text: root.connected ? "Connected" : "Disconnected"
                            color: root.connected
                                ? Kirigami.Theme.positiveTextColor
                                : Kirigami.Theme.negativeTextColor
                        }
                    }

                    // ── Actions ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaExtras.Heading {
                        text: "Actions"
                        level: 4
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing

                        PlasmaComponents3.Button {
                            Layout.fillWidth: true
                            icon.name: "view-refresh"
                            text: "Refresh"
                            onClicked: {
                                root.fetchData();
                                root.fetchMonitor();
                            }
                        }

                        PlasmaComponents3.Button {
                            Layout.fillWidth: true
                            icon.name: "utilities-terminal"
                            text: "Status"
                            onClicked: actSource.connectSource("konsole -e 'apple-kb-monitor --status; read'")
                        }

                        PlasmaComponents3.Button {
                            Layout.fillWidth: true
                            icon.name: "office-chart-line"
                            text: "Graph"
                            onClicked: actSource.connectSource("konsole -e 'apple-kb-monitor --graph; read'")
                        }
                    }

                    // ── Status line ──
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    PlasmaComponents3.Label {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.largeSpacing
                        text: fullRep.inputName(root.monInput)
                            + " \u00B7 " + fullRep.picModeName(root.monPictureMode)
                            + (root.monMute === 1 ? " \u00B7 MUTED" : "")
                            + " \u00B7 " + root.monUsageHours + "h"
                        color: Kirigami.Theme.disabledTextColor
                        font: Kirigami.Theme.smallFont
                        elide: Text.ElideRight
                    }

                    Item { Layout.fillHeight: true }
                }
            }
        }
    }

    // ── Action data source (konsole launches) ──
    P5.DataSource {
        id: actSource
        engine: "executable"
        onNewData: function(source, data) { disconnectSource(source); }
    }
}
