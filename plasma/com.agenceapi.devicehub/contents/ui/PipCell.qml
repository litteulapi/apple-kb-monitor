import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip

Rectangle {
    id: cell
    property string label: ""
    property string value: ""
    property color valueColor: Pip.PHOSPHOR
    readonly property string shown: value !== "" ? Pip.glyphs(value) : "---"

    Layout.fillWidth: true
    Layout.preferredWidth: 1
    Layout.fillHeight: true
    implicitHeight: col.implicitHeight + 12
    color: Qt.rgba(0.016, 0.094, 0.039, 0.55)
    border.color: Pip.GREEN_FRAME
    border.width: 1
    Accessible.role: Accessible.StaticText
    Accessible.name: label + " " + shown

    Column {
        id: col
        x: 8
        y: 6
        width: cell.width - 16
        PipText {
            width: parent.width
            text: cell.label.toLocaleUpperCase()
            color: Pip.GREEN_MID
            font.pixelSize: Pip.SMALL
            wrapMode: Text.NoWrap
            elide: Text.ElideRight
            Accessible.ignored: true
        }
        PipText {
            width: parent.width
            text: cell.shown
            glow: cell.value !== ""
            color: cell.value !== "" ? cell.valueColor : Pip.GREEN_MID
            font.pixelSize: cell.shown.length * Pip.ADVANCE * Pip.VALUE <= width ? Pip.VALUE : Pip.TITLE
            Accessible.ignored: true
        }
    }
}
