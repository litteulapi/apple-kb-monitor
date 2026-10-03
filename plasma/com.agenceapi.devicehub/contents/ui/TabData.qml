import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip

// DATA: battery history over 24 h / 7 / 30 / 90 days as a cathode trace
// (percentage, and voltage when measured), the batteries in detail (voltage
// placed on the keyboard's thresholds, estimate, chemistry, autonomy) and
// the device.
ColumnLayout {
    id: page
    property var applet
    // Popup at the tray's own size: smaller figures.
    property bool compact: false
    spacing: Pip.GAP * 1.5

    readonly property bool hasChart: applet.historyPct.length >= 2
    readonly property bool hasVoltage: applet.historyVolt.length >= 2

    function num(v, digits) {
        return Number(v).toLocaleString(Qt.locale(), "f", digits);
    }
    // Label of a time on the axis: hour for 24 h, day and month otherwise.
    function tick(t) {
        var d = new Date(t * 1000);
        return applet.historyDays <= 1 ? Qt.formatTime(d, "HH:mm") : Qt.formatDate(d, "dd/MM");
    }

    PipPanel {
        title: i18n("Battery history")
        RowLayout {
            Layout.fillWidth: true
            spacing: Pip.GAP
            Repeater {
                model: [[1, i18n("24 h")], [7, i18n("7 d")], [30, i18n("30 d")], [90, i18n("90 d")]]
                delegate: PipChoice {
                    required property var modelData
                    Layout.fillWidth: true
                    text: modelData[1]
                    chosen: applet.historyDays === modelData[0]
                    enabled: applet.daemonRunning
                    onClicked: applet.showHistory(modelData[0])
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            PipButton {
                text: i18n("Refresh")
                enabled: applet.daemonRunning && !applet.historyLoading
                onClicked: applet.showHistory(applet.historyDays)
            }
            PipText {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignRight
                text: applet.historyLoading ? i18n("LOADING…")
                    : applet.historyCount > 0 ? i18n("Readings: %1", applet.historyCount) : ""
                color: applet.historyLoading ? Pip.AMBER : Pip.GREEN_MID
                font.pixelSize: Pip.SMALL
            }
        }

        // Chart: axis labels on the left, cathode trace on a CRT grid.
        Item {
            Layout.fillWidth: true
            visible: page.hasChart
            implicitHeight: 196
            Rectangle {
                x: axis.width
                width: parent.width - axis.width
                height: 172
                color: Pip.BG
                border.color: Pip.GREEN_FRAME
                border.width: 1
                LineChart {
                    anchors.fill: parent
                    anchors.margins: 1
                    crt: true
                    primary: applet.historyPct
                    secondary: applet.historyVolt
                    tMin: applet.historyFrom
                    tMax: applet.historyTo
                    primaryMin: 0
                    primaryMax: 100
                    secondaryMin: applet.historyVoltMin - 0.05
                    secondaryMax: applet.historyVoltMax + 0.05
                    primaryColor: Pip.PHOSPHOR
                    secondaryColor: Pip.AMBER
                    gridColor: Pip.GREEN_FRAME
                    gridLines: 5
                    vGridLines: 5
                    lineWidth: 2
                }
            }
            Item {
                id: axis
                width: 44
                height: 172
                Repeater {
                    model: ["100%", "75%", "50%", "25%", "0%"]
                    delegate: PipText {
                        required property int index
                        required property string modelData
                        width: 40
                        y: Math.min(172 - implicitHeight, Math.max(0, index * 172 / 4 - implicitHeight / 2))
                        text: modelData
                        horizontalAlignment: Text.AlignRight
                        font.pixelSize: Pip.SMALL
                        color: Pip.GREEN_MID
                        wrapMode: Text.NoWrap
                    }
                }
            }
            Repeater {
                model: 3
                delegate: PipText {
                    required property int index
                    readonly property real f: index / 2
                    x: axis.width + f * (parent.width - axis.width) - f * implicitWidth
                    y: 176
                    text: page.tick(applet.historyFrom + f * (applet.historyTo - applet.historyFrom))
                    font.pixelSize: Pip.SMALL
                    color: Pip.GREEN_MID
                    wrapMode: Text.NoWrap
                }
            }
        }
        // Legend.
        Flow {
            Layout.fillWidth: true
            visible: page.hasChart
            spacing: Pip.GAP * 2
            Row {
                spacing: 6
                Rectangle { width: 18; height: 2; color: Pip.PHOSPHOR; anchors.verticalCenter: parent.verticalCenter }
                PipText { text: i18n("Batteries %"); font.pixelSize: Pip.SMALL; wrapMode: Text.NoWrap }
            }
            Row {
                visible: page.hasVoltage
                spacing: 6
                Rectangle { width: 18; height: 2; color: Pip.AMBER; anchors.verticalCenter: parent.verticalCenter }
                PipText {
                    text: i18n("Voltage %1 to %2 V", page.num(applet.historyVoltMin, 2), page.num(applet.historyVoltMax, 2))
                    font.pixelSize: Pip.SMALL
                    color: Pip.AMBER
                    wrapMode: Text.NoWrap
                }
            }
        }
        PipText {
            Layout.fillWidth: true
            visible: !applet.historyLoading && !page.hasChart
            text: applet.historyError !== "" ? i18n("History not available: %1", applet.historyError)
                : !applet.daemonRunning ? i18n("Monitor stopped: no history.")
                : i18n("Not enough readings over this period to draw a curve.")
            color: applet.historyError !== "" ? Pip.RED : Pip.GREEN_MID
        }
    }

    PipPanel {
        title: i18n("Batteries")
        PipKv {
            label: i18n("Voltage")
            value: applet.connected && applet.voltage > 0 ? i18n("%1 V", page.num(applet.voltage, 2)) : ""
            valueColor: Pip.thresholdColor(applet.thresholdLevel)
        }
        // Voltage placed on the keyboard's own thresholds.
        Item {
            id: rail
            Layout.fillWidth: true
            visible: applet.connected && applet.thresholds !== null && applet.voltage > 0
            implicitHeight: 58
            readonly property var th: applet.thresholds || { empty_mv: 0, critical_mv: 0, low_mv: 0, full_mv: 1 }
            readonly property real lo: th.empty_mv - 100
            readonly property real hi: th.full_mv + 100
            function px(mv) { return 8 + Math.min(1, Math.max(0, (mv - lo) / (hi - lo))) * (width - 16); }
            Rectangle { x: 8; y: 14; width: rail.width - 16; height: 1; color: Pip.GREEN_MID }
            Repeater {
                model: [[rail.th.empty_mv, i18n("EMPTY"), 0], [rail.th.critical_mv, i18n("CRITICAL"), 1],
                        [rail.th.low_mv, i18n("LOW"), 0], [rail.th.full_mv, i18n("FULL"), 1]]
                delegate: Item {
                    required property var modelData
                    x: rail.px(modelData[0])
                    Rectangle { x: 0; y: 8; width: 1; height: 13; color: Pip.GREEN_MID }
                    PipText {
                        x: -implicitWidth / 2
                        y: modelData[2] ? 38 : 22
                        text: modelData[1]
                        font.pixelSize: Pip.SMALL
                        color: Pip.GREEN_MID
                        wrapMode: Text.NoWrap
                    }
                }
            }
            Rectangle {
                x: rail.px(applet.voltage * 1000) - 2
                y: 2
                width: 4
                height: 24
                color: Pip.thresholdColor(applet.thresholdLevel)
            }
        }
        PipKv {
            label: i18n("Thresholds")
            value: applet.thresholdsText
            valueColor: Pip.GREEN_MID
        }
        PipKv {
            label: i18n("Estimate")
            value: applet.connected && (applet.hasEstimate || applet.newBatteries) ? applet.estimateText : ""
        }
        PipKv {
            label: i18n("Apple display")
            value: applet.connected && applet.applePct >= 0 ? i18n("%1%", Math.round(applet.applePct)) : ""
        }
        PipKv {
            label: i18n("Chemistry")
            value: applet.connected ? applet.batteryType : ""
        }
        PipKv {
            label: i18n("Installed on")
            value: applet.installedAt > 0 ? Qt.formatDate(new Date(applet.installedAt * 1000), "dd/MM/yyyy") : ""
        }
        PipKv {
            label: i18n("Discharge")
            value: applet.forecastRate > 0 ? i18n("−%1 % per day", page.num(applet.forecastRate, 1)) : ""
        }
        PipKv {
            label: i18n("Empty around")
            value: applet.forecastEmptyAt > 0 ? Qt.formatDate(new Date(applet.forecastEmptyAt * 1000), "dd/MM/yyyy") : ""
        }
        PipKv {
            label: i18n("Last reading")
            value: applet.lastUpdate > 0 ? applet.updatedText : ""
        }
        PipText {
            Layout.fillWidth: true
            visible: applet.readIncomplete
            text: i18n("The last reading missed a report of the keyboard.")
            color: Pip.AMBER
            font.pixelSize: Pip.SMALL
        }
    }

    PipPanel {
        title: i18n("Device")
        PipKv { label: i18n("Model"); value: applet.kbModel }
        PipKv { label: i18n("Name"); value: applet.kbName }
        PipKv { label: i18n("Address"); value: applet.kbMac }
        PipKv { label: i18n("Chip"); value: applet.kbChip }
        PipKv { label: i18n("Driver"); value: applet.kbDriver }
        PipKv {
            label: i18n("Firmware")
            value: applet.fwVersion
            valueColor: applet.fwStatus === "update_available" ? Pip.AMBER : Pip.PHOSPHOR
        }
        PipText {
            Layout.fillWidth: true
            visible: applet.fwText !== ""
            text: applet.fwText
            font.pixelSize: Pip.SMALL
            color: applet.fwStatus === "update_available" ? Pip.AMBER : Pip.GREEN_MID
        }
    }
}
