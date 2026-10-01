import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents3

// Notification-area icon: scarab + battery badge (percent). Greyed out when
// the keyboard or the daemon is absent.
MouseArea {
    id: compact

    readonly property int size: Math.min(width, height)

    hoverEnabled: true
    acceptedButtons: Qt.LeftButton | Qt.MiddleButton
    onClicked: root.expanded = !root.expanded

    Layout.minimumWidth: Kirigami.Units.iconSizes.small
    Layout.minimumHeight: Kirigami.Units.iconSizes.small

    Accessible.role: Accessible.Button
    Accessible.name: root.accessibleSummary
    Accessible.description: root.toolTipSubText
    Accessible.onPressAction: root.expanded = !root.expanded

    Kirigami.Icon {
        anchors.fill: parent
        source: "apihub-scarab"
        active: compact.containsMouse
        // Monochrome tint only while idle so the icon follows light/dark.
        opacity: root.connected ? 1 : 0.5
    }

    Rectangle {
        id: badge
        visible: root.hasBattery && compact.size >= Kirigami.Units.iconSizes.smallMedium
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        width: Math.max(height, badgeLabel.implicitWidth + Kirigami.Units.smallSpacing)
        height: badgeLabel.implicitHeight
        radius: height / 2
        color: root.batteryPercent <= 15
            ? Kirigami.Theme.negativeBackgroundColor
            : root.batteryPercent <= 50
                ? Kirigami.Theme.neutralBackgroundColor
                : Kirigami.Theme.backgroundColor
        border.width: 1
        border.color: root.batteryPercent <= 15
            ? Kirigami.Theme.negativeTextColor
            : root.batteryPercent <= 50
                ? Kirigami.Theme.neutralTextColor
                : Kirigami.Theme.positiveTextColor

        PlasmaComponents3.Label {
            id: badgeLabel
            anchors.centerIn: parent
            text: root.batteryPercent
            font.pixelSize: Math.max(compact.size * 0.3, Kirigami.Theme.smallFont.pixelSize)
            font.bold: true
            color: Kirigami.Theme.textColor
            Accessible.ignored: true
        }
    }
}
