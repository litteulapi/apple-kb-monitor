// SPDX-License-Identifier: GPL-2.0-or-later
// Name: alias on this computer only (BlueZ Alias via the daemon's SetAlias).
// Nothing here writes into the keyboard.
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

    // Name for the device-name commands: printable ASCII, 1-32 characters
    // (what akmctl accepts), else a placeholder.
    function deviceName() {
        const t = nameField.text.trim();
        return /^[\x20-\x7e]{1,32}$/.test(t) ? t : i18nc("placeholder in a command", "NAME");
    }

    // POSIX single quotes: the copied command never runs anything else.
    function shellQuote(t) {
        return "'" + String(t).replace(/'/g, "'\\''") + "'";
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
    }

    Kirigami.Heading {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.topMargin: Kirigami.Units.largeSpacing
        level: 3
        text: i18nc("@title:group", "Inside the keyboard: terminal only")
    }
    QQC2.Label {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        wrapMode: Text.Wrap
        text: i18n("This module never writes into the keyboard. Writing the name stored in the keyboard, or making it forget its pairings, changes the keyboard itself: these operations stay in a terminal, where they show what they will send and ask you to type a confirmation.")
    }

    QQC2.Label {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        wrapMode: Text.Wrap
        text: root.dev.name_on_keyboard
            ? i18n("Name stored in the keyboard now: %1", root.dev.name_on_keyboard)
            : i18n("Name stored in the keyboard: not read yet (the service reads it once per connection).")
    }

    Repeater {
        model: [
            { title: i18n("Name stored in the keyboard, preview (shows the bytes, writes nothing):"),
              cmd: "akmctl rename --device-name " + root.shellQuote(root.deviceName()) + " --dry-run" },
            { title: i18n("Then the write itself (pre-flight checks, backup, name typed again; refused while the sequence is not proven, issue #248):"),
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
