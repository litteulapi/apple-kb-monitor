import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents3

// Diagnostic page of the popup (#120): what the daemon already publishes
// (GetState, DaemonVersion, Link.Status), and the two actions it already
// offers (Refresh, Link.Reconnect). Nothing here runs a command or reads the
// keyboard. `applet` is the PlasmoidItem (main.qml).
ColumnLayout {
    id: page

    required property var applet
    spacing: Kirigami.Units.smallSpacing

    Kirigami.FormLayout {
        Layout.fillWidth: true
        twinFormLayouts: []

        PlasmaComponents3.Label {
            Kirigami.FormData.label: i18n("Monitor:")
            text: page.applet.daemonVersion !== ""
                ? i18n("running, version %1", page.applet.daemonVersion)
                : i18n("running")
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
        }
        PlasmaComponents3.Label {
            Kirigami.FormData.label: i18n("Keyboard:")
            text: page.applet.stateText
            textFormat: Text.PlainText
            color: page.applet.connected ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.neutralTextColor
            wrapMode: Text.Wrap
        }
        PlasmaComponents3.Label {
            Kirigami.FormData.label: i18n("Last error:")
            visible: page.applet.lastError !== ""
            text: page.applet.lastError
            textFormat: Text.PlainText
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        PlasmaComponents3.Label {
            Kirigami.FormData.label: i18n("Last reading:")
            text: page.applet.updatedText
            textFormat: Text.PlainText
        }
        PlasmaComponents3.Label {
            Kirigami.FormData.label: i18n("Reading:")
            visible: page.applet.readIncomplete
            text: i18n("incomplete (a report did not answer in time)")
            textFormat: Text.PlainText
            color: Kirigami.Theme.neutralTextColor
            wrapMode: Text.Wrap
        }
        PlasmaComponents3.Label {
            Kirigami.FormData.label: i18n("Signal:")
            text: page.applet.rssiText
            textFormat: Text.PlainText
        }
        PlasmaComponents3.Label {
            Kirigami.FormData.label: i18n("Link:")
            visible: page.applet.linkText !== ""
            text: page.applet.linkText
            textFormat: Text.PlainText
            color: page.applet.linkHealth === "connected" ? Kirigami.Theme.textColor : Kirigami.Theme.neutralTextColor
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        PlasmaComponents3.Label {
            Kirigami.FormData.label: i18n("Last link error:")
            visible: page.applet.linkError !== ""
            text: page.applet.linkError
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        PlasmaComponents3.Label {
            Kirigami.FormData.label: i18n("Firmware:")
            visible: page.applet.fwText !== ""
            text: page.applet.fwText
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        PlasmaComponents3.Label {
            Kirigami.FormData.label: i18n("Function keys:")
            visible: page.applet.fnModeText !== ""
            text: page.applet.fnModeText
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
        }
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: Kirigami.Units.smallSpacing

        PlasmaComponents3.Button {
            Layout.fillWidth: true
            icon.name: "view-refresh"
            text: i18n("Read again")
            Accessible.description: i18n("Asks the monitor for a new reading of the keyboard")
            onClicked: page.applet.requestRefresh()
        }
        PlasmaComponents3.Button {
            Layout.fillWidth: true
            icon.name: "network-connect"
            text: i18n("Reconnect")
            Accessible.description: i18n("Asks the monitor to call the keyboard now")
            onClicked: page.applet.requestReconnect()
        }
    }

    PlasmaComponents3.Label {
        Layout.fillWidth: true
        visible: page.applet.diagHint !== ""
        text: page.applet.diagHint
        textFormat: Text.PlainText
        wrapMode: Text.Wrap
    }

    PlasmaComponents3.Label {
        Layout.fillWidth: true
        text: i18n("Full check of the installed components: System Settings, Apple Keyboard, Diagnostics.")
        textFormat: Text.PlainText
        font: Kirigami.Theme.smallFont
        opacity: 0.7
        wrapMode: Text.Wrap
    }
}
