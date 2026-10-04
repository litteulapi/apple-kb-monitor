pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip
import "FnMode.js" as Fn

ColumnLayout {
    id: page
    readonly property SignalIssue signalIssue: SignalIssue { code: page.applet ? page.applet.rssiIssue : "" }
    required property var applet
    property bool compact: false
    spacing: Pip.GAP * 1.5

    readonly property bool hasPct: applet.connected && applet.batteryPercent >= 0
    readonly property color level: hasPct ? Pip.levelColor(applet.batteryPercent) : Pip.GREEN_MID
    // Days left from the daemon's forecast, re-evaluated at each reading, never by a timer.
    readonly property real days: {
        var reading = applet.lastUpdate;
        return Pip.daysLeft({ empty_at: applet.forecastEmptyAt }, Date.now() / 1000);
    }
    property bool renaming: false
    readonly property string stateWord: !applet.connected ? "" : applet.thresholdLevel === "ok" ? i18n("OK")
        : applet.thresholdLevel === "low" ? i18n("LOW")
        : applet.thresholdLevel === "critical" ? i18n("CRITICAL")
        : applet.thresholdLevel === "empty" ? i18n("EMPTY") : ""
    readonly property var facts: [
        [i18n("State"), page.stateWord, Pip.thresholdColor(applet.thresholdLevel)],
        [i18n("Voltage"), applet.connected && applet.voltage > 0 ? i18n("%1 V", Pip.num(applet.voltage, 2)) : "",
         Pip.thresholdColor(applet.thresholdLevel)],
        [i18n("Autonomy"), page.days < 0 ? "" : page.days >= 1.5 ? i18np("≈ %1 day", "≈ %1 days", Math.round(page.days))
            : i18n("≈ %1 h", Math.max(0, Math.round(page.days * 24))),
         page.days >= 0 && page.days < 7 ? Pip.AMBER : Pip.PHOSPHOR],
        [i18n("Signal"), applet.hasRssi ? applet.rssiText : (applet.connected ? page.signalIssue.shortText : ""),
         applet.hasRssi ? Pip.qualityColor(applet.rssiQuality) : Pip.AMBER]]


    PipPanel {
        title: i18n("Batteries")
        RowLayout {
            Layout.fillWidth: true
            spacing: Pip.GAP * 2
            Rectangle {
                Layout.alignment: Qt.AlignVCenter
                visible: !page.applet.connected
                implicitWidth: plaque.implicitWidth + 24
                implicitHeight: plaque.implicitHeight + 12
                color: Qt.rgba(1, 0.35, 0.24, 0.10)
                border.color: Pip.RED
                border.width: 2
                PipText {
                    id: plaque
                    anchors.centerIn: parent
                    text: page.applet.daemonRunning ? i18n("OFFLINE") : i18n("NO DAEMON")
                    font.pixelSize: Pip.VALUE
                    color: page.applet.daemonRunning ? Pip.RED : Pip.AMBER
                    glow: true
                    wrapMode: Text.NoWrap
                }
            }
            Row {
                Layout.alignment: Qt.AlignTop
                visible: page.applet.connected
                // the percent sign goes where the language puts it ("50%", "50 %", "%50").
                readonly property var sign: Pip.affixes(i18n("%1%", "%1"))
                PipText {
                    visible: page.hasPct && text !== ""
                    anchors.baseline: hero.baseline
                    text: parent.sign[0]
                    font.pixelSize: Pip.VALUE
                    color: page.level
                    glow: true
                    wrapMode: Text.NoWrap
                }
                PipText {
                    id: hero
                    text: page.hasPct ? String(page.applet.batteryPercent) : "---"
                    font.pixelSize: page.compact ? Pip.HERO * 0.78 : Pip.HERO
                    color: page.level
                    glow: page.hasPct
                    wrapMode: Text.NoWrap
                    lineHeight: 0.8
                    Accessible.name: page.applet.accessibleSummary
                }
                PipText {
                    visible: page.hasPct && text !== ""
                    anchors.baseline: hero.baseline
                    text: parent.sign[1]
                    font.pixelSize: Pip.VALUE
                    color: page.level
                    glow: true
                    wrapMode: Text.NoWrap
                }
            }
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
                        text: modelData[0].toLocaleUpperCase()
                        font.pixelSize: Pip.SMALL
                        color: Pip.GREEN_MID
                        wrapMode: Text.NoWrap
                    }
                }
                Repeater {
                    model: page.facts
                    delegate: RowLayout {
                        id: fact
                        required property var modelData
                        required property int index
                        Layout.row: index
                        Layout.column: 1
                        Layout.fillWidth: true
                        spacing: Pip.GAP
                        PipText {
                            Layout.fillWidth: true
                            text: fact.modelData[1] !== "" ? Pip.glyphs(fact.modelData[1]) : "---"
                            font.pixelSize: Pip.TITLE
                            color: fact.modelData[1] !== "" ? fact.modelData[2] : Pip.GREEN_MID
                            glow: fact.modelData[1] !== ""
                            Accessible.name: fact.modelData[0] + " " + text
                        }
                        PipBars {
                            visible: fact.index === 3 && page.applet.hasRssi
                            lit: Pip.qualityBars(page.applet.rssiQuality)
                            color: Pip.qualityColor(page.applet.rssiQuality)
                            barHeight: 16
                        }
                    }
                }
            }
        }
        PipSegments {
            Layout.fillWidth: true
            fraction: page.hasPct ? page.applet.batteryPercent / 100 : 0
            color: page.level
        }
        PipText {
            Layout.fillWidth: true
            text: i18n("Keyboard indication: its own percentage, it moves in steps (often at a reconnection).")
            font.pixelSize: Pip.SMALL
            color: Pip.GREEN_MID
        }
        PipKv {
            // A slope fitted on the keyboard's steps: a trend over days, not an hourly reading.
            label: i18n("Trend")
            value: page.days < 0 ? i18n("not enough history yet")
                : i18np("−%2% per day over %1 day", "−%2% per day over %1 days", Math.max(1, Math.round(page.applet.forecastSpanDays)), Pip.num(page.applet.forecastRate, 1))
            valueColor: page.days < 0 ? Pip.GREEN_MID : Pip.PHOSPHOR
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: false
            visible: page.applet.sparkPct.length >= 2
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
                primary: page.applet.sparkPct
                tMin: page.applet.sparkFrom
                tMax: page.applet.sparkTo
                primaryColor: page.level
                gridColor: Pip.GREEN_FRAME
                gridLines: 2
                lineWidth: 1.5
            }
        }
        PipKv {
            label: i18n("Estimate")
            value: page.applet.connected && (page.applet.hasEstimate || page.applet.newBatteries) ? page.applet.estimateText : ""
        }
        PipKv {
            label: i18n("Chemistry")
            value: page.applet.connected ? page.applet.batteryType : ""
        }
        PipKv {
            label: i18n("Apple display")
            value: page.applet.connected && page.applet.applePct >= 0 ? i18n("%1%", Math.round(page.applet.applePct)) : ""
        }
    }

    PipPanel {
        title: i18n("Keyboard")
        PipKv {
            label: i18n("Name")
            value: page.applet.kbName
        }
        PipKv {
            label: i18n("Model")
            value: page.applet.kbModel
        }
        PipKv {
            label: i18n("Firmware")
            value: page.applet.fwVersion === "" ? ""
                : page.applet.fwStatus === "up_to_date" ? i18n("%1, up to date", page.applet.fwVersion)
                : page.applet.fwStatus === "update_available" ? i18n("%1, update %2 at Apple", page.applet.fwVersion, page.applet.fwLatest)
                : i18n("%1, not in the table", page.applet.fwVersion)
            valueColor: page.applet.fwStatus === "update_available" ? Pip.AMBER : Pip.PHOSPHOR
        }
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
                    maximumLength: 64 // akm_core::alias::MAX_CHARS, as in the KCM
                    Accessible.role: Accessible.EditableText
                    Accessible.name: i18n("Keyboard name")
                    onAccepted: {
                        if (text.trim() === "") return;
                        page.applet.renameKeyboard(text.trim());
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
                        page.applet.renameKeyboard("");
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
            visible: page.applet.renameError !== ""
            text: i18n("Rename refused: %1", page.applet.renameError)
            color: Pip.RED
            font.pixelSize: Pip.SMALL
        }
    }

    PipPanel {
        title: i18n("Actions")
        PipButton {
            Layout.fillWidth: true
            text: i18n("Reconnect")
            enabled: page.applet.daemonRunning
            onClicked: page.applet.requestReconnect()
        }
        PipButton {
            Layout.fillWidth: true
            visible: Fn.next(page.applet.fnMode) > 0
            enabled: !page.applet.fnBusy && page.applet.kbMac !== ""
            tint: page.applet.fnBusy ? Pip.AMBER : Pip.PHOSPHOR
            // The button says what it does, as the menu does; the mode is in KEYS.
            text: page.applet.fnBusy ? i18n("Fn: waiting for authorisation…") : page.applet.fnToggleText
            Accessible.description: i18n("Current mode: %1", page.applet.fnModeText)
            onClicked: page.applet.toggleFnMode()
        }
        PipButton {
            Layout.fillWidth: true
            text: i18n("Rename")
            enabled: page.applet.connected && page.applet.kbMac !== "" && !page.renaming
            onClicked: {
                field.text = page.applet.kbName;
                page.renaming = true;
                field.forceActiveFocus();
                field.selectAll();
            }
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.diagHint !== ""
            text: page.applet.diagHint
            color: Pip.GREEN_MID
            font.pixelSize: Pip.SMALL
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.fnError !== ""
            text: i18n("Fn mode not changed: %1", page.applet.fnError)
            color: Pip.RED
            font.pixelSize: Pip.SMALL
        }
    }
}
