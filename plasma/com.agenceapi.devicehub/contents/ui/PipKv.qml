import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip

// "LABEL ........ value" row (theme.rs Theme::kv): the value moves under its
// label and wraps when both do not fit on one line. Never empty: an unknown
// value is written "---".
Item {
    id: kv
    property string label: ""
    property string value: ""
    property color valueColor: Pip.PHOSPHOR
    readonly property string shown: value !== "" ? value : "---"
    readonly property bool inline: keyMetrics.advanceWidth + valMetrics.advanceWidth + 3 * Pip.GAP <= width

    Layout.fillWidth: true
    implicitHeight: inline ? Pip.ROW : key.implicitHeight + val.implicitHeight

    Accessible.role: Accessible.StaticText
    Accessible.name: label + " " + shown

    TextMetrics { id: keyMetrics; font: key.font; text: key.text }
    TextMetrics { id: valMetrics; font: val.font; text: val.text }

    PipText {
        id: key
        text: kv.label.toUpperCase()
        color: Pip.GREEN_MID
        wrapMode: Text.NoWrap
        width: Math.min(implicitWidth, kv.width)
        elide: Text.ElideRight
        Accessible.ignored: true
    }
    // Dotted leader between the label and the value.
    Row {
        visible: kv.inline
        x: key.implicitWidth + Pip.GAP / 2
        y: Pip.ROW - 7
        width: kv.width - x - valMetrics.advanceWidth - Pip.GAP
        spacing: 4
        clip: true
        Repeater {
            model: Math.max(0, Math.floor((kv.width - key.implicitWidth - valMetrics.advanceWidth - Pip.GAP * 1.5) / 6))
            delegate: Rectangle { width: 2; height: 2; color: Pip.GREEN_FRAME }
        }
    }
    PipText {
        id: val
        text: Pip.glyphs(kv.shown)
        color: kv.value !== "" ? kv.valueColor : Pip.GREEN_MID
        x: kv.inline ? kv.width - implicitWidth : Pip.GUTTER
        y: kv.inline ? 0 : key.implicitHeight
        width: kv.inline ? implicitWidth : kv.width - Pip.GUTTER
        wrapMode: kv.inline ? Text.NoWrap : Text.Wrap
        horizontalAlignment: Text.AlignLeft
        Accessible.ignored: true
    }
}
