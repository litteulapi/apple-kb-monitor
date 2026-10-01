import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents3
import org.kde.plasma.extras as PlasmaExtras

PlasmaExtras.Representation {
    id: full

    Layout.preferredWidth: Kirigami.Units.gridUnit * 20
    Layout.minimumWidth: Kirigami.Units.gridUnit * 16
    Layout.preferredHeight: content.implicitHeight + header.height + Kirigami.Units.largeSpacing * 2

    readonly property color levelColor: root.batteryPercent < 0
        ? Kirigami.Theme.disabledTextColor
        : root.batteryPercent <= 15
            ? Kirigami.Theme.negativeTextColor
            : root.batteryPercent <= 50
                ? Kirigami.Theme.neutralTextColor
                : Kirigami.Theme.positiveTextColor

    header: PlasmaExtras.PlasmoidHeading {
        RowLayout {
            anchors.fill: parent
            Kirigami.Icon {
                source: "apihub-scarab"
                implicitWidth: Kirigami.Units.iconSizes.smallMedium
                implicitHeight: implicitWidth
            }
            Kirigami.Heading {
                level: 3
                text: root.connected ? (root.kbName !== "" ? root.kbName : root.kbModel) : i18n("ApiHub")
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
            PlasmaComponents3.Label {
                text: root.stateText
                font: Kirigami.Theme.smallFont
                color: root.connected ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.disabledTextColor
            }
        }
    }

    contentItem: ColumnLayout {
        id: content
        spacing: Kirigami.Units.smallSpacing

        // Daemon absent: nothing else to show.
        PlasmaExtras.PlaceholderMessage {
            visible: !root.daemonRunning
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing
            iconName: "dialog-warning"
            text: i18n("Monitor stopped")
            explanation: i18n("apple-kb-monitord is not on the session bus.")
        }

        PlasmaExtras.PlaceholderMessage {
            visible: root.daemonRunning && !root.connected
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing
            iconName: "input-keyboard"
            text: i18n("No Apple keyboard connected")
            explanation: root.lastError
        }

        ColumnLayout {
            visible: root.connected
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.smallSpacing

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.largeSpacing

                PlasmaComponents3.Label {
                    id: bigPercent
                    text: root.batteryText
                    font.pixelSize: Kirigami.Units.gridUnit * 2
                    font.bold: true
                    color: full.levelColor
                    Accessible.name: root.accessibleSummary
                }
                PlasmaComponents3.ProgressBar {
                    Layout.fillWidth: true
                    from: 0
                    to: 100
                    value: Math.max(0, root.batteryPercent)
                    Accessible.name: i18n("Battery level")
                }
            }

            PlasmaComponents3.Label {
                Layout.fillWidth: true
                text: i18n("Keyboard indication (its own percentage, not a linear charge)")
                opacity: 0.7
                wrapMode: Text.Wrap
            }

            Kirigami.FormLayout {
                Layout.fillWidth: true
                twinFormLayouts: []

                PlasmaComponents3.Label {
                    Kirigami.FormData.label: i18n("Estimate:")
                    visible: root.hasEstimate || root.newBatteries
                    text: root.estimateText
                    wrapMode: Text.Wrap
                }
                PlasmaComponents3.Label {
                    Kirigami.FormData.label: i18n("Last reading:")
                    text: root.updatedText
                    Accessible.description: i18n("The keyboard's percentage only goes down when it reconnects")
                }

                PlasmaComponents3.Label {
                    Kirigami.FormData.label: i18n("Voltage:")
                    text: root.voltage > 0 ? i18n("%1 V", root.voltage.toFixed(2)) : "—"
                }
                PlasmaComponents3.Label {
                    Kirigami.FormData.label: i18n("Curve estimate:")
                    visible: root.curvePercent >= 0
                    text: i18n("%1% (estimate)", root.curvePercent.toFixed(1))
                }
                PlasmaComponents3.Label {
                    Kirigami.FormData.label: i18n("Battery type (estimate):")
                    visible: root.batteryType !== ""
                    text: root.batteryType
                }
                PlasmaComponents3.Label {
                    Kirigami.FormData.label: i18n("Signal:")
                    text: root.rssiText
                }
                PlasmaComponents3.Label {
                    Kirigami.FormData.label: i18n("Estimated autonomy:")
                    visible: root.remaining !== ""
                    text: root.remaining
                }
                RowLayout {
                    Kirigami.FormData.label: i18n("Name:")
                    Layout.fillWidth: true
                    PlasmaComponents3.TextField {
                        id: nameField
                        Layout.fillWidth: true
                        maximumLength: 64
                        placeholderText: i18n("Keyboard name")
                        Accessible.name: i18n("Keyboard name")
                        // Follow the daemon unless the user is typing.
                        Binding on text {
                            value: root.kbName
                            when: !nameField.activeFocus
                            restoreMode: Binding.RestoreNone
                        }
                        onAccepted: root.renameKeyboard(text.trim())
                    }
                    PlasmaComponents3.Button {
                        icon.name: "edit-rename"
                        text: i18n("Rename")
                        enabled: nameField.text.trim() !== "" && nameField.text.trim() !== root.kbName
                        onClicked: root.renameKeyboard(nameField.text.trim())
                    }
                    PlasmaComponents3.Button {
                        icon.name: "edit-undo"
                        text: i18n("Reset")
                        Accessible.description: i18n("Restore the keyboard's own name")
                        onClicked: root.renameKeyboard("")
                    }
                }
                PlasmaComponents3.Label {
                    Kirigami.FormData.label: ""
                    visible: root.renameError !== ""
                    text: root.renameError
                    color: Kirigami.Theme.negativeTextColor
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
                PlasmaComponents3.Label {
                    Kirigami.FormData.label: i18n("Model:")
                    text: root.kbModel
                    wrapMode: Text.Wrap
                }
                PlasmaComponents3.Label {
                    Kirigami.FormData.label: i18n("Firmware:")
                    visible: root.fwVersion !== ""
                    text: root.fwVersion
                }
            }
        }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            visible: root.windowHint !== ""
            type: Kirigami.MessageType.Warning
            text: root.windowHint
        }

        PlasmaComponents3.Button {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing
            icon.name: "window-new"
            text: i18n("Open ApiHub window")
            onClicked: root.openWindow()
            Accessible.name: text
        }
    }
}
