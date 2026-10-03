import QtQuick
import "Pip.js" as Pip

// Four signal bars of rising height, `lit` of them in `color` (theme.rs
// signal_bars).
Row {
    id: bars
    property int lit: 0
    property color color: Pip.PHOSPHOR
    property real barHeight: 24
    spacing: 3
    Accessible.ignored: true
    Repeater {
        model: 4
        delegate: Rectangle {
            required property int index
            anchors.bottom: parent ? parent.bottom : undefined
            width: Math.round(bars.barHeight / 3)
            height: bars.barHeight * (0.25 + 0.25 * index)
            color: index < bars.lit ? bars.color : "transparent"
            border.color: index < bars.lit ? bars.color : Pip.GREEN_FRAME
            border.width: 1
        }
    }
}
