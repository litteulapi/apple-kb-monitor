// SPDX-License-Identifier: GPL-2.0-or-later
// Keys: effective table F1..F12/Eject for the current Fn mode, hid_apple
// parameters (polkit), manual mapping (keymap.toml -> udev hwdb, polkit),
// KDE shortcut for F4.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: root

    required property Store store

    readonly property var table: store.keyTable
    readonly property var params: table && table.params ? table.params : ({})
    readonly property var keymap: store.keymap
    readonly property var profile: keymap && keymap.profiles && keymap.active ? keymap.profiles[keymap.active] : null
    readonly property var actions: table && table.kde_actions ? table.kde_actions : null
    property bool busy: false

    spacing: Kirigami.Units.smallSpacing

    function report(ok, okText, msg) {
        result.type = ok ? Kirigami.MessageType.Positive : Kirigami.MessageType.Error;
        if (ok) {
            result.text = msg ? i18nc("result, then the detail", "%1 — %2", okText, msg) : okText;
        } else if (msg === "timeout") {
            result.text = i18n("No answer in time: nothing was confirmed. Check the state before trying again.");
        } else {
            result.text = i18n("Failed: %1", msg || i18n("unknown error"));
        }
        result.visible = true;
    }

    function fnModeName(m) {
        switch (m) {
        case 0: return i18nc("fnmode 0", "0 — Fn key disabled");
        case 1: return i18nc("fnmode 1", "1 — special functions first (Apple legend), F1-F12 with fn");
        case 2: return i18nc("fnmode 2", "2 — F1-F12 first, special functions with fn");
        case 3: return i18nc("fnmode 3", "3 — automatic (same as 1 on this keyboard)");
        case 4: return i18nc("fnmode 4", "4 — F1-F12 only, special functions disabled");
        default: return i18n("unknown");
        }
    }

    // "KEY_VOLUMEUP · Volume Up" then, on a second line, "KDE: Increase Volume"
    function sideText(side) {
        if (!side || !side.code) return "—";
        let t = side.code;
        if (side.qt_name) t += " · " + side.qt_name;
        if (root.actions !== null && side.qt_key !== undefined && side.qt_key !== null) {
            const a = root.actions[String(side.qt_key)];
            t += "\n" + (a ? i18nc("KDE shortcut action", "KDE: %1", a.action_name || a.action)
                           : i18n("no KDE shortcut"));
        }
        return t;
    }

    Kirigami.InlineMessage {
        id: result
        Layout.fillWidth: true
        Layout.margins: Kirigami.Units.smallSpacing
        visible: false
        showCloseButton: true
    }

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        Layout.margins: Kirigami.Units.smallSpacing
        visible: root.store.keyTableError !== "" && root.table === null
        type: Kirigami.MessageType.Error
        text: i18n("The key table could not be read: %1", root.store.keyTableError)
        actions: [
            Kirigami.Action {
                text: i18n("Try again")
                icon.name: "view-refresh"
                onTriggered: { root.store.fetchKeys(); root.store.fetchKeymap(); }
            }
        ]
    }

    // ── effective table ───────────────────────────────────────────────
    Kirigami.Heading {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.topMargin: Kirigami.Units.largeSpacing
        level: 3
        text: i18nc("@title:group", "What each key does now")
    }
    QQC2.Label {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        wrapMode: Text.Wrap
        text: root.table
            ? i18n("Fn mode %1. Read from the kernel and the installed mapping; no key press is read.", root.fnModeName(root.params.fnmode))
            : (root.store.keyTableBusy ? i18n("Reading the key table…") : "")
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        visible: root.table !== null
        spacing: Kirigami.Units.largeSpacing
        Accessible.ignored: true
        QQC2.Label { text: i18nc("@title:column", "Key"); font.bold: true; Layout.fillWidth: true; Layout.preferredWidth: 1; Layout.horizontalStretchFactor: 2 }
        QQC2.Label { text: i18nc("@title:column", "Pressed alone"); font.bold: true; Layout.fillWidth: true; Layout.preferredWidth: 1; Layout.horizontalStretchFactor: 4 }
        QQC2.Label { text: i18nc("@title:column", "With fn"); font.bold: true; Layout.fillWidth: true; Layout.preferredWidth: 1; Layout.horizontalStretchFactor: 4 }
    }

    // One row per key; each row is one accessible element.
    Repeater {
        model: root.table && root.table.rows ? root.table.rows : []
        delegate: RowLayout {
            id: row
            required property var modelData
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.largeSpacing
            Accessible.role: Accessible.Row
            Accessible.name: i18nc("key, legend, alone, with fn", "%1 (%2): alone %3; with fn %4",
                                   row.modelData.key, row.modelData.legend, root.sideText(row.modelData.plain), root.sideText(row.modelData.fn))
            QQC2.Label {
                // columns 2:4:4 of the width, whatever the scale
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.horizontalStretchFactor: 2
                wrapMode: Text.Wrap
                text: row.modelData.key + (row.modelData.remapped ? " *" : "") + "\n" + row.modelData.legend
                font.bold: true
            }
            QQC2.Label {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.horizontalStretchFactor: 4
                text: root.sideText(row.modelData.plain)
                wrapMode: Text.Wrap
            }
            QQC2.Label {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                Layout.horizontalStretchFactor: 4
                text: root.sideText(row.modelData.fn)
                wrapMode: Text.Wrap
            }
        }
    }
    QQC2.Label {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        visible: root.table !== null
        wrapMode: Text.Wrap
        font: Kirigami.Theme.smallFont
        text: i18n("* = changed by the installed manual mapping.") + (root.table && root.table.pending ? " " + i18n("The active profile differs from what is installed: use “Install the mapping”.") : "")
    }

    // ── Fn mode and hid_apple parameters ──────────────────────────────
    Kirigami.FormLayout {
        Layout.fillWidth: true

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Fn key and modifier keys")
        }
        Kirigami.InlineMessage {
            Kirigami.FormData.isSection: true
            Layout.fillWidth: true
            visible: true
            type: Kirigami.MessageType.Warning
            text: i18n("These are parameters of the kernel driver hid_apple: they apply to ALL Apple keyboards of this computer, are kept after a restart, and each change asks for the administrator password.")
        }
        QQC2.ComboBox {
            id: fnmode
            Kirigami.FormData.label: i18n("Fn mode:")
            Layout.fillWidth: true
            model: [0, 1, 2, 3, 4].map(function (m) { return root.fnModeName(m); })
            currentIndex: typeof root.params.fnmode === "number" ? root.params.fnmode : -1
            Accessible.name: i18n("Fn mode")
        }
        QQC2.ComboBox {
            id: optcmd
            Kirigami.FormData.label: i18n("Option and Command:")
            Layout.fillWidth: true
            model: [i18n("Mac order (Option = Alt, Command = Meta)"), i18n("swapped on both sides (PC order)"), i18n("swapped on the left side only")]
            currentIndex: typeof root.params.swap_opt_cmd === "number" ? root.params.swap_opt_cmd : -1
            Accessible.name: i18n("Option and Command keys")
        }
        QQC2.CheckBox {
            id: ctrlcmd
            Kirigami.FormData.label: i18n("Control and Command:")
            text: i18n("Swap Control and Command")
            checked: root.params.swap_ctrl_cmd === 1
        }
        QQC2.CheckBox {
            id: fnctrl
            Kirigami.FormData.label: i18n("fn and left Control:")
            text: i18n("Swap fn and the left Control key")
            checked: root.params.swap_fn_leftctrl === 1
        }
        QQC2.ComboBox {
            id: iso
            Kirigami.FormData.label: i18n("ISO keys ` and <:")
            Layout.fillWidth: true
            model: [i18n("automatic"), i18n("as engraved"), i18n("swapped")]
            currentIndex: typeof root.params.iso_layout === "number" ? root.params.iso_layout + 1 : -1
            Accessible.name: i18n("ISO keys")
        }
        RowLayout {
            QQC2.Button {
                id: applyParams
                text: i18n("Apply these parameters…")
                icon.name: "dialog-password"
                readonly property var changes: {
                    const c = [];
                    const p = root.params;
                    if (fnmode.currentIndex >= 0 && fnmode.currentIndex !== p.fnmode) c.push(["fnmode", fnmode.currentIndex]);
                    if (optcmd.currentIndex >= 0 && optcmd.currentIndex !== p.swap_opt_cmd) c.push(["swap_opt_cmd", optcmd.currentIndex]);
                    if ((ctrlcmd.checked ? 1 : 0) !== p.swap_ctrl_cmd && p.swap_ctrl_cmd !== undefined) c.push(["swap_ctrl_cmd", ctrlcmd.checked ? 1 : 0]);
                    if ((fnctrl.checked ? 1 : 0) !== p.swap_fn_leftctrl && p.swap_fn_leftctrl !== undefined) c.push(["swap_fn_leftctrl", fnctrl.checked ? 1 : 0]);
                    if (iso.currentIndex >= 0 && iso.currentIndex - 1 !== p.iso_layout) c.push(["iso_layout", iso.currentIndex - 1]);
                    return c;
                }
                enabled: !root.busy && changes.length > 0
                Accessible.description: i18n("Asks for the administrator password once per changed parameter")
                onClicked: {
                    const todo = changes.slice();
                    root.busy = true;
                    const next = function () {
                        if (todo.length === 0) {
                            root.busy = false;
                            root.report(true, i18n("Parameters applied to every Apple keyboard."), "");
                            return;
                        }
                        const c = todo.shift();
                        root.store.setParam(c[0], c[1], function (ok, msg) {
                            if (!ok) {
                                root.busy = false;
                                root.report(false, "", c[0] + ": " + msg);
                                return;
                            }
                            next();
                        });
                    };
                    next();
                }
            }
            QQC2.Button {
                text: i18n("Undo changes")
                icon.name: "edit-undo"
                enabled: applyParams.changes.length > 0 && !root.busy
                onClicked: {
                    const p = root.params;
                    fnmode.currentIndex = typeof p.fnmode === "number" ? p.fnmode : -1;
                    optcmd.currentIndex = typeof p.swap_opt_cmd === "number" ? p.swap_opt_cmd : -1;
                    ctrlcmd.checked = p.swap_ctrl_cmd === 1;
                    fnctrl.checked = p.swap_fn_leftctrl === 1;
                    iso.currentIndex = typeof p.iso_layout === "number" ? p.iso_layout + 1 : -1;
                }
            }
            QQC2.BusyIndicator {
                visible: root.busy
                running: visible
                implicitHeight: Kirigami.Units.iconSizes.medium
                implicitWidth: implicitHeight
                Accessible.name: i18n("Waiting for the authentication")
            }
        }

        // ── manual mapping ──
        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Manual key mapping")
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Profile:")
            text: root.keymap ? root.keymap.active : "—"
        }
        RowLayout {
            Kirigami.FormData.label: i18n("Preset:")
            QQC2.ComboBox {
                id: preset
                readonly property var names: ["none"].concat(root.keymap && root.keymap.presets ? root.keymap.presets.map(function (p) { return p.name; }) : [])
                model: [i18n("none (parameters left as they are)")].concat(root.keymap && root.keymap.presets ? root.keymap.presets.map(function (p) { return p.title; }) : [])
                currentIndex: Math.max(0, names.indexOf(root.profile && root.profile.preset ? root.profile.preset : "none"))
                Accessible.name: i18n("Preset of the profile")
            }
            QQC2.Button {
                text: i18n("Use this preset")
                enabled: !root.busy && root.keymap !== null && preset.names[preset.currentIndex] !== (root.profile && root.profile.preset ? root.profile.preset : "none")
                onClicked: {
                    root.busy = true;
                    root.store.setPreset(preset.names[preset.currentIndex], function (ok, msg) {
                        root.busy = false;
                        root.report(ok, i18n("Preset saved in the profile; use “Install the mapping” to apply it."), ok ? "" : msg);
                    });
                }
            }
        }
        ColumnLayout {
            Kirigami.FormData.label: i18n("Remapped keys:")
            Layout.fillWidth: true
            QQC2.Label {
                visible: !root.profile || Object.keys(root.profile.keys || {}).length === 0
                text: i18n("none")
            }
            Repeater {
                model: root.profile ? Object.keys(root.profile.keys || {}) : []
                delegate: RowLayout {
                    id: mapRow
                    required property string modelData
                    QQC2.Label {
                        text: i18nc("key -> code", "%1 → %2", mapRow.modelData, root.profile.keys[mapRow.modelData])
                        font.family: "monospace"
                    }
                    QQC2.ToolButton {
                        icon.name: "edit-delete-remove"
                        text: i18n("Remove")
                        display: QQC2.AbstractButton.IconOnly
                        enabled: !root.busy
                        Accessible.name: i18n("Remove the mapping of %1", mapRow.modelData)
                        QQC2.ToolTip.text: Accessible.name
                        QQC2.ToolTip.visible: hovered || activeFocus
                        onClicked: {
                            root.busy = true;
                            root.store.setKey(mapRow.modelData, "", function (ok, msg) {
                                root.busy = false;
                                root.report(ok, i18n("Mapping removed from the profile."), ok ? "" : msg);
                            });
                        }
                    }
                }
            }
        }
        RowLayout {
            Kirigami.FormData.label: i18n("Remap:")
            QQC2.ComboBox {
                id: keyPick
                readonly property var ids: root.keymap && root.keymap.keys ? root.keymap.keys.map(function (k) { return k.id; }) : []
                model: root.keymap && root.keymap.keys ? root.keymap.keys.map(function (k) { return k.id + " — " + k.legend; }) : []
                Accessible.name: i18n("Key to remap")
            }
            QQC2.TextField {
                id: codeField
                placeholderText: "KEY_DELETE"
                validator: RegularExpressionValidator { regularExpression: /KEY_[A-Z0-9_]{1,30}/ }
                Accessible.name: i18n("Linux key code (KEY_…)")
                onAccepted: addMap.clicked()
            }
            QQC2.Button {
                id: addMap
                text: i18n("Add")
                icon.name: "list-add"
                enabled: !root.busy && codeField.acceptableInput && keyPick.currentIndex >= 0
                onClicked: {
                    root.busy = true;
                    root.store.setKey(keyPick.ids[keyPick.currentIndex], codeField.text, function (ok, msg) {
                        root.busy = false;
                        if (ok) codeField.clear();
                        root.report(ok, i18n("Mapping saved in the profile; use “Install the mapping” to apply it."), ok ? "" : msg);
                    });
                }
            }
        }
        RowLayout {
            QQC2.Button {
                text: i18n("Install the mapping…")
                icon.name: "dialog-password"
                enabled: !root.busy
                Accessible.description: i18n("Installs the active profile (udev hwdb and its parameters); asks for the administrator password")
                onClicked: {
                    root.busy = true;
                    root.store.applyKeymap(function (ok, msg) {
                        root.busy = false;
                        root.report(ok, i18n("Mapping installed."), msg);
                    });
                }
            }
            QQC2.Button {
                text: i18n("Back to the kernel mapping…")
                icon.name: "edit-reset"
                enabled: !root.busy
                onClicked: {
                    root.busy = true;
                    root.store.resetKeymap(function (ok, msg) {
                        root.busy = false;
                        root.report(ok, i18n("Kernel mapping restored."), msg);
                    });
                }
            }
        }

        // ── KDE ──
        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "KDE shortcuts")
        }
        RowLayout {
            Kirigami.FormData.label: i18n("F4 (Launchpad):")
            QQC2.Button {
                text: i18n("Bind F4 to the application launcher")
                icon.name: "start-here-kde"
                enabled: !root.busy
                Accessible.description: i18n("Adds the missing KDE shortcut for the Launch (D) key, never replacing an existing one")
                onClicked: {
                    root.busy = true;
                    root.store.kdeApply(function (ok, msg) {
                        root.busy = false;
                        root.report(ok, i18n("KDE shortcuts updated."), msg);
                    });
                }
            }
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 26
            wrapMode: Text.Wrap
            font: Kirigami.Theme.smallFont
            text: i18n("Undo from a terminal: akmctl keymap kde-apply --undo")
        }
    }
}
