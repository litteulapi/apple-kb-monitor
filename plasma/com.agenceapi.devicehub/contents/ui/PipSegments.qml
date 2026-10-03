import QtQuick
import "Pip.js" as Pip

// Gauge in slanted blocks (theme.rs paint_segments): `fraction` of the blocks
// lit in `color`, the others drawn as frames. `colors`, when given, sets the
// colour of each block (null = unlit) and overrides `fraction`.
Canvas {
    id: g
    property real fraction: 0
    property color color: Pip.PHOSPHOR
    property var colors: null
    readonly property int count: colors ? colors.length : Pip.segmentCount(width)

    implicitHeight: 22
    onFractionChanged: requestPaint()
    onColorChanged: requestPaint()
    onColorsChanged: requestPaint()
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
    Accessible.role: Accessible.ProgressBar
    Accessible.name: Math.round(fraction * 100) + " %"

    onPaint: {
        var ctx = getContext("2d");
        ctx.reset();
        var n = g.count;
        if (n <= 0 || width < Pip.SEGMENT_SKEW + 2) return;
        var lit = g.colors ? -1 : Pip.litSegments(g.fraction, n);
        var step = (width - Pip.SEGMENT_SKEW + Pip.SEGMENT_GAP) / n;
        var w = Math.max(1, step - Pip.SEGMENT_GAP);
        for (var i = 0; i < n; i++) {
            var x = i * step;
            var c = g.colors ? g.colors[i] : (i < lit ? g.color : null);
            ctx.beginPath();
            ctx.moveTo(x + Pip.SEGMENT_SKEW, 0.5);
            ctx.lineTo(x + Pip.SEGMENT_SKEW + w, 0.5);
            ctx.lineTo(x + w, height - 0.5);
            ctx.lineTo(x, height - 0.5);
            ctx.closePath();
            if (c) {
                ctx.fillStyle = c;
                ctx.fill();
            } else {
                ctx.strokeStyle = Pip.GREEN_FRAME;
                ctx.lineWidth = 1;
                ctx.stroke();
            }
        }
    }
}
