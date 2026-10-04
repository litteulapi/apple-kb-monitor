pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip

ColumnLayout {
    id: page
    required property var applet
    property bool compact: false
    spacing: Pip.GAP * 1.5

    readonly property int failed: applet.diagTotal - applet.diagPassed

    PipPanel {
        title: i18n("System diagnostic")
        PipButton {
            Layout.fillWidth: true
            text: page.applet.diagRunning ? i18n("Checking…") : i18n("Run the full check")
            tint: page.applet.diagRunning ? Pip.AMBER : Pip.PHOSPHOR
            enabled: page.applet.daemonRunning && !page.applet.diagRunning
            onClicked: page.applet.runDiagnose()
        }
        Flow {
            Layout.fillWidth: true
            visible: page.applet.diagTotal > 0
            spacing: Pip.GAP * 2
            PipText {
                text: i18n("%1/%2 PASSED", page.applet.diagPassed, page.applet.diagTotal)
                font.pixelSize: Pip.VALUE
                color: page.failed > 0 ? Pip.AMBER : Pip.PHOSPHOR
                glow: true
                wrapMode: Text.NoWrap
            }
            PipText {
                visible: page.failed > 0
                text: i18n("PROBLEMS: %1", page.failed)
                font.pixelSize: Pip.VALUE
                color: Pip.RED
                glow: true
                wrapMode: Text.NoWrap
            }
        }
        PipSegments {
            Layout.fillWidth: true
            visible: page.applet.diagChecks.length > 0
            colors: page.applet.diagChecks.map(function (c) { return c.ok ? Pip.PHOSPHOR : Pip.RED; })
            // one segment per check, not a percentage: the "N/M PASSED" line above says it
            Accessible.ignored: true
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.diagError !== ""
            text: i18n("Check not run: %1", page.applet.diagError)
            color: Pip.RED
        }
    }

    PipPanel {
        title: i18n("Monitor")
        PipKv {
            label: i18n("State")
            value: page.applet.daemonRunning ? i18n("RUNNING") : i18n("STOPPED")
            valueColor: page.applet.daemonRunning ? Pip.PHOSPHOR : Pip.AMBER
        }
        PipKv {
            label: i18n("Version")
            value: page.applet.daemonVersion
        }
        PipKv {
            label: i18n("Keyboard")
            value: page.applet.daemonRunning ? page.applet.stateText : ""
            valueColor: page.applet.connected ? Pip.PHOSPHOR : Pip.RED
        }
        PipKv {
            label: i18n("Link")
            value: page.applet.linkText
        }
        PipKv {
            label: i18n("Last reading")
            value: page.applet.lastUpdate > 0 ? page.applet.updatedText : ""
        }
        PipKv {
            visible: page.applet.lastError !== ""
            label: i18n("Last error")
            value: page.applet.lastError
            valueColor: Pip.RED
        }
        PipText {
            Layout.fillWidth: true
            visible: !page.applet.daemonRunning
            text: i18n("Start it with: systemctl --user start apple-kb-monitord")
            color: Pip.AMBER
        }
        PipButton {
            text: i18n("Refresh")
            enabled: page.applet.daemonRunning
            onClicked: {
                page.applet.fetchDetails();
                page.applet.requestRefresh();
            }
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.diagHint !== ""
            text: page.applet.diagHint
            font.pixelSize: Pip.SMALL
        }
    }

    PipPanel {
        title: i18n("Log")
        visible: page.applet.diagChecks.length > 0
        Repeater {
            model: page.applet.diagChecks
            delegate: RowLayout {
                id: check
                required property var modelData
                Layout.fillWidth: true
                spacing: Pip.GAP
                PipText {
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: 64
                    text: check.modelData.ok ? "[ OK ]" : i18n("[FAIL]")
                    color: check.modelData.ok ? Pip.PHOSPHOR : Pip.RED
                    wrapMode: Text.NoWrap
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    PipText {
                        Layout.fillWidth: true
                        text: String(check.modelData.label || check.modelData.id).toLocaleUpperCase()
                        color: check.modelData.ok ? Pip.PHOSPHOR : Pip.RED
                    }
                    PipText {
                        Layout.fillWidth: true
                        text: Pip.glyphs(String(check.modelData.detail || ""))
                        visible: text !== ""
                        font.pixelSize: Pip.SMALL
                        color: check.modelData.ok ? Pip.GREEN_MID : Pip.PHOSPHOR
                    }
                }
            }
        }
    }
    PipText {
        Layout.fillWidth: true
        visible: page.applet.diagChecks.length === 0 && !page.applet.diagRunning && page.applet.diagError === ""
        text: i18n("The check is run by the monitor (about ten checks, never the keyboard).")
        color: Pip.GREEN_MID
        font.pixelSize: Pip.SMALL
    }
}
