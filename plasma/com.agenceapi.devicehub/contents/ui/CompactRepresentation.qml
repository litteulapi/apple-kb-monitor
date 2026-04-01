import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

MouseArea {
    id: compact
    onClicked: root.expanded = !root.expanded

    Kirigami.Icon {
        anchors.fill: parent
        source: {
            if (!root.connected) return "network-bluetooth-inactive"
            if (root.batteryPercent <= 15) return "battery-caution"
            if (root.batteryPercent <= 50) return "battery-050"
            return "battery-100"
        }

        // Dynamic color overlay based on battery
        color: {
            if (!root.connected) return Kirigami.Theme.disabledTextColor
            if (root.batteryPercent <= 15) return "#FF4444"
            if (root.batteryPercent <= 50) return "#FFaa00"
            return Kirigami.Theme.textColor
        }
    }

    // Battery percentage text overlay
    Text {
        anchors.centerIn: parent
        anchors.verticalCenterOffset: parent.height * 0.15
        text: root.connected ? root.batteryPercent : ""
        color: root.batteryPercent <= 15 ? "#FF4444" : "#FFFFFF"
        font.pixelSize: parent.height * 0.3
        font.bold: true
        font.family: "monospace"
        visible: root.connected
    }
}
