pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip
import "FnMode.js" as Fn

ColumnLayout {
    id: page
    required property var applet
    property bool compact: false
    spacing: Pip.GAP * 1.5

    readonly property string kind: Fn.kind(applet.fnMode)

    PipPanel {
        title: i18n("Function keys")
        PipChoice {
            Layout.fillWidth: true
            text: i18n("Media keys first")
            chosen: page.kind === "media"
            enabled: !page.applet.fnBusy && page.applet.kbMac !== "" && Fn.next(page.applet.fnMode) > 0
            onClicked: if (!chosen) page.applet.setFnModeTo(1)
        }
        PipChoice {
            Layout.fillWidth: true
            text: i18n("F1–F12 first")
            chosen: page.kind === "fkeys"
            enabled: !page.applet.fnBusy && page.applet.kbMac !== "" && Fn.next(page.applet.fnMode) > 0
            onClicked: if (!chosen) page.applet.setFnModeTo(2)
        }
        PipText {
            Layout.fillWidth: true
            visible: page.kind === "off" || page.kind === "nofkeys"
            text: i18n("Current mode: %1", page.applet.fnModeText)
            color: Pip.AMBER
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.fnMode < 0
            text: page.applet.connected ? i18n("Fn mode not read yet.") : i18n("Fn mode unknown: the keyboard is not connected.")
            color: Pip.GREEN_MID
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.fnBusy
            text: i18n("Administrator authentication requested…")
            color: Pip.AMBER
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.fnError !== ""
            text: i18n("Fn mode not changed: %1", page.applet.fnError)
            color: Pip.RED
        }
        PipText {
            Layout.fillWidth: true
            text: i18n("hid_apple applies to every Apple keyboard of this computer.")
            font.pixelSize: Pip.SMALL
            color: Pip.GREEN_MID
        }
    }

    PipPanel {
        title: i18n("Locks")
        RowLayout {
            Layout.fillWidth: true
            spacing: Pip.GAP * 2
            Repeater {
                model: [[i18n("Caps Lock"), page.applet.capsLock], [i18n("Num Lock"), page.applet.numLock]]
                delegate: Rectangle {
                    id: cap
                    required property var modelData
                    readonly property bool on: page.applet.connected && modelData[1]
                    Layout.fillWidth: true
                    implicitHeight: 74
                    color: Pip.BG
                    border.color: cap.on ? Pip.PHOSPHOR : Pip.GREEN_MID
                    border.width: 2
                    radius: 3
                    Accessible.role: Accessible.Indicator
                    Accessible.name: modelData[0] + " " + (cap.on ? i18n("on") : i18n("off"))
                    Rectangle {
                        anchors.fill: parent
                        anchors.margins: 5
                        color: "transparent"
                        border.color: Pip.GREEN_FRAME
                        border.width: 1
                        radius: 2
                    }
                    Rectangle {
                        x: 14
                        y: 14
                        width: 12
                        height: 12
                        radius: 6
                        color: cap.on ? Pip.PHOSPHOR : "transparent"
                        border.color: cap.on ? Pip.PHOSPHOR : Pip.GREEN_FRAME
                        border.width: 1
                        Rectangle {
                            anchors.centerIn: parent
                            visible: cap.on
                            width: 22
                            height: 22
                            radius: 11
                            color: Qt.rgba(0.08, 1, 0, 0.22)
                        }
                    }
                    PipText {
                        x: 34
                        y: 8
                        width: parent.width - 42
                        text: cap.modelData[0].toLocaleUpperCase()
                        color: cap.on ? Pip.PHOSPHOR : Pip.GREEN_MID
                        glow: cap.on
                        Accessible.ignored: true
                    }
                    PipText {
                        x: 14
                        anchors.bottom: parent.bottom
                        anchors.bottomMargin: 8
                        text: !page.applet.connected ? "---" : (cap.on ? i18n("ON") : i18n("OFF"))
                        font.pixelSize: Pip.TITLE
                        color: cap.on ? Pip.PHOSPHOR : Pip.GREEN_MID
                        glow: cap.on
                        wrapMode: Text.NoWrap
                        Accessible.ignored: true
                    }
                }
            }
        }
        PipText {
            Layout.fillWidth: true
            text: i18n("Read from the keyboard's LEDs at each reading.")
            font.pixelSize: Pip.SMALL
            color: Pip.GREEN_MID
        }
    }

    PipPanel {
        title: i18n("Special keys")
        PipText {
            Layout.fillWidth: true
            text: i18n("The table of the special keys (code and KDE action with and without Fn) and the key mapping editor are in System Settings, Apple Keyboard module, Keys page.")
            color: Pip.GREEN_MID
        }
        PipButton {
            text: i18n("Edit the keys")
            onClicked: page.applet.openSettings("keys")
        }
    }
}
