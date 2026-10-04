.pragma library

// Entries of Tray.MenuItems shown in the widget's right-click menu, with their separator group.
var IDS = [12, 13, 14, 15, 16, 23, 24, 25, 19, 22];

function group(id) {
    return id >= 23 ? 1 : (id === 19 || id === 22 ? 2 : 0);
}

// [{id, text, enabled, checked}] or {separator: true}, from the MenuItems JSON.
function build(json) {
    var items = JSON.parse(json), out = [], last = -1;
    for (var i = 0; i < items.length; ++i) {
        var it = items[i];
        if (IDS.indexOf(it.id) < 0 || !it.visible) continue;
        var g = group(it.id);
        if (last >= 0 && g !== last) out.push({ separator: true });
        last = g;
        out.push({ id: it.id, text: it.label, enabled: !!it.enabled,
                   checked: it.checked === null || it.checked === undefined ? null : !!it.checked });
    }
    return out;
}

// True when the actions already hold these entries in the same order: update them in place.
function sameShape(cur, entries) {
    if (cur.length !== entries.length) return false;
    for (var i = 0; i < cur.length; ++i) {
        var a = cur[i], e = entries[i];
        if (!a || a.isSeparator !== !!e.separator || (!e.separator && a.itemId !== e.id)) return false;
    }
    return true;
}

// Sets owner[prop] = list and returns a COPY of the previous entries: a QML list property read is live.
function swap(owner, prop, list) {
    var cur = owner[prop], old = [];
    for (var i = 0; i < cur.length; ++i) old.push(cur[i]);
    owner[prop] = list;
    return old;
}

// The checkable entries are the Fn modes, one of many: radio items of one exclusive group.
function applyCheck(action, entry, fnGroup) {
    const choice = entry.checked !== null;
    action.checkable = choice;
    action.actionGroup = choice ? fnGroup : null;
    if (choice) action.checked = entry.checked;
}
