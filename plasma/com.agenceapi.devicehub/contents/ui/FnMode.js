.pragma library

function next(mode) {
    if (mode === 2) return 1;
    if (mode === 1 || mode === 3) return 2;
    if (mode === 0 || mode === 4) return 1;
    return -1;
}

function kind(mode) {
    if (mode === 1 || mode === 3) return "media";
    if (mode === 2) return "fkeys";
    if (mode === 0) return "off";
    if (mode === 4) return "nofkeys";
    return "";
}

function devicePath(base, mac) {
    if (!/^[0-9A-Fa-f]{2}(:[0-9A-Fa-f]{2}){5}$/.test(mac)) return "";
    return base + "/devices/" + mac.toUpperCase().replace(/:/g, "_");
}
