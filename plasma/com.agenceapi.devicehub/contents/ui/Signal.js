.pragma library

// Link quality from the BR/EDR RSSI (#174). The value is the gap in dB to the
// controller's ideal reception range: 0 = ideal, negative = below, positive =
// above (legal). It is NOT a power in dBm. 127 is the "unknown" sentinel.
// Mirrors akm-core/src/signal.rs.

// Relative RSSI from the `radio` object of the JSON snapshot; `rssi_dbm` is the
// former name of the same value. NaN = unknown.
function rssiOf(radio) {
    if (!radio) return NaN;
    var raw = (radio.rssi_rel_db !== null && radio.rssi_rel_db !== undefined)
        ? radio.rssi_rel_db : radio.rssi_dbm;
    if (raw === null || raw === undefined) return NaN;
    var v = Number(raw);
    return (isNaN(v) || v === 127 || v < -127 || v > 20) ? NaN : v;
}

// "excellent" (>= 0) | "good" (-1..-5) | "weak" (< -5) | "" (unknown)
function qualityOf(v) {
    if (isNaN(v)) return "";
    return v >= 0 ? "excellent" : (v >= -5 ? "good" : "weak");
}

// Raw value without unit: "0", "−3", "+2".
function rawRssi(v) {
    return v < 0 ? "−" + (-v) : (v > 0 ? "+" + v : "0");
}
