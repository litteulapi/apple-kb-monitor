// SPDX-License-Identifier: GPL-2.0-or-later
//
// Diagnostics: akmctl doctor / selftest, reconnection request, copy of everything for a bug report.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: root
    objectName: "diagPage"

    required property Store store

    readonly property string docUrl: "https://github.com/litteulapi/apple-kb-monitor/blob/main/docs/TROUBLESHOOTING.md"
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
    function topicName(t) {
        switch (String(t)) {
        case "adapter": return i18nc("doctor topic", "adapter");
        case "adapter-pm": return i18nc("doctor topic", "adapter power");
        case "bluez-conf": return i18nc("doctor topic", "BlueZ settings");
        case "daemon": return i18nc("doctor topic", "service");
        case "hidraw": return i18nc("doctor topic", "HID access");
        case "journal": return i18nc("doctor topic", "journal");
        case "link": return i18nc("doctor topic", "link");
        case "link-key": return i18nc("doctor topic", "link key");
        case "link-quality": return i18nc("doctor topic", "link quality");
        case "pairing": return i18nc("doctor topic", "pairing");
        case "signal": return i18nc("doctor topic", "signal");
        case "upower": return i18nc("doctor topic", "UPower");
        case "versions": return i18nc("doctor topic", "versions");
        default: return String(t);
        }
    }
    // never by its English phrase.
    function adviceText(v) {
        switch (String(v && v.id || "")) {
        case "refused": return i18nc("doctor verdict", "the pairing is refused or missing: run `akmctl repair`");
        case "no-pairing": return i18nc("doctor verdict", "no usable pairing: run `akmctl repair`");
        // i18n: %1 = the doctor topics to check, comma-separated
        case "link-up-fix": return i18nc("doctor verdict", "Link up; to check: %1 (akmctl doctor)", (v.args || []).map(root.topicName).join(", "));
        case "link-up-ok": return i18nc("doctor verdict", "link up and configuration sound");
        case "not-connected": return i18nc("doctor verdict", "keyboard not connected: press a key and wait 10 s; the service pages it on its own. Pair it again only if `akmctl doctor` reports auth-failed");
        default: return String(v && v.advice || "");
        }
    }
    function findingText(x) {
        var a = x.args || [];
        switch (String(x.id || "")) {
        case "signal.ok": return i18nc("doctor finding", "rssi-helper executable, capability cap_net_admin present");
        case "signal.helper_missing": return i18nc("doctor finding", "the rssi-helper utility is not installed");
        case "signal.helper_failed": return i18nc("doctor finding", "the rssi-helper utility failed (missing capability, Bluetooth refused)");
        case "signal.timeout": return i18nc("doctor finding", "the rssi-helper utility does not answer in time");
        case "signal.unavailable": return i18nc("doctor finding", "the Bluetooth adapter gives no measure for this link");
        case "signal.other": return i18nc("doctor finding", "the signal cannot be measured");
        case "bluez-conf.ignored": return i18nc("doctor finding, %1 = settings", "ignored by bluetoothd: %1", a[0]);
        case "bluez-conf.reconnect-ok": return i18nc("doctor finding", "[Policy] Reconnect* in place");
        case "bluez-conf.fast-ok": return i18nc("doctor finding", "FastConnectable = true");
        case "bluez-conf.fast-off": return i18nc("doctor finding", "FastConnectable off: slower answer to the keyboard's page after a key press");
        case "bluez-conf.unreadable": return i18nc("doctor finding, %1 = file, %2 = error", "%1: %2", a[0], a[1]);
        case "upower.no-poll": return i18nc("doctor finding", "NoPollBatteries = true (no 30 s HID polling)");
        case "upower.polls": return i18nc("doctor finding", "UPower reads the keyboard battery every 30 s (2 GET_REPORT over the air): the keyboard never sleeps");
        case "adapter-pm.not-usb": return i18nc("doctor finding", "adapter is not on USB (or not found)");
        case "adapter-pm.off": return i18nc("doctor finding, %1 = USB id", "%1 autosuspend off (power/control=on)", a[0]);
        case "adapter-pm.active":
            return i18nc("doctor finding", "%1 USB autosuspend active (control=%2, delay %3 ms, now %4, %5 s suspended since boot)", a[0], a[1], a[2], a[3], a[4])
                + (a[5] ? i18nc("doctor finding, appended", "; rule installed but not applied yet") : "");
        case "adapter-pm.btusb": return i18nc("doctor finding", "btusb enable_autosuspend = %1", a[0]);
        case "adapter.powered": return i18nc("doctor finding", "Bluetooth adapter powered");
        case "adapter.off": return i18nc("doctor finding", "Bluetooth adapter off");
        case "adapter.bluez-unreachable": return i18nc("doctor finding", "BlueZ unreachable (bluetooth.service stopped?)");
        case "pairing.none": return i18nc("doctor finding", "no paired Apple keyboard known to BlueZ");
        case "pairing.not-paired": return i18nc("doctor finding, %1 = address", "%1 is not paired", root.store.maskMac(a[0]));
        case "pairing.ok":
            return i18nc("doctor finding, %1 = address, %2 = name", "%1 “%2” paired", root.store.maskMac(a[0]), a[1])
                + (a[2] ? i18nc("doctor finding, appended", ", bonded") : "")
                + (a[3] ? i18nc("doctor finding, appended", ", legacy PIN pairing") : "");
        case "pairing.not-trusted": return i18nc("doctor finding", "not trusted: BlueZ may refuse the keyboard's own reconnection");
        case "pairing.blocked": return i18nc("doctor finding", "blocked in BlueZ");
        case "link.connected": return i18nc("doctor finding", "connected");
        case "link.not-connected": return i18nc("doctor finding", "not connected (asleep? press a key)");
        case "link-key.unchecked": return i18nc("doctor finding", "link key not checked (needs root: sudo akmctl doctor)");
        case "link-key.none": return i18nc("doctor finding", "no [LinkKey] stored for the keyboard");
        case "link-key.same": return i18nc("doctor finding", "stored key == kernel key (fingerprint %1)", a[0]);
        case "link-key.differs": return i18nc("doctor finding", "stored key %1 != kernel key %2: will change at next restart", a[0], a[1]);
        case "link-key.no-kernel": return i18nc("doctor finding", "stored key %1; kernel copy not readable (debugfs not mounted?)", a[0]);
        case "link-quality":
            return i18ncp("doctor finding", "link quality: %1 disconnection in the last hour, %2 in 24 h, %3 in 7 days", "link quality: %1 disconnections in the last hour, %2 in 24 h, %3 in 7 days", Number(a[0]), a[1], a[2])
                + (a[3] ? i18ncp("doctor finding, appended", "; relative signal over 7 days: mean %2 dB, min %3 dB (%1 measurement)", "; relative signal over 7 days: mean %2 dB, min %3 dB (%1 measurements)", Number(a[5]), Number(a[3]).toLocaleString(Qt.locale(), "f", 1), a[4]) : "")
                + (a[6] ? i18nc("doctor finding, appended", " — UNSTABLE: %1 unexpected within the hour", a[6]) : "");
        case "hidraw.readable": return i18nc("doctor finding, %1 = device", "%1 readable", a[0]);
        case "hidraw.unreadable": return i18nc("doctor finding, %1 = device", "%1 not readable by this user", a[0]);
        case "hidraw.no-node": return i18nc("doctor finding", "connected but no hidraw node (uhid not created yet?)");
        case "journal.clean": return i18nc("doctor finding", "no bluetoothd error in the last 6 hours of this boot");
        case "journal.unreadable": return i18nc("doctor finding", "bluetoothd journal not readable (add yourself to group systemd-journal, or use sudo)");
        case "journal.page-timeout": return i18ncp("doctor finding, %1 = count, %2 = time", "%1 page timeout (keyboard asleep or not listening), last %2", "%1 page timeouts (keyboard asleep or not listening), last %2", Number(a[0]), a[1]);
        case "journal.get-report-timeout": return i18ncp("doctor finding, %1 = count, %2 = time", "%1 HIDP GET_REPORT timeout, last %2", "%1 HIDP GET_REPORT timeouts, last %2", Number(a[0]), a[1]);
        case "journal.auth": return i18ncp("doctor finding, %1 = count, %2 = time", "%1 authentication / key error, last %2", "%1 authentication / key errors, last %2", Number(a[0]), a[1]);
        case "journal.refused": return i18ncp("doctor finding, %1 = count, %2 = time", "%1 connection reset/refused by the keyboard, last %2", "%1 connections reset/refused by the keyboard, last %2", Number(a[0]), a[1]);
        case "journal.config-ignored": return i18ncp("doctor finding, %1 = count, %2 = time", "%1 main.conf key ignored by bluetoothd at start, last %2", "%1 main.conf keys ignored by bluetoothd at start, last %2", Number(a[0]), a[1]);
        case "journal.storage-error": return i18ncp("doctor finding, %1 = count, %2 = time", "%1 BlueZ storage write failure (a new link key may be lost at reboot), last %2", "%1 BlueZ storage write failures (a new link key may be lost at reboot), last %2", Number(a[0]), a[1]);
        case "daemon.unreachable": return i18nc("doctor finding", "apple-kb-monitord link keeper not reachable (no automatic reconnection)");
        case "daemon.health": return i18nc("doctor finding, %1 = state", "link health: %1", a[0]);
        case "daemon.untracked": return i18nc("doctor finding", "keeper running, keyboard not tracked");
        case "versions.deleted-exe": return i18nc("doctor finding, %1 = program, %2 = pid, %3 = path", "%1 (pid %2) still runs the replaced binary %3 (deleted): the old version", a[0], a[1], a[2]);
        case "versions.plasma-older": return i18nc("doctor finding, %1 = pid", "plasmashell (pid %1) started before the widget was installed: it shows the old widget", a[0]);
        default: return String(x.text || "");
        }
    }
    function findingFix(x) {
        if (!x.fix) return "";
        switch (String(x.id || "")) {
        case "signal.helper_missing": return i18nc("doctor fix", "reinstall the apple-kb-monitor package");
        case "signal.helper_failed": return i18nc("doctor fix", "reinstall the apple-kb-monitor package, then run akmctl doctor");
        case "signal.timeout": return i18nc("doctor fix", "check the Bluetooth service: systemctl status bluetooth");
        case "signal.unavailable": return i18nc("doctor fix", "bring the keyboard closer, or reconnect it");
        case "signal.other": return i18nc("doctor fix", "run akmctl doctor");
        case "upower.polls": return i18nc("doctor fix, %1 = command", "optional: %1", String(x.fix).replace(/^optional: /, ""));
        case "pairing.none": return i18nc("doctor fix", "akmctl repair (pairing assistant)");
        case "link-key.differs": return i18nc("doctor fix", "check disk space, then akmctl repair if the keyboard is refused");
        case "link-quality": return i18nc("doctor fix", "check the batteries, the distance, USB 3 devices near the adapter");
        case "hidraw.unreadable": return i18nc("doctor fix", "install udev/70-apple-kb-hidraw.rules (uaccess)");
        case "journal.get-report-timeout": return i18nc("doctor fix", "avoid HID scanning tools while measuring; see docs/RECONNECTION-PAIRING.md §3.5");
        case "journal.storage-error": return i18nc("doctor fix", "free disk space (btrfs: check metadata), then verify with sudo akmctl doctor");
        default: return String(x.fix);
        }
    }
    // akmctl selftest lines, translated from their message id and values, never from the English text.
    function selftestText(c) {
        var a = c.args || [];
        switch (String(c.msg || "")) {
        // i18n: %1 = daemon version, %2 = akmctl version, %3 = package version
        case "versions.differ": return i18nc("selftest check", "versions differ: daemon %1, akmctl %2, package %3: restart the daemon after an upgrade (systemctl --user restart apple-kb-monitord.service)", a[0], a[1], a[2]);
        // i18n: %1 = daemon version, %2 = package version
        case "versions.same": return i18nc("selftest check", "daemon %1 = akmctl, package %2", a[0], a[1]);
        case "versions.unknown": return i18nc("selftest check", "daemon version unknown (old interface)");
        case "versions.deleted-exe":
        case "versions.plasma-older": return findingText({ id: c.msg, args: a }) + ": " + a[a.length - 1];
        case "alias.same": return i18nc("selftest check", "keyboard alias as last set through this monitor");
        // i18n: %1 = current alias, %2 = alias set last, %3 = by whom, %4 = Unix time
        case "alias.differs": return i18nc("selftest check", "keyboard alias is %1, last set to %2 by %3 at %4 (unix): changed elsewhere (KDE Bluetooth settings, bluetoothctl) or the pairing was removed (akmctl repair, Plasma Forget); see the daemon journal for \"BlueZ alias\" lines", a[0], a[1], a[2], a[3]);
        case "daemon-restarts.count": return i18ncp("selftest check", "apple-kb-monitord restarted %1 time by systemd since the last pass (crash loop?)", "apple-kb-monitord restarted %1 times by systemd since the last pass (crash loop?)", Number(a[0]));
        // i18n: %1 = old process id, %2 = new process id
        case "daemon-restarts.pid": return i18nc("selftest check", "daemon pid changed %1 -> %2 (restart)", a[0], a[1]);
        case "daemon.slow": return i18nc("selftest check, %1 = milliseconds", "daemon answered GetState in %1 ms (> 1 s)", a[0]);
        case "daemon.ok": return i18nc("selftest check, %1 = milliseconds", "daemon on the bus, GetState in %1 ms", a[0]);
        // i18n: %1 = error, %2 = systemd unit state
        case "daemon.unreachable": return i18nc("selftest check", "daemon unreachable (%1); unit state: %2", a[0], a[1]);
        case "daemon-error": return i18nc("selftest check, %1 = error", "daemon last_error: %1", a[0]);
        case "freshness.never": return i18nc("selftest check", "keyboard connected but never acquired");
        case "freshness.stale": return i18nc("selftest check, %1 = hours", "keyboard connected, last acquisition %1 h ago", Number(a[0]).toLocaleString(Qt.locale(), "f", 1));
        case "freshness.ok": return i18nc("selftest check, %1 = minutes", "last acquisition %1 min ago", a[0]);
        case "freshness.disconnected": return i18nc("selftest check", "keyboard not connected (nothing to acquire)");
        case "journal-daemon.ok": return i18nc("selftest check", "no daemon warning since the last pass");
        case "journal-daemon.lines": return i18ncp("selftest check, %2 = journal line", "%1 daemon warning/error line since the last pass, last: %2", "%1 daemon warning/error lines since the last pass, last: %2", Number(a[0]), a[1]);
        case "journal-daemon.unreadable": return i18nc("selftest check", "user journal not readable");
        case "journal-bluetooth.ok": return i18nc("selftest check", "no bluetoothd error since the last pass");
        case "journal-bluetooth.errors": return i18ncp("selftest check", "%1 bluetoothd error line since the last pass", "%1 bluetoothd error lines since the last pass", Number(a[0]));
        case "journal-bluetooth.unreadable": return i18nc("selftest check", "system journal not readable (group systemd-journal)");
        case "coredumps.ok": return i18nc("selftest check", "no crash of our programs since the last pass");
        // i18n: %1 = program, %2 = signal, %3 = process id
        case "coredumps.crash": return i18nc("selftest check", "%1 crashed (%2, pid %3): coredumpctl info %3", a[0], a[1], a[2]);
        case "coredumps.unreadable": return i18nc("selftest check", "coredump journal not readable");
        // i18n: %1 = directory, %2 = free MiB, %3 = total MiB
        case "disk.free": return i18nc("selftest check", "%1: %2 MiB free of %3 MiB", a[0], a[1], a[2]);
        case "disk.unknown": return i18nc("selftest check, %1 = directory", "%1: statvfs failed", a[0]);
        // i18n: %1 = unreadable lines, %2 = file, %3 = "N lines", %4 = size
        case "history.stats": return i18ncp("selftest check", "%2: %3, %4 KiB, %1 unreadable line in the last 1000", "%2: %3, %4 KiB, %1 unreadable lines in the last 1000", Number(a[0]), a[1], i18ncp("selftest check, history size", "%1 line", "%1 lines", Number(a[2])), a[3]);
        case "history.absent": return i18nc("selftest check", "no history yet");
        case "link.doctor": return i18nc("selftest check, %1 = doctor verdict", "akmctl doctor: %1", adviceText({ id: a[1], args: a.slice(2), advice: a[0] }));
        case "link.doctor-stuck": return i18nc("selftest check, %1 = seconds", "akmctl doctor did not finish within %1 s (BlueZ or the daemon does not answer)", a[0]);
        default: return String(c.text || "");
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
                doctorError = root.store.failureText((err || out || String(code)).trim());
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
                selftestError = root.store.failureText((err || out || String(code)).trim());
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
        return root.store.maskMacs(parts.join("\n\n"));  // meant for a public bug report
    }

    PlainMessage {
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
                root.store.copyText(root.report());
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

    Kirigami.Heading {
        textFormat: Text.PlainText
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
        textFormat: Text.PlainText
        objectName: "diagDoctorVerdict"
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        wrapMode: Text.Wrap
        visible: !root.doctorBusy
        text: root.doctor && root.doctor.verdict
            ? i18nc("level, advice", "Verdict: %1 — %2", root.levelName(root.doctor.verdict.level), root.adviceText(root.doctor.verdict))
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
            Accessible.name: root.levelName(finding.modelData.level) + ": " + root.findingText(finding.modelData)
            Kirigami.Icon {
                Layout.alignment: Qt.AlignTop
                source: root.levelIcon(finding.modelData.level)
                implicitWidth: Kirigami.Units.iconSizes.small
                implicitHeight: Kirigami.Units.iconSizes.small
                Accessible.ignored: true
            }
            QQC2.Label {
                textFormat: Text.PlainText
                objectName: "diagFinding" + finding.index
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: (finding.modelData.topic ? "[" + root.topicName(finding.modelData.topic) + "] " : "") + root.findingText(finding.modelData)
                      + (finding.modelData.fix ? "\n→ " + root.findingFix(finding.modelData) : "")
            }
        }
    }

    Kirigami.Heading {
        textFormat: Text.PlainText
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
        textFormat: Text.PlainText
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
            Accessible.name: root.levelName(check.modelData.level) + ": " + root.selftestText(check.modelData)
            Kirigami.Icon {
                Layout.alignment: Qt.AlignTop
                source: root.levelIcon(check.modelData.level)
                implicitWidth: Kirigami.Units.iconSizes.small
                implicitHeight: Kirigami.Units.iconSizes.small
                Accessible.ignored: true
            }
            QQC2.Label {
                textFormat: Text.PlainText
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: root.selftestText(check.modelData)
            }
        }
    }
    Item { Layout.preferredHeight: Kirigami.Units.largeSpacing }
}
