import QtQuick
import QtQuick.Templates as T
import "Pip.js" as Pip

T.AbstractButton {
    id: c
    property bool chosen: false
    readonly property bool lit: chosen || (enabled && (hovered || visualFocus))

    focusPolicy: Qt.StrongFocus
    hoverEnabled: true
    implicitHeight: Math.max(Pip.TARGET, label.implicitHeight + 6)
    implicitWidth: label.implicitWidth + 8
    Accessible.role: Accessible.RadioButton
    Accessible.checkable: true
    Accessible.checked: chosen
    Accessible.name: text

    Keys.onReturnPressed: c.clicked()
    Keys.onEnterPressed: c.clicked()

    background: Rectangle {
        color: c.chosen ? Pip.PHOSPHOR : (c.lit ? Qt.rgba(0.08, 1, 0, 0.16) : "transparent")
        border.color: c.chosen ? Pip.PHOSPHOR : (c.enabled ? Pip.GREEN_MID : Pip.GREEN_FRAME)
        border.width: 1
    }
    contentItem: PipText {
        id: label
        text: c.text.toLocaleUpperCase()
        color: c.chosen ? Pip.BG : (c.enabled ? Pip.PHOSPHOR : Pip.GREEN_FRAME)
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        leftPadding: 10
        rightPadding: 10
        Accessible.ignored: true
    }
}
