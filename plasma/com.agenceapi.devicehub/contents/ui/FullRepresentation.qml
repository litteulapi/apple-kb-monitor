import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents3
import org.kde.plasma.extras as PlasmaExtras
import org.kde.plasma.plasma5support as P5

ColumnLayout {
    id: fullRep

    Layout.preferredWidth: Kirigami.Units.gridUnit * 20
    Layout.preferredHeight: implicitHeight
    Layout.minimumWidth: Kirigami.Units.gridUnit * 16

    spacing: Kirigami.Units.smallSpacing

    // ── Helper maps ──
    function picModeName(v) {
        var map = {
            1: "Reader", 6: "Gamer 2", 17: "Custom", 19: "RTS",
            20: "Vivid", 21: "sRGB", 25: "EBU", 32: "Photo",
            40: "FPS 1", 41: "FPS 2", 45: "Custom 2", 72: "Cinema"
        }
        return map[v] || "Mode " + v
    }

    function inputName(v) {
        var map = {15: "DP-1", 16: "DP-2", 17: "HDMI-1", 18: "HDMI-2"}
        return map[v] || "??"
    }

    // ── Header ──
    PlasmaExtras.Heading {
        text: "ApiHub"
        level: 3
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.topMargin: Kirigami.Units.smallSpacing
    }

    // ── Battery section ──
    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.smallSpacing

        PlasmaComponents3.Label {
            text: root.batteryPercent + "%"
            font.pixelSize: Kirigami.Units.gridUnit * 2
            font.bold: true
            color: root.batteryPercent <= 15
                ? Kirigami.Theme.negativeTextColor
                : root.batteryPercent <= 50
                    ? Kirigami.Theme.neutralTextColor
                    : Kirigami.Theme.textColor
        }

        ColumnLayout {
            spacing: 0
            PlasmaComponents3.Label {
                text: root.voltage.toFixed(3) + " V"
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
            }
            PlasmaComponents3.Label {
                text: root.rssi + " dBm"
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
            }
        }

        Item { Layout.fillWidth: true }

        // RSSI signal dot
        Rectangle {
            width: Kirigami.Units.gridUnit * 0.6
            height: width
            radius: width / 2
            color: {
                if (!root.connected) return Kirigami.Theme.disabledTextColor
                if (root.rssi > -50) return Kirigami.Theme.positiveTextColor
                if (root.rssi > -70) return Kirigami.Theme.neutralTextColor
                return Kirigami.Theme.negativeTextColor
            }
        }

        PlasmaComponents3.Label {
            text: root.connected ? "Connected" : "Offline"
            font: Kirigami.Theme.smallFont
            color: root.connected
                ? Kirigami.Theme.positiveTextColor
                : Kirigami.Theme.negativeTextColor
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

    Kirigami.Separator { Layout.fillWidth: true }

    // ── Monitor quick controls ──
    PlasmaExtras.Heading {
        text: "Monitor"
        level: 4
        Layout.leftMargin: Kirigami.Units.largeSpacing
    }

    // Brightness slider
    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.smallSpacing

        Kirigami.Icon {
            source: "brightness-high"
            implicitWidth: Kirigami.Units.iconSizes.smallMedium
            implicitHeight: Kirigami.Units.iconSizes.smallMedium
        }

        PlasmaComponents3.Slider {
            Layout.fillWidth: true
            from: 0
            to: 100
            value: root.monBrightness
            stepSize: 1
            onMoved: {
                root.monBrightness = value
                root.setMonitorValue(16, value)
            }
        }

        PlasmaComponents3.Label {
            text: root.monBrightness + "%"
            Layout.minimumWidth: Kirigami.Units.gridUnit * 2.5
            horizontalAlignment: Text.AlignRight
            font.bold: true
        }
    }

    // Volume slider
    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.smallSpacing

        Kirigami.Icon {
            source: root.monMute === 1 ? "audio-volume-muted" : "audio-volume-high"
            implicitWidth: Kirigami.Units.iconSizes.smallMedium
            implicitHeight: Kirigami.Units.iconSizes.smallMedium

            MouseArea {
                anchors.fill: parent
                onClicked: {
                    var newVal = root.monMute === 1 ? 2 : 1
                    root.monMute = newVal
                    root.setMonitorValue(141, newVal)
                }
            }
        }

        PlasmaComponents3.Slider {
            Layout.fillWidth: true
            from: 0
            to: 100
            value: root.monVolume
            stepSize: 1
            onMoved: {
                root.monVolume = value
                root.setMonitorValue(98, value)
            }
        }

        PlasmaComponents3.Label {
            text: root.monVolume + "%"
            Layout.minimumWidth: Kirigami.Units.gridUnit * 2.5
            horizontalAlignment: Text.AlignRight
            font.bold: true
            color: root.monMute === 1
                ? Kirigami.Theme.negativeTextColor
                : Kirigami.Theme.textColor
        }
    }

    // Status line
    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing

        PlasmaComponents3.Label {
            text: fullRep.picModeName(root.monPictureMode)
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }

        PlasmaComponents3.Label {
            text: "FreeSync " + (root.monFreeSync === 1 ? "ON" : "OFF")
            font: Kirigami.Theme.smallFont
            color: root.monFreeSync === 1
                ? Kirigami.Theme.positiveTextColor
                : Kirigami.Theme.disabledTextColor
        }

        PlasmaComponents3.Label {
            text: fullRep.inputName(root.monInput)
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }
    }

    Kirigami.Separator { Layout.fillWidth: true }

    // ── Open settings button ──
    PlasmaComponents3.Button {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        Layout.bottomMargin: Kirigami.Units.smallSpacing
        icon.name: "configure"
        text: "ApiHub Settings\u2026"
        onClicked: root.openSettings()
    }
}
