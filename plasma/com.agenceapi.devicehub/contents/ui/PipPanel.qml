import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip

// Box whose title is written into its top rule (theme.rs Theme::panel).
// Children go into the inner column.
Item {
    id: panel
    property string title: ""
    property color frameColor: Pip.GREEN_FRAME
    default property alias content: column.data
    property alias spacing: column.spacing

    implicitHeight: column.implicitHeight + head.height / 2 + Pip.GAP * 2 + 4
    Layout.fillWidth: true

    Rectangle {
        anchors.fill: parent
        anchors.topMargin: head.height / 2
        color: Qt.rgba(0.016, 0.094, 0.039, 0.55)
        border.color: panel.frameColor
        border.width: 1
    }
    // The title interrupts the top rule.
    Rectangle {
        x: Pip.GUTTER - 4
        width: head.implicitWidth + 8
        height: head.height
        color: Pip.BG
        visible: panel.title !== ""
        PipText {
            id: head
            x: 4
            text: panel.title.toUpperCase()
            font.pixelSize: Pip.BODY
            color: Pip.PHOSPHOR
            wrapMode: Text.NoWrap
            Accessible.role: Accessible.Heading
        }
    }
    ColumnLayout {
        id: column
        x: Pip.GUTTER
        y: head.height + Pip.GAP - 2
        width: panel.width - Pip.GUTTER * 2
        spacing: Pip.GAP
    }
}
