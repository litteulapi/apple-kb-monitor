.pragma library

var PHOSPHOR = "#15ff00";    // main text, values, active element, glow
var GREEN_MID = "#0acc00";   // labels, secondary text, unknown value
var GREEN_FRAME = "#0a7a00"; // frames, rules, unlit blocks
var BG = "#021206";          // CRT background, ink of inverse video
var BG_PANEL = "#04180a";    // panels, centre of the tube gradient
var AMBER = "#ffb641";       // warning (low batteries, pending, daemon absent)
var RED = "#ff5a3c";         // error, alert

var SMALL = 18;
var BODY = 20;
var TITLE = 24;
var VALUE = 30;
var HERO = 92;
var ADVANCE = 0.4;

var GAP = 8;
var GUTTER = 12;
var TARGET = 30;   // smallest clickable height
var ROW = 24;
var SCANLINE_PITCH = 3;

var SEGMENT_WIDTH = 12;
var SEGMENT_GAP = 4;
var SEGMENT_SKEW = 5;
var SEGMENTS_MAX = 40;

// Text before and after %1 in a translated pattern such as "%1%", "%1 %" or "%%1".
function affixes(pattern) {
    var i = pattern.indexOf("%1");
    return i < 0 ? ["", ""] : [pattern.slice(0, i), pattern.slice(i + 2)];
}

function segmentCount(width) {
    var n = Math.floor((width - SEGMENT_SKEW + SEGMENT_GAP) / (SEGMENT_WIDTH + SEGMENT_GAP));
    return Math.min(SEGMENTS_MAX, Math.max(1, n));
}

function litSegments(frac, n) {
    if (!isFinite(frac) || frac <= 0 || n <= 0) return 0;
    if (frac >= 1) return n;
    return Math.min(Math.max(1, n - 1), Math.max(1, Math.round(frac * n)));
}

function levelColor(pct) {
    if (pct < 0 || !isFinite(pct)) return GREEN_MID;
    if (pct <= 10) return RED;
    if (pct <= 25) return AMBER;
    return PHOSPHOR;
}

function thresholdColor(level) {
    if (level === "ok") return PHOSPHOR;
    if (level === "low") return AMBER;
    if (level === "critical" || level === "empty") return RED;
    return GREEN_MID;
}

function qualityColor(q) {
    if (q === "excellent" || q === "good") return PHOSPHOR;
    if (q === "weak") return AMBER;
    return GREEN_MID;
}

function qualityBars(q) {
    return q === "excellent" ? 4 : q === "good" ? 3 : q === "weak" ? 1 : 0;
}

// Day and month of a locale's short date format, without the year (chart axis).
function dayMonthFormat(fmt) {
    var f = fmt.replace(/^[^dM]*y+[^dM]*|[^dM]*y+[^dM]*$/, "");
    return /y/.test(f) ? f.replace(/y+[^dM]*/, "") : f;
}

function maskMac(mac) {
    var p = String(mac || "").split(":");
    if (p.length !== 6 || !p.every(function (b) { return /^[0-9A-Fa-f]{2}$/.test(b); }))
        return "--:--:--:--:--:--";
    return (p[0] + ":" + p[1] + ":XX:XX:XX:" + p[5]).toUpperCase();
}

function glyphs(s) {
    return String(s).replace(/ /g, " ").replace(/→/g, ">").replace(/↔/g, "<>");
}

function daysLeft(forecast, now) {
    if (!forecast || !isFinite(Number(forecast.empty_at)) || Number(forecast.empty_at) <= 0) return -1;
    return Math.max(0, (Number(forecast.empty_at) - now) / 86400);
}

var SCALE_MIN = -20;
var SCALE_MAX = 5;
function scaleX(db) {
    if (!isFinite(db)) return -1;
    return Math.min(1, Math.max(0, (db - SCALE_MIN) / (SCALE_MAX - SCALE_MIN)));
}

// Locale-aware fixed-point number.
function num(v, digits) {
    return Number(v).toLocaleString(Qt.locale(), "f", digits);
}
