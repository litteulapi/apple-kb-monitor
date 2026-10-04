pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip

ColumnLayout {
    id: page
    required property var applet
    property bool compact: false
    spacing: Pip.GAP * 1.5

    readonly property bool hasChart: applet.historyPct.length >= 2
    readonly property bool hasVoltage: applet.historyVolt.length >= 2
    readonly property real voltMin: applet.historyVoltMin - 0.05
    readonly property real voltMax: applet.historyVoltMax + 0.05
    // Labels of the voltage scale, top to bottom, at the five grid lines.
    readonly property var voltTicks: [0, 1, 2, 3, 4].map(function (i) { return page.voltMax - i * (page.voltMax - page.voltMin) / 4; })

    function tick(t) {
        var d = new Date(t * 1000);
        return applet.historyDays <= 1 ? Qt.formatTime(d, Qt.locale().timeFormat(Locale.ShortFormat))
                                    : Qt.formatDate(d, Pip.dayMonthFormat(Qt.locale().dateFormat(Locale.ShortFormat)));
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
                    chosen: page.applet.historyDays === modelData[0]
                    enabled: page.applet.daemonRunning
                    onClicked: page.applet.showHistory(modelData[0])
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            PipButton {
                text: i18n("Refresh")
                enabled: page.applet.daemonRunning && !page.applet.historyLoading
                onClicked: page.applet.showHistory(page.applet.historyDays)
            }
            PipText {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignRight
                text: page.applet.historyLoading ? i18n("LOADING…")
                    : page.applet.historyCount > 0 ? i18n("Readings: %1", page.applet.historyCount) : ""
                color: page.applet.historyLoading ? Pip.AMBER : Pip.GREEN_MID
                font.pixelSize: Pip.SMALL
            }
        }

        Item {
            Layout.fillWidth: true
            visible: page.hasChart
            implicitHeight: 196
            Rectangle {
                x: axis.width
                width: parent.width - axis.width - voltAxis.width
                height: 172
                color: Pip.BG
                border.color: Pip.GREEN_FRAME
                border.width: 1
                LineChart {
                    anchors.fill: parent
                    anchors.margins: 1
                    primary: page.applet.historyPct
                    secondary: page.applet.historyVolt
                    tMin: page.applet.historyFrom
                    tMax: page.applet.historyTo
                    primaryMin: 0
                    primaryMax: 100
                    secondaryMin: page.voltMin
                    secondaryMax: page.voltMax
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
                    model: [100, 75, 50, 25, 0].map(function (v) { return i18n("%1%", v); })
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
            // The voltage has its own scale, on the right, in its colour.
            Item {
                id: voltAxis
                x: parent.width - width
                width: page.hasVoltage ? 52 : 0
                height: 172
                visible: page.hasVoltage
                Repeater {
                    model: page.voltTicks
                    delegate: PipText {
                        required property int index
                        required property real modelData
                        x: 4
                        y: Math.min(172 - implicitHeight, Math.max(0, index * 172 / 4 - implicitHeight / 2))
                        text: i18n("%1 V", Pip.num(modelData, 2))
                        font.pixelSize: Pip.SMALL
                        color: Pip.AMBER
                        wrapMode: Text.NoWrap
                    }
                }
            }
            Repeater {
                model: 3
                delegate: PipText {
                    required property int index
                    readonly property real f: index / 2
                    x: axis.width + f * (parent.width - axis.width - voltAxis.width) - f * implicitWidth
                    y: 176
                    text: page.tick(page.applet.historyFrom + f * (page.applet.historyTo - page.applet.historyFrom))
                    font.pixelSize: Pip.SMALL
                    color: Pip.GREEN_MID
                    wrapMode: Text.NoWrap
                }
            }
        }
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
                    text: i18n("Voltage %1 to %2 V", Pip.num(page.applet.historyVoltMin, 2), Pip.num(page.applet.historyVoltMax, 2))
                    font.pixelSize: Pip.SMALL
                    color: Pip.AMBER
                    wrapMode: Text.NoWrap
                }
            }
        }
        PipText {
            Layout.fillWidth: true
            visible: !page.applet.historyLoading && !page.hasChart
            text: page.applet.historyError !== "" ? i18n("History not available: %1", page.applet.historyError)
                : !page.applet.daemonRunning ? i18n("Monitor stopped: no history.")
                : i18n("Not enough readings over this period to draw a curve.")
            color: page.applet.historyError !== "" ? Pip.RED : Pip.GREEN_MID
        }
    }

    PipPanel {
        title: i18n("Batteries")
        PipKv {
            label: i18n("Voltage")
            value: page.applet.connected && page.applet.voltage > 0 ? i18n("%1 V", Pip.num(page.applet.voltage, 2)) : ""
            valueColor: Pip.thresholdColor(page.applet.thresholdLevel)
        }
        Item {
            id: rail
            Layout.fillWidth: true
            visible: page.applet.connected && page.applet.thresholds !== null && page.applet.voltage > 0
            implicitHeight: 58
            readonly property var th: page.applet.thresholds || { empty_mv: 0, critical_mv: 0, low_mv: 0, full_mv: 1 }
            readonly property real lo: th.empty_mv - 100
            readonly property real hi: th.full_mv + 100
            function px(mv) { return 8 + Math.min(1, Math.max(0, (mv - lo) / (hi - lo))) * (width - 16); }
            Rectangle { x: 8; y: 14; width: rail.width - 16; height: 1; color: Pip.GREEN_MID }
            Repeater {
                model: [[rail.th.empty_mv, i18n("EMPTY"), 0], [rail.th.critical_mv, i18n("CRITICAL"), 1],
                        [rail.th.low_mv, i18n("LOW"), 0], [rail.th.full_mv, i18n("FULL"), 1]]
                delegate: Item {
                    id: mark
                    required property var modelData
                    x: rail.px(modelData[0])
                    Rectangle { x: 0; y: 8; width: 1; height: 13; color: Pip.GREEN_MID }
                    PipText {
                        x: -implicitWidth / 2
                        y: mark.modelData[2] ? 38 : 22
                        text: mark.modelData[1]
                        font.pixelSize: Pip.SMALL
                        color: Pip.GREEN_MID
                        wrapMode: Text.NoWrap
                    }
                }
            }
            Rectangle {
                x: rail.px(page.applet.voltage * 1000) - 2
                y: 2
                width: 4
                height: 24
                color: Pip.thresholdColor(page.applet.thresholdLevel)
            }
        }
        PipKv {
            label: i18n("Thresholds")
            value: page.applet.thresholdsText
            valueColor: Pip.GREEN_MID
        }
        PipKv {
            label: i18n("Estimate")
            value: page.applet.connected && (page.applet.hasEstimate || page.applet.newBatteries) ? page.applet.estimateText : ""
        }
        PipKv {
            label: i18n("Apple display")
            value: page.applet.connected && page.applet.applePct >= 0 ? i18n("%1%", Math.round(page.applet.applePct)) : ""
        }
        PipKv {
            label: i18n("Chemistry")
            value: page.applet.connected ? page.applet.batteryType : ""
        }
        PipKv {
            label: i18n("Installed on")
            value: page.applet.installedAt > 0 ? Qt.formatDate(new Date(page.applet.installedAt * 1000), Qt.locale(), Locale.ShortFormat) : ""
        }
        PipKv {
            label: i18n("Trend")
            value: page.applet.forecastRate > 0 ? i18np("−%2% per day over %1 day", "−%2% per day over %1 days", Math.max(1, Math.round(page.applet.forecastSpanDays)), Pip.num(page.applet.forecastRate, 1)) : ""
        }
        PipKv {
            label: i18n("Empty around")
            value: page.applet.forecastEmptyAt > 0 ? Qt.formatDate(new Date(page.applet.forecastEmptyAt * 1000), Qt.locale(), Locale.ShortFormat) : ""
        }
        PipKv {
            label: i18n("Last reading")
            value: page.applet.lastUpdate > 0 ? page.applet.updatedText : ""
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.readIncomplete
            text: i18n("The last reading missed a report of the keyboard.")
            color: Pip.AMBER
            font.pixelSize: Pip.SMALL
        }
    }

    PipPanel {
        title: i18n("Device")
        PipKv { label: i18n("Model"); value: page.applet.kbModel }
        PipKv { label: i18n("Name"); value: page.applet.kbName }
        PipKv { label: i18n("Address"); value: Pip.maskMac(page.applet.kbMac) }
        PipKv { label: i18n("Chip"); value: page.applet.kbChip }
        PipKv { label: i18n("Driver"); value: page.applet.kbDriver }
        PipKv {
            label: i18n("Firmware")
            value: page.applet.fwVersion
            valueColor: page.applet.fwStatus === "update_available" ? Pip.AMBER : Pip.PHOSPHOR
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.fwText !== ""
            text: page.applet.fwText
            font.pixelSize: Pip.SMALL
            color: page.applet.fwStatus === "update_available" ? Pip.AMBER : Pip.GREEN_MID
        }
    }
}
