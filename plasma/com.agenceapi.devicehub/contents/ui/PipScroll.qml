import QtQuick
import "Pip.js" as Pip

Flickable {
    id: f
    default property alias content: body.data
    readonly property bool scrollable: contentHeight > height + 1
    contentWidth: width
    contentHeight: body.childrenRect.height + Pip.GAP
    clip: true
    boundsBehavior: Flickable.StopAtBounds
    flickableDirection: Flickable.VerticalFlick
    pixelAligned: true

    function scrollBy(dy) {
        f.contentY = Math.max(0, Math.min(f.contentHeight - f.height, f.contentY + dy));
    }

    Item {
        id: body
        parent: f.contentItem
        // The indicator has its own gutter: the width never depends on the height.
        width: f.width - 12
        height: childrenRect.height
    }

    Rectangle {
        parent: f
        visible: f.scrollable
        x: f.width - 6
        y: 0
        width: 6
        height: f.height
        color: "transparent"
        border.color: Pip.GREEN_FRAME
        border.width: 1
        Rectangle {
            x: 1
            width: 4
            y: f.visibleArea.yPosition * (parent.height - 2) + 1
            height: Math.max(12, f.visibleArea.heightRatio * (parent.height - 2))
            color: Pip.PHOSPHOR
        }
    }
}
