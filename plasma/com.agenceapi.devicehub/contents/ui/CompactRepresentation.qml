import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents3

MouseArea {
    id: compact
    readonly property int size: Math.min(width, height)
    onClicked: root.expanded = !root.expanded

    Kirigami.Icon {
        anchors.centerIn: parent
        width: compact.size
        height: compact.size
        source: "apihub-scarab"
        color: root.connected ? Kirigami.Theme.textColor : Kirigami.Theme.disabledTextColor
    }

    Rectangle {
        visible: root.connected
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        width: badgeLabel.implicitWidth + Kirigami.Units.smallSpacing * 2
        height: badgeLabel.implicitHeight + Kirigami.Units.smallSpacing
        radius: height / 2
        color: root.batteryPercent <= 15
            ? Kirigami.Theme.negativeTextColor
            : root.batteryPercent <= 50
                ? Kirigami.Theme.neutralTextColor
                : Kirigami.Theme.positiveTextColor

        PlasmaComponents3.Label {
            id: badgeLabel
            anchors.centerIn: parent
            text: root.batteryPercent
            font.pixelSize: Math.max(compact.size * 0.28, Kirigami.Theme.smallFont.pixelSize)
            font.bold: true
            color: Kirigami.Theme.highlightedTextColor
        }
    }
}
