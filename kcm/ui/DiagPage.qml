// SPDX-License-Identifier: GPL-2.0-or-later
// Diagnostics: akmctl doctor / selftest (read-only), reconnection request,
// copy of everything for a bug report.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: root

    required property Store store

    readonly property string docUrl: "https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/src/branch/main/docs/TROUBLESHOOTING.md"
    property var doctor: null
    property string doctorRaw: ""
    property string doctorError: ""
    property bool doctorBusy: false
    property var selftest: null
    property string selftestRaw: ""
    property string selftestError: ""
    property bool selftestBusy: false

    spacing: Kirigami.Units.smallSpacing

    function levelIcon(l) {
        switch (String(l)) {
        case "ok": case "good": return "data-success";
        case "info": return "data-information";
        case "warn": case "warning": return "data-warning";
        default: return "data-error";
        }
    }
    function levelName(l) {
        switch (String(l)) {
        case "ok": case "good": return i18nc("diagnosis level", "OK");
        case "info": return i18nc("diagnosis level", "information");
        case "warn": case "warning": return i18nc("diagnosis level", "warning");
        default: return i18nc("diagnosis level", "problem");
        }
    }
    function runDoctor() {
        doctorBusy = true;
        root.store.cmd("akmctl", ["doctor", "--json"], root.store.longCmdTimeout, function (code, out, err, timedOut) {
            doctorBusy = false;
            doctorRaw = out;
            if (timedOut) { doctor = null; doctorError = i18n("no answer within 60 seconds"); return; }
            try {
                doctor = JSON.parse(out);
                doctorError = "";
            } catch (e) {
                doctor = null;
                doctorError = (err || out || String(code)).trim();
            }
        });
    }
    function runSelftest() {
        selftestBusy = true;
        root.store.cmd("akmctl", ["selftest", "--json", "--no-save"], root.store.longCmdTimeout, function (code, out, err, timedOut) {
            selftestBusy = false;
            selftestRaw = out;
            if (timedOut) { selftest = null; selftestError = i18n("no answer within 60 seconds"); return; }
            try {
                selftest = JSON.parse(out);
                selftestError = "";
            } catch (e) {
                selftest = null;
                selftestError = (err || out || String(code)).trim();
            }
        });
    }
    function report() {
        const parts = [];
        parts.push("# Apple keyboard — diagnosis (" + new Date().toISOString() + ")");
        parts.push("service: " + (root.store.present ? "running " + root.store.daemonVersion : "not running"));
        if (root.store.stateError) parts.push("state error: " + root.store.stateError);
        if (root.store.state) parts.push("## state\n" + JSON.stringify(root.store.state, null, 1));
        if (root.store.keyTable) parts.push("## keys\nparams " + JSON.stringify(root.store.keyTable.params) + " pending " + root.store.keyTable.pending);
        parts.push("## akmctl doctor --json\n" + (doctorRaw || doctorError || "not run"));
        parts.push("## akmctl selftest --json\n" + (selftestRaw || selftestError || "not run"));
        return parts.join("\n\n");
    }

    Kirigami.InlineMessage {
        id: result
        Layout.fillWidth: true
        Layout.margins: Kirigami.Units.smallSpacing
        visible: false
        showCloseButton: true
    }

    Flow {
        Layout.fillWidth: true
        Layout.margins: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.smallSpacing
        QQC2.Button {
            objectName: "diagDoctorBtn"
            text: i18n("Check the link")
            icon.name: "system-search"
            // stays enabled while running so that the keyboard focus is kept
            Accessible.description: i18n("Runs akmctl doctor: Bluetooth, pairing, configuration, journal, service. Reads only.")
            onClicked: if (!root.doctorBusy) root.runDoctor()
        }
        QQC2.Button {
            text: i18n("Self-check")
            icon.name: "checkmark"
            Accessible.description: i18n("Runs akmctl selftest: service, link, journal, crashes, disk. Reads only.")
            onClicked: if (!root.selftestBusy) root.runSelftest()
        }
        QQC2.Button {
            id: reconnectBtn
            text: i18n("Reconnect")
            icon.name: "network-connect"
            enabled: root.store.present
            Accessible.description: i18n("Asks the service to page the keyboard now (at most once every 20 seconds)")
            onClicked: {
                enabled = false;
                root.store.reconnect(function (ok, msg) {
                    reconnectBtn.enabled = Qt.binding(function () { return root.store.present; });
                    result.type = ok ? Kirigami.MessageType.Information : Kirigami.MessageType.Error;
                    result.text = ok ? i18n("Reconnection requested. Press a key on the keyboard if it is asleep.")
                                     : i18n("Reconnection not requested: %1", msg || i18n("refused by the service (asleep, too soon, or pairing refused)"));
                    result.visible = true;
                });
            }
        }
        QQC2.Button {
            text: i18n("Copy the diagnosis")
            icon.name: "edit-copy"
            onClicked: {
                root.store.bridge.copyText(root.report());
                result.type = Kirigami.MessageType.Information;
                result.text = i18n("Diagnosis copied: paste it into a bug report.");
                result.visible = true;
            }
        }
        QQC2.Button {
            text: i18n("Documentation")
            icon.name: "help-contents"
            Accessible.description: root.docUrl
            onClicked: Qt.openUrlExternally(root.docUrl)
        }
    }

    // ── doctor ──
    Kirigami.Heading {
        Layout.leftMargin: Kirigami.Units.largeSpacing
        level: 3
        text: i18nc("@title:group", "Link check (akmctl doctor)")
    }
    QQC2.BusyIndicator {
        Layout.leftMargin: Kirigami.Units.largeSpacing
        visible: root.doctorBusy
        running: visible
        Accessible.name: i18n("Link check running")
    }
    QQC2.Label {
        objectName: "diagDoctorVerdict"
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        wrapMode: Text.Wrap
        visible: !root.doctorBusy
        text: root.doctor && root.doctor.verdict
            ? i18nc("level, advice", "Verdict: %1 — %2", root.levelName(root.doctor.verdict.level), root.doctor.verdict.advice || "")
            : (root.doctorError !== "" ? i18n("Failed: %1", root.doctorError) : i18n("Not run yet."))
    }
    Repeater {
        model: root.doctor && root.doctor.findings ? root.doctor.findings : []
        delegate: RowLayout {
            id: finding
            required property var modelData
            required property int index
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Accessible.role: Accessible.StaticText
            Accessible.name: root.levelName(finding.modelData.level) + ": " + finding.modelData.text
            Kirigami.Icon {
                Layout.alignment: Qt.AlignTop
                source: root.levelIcon(finding.modelData.level)
                implicitWidth: Kirigami.Units.iconSizes.small
                implicitHeight: Kirigami.Units.iconSizes.small
                Accessible.ignored: true
            }
            QQC2.Label {
                objectName: "diagFinding" + finding.index
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: (finding.modelData.topic ? "[" + finding.modelData.topic + "] " : "") + finding.modelData.text
                      + (finding.modelData.fix ? "\n→ " + finding.modelData.fix : "")
            }
        }
    }

    // ── selftest ──
    Kirigami.Heading {
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.topMargin: Kirigami.Units.largeSpacing
        level: 3
        text: i18nc("@title:group", "Self-check (akmctl selftest)")
    }
    QQC2.BusyIndicator {
        Layout.leftMargin: Kirigami.Units.largeSpacing
        visible: root.selftestBusy
        running: visible
        Accessible.name: i18n("Self-check running")
    }
    QQC2.Label {
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        wrapMode: Text.Wrap
        visible: !root.selftestBusy
        text: root.selftest ? i18n("Verdict: %1", root.levelName(root.selftest.verdict))
            : (root.selftestError !== "" ? i18n("Failed: %1", root.selftestError) : i18n("Not run yet."))
    }
    Repeater {
        model: root.selftest && root.selftest.checks ? root.selftest.checks : []
        delegate: RowLayout {
            id: check
            required property var modelData
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Accessible.role: Accessible.StaticText
            Accessible.name: root.levelName(check.modelData.level) + ": " + check.modelData.text
            Kirigami.Icon {
                Layout.alignment: Qt.AlignTop
                source: root.levelIcon(check.modelData.level)
                implicitWidth: Kirigami.Units.iconSizes.small
                implicitHeight: Kirigami.Units.iconSizes.small
                Accessible.ignored: true
            }
            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: check.modelData.text
            }
        }
    }
    Item { Layout.preferredHeight: Kirigami.Units.largeSpacing }
}
