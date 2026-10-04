import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip

Rectangle {
    id: a
    property string text: ""
    property color tint: Pip.AMBER
    Layout.fillWidth: true
    implicitHeight: msg.implicitHeight + 8
    color: Qt.rgba(tint.r, tint.g, tint.b, 0.10)
    border.color: tint
    border.width: 1
    Accessible.role: Accessible.AlertMessage
    Accessible.name: text

    Rectangle {
        width: 4
        height: parent.height
        color: a.tint
    }
    PipText {
        id: mark
        x: 12
        y: 4
        text: "[!]"
        color: a.tint
        wrapMode: Text.NoWrap
        Accessible.ignored: true
    }
    PipText {
        id: msg
        x: mark.x + mark.implicitWidth + 8
        y: 4
        width: a.width - x - 8
        text: Pip.glyphs(a.text)
        color: a.tint
        Accessible.ignored: true
    }
}
