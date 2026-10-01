// SPDX-License-Identifier: GPL-2.0-or-later
// State: what the daemon reports (GetState), read-only.
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: root

    required property Store store

    readonly property var st: store.state
    readonly property var kb: st && st.keyboard ? st.keyboard : null
    readonly property var dev: kb && kb.device ? kb.device : ({})
    readonly property var bat: kb && kb.battery ? kb.battery : ({})
    readonly property var radio: kb && kb.radio ? kb.radio : ({})
    readonly property var fw: kb && kb.firmware ? kb.firmware : ({})
    property double now: Date.now()

    spacing: Kirigami.Units.smallSpacing

    Timer {
        // Only the "age" lines change every second; nothing is re-read.
        interval: 1000
        repeat: true
        running: root.visible
        triggeredOnStart: true
        onTriggered: root.now = Date.now()
    }

    function dash(v) {
        return (v === undefined || v === null || v === "") ? "—" : String(v);
    }
    function pct(v) {
        return (typeof v === "number" && isFinite(v)) ? i18nc("percentage", "%1 %", Math.round(v)) : "—";
    }
    function ago(seconds) {
        if (seconds < 0) seconds = 0;
        if (seconds < 60) return i18np("%1 second ago", "%1 seconds ago", Math.floor(seconds));
        if (seconds < 3600) return i18np("%1 minute ago", "%1 minutes ago", Math.floor(seconds / 60));
        if (seconds < 172800) return i18np("%1 hour ago", "%1 hours ago", Math.floor(seconds / 3600));
        return i18np("%1 day ago", "%1 days ago", Math.floor(seconds / 86400));
    }
    function chemistryName(c) {
        switch (String(c).toLowerCase()) {
        case "alkaline": return i18nc("battery chemistry", "alkaline");
        case "nimh": return i18nc("battery chemistry", "NiMH (rechargeable)");
        case "lithium": return i18nc("battery chemistry", "lithium");
        default: return i18nc("battery chemistry", "unknown");
        }
    }
    function signalText() {
        const rel = (typeof radio.rssi_rel_db === "number") ? radio.rssi_rel_db : radio.rssi_dbm;
        if (typeof rel !== "number") return i18n("unknown");
        let q;
        switch (radio.rssi_quality) {
        case "excellent": q = i18nc("signal quality", "excellent"); break;
        case "good": q = i18nc("signal quality", "good"); break;
        case "weak": q = i18nc("signal quality", "weak"); break;
        default: q = rel >= 0 ? i18nc("signal quality", "excellent") : (rel >= -5 ? i18nc("signal quality", "good") : i18nc("signal quality", "weak"));
        }
        return i18nc("signal quality, then the relative value in dB (0 = ideal range)", "%1 (%2)", q, rel);
    }
    function firmwareStatus(s) {
        switch (s) {
        case "up_to_date": return i18n("up to date");
        case "update_available": return i18n("a newer version exists (this module never updates the firmware)");
        case "unknown": return i18n("model not in the table");
        default: return i18n("not assessed yet");
        }
    }

    Kirigami.PlaceholderMessage {
        Layout.fillWidth: true
        Layout.margins: Kirigami.Units.gridUnit * 2
        visible: !root.store.present
        icon.name: "input-keyboard"
        text: root.store.known ? i18n("No data: the keyboard service is not running") : i18n("Looking for the keyboard service…")
        explanation: root.store.known ? i18n("Use “Start the service” above, or run: systemctl --user start apple-kb-monitord.service") : ""
    }

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        Layout.margins: Kirigami.Units.smallSpacing
        visible: root.store.present && root.store.stateError !== ""
        type: Kirigami.MessageType.Error
        text: root.store.stateError === "timeout"
            ? i18n("The keyboard service did not answer within 5 seconds. It is busy or stuck; the values shown may be old.")
            : i18n("The keyboard service answered with an error: %1", root.store.stateError)
        actions: [
            Kirigami.Action {
                text: i18n("Try again")
                icon.name: "view-refresh"
                onTriggered: root.store.fetchState()
            }
        ]
    }

    QQC2.BusyIndicator {
        Layout.alignment: Qt.AlignHCenter
        visible: root.store.present && root.st === null && root.store.stateError === ""
        running: visible
        Accessible.name: i18n("Loading the keyboard state")
    }

    Kirigami.FormLayout {
        Layout.fillWidth: true
        visible: root.st !== null

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Keyboard")
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Name:")
            text: root.dash(root.dev.alias || root.dev.name)
            elide: Text.ElideRight
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            Accessible.name: i18n("Name: %1", text)
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Model:")
            text: root.dash(root.dev.model)
            wrapMode: Text.Wrap
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Address:")
            text: root.dash(root.dev.mac)
            font.family: "monospace"
        }
        RowLayout {
            Kirigami.FormData.label: i18n("Connection:")
            Kirigami.Icon {
                source: root.st && root.st.connected ? "network-connect" : "network-disconnect"
                implicitWidth: Kirigami.Units.iconSizes.small
                implicitHeight: Kirigami.Units.iconSizes.small
                Accessible.ignored: true
            }
            QQC2.Label {
                text: root.st && root.st.connected ? i18n("connected") : i18n("disconnected")
                Accessible.name: i18n("Connection: %1", text)
            }
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Problem:")
            visible: !!(root.st && root.st.kb_error)
            text: root.dash(root.st ? root.st.kb_error : "")
            wrapMode: Text.Wrap
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Battery")
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Keyboard indication:")
            text: root.pct(root.bat.percentage)
            Accessible.name: i18n("Keyboard indication: %1", text)
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Apple display:")
            visible: typeof root.bat.apple_display_pct === "number"
            text: i18nc("percentage as macOS shows it", "%1 (as macOS shows it)", root.pct(root.bat.apple_display_pct))
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Estimated charge:")
            readonly property var est: root.bat.charge_estimate
            text: root.bat.new_batteries
                ? i18n("new batteries: no estimate during the first two days")
                : (est ? i18nc("estimate, low bound, high bound, chemistry", "%1 (between %2 and %3, %4 batteries)",
                               root.pct(est.pct), root.pct(est.low), root.pct(est.high), root.chemistryName(est.chemistry))
                       : i18n("not available"))
            wrapMode: Text.Wrap
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            Accessible.name: i18n("Estimated charge: %1", text)
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Voltage:")
            visible: typeof root.bat.voltage === "number"
            text: typeof root.bat.voltage === "number" ? i18nc("volts", "%1 V", root.bat.voltage.toFixed(2)) : ""
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Remaining:")
            visible: !!(root.st && root.st.remaining_display)
            text: root.dash(root.st ? root.st.remaining_display : "")
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Batteries installed:")
            visible: !!(root.st && root.st.batteries_installed_at)
            text: root.st && root.st.batteries_installed_at
                ? Qt.formatDate(new Date(root.st.batteries_installed_at * 1000), Qt.locale(), Locale.LongFormat) : ""
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Link")
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Signal:")
            text: root.signalText()
            Accessible.name: i18n("Signal: %1", text)
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Last measurement:")
            text: root.st && root.st.last_update > 0 ? root.ago(root.now / 1000 - root.st.last_update) : i18n("never")
            Accessible.name: i18n("Last measurement: %1", text)
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Firmware")
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Version:")
            text: root.dash(root.fw.version_hex || root.fw.version)
            font.family: "monospace"
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Latest known:")
            visible: !!root.fw.latest_known
            text: root.dash(root.fw.latest_known)
            font.family: "monospace"
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Status:")
            text: root.firmwareStatus(root.fw.status)
            wrapMode: Text.Wrap
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Service")
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Version:")
            text: root.dash(root.store.daemonVersion)
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("Last error:")
            visible: !!(root.st && root.st.last_error)
            text: root.dash(root.st ? root.st.last_error : "")
            wrapMode: Text.Wrap
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
        }
    }
}
