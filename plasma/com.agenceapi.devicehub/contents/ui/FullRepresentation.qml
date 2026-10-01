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
