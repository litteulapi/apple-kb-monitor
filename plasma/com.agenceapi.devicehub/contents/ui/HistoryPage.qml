import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents3

// History page of the popup (#120): battery and voltage over 7, 30 or 90
// days, read from the daemon (`History(since)`), never from a file or the
// keyboard. `applet` is the PlasmoidItem (main.qml): this page only reads
// its history* properties and asks for another period.
ColumnLayout {
    id: page

    required property var applet
    spacing: Kirigami.Units.smallSpacing

    readonly property bool hasChart: page.applet.historyCount >= 2
    readonly property bool hasVoltage: page.applet.historyVolt.length >= 2

    RowLayout {
        Layout.fillWidth: true
        spacing: Kirigami.Units.smallSpacing

        Repeater {
            model: [7, 30, 90]
            PlasmaComponents3.ToolButton {
                required property int modelData
                checkable: true
                checked: page.applet.historyDays === modelData
                text: i18n("%1 d", modelData)
                Accessible.name: i18n("Last %1 days", modelData)
                onClicked: page.applet.showHistory(modelData)
            }
        }
        Item {
            Layout.fillWidth: true
        }
        PlasmaComponents3.BusyIndicator {
            implicitWidth: Kirigami.Units.iconSizes.smallMedium
            implicitHeight: Kirigami.Units.iconSizes.smallMedium
            visible: page.applet.historyLoading
            running: visible
        }
        PlasmaComponents3.ToolButton {
            icon.name: "view-refresh"
            text: i18n("Refresh")
            display: PlasmaComponents3.AbstractButton.IconOnly
            enabled: !page.applet.historyLoading
            Accessible.name: text
            onClicked: page.applet.showHistory(page.applet.historyDays)
        }
    }

    PlasmaComponents3.Label {
        Layout.fillWidth: true
        visible: page.applet.historyError !== ""
        text: i18n("History not available: %1", page.applet.historyError)
        textFormat: Text.PlainText
        color: Kirigami.Theme.negativeTextColor
        wrapMode: Text.Wrap
    }

    PlasmaComponents3.Label {
        Layout.fillWidth: true
        visible: page.applet.historyError === ""
        text: page.hasChart
            ? i18n("%1 readings over %2 days", page.applet.historyCount, page.applet.historySpanDays)
            : (page.applet.historyLoading
                ? i18n("Loading history…")
                : i18n("Not enough readings over the last %1 days.", page.applet.historyDays))
        textFormat: Text.PlainText
        opacity: 0.7
        wrapMode: Text.Wrap
    }

    LineChart {
        Layout.fillWidth: true
        Layout.preferredHeight: Kirigami.Units.gridUnit * 7
        visible: page.hasChart
        primary: page.applet.historyPct
        secondary: page.applet.historyVolt
        tMin: page.applet.historyFrom
        tMax: page.applet.historyTo
        primaryMin: 0
        primaryMax: 100
        // 5 % of margin so that the voltage line never touches the frame.
        secondaryMin: page.applet.historyVoltMin - Math.max(0.05, (page.applet.historyVoltMax - page.applet.historyVoltMin) * 0.05)
        secondaryMax: page.applet.historyVoltMax + Math.max(0.05, (page.applet.historyVoltMax - page.applet.historyVoltMin) * 0.05)
        primaryColor: Kirigami.Theme.positiveTextColor
        secondaryColor: Kirigami.Theme.linkColor
        gridColor: Kirigami.Theme.textColor
        gridLines: 5
        lineWidth: 2
        Accessible.role: Accessible.Graphic
        Accessible.name: i18n("Battery and voltage chart, last %1 days", page.applet.historyDays)
    }

    RowLayout {
        Layout.fillWidth: true
        visible: page.hasChart
        PlasmaComponents3.Label {
            text: Qt.formatDate(new Date(page.applet.historyFrom * 1000), Qt.locale().dateFormat(Locale.ShortFormat))
            textFormat: Text.PlainText
            font: Kirigami.Theme.smallFont
            opacity: 0.7
        }
        Item {
            Layout.fillWidth: true
        }
        PlasmaComponents3.Label {
            text: Qt.formatDate(new Date(page.applet.historyTo * 1000), Qt.locale().dateFormat(Locale.ShortFormat))
            textFormat: Text.PlainText
            font: Kirigami.Theme.smallFont
            opacity: 0.7
        }
    }

    PlasmaComponents3.Label {
        Layout.fillWidth: true
        visible: page.hasChart
        text: i18n("Battery: %1% to %2%", Math.round(page.applet.historyPctMin), Math.round(page.applet.historyPctMax))
        textFormat: Text.PlainText
        color: Kirigami.Theme.positiveTextColor
        wrapMode: Text.Wrap
    }
    PlasmaComponents3.Label {
        Layout.fillWidth: true
        visible: page.hasChart && page.hasVoltage
        text: i18n("Voltage: %1 to %2 V", page.applet.historyVoltMin.toFixed(2), page.applet.historyVoltMax.toFixed(2))
        textFormat: Text.PlainText
        color: Kirigami.Theme.linkColor
        wrapMode: Text.Wrap
    }
}
