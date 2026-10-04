import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip

Item {
    id: panel
    property string title: ""
    default property alias content: column.data
    property alias spacing: column.spacing

    implicitHeight: column.implicitHeight + head.height / 2 + Pip.GAP * 2 + 4
    Layout.fillWidth: true

    Rectangle {
        anchors.fill: parent
        anchors.topMargin: head.height / 2
        color: Qt.rgba(0.016, 0.094, 0.039, 0.55)
        border.color: Pip.GREEN_FRAME
        border.width: 1
    }
    Rectangle {
        x: Pip.GUTTER - 4
        width: head.implicitWidth + 8
        height: head.height
        color: Pip.BG
        visible: panel.title !== ""
        PipText {
            id: head
            x: 4
            text: panel.title.toLocaleUpperCase()
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
