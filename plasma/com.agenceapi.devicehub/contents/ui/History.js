.pragma library

// Battery history of the daemon (`History(t since) -> s`, a JSON array of
// {ts, pct, voltage?, voltage_valid?}) turned into bounded chart series
// (#97 sparkline, #120 History page). Pure functions, no D-Bus, no clock:
// tested by plasma/tests/tst_history.qml. Mirrors apihub-app/src/view.rs
// (chart_points, downsample).

// Most points a chart of the widget ever draws, per series.
var POINTS_MAX = 240;
// Clock skew tolerated after `now` before a point counts as future.
var CLOCK_SLACK_S = 600;

// Reduce a time-sorted series of [t, y] to at most `max` points: first and
// last points, lowest and highest value of each slice in between (a dip one
// reading wide stays visible). Unchanged when it already fits.
function downsample(pts, max) {
    if (pts.length <= max || max < 4) return pts;
    var inner = pts.length - 2;
    var buckets = Math.floor((max - 2) / 2);
    var out = [pts[0]];
    for (var b = 0; b < buckets; b++) {
        var from = 1 + Math.floor(b * inner / buckets);
        var to = 1 + Math.floor((b + 1) * inner / buckets);
        if (to <= from) continue;
        var lo = from, hi = from;
        for (var i = from; i < to; i++) {
            if (pts[i][1] < pts[lo][1]) lo = i;
            if (pts[i][1] > pts[hi][1]) hi = i;
        }
        out.push(pts[Math.min(lo, hi)]);
        if (lo !== hi) out.push(pts[Math.max(lo, hi)]);
    }
    out.push(pts[pts.length - 1]);
    return out;
}

function finite(v) {
    return typeof v === "number" && isFinite(v);
}

// Series of the readings between `since` and `now` (Unix seconds).
// Returns {pct, volt: arrays of [t, y] sorted by time, at most `max` points
// each; count: readings in the period; tMin, tMax; pctMin, pctMax;
// voltMin, voltMax (NaN without a measured voltage)}.
// Invalid entries are dropped, never drawn: non-finite numbers, a percentage
// outside 0..100, a voltage that is not a measurement (voltage_valid false)
// or outside 0..10 V, a date in the future.
function series(entries, since, now, max) {
    var pct = [], volt = [];
    if (entries && typeof entries.length === "number") {
        for (var i = 0; i < entries.length; i++) {
            var e = entries[i];
            if (!e || !finite(e.ts) || !finite(e.pct)) continue;
            if (e.ts < since || e.ts > now + CLOCK_SLACK_S) continue;
            if (e.pct < 0 || e.pct > 100) continue;
            pct.push([e.ts, e.pct]);
            if (finite(e.voltage) && e.voltage_valid !== false && e.voltage > 0 && e.voltage < 10)
                volt.push([e.ts, e.voltage]);
        }
    }
    var byTime = function (a, b) { return a[0] - b[0]; };
    pct.sort(byTime);
    volt.sort(byTime);
    var range = function (pts) {
        var lo = NaN, hi = NaN;
        for (var k = 0; k < pts.length; k++) {
            if (isNaN(lo) || pts[k][1] < lo) lo = pts[k][1];
            if (isNaN(hi) || pts[k][1] > hi) hi = pts[k][1];
        }
        return [lo, hi];
    };
    var pr = range(pct), vr = range(volt);
    return {
        count: pct.length,
        tMin: pct.length > 0 ? pct[0][0] : since,
        tMax: pct.length > 0 ? pct[pct.length - 1][0] : now,
        pctMin: pr[0], pctMax: pr[1],
        voltMin: vr[0], voltMax: vr[1],
        pct: downsample(pct, max),
        volt: downsample(volt, max)
    };
}

// Same, from the JSON text of the daemon; a reply that is not a JSON array
// gives empty series (never an exception in the applet).
function parse(json, since, now, max) {
    var entries = [];
    try {
        entries = JSON.parse(json);
    } catch (e) {
        entries = [];
    }
    return series(entries, since, now, max);
}

// Whole days covered by the series (at least 1 when there are two readings).
function spanDays(s) {
    if (s.count < 2) return 0;
    return Math.max(1, Math.round((s.tMax - s.tMin) / 86400));
}
