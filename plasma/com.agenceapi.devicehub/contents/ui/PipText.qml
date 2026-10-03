import QtQuick
import "Pip.js" as Pip

// Text of the terminal: VT323, plain text only (#205), wraps instead of
// being cut (DA-PIPBOY §1.4). `glow` adds the static phosphor halo of the
// window (theme.rs HALO: four copies at 1 px, four at 2 px).
Text {
    id: t
    property bool glow: false
    font.family: "VT323"
    font.pixelSize: Pip.BODY
    color: Pip.PHOSPHOR
    textFormat: Text.PlainText
    wrapMode: Text.Wrap
    renderType: Text.QtRendering
    Accessible.role: Accessible.StaticText
    Accessible.name: text

    Repeater {
        model: t.glow ? [[-1, 0, 0.14], [1, 0, 0.14], [0, -1, 0.14], [0, 1, 0.14],
                         [-2, 0, 0.06], [2, 0, 0.06], [0, -2, 0.06], [0, 2, 0.06]] : []
        delegate: Text {
            required property var modelData
            x: modelData[0] * Math.max(1, t.font.pixelSize / 24)
            y: modelData[1] * Math.max(1, t.font.pixelSize / 24)
            z: -1
            width: t.width
            text: t.text
            textFormat: Text.PlainText
            font: t.font
            color: t.color
            opacity: modelData[2]
            wrapMode: t.wrapMode
            horizontalAlignment: t.horizontalAlignment
            lineHeight: t.lineHeight
            renderType: Text.QtRendering
            Accessible.ignored: true
        }
    }
}
