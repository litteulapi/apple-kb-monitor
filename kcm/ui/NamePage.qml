// SPDX-License-Identifier: GPL-2.0-or-later
// Name: the alias on this computer (BlueZ Alias via the daemon's SetAlias),
// and the name stored inside the keyboard (#248). For the latter the page asks
// one confirmation, then runs `akmctl rename --device-name=<name> --yes`
// through the bridge (QProcess, argument list, never a shell, bounded delay)
// and shows akmctl's verdict here; "Check" runs the same command with
// `--check` (whole pre-flight, nothing written).
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
    // A device-name command (check or write) is running.
    property bool deviceBusy: false

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

    // #290: a command holding the name is shown (and can be copied) only
    // once a valid name is typed: never a stand-in such as "NAME", which
    // pasted and confirmed would be written into the keyboard.
    function nameCommand(flag) {
        const t = deviceNameField.text;
        return root.validDeviceName(t) ? root.store.deviceNameCommand(t, flag) : "";
    }

    // Run akmctl on the name stored in the keyboard (C++: QProcess with an
    // argument list, never a shell; the name is one literal argument) and show
    // its verdict in the page. Exit codes of akmctl: 0 done, 11 pre-flight
    // refused, 13 written but not read back, 14 read back different, 15 the
    // write failed on its way (a frame may have been sent: never "not
    // written"); akmctl killed or crashed meanwhile is the same (#288).
    // Their meaning is decided in C++ (Store.deviceNameVerdict).
    function runDeviceName(checkOnly) {
        const name = deviceNameField.text;
        root.deviceBusy = true;
        deviceResult.visible = false;
        root.store.deviceName(name, checkOnly, function (code, out, err, timedOut, crashed) {
            root.deviceBusy = false;
            let detail = (String(out || "").trim() || String(err || "").trim());
            const verdict = root.store.deviceNameVerdict(code, checkOnly, timedOut, !!crashed);
            // #288: akmctl ended abruptly (signal, crash, panic): say so
            if (crashed || code === 101)
                detail = i18n("akmctl stopped abruptly before giving its verdict (crash or signal).") + (detail !== "" ? "\n" + detail : "");
            if (verdict === "timeout") {
                deviceResult.type = Kirigami.MessageType.Error;
                deviceResult.text = i18n("No answer in time. Nothing more is attempted; look at the stored name above in a moment.");
            } else if (verdict === "check-ok") {
                deviceResult.type = Kirigami.MessageType.Positive;
                deviceResult.text = i18n("Check passed: “%1” can be written. Nothing was written.", name);
            } else if (verdict === "written") {
                deviceResult.type = Kirigami.MessageType.Positive;
                deviceResult.text = i18n("“%1” is now stored in the keyboard: written, then read back identical. This computer may show the old name until a later connection.", name);
            } else if (verdict === "unverified") {
                deviceResult.type = Kirigami.MessageType.Warning;
                deviceResult.text = i18n("The name was written, but it could not be read back. Look at the stored name above in a moment.\n%1", detail);
            } else if (verdict === "mismatch") {
                deviceResult.type = Kirigami.MessageType.Error;
                deviceResult.text = i18n("The name was written, but the keyboard reads back another one.\n%1", detail);
            } else if (verdict === "uncertain") {
                deviceResult.type = Kirigami.MessageType.Warning;
                deviceResult.text = i18n("The write failed on its way: the name may or may not have been written. Look at the stored name above in a moment.\n%1", detail);
            } else if (verdict === "check-failed") {
                deviceResult.type = Kirigami.MessageType.Error;
                deviceResult.text = i18n("Check failed, nothing was written.\n%1", detail);
            } else {
                deviceResult.type = Kirigami.MessageType.Error;
                deviceResult.text = i18n("Not written.\n%1", detail);
            }
            deviceResult.visible = true;
            if (!checkOnly) root.store.fetchState();
        });
    }

    Kirigami.PromptDialog {
        id: confirmDialog
        objectName: "nameConfirmDialog"
        property string name: ""
        title: i18nc("@title:window", "Write the name into the keyboard")
        subtitle: i18n("Write “%1” into the keyboard's memory?", name)
        standardButtons: Kirigami.Dialog.Ok | Kirigami.Dialog.Cancel
        onAccepted: root.runDeviceName(false)
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
            objectName: "nameDeviceField"
            Kirigami.FormData.label: i18n("Name to write:")
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 20
            maximumLength: 32
            placeholderText: i18n("1 to 32 ASCII characters")
            validator: RegularExpressionValidator { regularExpression: root.deviceNameRe }
            Accessible.name: i18n("Name to write into the keyboard")
            Accessible.description: i18n("1 to 32 printable ASCII characters, no leading or trailing space, no backslash")
            enabled: !root.deviceBusy
        }
        RowLayout {
            QQC2.Button {
                id: checkBtn
                text: i18n("Check (nothing written)")
                icon.name: "dialog-ok-apply"
                enabled: !root.deviceBusy && root.validDeviceName(deviceNameField.text)
                Accessible.description: i18n("Runs every check that precedes the write and saves the current name; nothing is written")
                onClicked: root.runDeviceName(true)
            }
            QQC2.Button {
                id: writeBtn
                objectName: "nameWriteBtn"
                text: i18n("Write the name into the keyboard…")
                icon.name: "document-edit-sign"
                enabled: !root.deviceBusy && root.validDeviceName(deviceNameField.text)
                Accessible.description: i18n("Asks for a confirmation, then writes the name into the keyboard and reads it back")
                onClicked: {
                    confirmDialog.name = deviceNameField.text;
                    confirmDialog.open();
                }
            }
            QQC2.BusyIndicator {
                visible: root.deviceBusy
                running: visible
                implicitHeight: writeBtn.implicitHeight
                implicitWidth: implicitHeight
            }
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 26
            wrapMode: Text.Wrap
            font: Kirigami.Theme.smallFont
            text: deviceNameField.text.length > 0 && !root.validDeviceName(deviceNameField.text)
                ? i18n("Refused: 1 to 32 printable ASCII characters, no leading or trailing space, no backslash.")
                : i18n("Every computer and phone sees this name. The current name is saved first, the keyboard is checked, the name is written once and read back at once. Not measured: whether the name survives a battery change.")
        }
    }

    Kirigami.InlineMessage {
        id: deviceResult
        objectName: "nameDeviceResult"
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        visible: false
        showCloseButton: true
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
              cmd: root.nameCommand("--dry-run") },
            { title: i18n("Every check and the backup of the current name (writes nothing):"),
              cmd: root.nameCommand("--check") },
            { title: i18n("The write itself (checks, backup, one confirmation, one write, read back; issue #248):"),
              cmd: root.nameCommand("") },
            { title: i18n("Forget the keyboard cleanly, as macOS does, then pair it again (only after typing the confirmation word, issue #217):"),
              cmd: "akmctl repair --force" }
        ]
        delegate: ColumnLayout {
            id: cmdItem
            required property var modelData
            required property int index
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
                    objectName: "nameCmd" + cmdItem.index
                    readOnly: true
                    text: cmdItem.modelData.cmd
                    placeholderText: i18n("Type a valid name to write above first")
                    font.family: "monospace"
                    Accessible.name: i18n("Command: %1", text)
                }
                QQC2.Button {
                    objectName: "nameCopy" + cmdItem.index
                    text: i18n("Copy")
                    icon.name: "edit-copy"
                    Accessible.name: i18n("Copy the command")
                    enabled: cmdItem.modelData.cmd !== ""
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
