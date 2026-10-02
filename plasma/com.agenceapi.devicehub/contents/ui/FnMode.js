.pragma library

// `hid_apple.fnmode` as the daemon reports it (`Device.FnMode`): 1 = media
// keys first (kernel default; 3 = auto behaves the same on an Apple
// keyboard), 2 = F1–F12 first, 0 = Fn disabled, 4 = F-keys disabled; negative
// = module not loaded. Mirrors apihub-app/src/fn_toggle.rs.

// The mode the toggle button switches to; -1 = nothing can be written.
function next(mode) {
    if (mode === 2) return 1;
    if (mode === 1 || mode === 3) return 2;
    if (mode === 0 || mode === 4) return 1;
    return -1;
}

// "media" | "fkeys" | "off" | "nofkeys" | "" (unknown)
function kind(mode) {
    if (mode === 1 || mode === 3) return "media";
    if (mode === 2) return "fkeys";
    if (mode === 0) return "off";
    if (mode === 4) return "nofkeys";
    return "";
}

// Object path of a keyboard on the daemon ("" when `mac` is not an address).
function devicePath(base, mac) {
    if (!/^[0-9A-Fa-f]{2}(:[0-9A-Fa-f]{2}){5}$/.test(mac)) return "";
    return base + "/devices/" + mac.toUpperCase().replace(/:/g, "_");
}
