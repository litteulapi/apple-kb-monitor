import QtQuick
import QtQuick.Templates as T
import "Pip.js" as Pip

// Terminal button "[ LABEL ]" (theme.rs action): framed, inverse video when
// hovered, pressed or focused from the keyboard; amber or red for a pending
// or dangerous action. Activated by click, Space or Enter.
T.AbstractButton {
    id: b
    property color tint: Pip.PHOSPHOR
    property bool brackets: true
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
        text: b.brackets ? "[ " + b.text.toUpperCase() + " ]" : b.text.toUpperCase()
        color: !b.enabled ? Pip.GREEN_FRAME : (b.lit ? Pip.BG : b.tint)
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        wrapMode: Text.Wrap
        leftPadding: 10
        rightPadding: 10
        Accessible.ignored: true
    }
}
