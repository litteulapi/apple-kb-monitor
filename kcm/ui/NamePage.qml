// SPDX-License-Identifier: GPL-2.0-or-later
// Name: alias on this computer only (BlueZ Alias via the daemon's SetAlias).
// Nothing here writes into the keyboard: the two buttons of the second group
// only open a terminal on `akmctl rename --device-name=… --check` or
// `--write-device-name` (#248), where akmctl shows the plan, makes the backup
// and demands the typed consent and the name again before its single write.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: root

    required property Store store

    readonly property var dev: store.state && store.state.keyboard && store.state.keyboard.device ? store.state.keyboard.device : ({})
    readonly property string mac: dev.mac ? String(dev.mac) : ""
    property bool busy: false

    spacing: Kirigami.Units.smallSpacing

    function validName(t) {
        // same rule as `akmctl rename`: 1-64 characters, no control character
        return t.trim().length > 0 && t.length <= 64 && !/[\u0000-\u001f\u007f]/.test(t);
    }

    // Same rules as akm_core::devname::validate (what `akmctl rename
    // --device-name` accepts): 1-32 printable ASCII characters, no leading or
    // trailing space, no backslash. Nothing is trimmed: what is validated is
    // exactly what would be written.
    readonly property var deviceNameRe: /^[\x21-\x5b\x5d-\x7e](?:[\x20-\x5b\x5d-\x7e]{0,30}[\x21-\x5b\x5d-\x7e])?$/
    function validDeviceName(t) {
        return root.deviceNameRe.test(t);
    }

    // Name for the copied commands, else a placeholder.
    function deviceName() {
        const t = deviceNameField.text;
        return root.validDeviceName(t) ? t : i18nc("placeholder in a command", "NAME");
    }

    // POSIX single quotes: the copied command never runs anything else.
    function shellQuote(t) {
        return "'" + String(t).replace(/'/g, "'\\''") + "'";
    }

    // Open a terminal on akmctl (C++: QProcess with an argument list, never a
    // shell; the name is one literal argument). The module writes nothing.
    function openTerminal(checkOnly) {
        const err = root.store.bridge.openDeviceNameTerminal(deviceNameField.text, checkOnly);
        if (err) {
            result.type = Kirigami.MessageType.Error;
            result.text = i18n("Terminal not opened: %1", err);
        } else {
            result.type = Kirigami.MessageType.Information;
            result.text = checkOnly
                ? i18n("A terminal was opened: akmctl runs the whole pre-flight there (one administrator authentication to read the Bluetooth channel size) and writes nothing.")
                : i18n("A terminal was opened: nothing is written until you type ECRIRE and the name again there.");
        }
        result.visible = true;
    }

    Kirigami.InlineMessage {
        id: result
        Layout.fillWidth: true
        Layout.margins: Kirigami.Units.smallSpacing
        visible: false
        showCloseButton: true
    }

    Kirigami.FormLayout {
        Layout.fillWidth: true

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Name on this computer")
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Shown as:")
            text: root.dev.alias || root.dev.name || "—"
            elide: Text.ElideRight
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Keyboard's own name:")
            visible: !!root.dev.name
            text: root.dev.name || ""
            elide: Text.ElideRight
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
        }
        QQC2.TextField {
            id: nameField
            Kirigami.FormData.label: i18n("New name:")
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 20
            maximumLength: 64
            placeholderText: i18n("e.g. Office keyboard")
            enabled: root.store.present && root.mac !== "" && !root.busy
            Accessible.name: i18n("New name of the keyboard on this computer")
            onAccepted: if (renameBtn.enabled) renameBtn.clicked()
        }
        RowLayout {
            QQC2.Button {
                id: renameBtn
                text: i18n("Rename")
                icon.name: "edit-rename"
                enabled: nameField.enabled && root.validName(nameField.text)
                onClicked: {
                    root.busy = true;
                    root.store.setAlias(root.mac, nameField.text.trim(), function (ok, msg) {
                        root.busy = false;
                        result.type = ok ? Kirigami.MessageType.Positive : Kirigami.MessageType.Error;
                        result.text = ok ? i18n("Renamed to “%1” on this computer.", msg) : i18n("Not renamed: %1", msg === "timeout" ? i18n("no answer in time") : msg);
                        result.visible = true;
                        if (ok) nameField.clear();
                    });
                }
            }
            QQC2.Button {
                text: i18n("Use the keyboard's own name")
                icon.name: "edit-undo"
                enabled: root.store.present && root.mac !== "" && !root.busy && !!root.dev.alias && root.dev.alias !== root.dev.name
                onClicked: {
                    root.busy = true;
                    root.store.setAlias(root.mac, "", function (ok, msg) {
                        root.busy = false;
                        result.type = ok ? Kirigami.MessageType.Positive : Kirigami.MessageType.Error;
                        result.text = ok ? i18n("The keyboard's own name is used again.") : i18n("Not renamed: %1", msg);
                        result.visible = true;
                    });
                }
            }
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 26
            wrapMode: Text.Wrap
            font: Kirigami.Theme.smallFont
            text: !root.store.present ? i18n("Renaming needs the keyboard service.")
                : (root.mac === "" ? i18n("No keyboard is known yet.")
                   : i18n("Only this computer sees this name (Bluetooth alias); the keyboard itself is not changed."))
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Name stored inside the keyboard")
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Stored now:")
            text: root.dev.name_on_keyboard || i18n("not read yet (the service reads it once per connection)")
            elide: Text.ElideRight
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
        }
        QQC2.TextField {
            id: deviceNameField
            Kirigami.FormData.label: i18n("Name to write:")
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 20
            maximumLength: 32
            placeholderText: i18n("1 to 32 ASCII characters")
            validator: RegularExpressionValidator { regularExpression: root.deviceNameRe }
            Accessible.name: i18n("Name to write into the keyboard")
            Accessible.description: i18n("1 to 32 printable ASCII characters, no leading or trailing space, no backslash; the write itself happens in a terminal")
        }
        RowLayout {
            QQC2.Button {
                id: checkBtn
                text: i18n("Check (nothing written)")
                icon.name: "dialog-ok-apply"
                enabled: root.validDeviceName(deviceNameField.text)
                Accessible.description: i18n("Opens a terminal on akmctl rename --device-name --check: whole pre-flight, backup and plan, nothing written")
                onClicked: root.openTerminal(true)
            }
            QQC2.Button {
                id: writeBtn
                text: i18n("Write the name into the keyboard…")
                icon.name: "document-edit-sign"
                enabled: root.validDeviceName(deviceNameField.text)
                Accessible.description: i18n("Opens a terminal on akmctl rename --device-name --write-device-name: the write happens there, only after you type ECRIRE and the name again")
                onClicked: root.openTerminal(false)
            }
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 26
            wrapMode: Text.Wrap
            font: Kirigami.Theme.smallFont
            text: deviceNameField.text.length > 0 && !root.validDeviceName(deviceNameField.text)
                ? i18n("Refused: 1 to 32 printable ASCII characters, no leading or trailing space, no backslash.")
                : i18n("This module writes nothing into the keyboard. Both buttons only open a terminal where akmctl shows the exact bytes, saves the current name, runs the pre-flight (one administrator authentication to read the Bluetooth channel size), then asks you to type ECRIRE and the name again before its single write.")
        }
    }

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        visible: true
        type: Kirigami.MessageType.Warning
        text: i18n("Writing into the keyboard's firmware carries two risks that are not measured: how the firmware answers the write (U5), and whether the name survives a battery change (U3). Have a second working keyboard at hand. The terminal saves the current name first and shows the exact rollback command; the keyboard must be switched off and on afterwards so the name is read back. Details: docs/RENOMMER-CLAVIER.md §5.3 and §6.")
    }

    Kirigami.Heading {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.topMargin: Kirigami.Units.largeSpacing
        level: 3
        text: i18nc("@title:group", "Same operations, to copy into a terminal")
    }

    Repeater {
        model: [
            { title: i18n("Name stored in the keyboard, preview (shows the bytes, writes nothing, no authentication):"),
              cmd: "akmctl rename --device-name " + root.shellQuote(root.deviceName()) + " --dry-run" },
            { title: i18n("Whole pre-flight, backup and plan (one administrator authentication, writes nothing):"),
              cmd: "akmctl rename --device-name " + root.shellQuote(root.deviceName()) + " --check" },
            { title: i18n("The write itself (pre-flight, backup, ECRIRE typed, name typed again, one frame, then switch the keyboard off and on; issue #248):"),
              cmd: "akmctl rename --device-name " + root.shellQuote(root.deviceName()) + " --write-device-name" },
            { title: i18n("Forget the keyboard cleanly, as macOS does, then pair it again (only after typing the confirmation word, issue #217):"),
              cmd: "akmctl repair --force" }
        ]
        delegate: ColumnLayout {
            id: cmdItem
            required property var modelData
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: cmdItem.modelData.title
            }
            RowLayout {
                Layout.fillWidth: true
                QQC2.TextField {
                    Layout.fillWidth: true
                    readOnly: true
                    text: cmdItem.modelData.cmd
                    font.family: "monospace"
                    Accessible.name: i18n("Command: %1", text)
                }
                QQC2.Button {
                    text: i18n("Copy")
                    icon.name: "edit-copy"
                    Accessible.name: i18n("Copy the command")
                    onClicked: {
                        root.store.bridge.copyText(cmdItem.modelData.cmd);
                        result.type = Kirigami.MessageType.Information;
                        result.text = i18n("Command copied.");
                        result.visible = true;
                    }
                }
            }
        }
    }
}
