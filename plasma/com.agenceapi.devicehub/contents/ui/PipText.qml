pragma ComponentBehavior: Bound

import QtQuick
import "Pip.js" as Pip

// Terminal text: VT323, plain text only, wrapped, never cut.
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
