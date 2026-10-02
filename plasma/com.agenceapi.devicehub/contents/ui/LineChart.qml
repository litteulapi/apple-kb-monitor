import QtQuick

// Line chart of one or two series of [t, y] points on a shared time axis,
// each with its own vertical range. Pure drawing (Canvas, no chart library):
// the points are prepared and bounded by History.js. Used as the 7-day
// sparkline (#97) and as the chart of the History page (#120).
Canvas {
    id: chart

    property var primary: []
    property var secondary: []
    property real tMin: 0
    property real tMax: 1
    property real primaryMin: 0
    property real primaryMax: 100
    property real secondaryMin: 0
    property real secondaryMax: 1
    property color primaryColor: "green"
    property color secondaryColor: "blue"
    property color gridColor: "gray"
    // Horizontal grid lines, bounds included (0 = none, as for a sparkline).
    property int gridLines: 0
    property real lineWidth: 1.5

    onPrimaryChanged: requestPaint()
    onSecondaryChanged: requestPaint()
    onTMinChanged: requestPaint()
    onTMaxChanged: requestPaint()
    onPrimaryColorChanged: requestPaint()
    onSecondaryColorChanged: requestPaint()
    onGridColorChanged: requestPaint()
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()

    function drawSeries(ctx, pts, lo, hi, color, lw) {
        if (!pts || pts.length < 2) return;
        var span = Math.max(1, chart.tMax - chart.tMin);
        var range = hi - lo;
        ctx.beginPath();
        for (var i = 0; i < pts.length; i++) {
            var x = Math.min(1, Math.max(0, (pts[i][0] - chart.tMin) / span)) * (chart.width - lw) + lw / 2;
            // A constant series is a flat line in the middle, not a division by 0.
            var f = range > 0 ? Math.min(1, Math.max(0, (pts[i][1] - lo) / range)) : 0.5;
            var y = (1 - f) * (chart.height - lw) + lw / 2;
            if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
        }
        ctx.strokeStyle = color;
        ctx.lineWidth = lw;
        ctx.lineJoin = "round";
        ctx.stroke();
    }

    onPaint: {
        var ctx = getContext("2d");
        ctx.reset();
        if (chart.width < 4 || chart.height < 4) return;
        if (chart.gridLines > 1) {
            ctx.strokeStyle = chart.gridColor;
            ctx.lineWidth = 1;
            ctx.globalAlpha = 0.3;
            for (var g = 0; g < chart.gridLines; g++) {
                var gy = Math.round(g * (chart.height - 1) / (chart.gridLines - 1)) + 0.5;
                ctx.beginPath();
                ctx.moveTo(0, gy);
                ctx.lineTo(chart.width, gy);
                ctx.stroke();
            }
            ctx.globalAlpha = 1;
        }
        drawSeries(ctx, chart.secondary, chart.secondaryMin, chart.secondaryMax, chart.secondaryColor, Math.max(1, chart.lineWidth - 0.5));
        drawSeries(ctx, chart.primary, chart.primaryMin, chart.primaryMax, chart.primaryColor, chart.lineWidth);
    }
}
