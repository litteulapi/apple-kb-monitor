import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip

// DIAG: state and version of the monitor, a new reading on demand, and the
// checks of the window's Diag tab run by the daemon (Diagnose, #120): count
// of passed checks, one gauge block per check, then the terminal log
// "[ OK ]" / "[FAIL]" whose failed details carry the fix.
ColumnLayout {
    id: page
    property var applet
    spacing: Pip.GAP * 1.5

    readonly property int failed: applet.diagTotal - applet.diagPassed

    PipPanel {
        title: i18n("Monitor")
        PipKv {
            label: i18n("State")
            value: applet.daemonRunning ? i18n("RUNNING") : i18n("STOPPED")
            valueColor: applet.daemonRunning ? Pip.PHOSPHOR : Pip.AMBER
        }
        PipKv {
            label: i18n("Version")
            value: applet.daemonVersion
        }
        PipKv {
            label: i18n("Keyboard")
            value: applet.daemonRunning ? applet.stateText : ""
            valueColor: applet.connected ? Pip.PHOSPHOR : Pip.RED
        }
        PipKv {
            label: i18n("Link")
            value: applet.linkText
        }
        PipKv {
            label: i18n("Last reading")
            value: applet.lastUpdate > 0 ? applet.updatedText : ""
        }
        PipKv {
            visible: applet.lastError !== ""
            label: i18n("Last error")
            value: applet.lastError
            valueColor: Pip.RED
        }
        PipText {
            Layout.fillWidth: true
            visible: !applet.daemonRunning
            text: i18n("Start it with: systemctl --user start apple-kb-monitord")
            color: Pip.AMBER
        }
        PipButton {
            text: i18n("Refresh")
            enabled: applet.daemonRunning
            onClicked: {
                applet.fetchDetails();
                applet.requestRefresh();
            }
        }
        PipText {
            Layout.fillWidth: true
            visible: applet.diagHint !== ""
            text: applet.diagHint
            font.pixelSize: Pip.SMALL
        }
    }

    PipPanel {
        title: i18n("System diagnostic")
        PipButton {
            Layout.fillWidth: true
            text: applet.diagRunning ? i18n("Checking…") : i18n("Run the full check")
            tint: applet.diagRunning ? Pip.AMBER : Pip.PHOSPHOR
            enabled: applet.daemonRunning && !applet.diagRunning
            onClicked: applet.runDiagnose()
        }
        Flow {
            Layout.fillWidth: true
            visible: applet.diagTotal > 0
            spacing: Pip.GAP * 2
            PipText {
                text: i18n("%1/%2 PASSED", applet.diagPassed, applet.diagTotal)
                font.pixelSize: Pip.VALUE
                color: page.failed > 0 ? Pip.AMBER : Pip.PHOSPHOR
                glow: true
                wrapMode: Text.NoWrap
            }
            PipText {
                visible: page.failed > 0
                text: i18n("%1 PROBLEM(S)", page.failed)
                font.pixelSize: Pip.VALUE
                color: Pip.RED
                glow: true
                wrapMode: Text.NoWrap
            }
        }
        PipSegments {
            Layout.fillWidth: true
            visible: applet.diagChecks.length > 0
            colors: applet.diagChecks.map(function (c) { return c.ok ? Pip.PHOSPHOR : Pip.RED; })
        }
        PipText {
            Layout.fillWidth: true
            visible: applet.diagError !== ""
            text: i18n("Check not run: %1", applet.diagError)
            color: Pip.RED
        }
    }

    PipPanel {
        title: i18n("Log")
        visible: applet.diagChecks.length > 0
        Repeater {
            model: applet.diagChecks
            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: Pip.GAP
                PipText {
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: 64
                    text: modelData.ok ? "[ OK ]" : i18n("[FAIL]")
                    color: modelData.ok ? Pip.PHOSPHOR : Pip.RED
                    wrapMode: Text.NoWrap
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    PipText {
                        Layout.fillWidth: true
                        text: String(modelData.label || modelData.id).toUpperCase()
                        color: modelData.ok ? Pip.PHOSPHOR : Pip.RED
                    }
                    PipText {
                        Layout.fillWidth: true
                        text: Pip.glyphs(String(modelData.detail || ""))
                        visible: text !== ""
                        font.pixelSize: Pip.SMALL
                        // A failed check's detail carries the fix: readable.
                        color: modelData.ok ? Pip.GREEN_MID : Pip.PHOSPHOR
                    }
                }
            }
        }
    }
    PipText {
        Layout.fillWidth: true
        visible: applet.diagChecks.length === 0 && !applet.diagRunning && applet.diagError === ""
        text: i18n("The check is run by the monitor (about ten checks, never the keyboard).")
        color: Pip.GREEN_MID
        font.pixelSize: Pip.SMALL
    }
}
