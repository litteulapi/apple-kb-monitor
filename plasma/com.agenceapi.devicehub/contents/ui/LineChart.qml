import QtQuick

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
    property int gridLines: 0
    property real lineWidth: 1.5
    property int vGridLines: 0

    onPrimaryChanged: requestPaint()
    onSecondaryChanged: requestPaint()
    onTMinChanged: requestPaint()
    onTMaxChanged: requestPaint()
    onPrimaryColorChanged: requestPaint()
    onSecondaryColorChanged: requestPaint()
    onGridColorChanged: requestPaint()
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()

    function drawSeries(ctx, pts, lo, hi, color, lw, mode) {
        if (!pts || pts.length < 2) return;
        var span = Math.max(1, chart.tMax - chart.tMin);
        var range = hi - lo;
        var x0 = 0, x1 = 0;
        ctx.beginPath();
        for (var i = 0; i < pts.length; i++) {
            var x = Math.min(1, Math.max(0, (pts[i][0] - chart.tMin) / span)) * (chart.width - lw) + lw / 2;
            var f = range > 0 ? Math.min(1, Math.max(0, (pts[i][1] - lo) / range)) : 0.5;
            var y = (1 - f) * (chart.height - lw) + lw / 2;
            if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
            if (i === 0) x0 = x;
            x1 = x;
        }
        ctx.lineJoin = "round";
        if (mode === "dim") {
            ctx.globalAlpha = 0.65;
        }
        if (mode === "glow") {
            ctx.strokeStyle = color;
            ctx.globalAlpha = 0.18;
            ctx.lineWidth = lw * 5;
            ctx.stroke();
            ctx.globalAlpha = 0.35;
            ctx.lineWidth = lw * 2.5;
            ctx.stroke();
            ctx.globalAlpha = 1;
        }
        ctx.strokeStyle = color;
        ctx.lineWidth = lw;
        ctx.stroke();
        ctx.globalAlpha = 1;
        if (mode === "glow") {
            ctx.lineTo(x1, chart.height);
            ctx.lineTo(x0, chart.height);
            ctx.closePath();
            var grad = ctx.createLinearGradient(0, 0, 0, chart.height);
            grad.addColorStop(0, Qt.rgba(chart.primaryColor.r, chart.primaryColor.g, chart.primaryColor.b, 0.22));
            grad.addColorStop(1, Qt.rgba(chart.primaryColor.r, chart.primaryColor.g, chart.primaryColor.b, 0.02));
            ctx.fillStyle = grad;
            ctx.fill();
        }
    }

    onPaint: {
        var ctx = getContext("2d");
        ctx.reset();
        if (chart.width < 4 || chart.height < 4) return;
        if (chart.gridLines > 1) {
            ctx.strokeStyle = chart.gridColor;
            ctx.lineWidth = 1;
            ctx.globalAlpha = 0.6;
            for (var g = 0; g < chart.gridLines; g++) {
                var gy = Math.round(g * (chart.height - 1) / (chart.gridLines - 1)) + 0.5;
                ctx.beginPath();
                ctx.moveTo(0, gy);
                ctx.lineTo(chart.width, gy);
                ctx.stroke();
            }
            ctx.globalAlpha = 1;
        }
        if (chart.vGridLines > 1) {
            ctx.strokeStyle = chart.gridColor;
            ctx.lineWidth = 1;
            ctx.globalAlpha = 0.6;
            for (var v = 0; v < chart.vGridLines; v++) {
                var gx = Math.round(v * (chart.width - 1) / (chart.vGridLines - 1)) + 0.5;
                ctx.beginPath();
                ctx.moveTo(gx, 0);
                ctx.lineTo(gx, chart.height);
                ctx.stroke();
            }
            ctx.globalAlpha = 1;
        }
        drawSeries(ctx, chart.secondary, chart.secondaryMin, chart.secondaryMax, chart.secondaryColor,
                   Math.max(1, chart.lineWidth - 0.5), "dim");
        drawSeries(ctx, chart.primary, chart.primaryMin, chart.primaryMax, chart.primaryColor, chart.lineWidth, "glow");
    }
}
