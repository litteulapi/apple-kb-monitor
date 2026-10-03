import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip

// Boxed cell of the stat strip (theme.rs Theme::cell): small label, big
// glowing value that shrinks, then wraps, rather than being cut.
Rectangle {
    id: cell
    property string label: ""
    property string value: ""
    property color valueColor: Pip.PHOSPHOR
    property string note: ""
    default property alias extra: corner.data
    readonly property string shown: value !== "" ? Pip.glyphs(value) : "---"

    Layout.fillWidth: true
    Layout.preferredWidth: 1
    // Cells of one row share its height.
    Layout.fillHeight: true
    implicitHeight: col.implicitHeight + 12
    color: Qt.rgba(0.016, 0.094, 0.039, 0.55)
    border.color: Pip.GREEN_FRAME
    border.width: 1
    Accessible.role: Accessible.StaticText
    Accessible.name: label + " " + shown + (note !== "" ? ", " + note : "")

    Column {
        id: col
        x: 8
        y: 6
        width: cell.width - 16
        PipText {
            width: parent.width - corner.width
            text: cell.label.toUpperCase()
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
            // VALUE when it fits on one line, else BODY (fit_size).
            font.pixelSize: cell.shown.length * Pip.ADVANCE * Pip.VALUE <= width ? Pip.VALUE : Pip.TITLE
            Accessible.ignored: true
        }
        PipText {
            width: parent.width
            visible: cell.note !== ""
            text: Pip.glyphs(cell.note)
            color: Pip.GREEN_MID
            font.pixelSize: Pip.SMALL
            Accessible.ignored: true
        }
    }
    Item {
        id: corner
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 8
        width: childrenRect.width
        height: childrenRect.height
    }
}
