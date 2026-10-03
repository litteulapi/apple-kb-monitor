.pragma library

// Pip-Boy art direction of the popup: the palette, grid and pure helpers of
// the ApiHub window (apihub-app/src/theme.rs, docs/DA-PIPBOY.md), so the
// popup and the window are the same device. No other colour is used.

var PHOSPHOR = "#15ff00";    // main text, values, active element, glow
var GREEN_MID = "#0acc00";   // labels, secondary text, unknown value
var GREEN_DIM = "#0a9a00";   // decoration only (never a text to read)
var GREEN_FRAME = "#0a7a00"; // frames, rules, unlit blocks
var BG = "#021206";          // CRT background, ink of inverse video
var BG_PANEL = "#04180a";    // panels, centre of the tube gradient
var AMBER = "#ffb641";       // warning (low batteries, pending, daemon absent)
var RED = "#ff5a3c";         // error, alert

// VT323 sizes (px): it is a small face, one letter advances 0.4 x the size.
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

// Gauge blocks (theme.rs SEGMENT_*): width, gap and slant of a block.
var SEGMENT_WIDTH = 12;
var SEGMENT_GAP = 4;
var SEGMENT_SKEW = 5;
var SEGMENTS_MAX = 40;

// Blocks of a gauge that fit in `width`.
function segmentCount(width) {
    var n = Math.floor((width - SEGMENT_SKEW + SEGMENT_GAP) / (SEGMENT_WIDTH + SEGMENT_GAP));
    return Math.min(SEGMENTS_MAX, Math.max(1, n));
}

// Lit blocks of a gauge of `n` blocks: none for 0 or unknown, at least one
// as soon as there is something, all only when full (theme.rs lit_segments).
function litSegments(frac, n) {
    if (!isFinite(frac) || frac <= 0 || n <= 0) return 0;
    if (frac >= 1) return n;
    return Math.min(Math.max(1, n - 1), Math.max(1, Math.round(frac * n)));
}

// Colour of a battery level: phosphor, amber at 25 % and below, red at 10 %.
function levelColor(pct) {
    if (pct < 0 || !isFinite(pct)) return GREEN_MID;
    if (pct <= 10) return RED;
    if (pct <= 25) return AMBER;
    return PHOSPHOR;
}

// Colour of the keyboard's own threshold level (ok / low / critical / empty).
function thresholdColor(level) {
    if (level === "ok") return PHOSPHOR;
    if (level === "low") return AMBER;
    if (level === "critical" || level === "empty") return RED;
    return GREEN_MID;
}

// Colour of a signal quality word.
function qualityColor(q) {
    if (q === "excellent" || q === "good") return PHOSPHOR;
    if (q === "weak") return AMBER;
    return GREEN_MID;
}

// Lit bars (0..4) of the four signal bars for a quality word.
function qualityBars(q) {
    return q === "excellent" ? 4 : q === "good" ? 3 : q === "weak" ? 1 : 0;
}

// Address shown in the status bar: first two and last bytes only.
function maskMac(mac) {
    var p = String(mac || "").split(":");
    if (p.length !== 6) return "--:--:--:--:--:--";
    return p[0] + ":" + p[1] + ":XX:XX:XX:" + p[5];
}

// Glyphs VT323 does not have (DA-PIPBOY §3): thin no-break space, arrows.
function glyphs(s) {
    return String(s).replace(/ /g, " ").replace(/→/g, ">").replace(/↔/g, "<>");
}

// Days left at `now` (Unix s) from a daemon forecast {empty_at}; -1 = none.
function daysLeft(forecast, now) {
    if (!forecast || !isFinite(Number(forecast.empty_at)) || Number(forecast.empty_at) <= 0) return -1;
    return Math.max(0, (Number(forecast.empty_at) - now) / 86400);
}

// Position (0..1) of a relative signal in dB on the RADIO scale -20..+5.
var SCALE_MIN = -20;
var SCALE_MAX = 5;
function scaleX(db) {
    if (!isFinite(db)) return -1;
    return Math.min(1, Math.max(0, (db - SCALE_MIN) / (SCALE_MAX - SCALE_MIN)));
}
