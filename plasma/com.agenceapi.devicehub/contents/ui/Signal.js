.pragma library

// Link quality from the BR/EDR RSSI: gap in dB to the ideal range, NOT dBm; 127 = unknown.

function rssiOf(radio) {
    if (!radio) return NaN;
    var raw = (radio.rssi_rel_db !== null && radio.rssi_rel_db !== undefined)
        ? radio.rssi_rel_db : radio.rssi_dbm;
    if (raw === null || raw === undefined) return NaN;
    var v = Number(raw);
    return (isNaN(v) || v === 127 || v < -127 || v > 20) ? NaN : v;
}

function qualityOf(v) {
    if (isNaN(v)) return "";
    return v >= 0 ? "excellent" : (v >= -5 ? "good" : "weak");
}

function rawRssi(v) {
    return v < 0 ? "−" + (-v) : (v > 0 ? "+" + v : "0");
}
