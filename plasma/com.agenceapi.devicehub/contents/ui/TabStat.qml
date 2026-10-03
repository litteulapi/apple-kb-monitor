import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip
import "FnMode.js" as Fn

// STAT: the batteries at a glance (giant percentage, block gauge, origin of
// the figure), the stat strip (state by the keyboard's thresholds, voltage,
// autonomy, signal), the keyboard (name, model, firmware, rename) and the
// actions (reconnect, Fn mode).
ColumnLayout {
    id: page
    property var applet
    // Popup at the tray's own size: smaller figures.
    property bool compact: false
    spacing: Pip.GAP * 1.5

    readonly property bool hasPct: applet.connected && applet.batteryPercent >= 0
    readonly property color level: hasPct ? Pip.levelColor(applet.batteryPercent) : Pip.GREEN_MID
    // Days left from the daemon's forecast, re-evaluated at each reading
    // (lastUpdate), never by a timer.
    readonly property real days: {
        var reading = applet.lastUpdate;
        return Pip.daysLeft({ empty_at: applet.forecastEmptyAt }, Date.now() / 1000);
    }
    property bool renaming: false
    readonly property string stateWord: !applet.connected ? "" : applet.thresholdLevel === "ok" ? i18n("OK")
        : applet.thresholdLevel === "low" ? i18n("LOW")
        : applet.thresholdLevel === "critical" ? i18n("CRITICAL")
        : applet.thresholdLevel === "empty" ? i18n("EMPTY") : ""
    // [label, value ("" = unknown), colour] of the stat strip.
    readonly property var facts: [
        [i18n("State"), page.stateWord, Pip.thresholdColor(applet.thresholdLevel)],
        [i18n("Voltage"), applet.connected && applet.voltage > 0 ? i18n("%1 V", page.num(applet.voltage, 2)) : "",
         Pip.thresholdColor(applet.thresholdLevel)],
        [i18n("Autonomy"), page.days < 0 ? "" : page.days >= 1.5 ? i18n("≈ %1 days", Math.round(page.days))
            : i18n("≈ %1 h", Math.max(0, Math.round(page.days * 24))),
         page.days >= 0 && page.days < 7 ? Pip.AMBER : Pip.PHOSPHOR],
        [i18n("Signal"), applet.hasRssi ? applet.rssiText : (applet.connected ? i18n("unavailable") : ""),
         applet.hasRssi ? Pip.qualityColor(applet.rssiQuality) : Pip.AMBER]]

    function num(v, digits) {
        return Number(v).toLocaleString(Qt.locale(), "f", digits);
    }

    PipPanel {
        title: i18n("Batteries")
        RowLayout {
            Layout.fillWidth: true
            spacing: Pip.GAP * 2
            // Keyboard away: an OFFLINE plaque instead of a figure.
            Rectangle {
                Layout.alignment: Qt.AlignVCenter
                visible: !applet.connected
                implicitWidth: plaque.implicitWidth + 24
                implicitHeight: plaque.implicitHeight + 12
                color: Qt.rgba(1, 0.35, 0.24, 0.10)
                border.color: Pip.RED
                border.width: 2
                PipText {
                    id: plaque
                    anchors.centerIn: parent
                    text: applet.daemonRunning ? i18n("OFFLINE") : i18n("NO DAEMON")
                    font.pixelSize: Pip.VALUE
                    color: applet.daemonRunning ? Pip.RED : Pip.AMBER
                    glow: true
                    wrapMode: Text.NoWrap
                }
            }
            Row {
                Layout.alignment: Qt.AlignTop
                visible: applet.connected
                PipText {
                    id: hero
                    text: page.hasPct ? String(applet.batteryPercent) : "---"
                    font.pixelSize: page.compact ? Pip.HERO * 0.78 : Pip.HERO
                    color: page.level
                    glow: page.hasPct
                    wrapMode: Text.NoWrap
                    lineHeight: 0.8
                    Accessible.name: applet.accessibleSummary
                }
                PipText {
                    visible: page.hasPct
                    anchors.baseline: hero.baseline
                    text: "%"
                    font.pixelSize: Pip.VALUE
                    color: page.level
                    glow: true
                    wrapMode: Text.NoWrap
                }
            }
            // Stat strip beside the figure: state by the keyboard's
            // thresholds, voltage, autonomy, signal.
            GridLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignVCenter
                columns: 2
                columnSpacing: Pip.GAP
                rowSpacing: 0
                Repeater {
                    model: page.facts
                    delegate: PipText {
                        required property var modelData
                        required property int index
                        Layout.row: index
                        Layout.column: 0
                        text: modelData[0].toUpperCase()
                        font.pixelSize: Pip.SMALL
                        color: Pip.GREEN_MID
                        wrapMode: Text.NoWrap
                    }
                }
                Repeater {
                    model: page.facts
                    delegate: RowLayout {
                        required property var modelData
                        required property int index
                        Layout.row: index
                        Layout.column: 1
                        Layout.fillWidth: true
                        spacing: Pip.GAP
                        PipText {
                            Layout.fillWidth: true
                            text: modelData[1] !== "" ? Pip.glyphs(modelData[1]) : "---"
                            font.pixelSize: Pip.TITLE
                            color: modelData[1] !== "" ? modelData[2] : Pip.GREEN_MID
                            glow: modelData[1] !== ""
                            Accessible.name: modelData[0] + " " + text
                        }
                        PipBars {
                            visible: index === 3 && applet.hasRssi
                            lit: Pip.qualityBars(applet.rssiQuality)
                            color: Pip.qualityColor(applet.rssiQuality)
                            barHeight: 16
                        }
                    }
                }
            }
        }
        PipSegments {
            Layout.fillWidth: true
            fraction: page.hasPct ? applet.batteryPercent / 100 : 0
            color: page.level
        }
        // Origin of the figure, then what the stat strip cannot say in one word.
        PipText {
            Layout.fillWidth: true
            text: i18n("Keyboard indication: its own percentage, it only goes down when the keyboard reconnects.")
            font.pixelSize: Pip.SMALL
            color: Pip.GREEN_MID
        }
        PipKv {
            visible: applet.connected && !applet.hasRssi
            label: i18n("Signal")
            value: i18n("see RADIO")
            valueColor: Pip.AMBER
        }
        PipKv {
            label: i18n("Discharge")
            value: page.days < 0 ? i18n("not enough history yet") : i18n("−%1 % per day", page.num(applet.forecastRate, 1))
            valueColor: page.days < 0 ? Pip.GREEN_MID : Pip.PHOSPHOR
        }
        // Last 7 days of the keyboard's percentage (#97), read when the popup opens.
        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: false
            visible: applet.sparkPct.length >= 2
            spacing: Pip.GAP
            PipText {
                text: i18n("7 d")
                font.pixelSize: Pip.SMALL
                color: Pip.GREEN_MID
                wrapMode: Text.NoWrap
            }
            LineChart {
                Layout.fillWidth: true
                implicitHeight: 34
                crt: true
                primary: applet.sparkPct
                tMin: applet.sparkFrom
                tMax: applet.sparkTo
                primaryColor: page.level
                gridColor: Pip.GREEN_FRAME
                gridLines: 2
                lineWidth: 1.5
            }
        }
        PipKv {
            label: i18n("Estimate")
            value: applet.connected && (applet.hasEstimate || applet.newBatteries) ? applet.estimateText : ""
        }
        PipKv {
            label: i18n("Chemistry")
            value: applet.connected ? applet.batteryType : ""
        }
        PipKv {
            label: i18n("Apple display")
            value: applet.connected && applet.applePct >= 0 ? i18n("%1%", Math.round(applet.applePct)) : ""
        }
    }

    // Stat strip: two cells per row.
    PipPanel {
        title: i18n("Keyboard")
        PipKv {
            label: i18n("Name")
            value: applet.kbName
        }
        PipKv {
            label: i18n("Model")
            value: applet.kbModel
        }
        PipKv {
            label: i18n("Firmware")
            value: applet.fwVersion === "" ? ""
                : applet.fwStatus === "up_to_date" ? i18n("%1, up to date", applet.fwVersion)
                : applet.fwStatus === "update_available" ? i18n("%1, update %2 at Apple", applet.fwVersion, applet.fwLatest)
                : i18n("%1, not in the table", applet.fwVersion)
            valueColor: applet.fwStatus === "update_available" ? Pip.AMBER : Pip.PHOSPHOR
        }
        // Rename on this computer (BlueZ alias); nothing is written into the keyboard.
        ColumnLayout {
            Layout.fillWidth: true
            visible: page.renaming
            spacing: Pip.GAP
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: Pip.TARGET + 4
                color: Pip.BG
                border.color: field.activeFocus ? Pip.PHOSPHOR : Pip.GREEN_MID
                border.width: 1
                TextInput {
                    id: field
                    anchors.fill: parent
                    anchors.margins: 6
                    verticalAlignment: TextInput.AlignVCenter
                    font.family: "VT323"
                    font.pixelSize: Pip.BODY
                    color: Pip.PHOSPHOR
                    selectionColor: Pip.PHOSPHOR
                    selectedTextColor: Pip.BG
                    selectByMouse: true
                    clip: true
                    maximumLength: 248
                    Accessible.role: Accessible.EditableText
                    Accessible.name: i18n("Keyboard name")
                    onAccepted: {
                        applet.renameKeyboard(text.trim());
                        page.renaming = false;
                    }
                    Keys.onEscapePressed: function (event) {
                        page.renaming = false;
                        event.accepted = true;
                    }
                }
            }
            Flow {
                Layout.fillWidth: true
                spacing: Pip.GAP
                PipButton {
                    text: i18n("Apply")
                    enabled: field.text.trim() !== ""
                    onClicked: field.accepted()
                }
                PipButton {
                    text: i18n("Keyboard's own name")
                    onClicked: {
                        applet.renameKeyboard("");
                        page.renaming = false;
                    }
                }
                PipButton {
                    text: i18n("Cancel")
                    onClicked: page.renaming = false
                }
            }
        }
        PipText {
            Layout.fillWidth: true
            visible: applet.renameError !== ""
            text: i18n("Rename refused: %1", applet.renameError)
            color: Pip.RED
            font.pixelSize: Pip.SMALL
        }
    }

    PipPanel {
        title: i18n("Actions")
        PipButton {
            Layout.fillWidth: true
            text: i18n("Reconnect")
            enabled: applet.daemonRunning
            onClicked: applet.requestReconnect()
        }
        PipButton {
            Layout.fillWidth: true
            visible: Fn.next(applet.fnMode) > 0
            enabled: !applet.fnBusy && applet.kbMac !== ""
            tint: applet.fnBusy ? Pip.AMBER : Pip.PHOSPHOR
            text: applet.fnBusy ? i18n("Fn: waiting for authorisation…")
                : Fn.kind(applet.fnMode) === "media" ? i18n("Fn: media keys first") : i18n("Fn: F1–F12 first")
            Accessible.description: applet.fnToggleText
            onClicked: applet.toggleFnMode()
        }
        PipButton {
            Layout.fillWidth: true
            text: i18n("Rename")
            enabled: applet.connected && applet.kbMac !== "" && !page.renaming
            onClicked: {
                field.text = applet.kbName;
                page.renaming = true;
                field.forceActiveFocus();
                field.selectAll();
            }
        }
        PipText {
            Layout.fillWidth: true
            visible: applet.diagHint !== ""
            text: applet.diagHint
            color: Pip.GREEN_MID
            font.pixelSize: Pip.SMALL
        }
        PipText {
            Layout.fillWidth: true
            visible: applet.fnError !== ""
            text: i18n("Fn mode not changed: %1", applet.fnError)
            color: Pip.RED
            font.pixelSize: Pip.SMALL
        }
    }
}
