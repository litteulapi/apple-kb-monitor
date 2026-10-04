import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents3

MouseArea {
    id: compact
    required property var applet

    readonly property int size: Math.min(width, height)

    // As Plasma's DefaultCompactRepresentation: the popup closes itself at the press
    // (focus lost), so the click must not read `expanded` again and reopen it.
    property bool wasExpanded: false

    hoverEnabled: true
    function press() { wasExpanded = compact.applet.expanded; }
    function click() { compact.applet.expanded = !wasExpanded; }
    onPressed: press()
    onClicked: click()

    Layout.minimumWidth: Kirigami.Units.iconSizes.small
    Layout.minimumHeight: Kirigami.Units.iconSizes.small

    Accessible.role: Accessible.Button
    Accessible.name: compact.applet.accessibleSummary
    Accessible.description: compact.applet.toolTipSubText
    Accessible.onPressAction: compact.applet.expanded = !compact.applet.expanded

    Kirigami.Icon {
        anchors.fill: parent
        source: "apihub-scarab"
        active: compact.containsMouse
        opacity: compact.applet.connected ? 1 : 0.5
    }

    Rectangle {
        id: badge
        visible: compact.applet.hasBattery && compact.size >= Kirigami.Units.iconSizes.smallMedium
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        width: Math.max(height, badgeLabel.implicitWidth + Kirigami.Units.smallSpacing)
        height: badgeLabel.implicitHeight
        radius: height / 2
        color: compact.applet.batteryPercent <= 15
            ? Kirigami.Theme.negativeBackgroundColor
            : compact.applet.batteryPercent <= 50
                ? Kirigami.Theme.neutralBackgroundColor
                : Kirigami.Theme.backgroundColor
        border.width: 1
        border.color: compact.applet.batteryPercent <= 15
            ? Kirigami.Theme.negativeTextColor
            : compact.applet.batteryPercent <= 50
                ? Kirigami.Theme.neutralTextColor
                : Kirigami.Theme.positiveTextColor

        PlasmaComponents3.Label {
            id: badgeLabel
            anchors.centerIn: parent
            text: compact.applet.batteryPercent
            textFormat: Text.PlainText
            font.pixelSize: Math.max(compact.size * 0.3, Kirigami.Theme.smallFont.pixelSize)
            font.bold: true
            color: Kirigami.Theme.textColor
            Accessible.ignored: true
        }
    }
}
