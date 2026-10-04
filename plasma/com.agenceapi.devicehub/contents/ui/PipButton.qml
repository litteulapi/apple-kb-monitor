import QtQuick
import QtQuick.Templates as T
import "Pip.js" as Pip

T.AbstractButton {
    id: b
    property color tint: Pip.PHOSPHOR
    readonly property bool lit: enabled && (hovered || down || visualFocus)

    focusPolicy: Qt.StrongFocus
    hoverEnabled: true
    implicitHeight: Math.max(Pip.TARGET, label.implicitHeight + 6)
    implicitWidth: label.implicitWidth + 20
    padding: 0
    Accessible.role: Accessible.Button
    Accessible.name: text

    Keys.onReturnPressed: b.clicked()
    Keys.onEnterPressed: b.clicked()

    background: Rectangle {
        color: b.lit ? b.tint : "transparent"
        border.color: b.enabled ? b.tint : Pip.GREEN_FRAME
        border.width: 1
    }
    contentItem: PipText {
        id: label
        text: "[ " + b.text.toLocaleUpperCase() + " ]"
        color: !b.enabled ? Pip.GREEN_FRAME : (b.lit ? Pip.BG : b.tint)
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        wrapMode: Text.Wrap
        leftPadding: 10
        rightPadding: 10
        Accessible.ignored: true
    }
}
