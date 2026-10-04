// SPDX-License-Identifier: GPL-2.0-or-later
// System Settings → Input & Output → Keyboard → Apple Keyboard.
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM

KCM.SimpleKCM {
    id: page

    readonly property Store store: Store { caller: kcm.busCall }

    header: ColumnLayout {
        spacing: 0

        PlainMessage {
            id: absent
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.smallSpacing
            visible: !page.store.present
            type: Kirigami.MessageType.Warning
            property bool starting: false
            // set by a failed start, cleared when the service appears: `text` stays a binding
            property string startError: ""
            text: startError !== "" ? i18n("The service could not be started: %1", startError)
                : i18n("The keyboard service (apple-kb-monitord) is not running: every setting is read and written by it, nothing can be shown or changed until it starts.")
            actions: [
                Kirigami.Action {
                    id: startAction
                    text: absent.starting ? i18n("Starting…") : i18n("Start the service")
                    icon.name: "media-playback-start"
                    enabled: !absent.starting
                    onTriggered: {
                        absent.starting = true;
                        page.store.startDaemon(function (ok, msg) {
                            absent.starting = false;
                            absent.startError = ok ? "" : (msg || i18n("unknown error"));
                        });
                    }
                }
            ]
        }

        QQC2.TabBar {
            id: tabs
            objectName: "tabs"
            Layout.fillWidth: true
            Accessible.name: i18n("Sections of the Apple keyboard settings")
            QQC2.TabButton { text: i18nc("@title:tab", "State"); focusPolicy: Qt.StrongFocus }
            QQC2.TabButton { text: i18nc("@title:tab", "Keys"); focusPolicy: Qt.StrongFocus }
            QQC2.TabButton { text: i18nc("@title:tab", "Notifications"); focusPolicy: Qt.StrongFocus }
            QQC2.TabButton { text: i18nc("@title:tab", "Name"); focusPolicy: Qt.StrongFocus }
            QQC2.TabButton { text: i18nc("@title:tab", "Diagnostics"); focusPolicy: Qt.StrongFocus }
        }
    }

    ColumnLayout {
        spacing: 0
        StatePage {
            Layout.fillWidth: true
            visible: tabs.currentIndex === 0
            store: page.store
        }
        KeysPage {
            Layout.fillWidth: true
            visible: tabs.currentIndex === 1
            store: page.store
        }
        AlertsPage {
            id: alerts
            Layout.fillWidth: true
            visible: tabs.currentIndex === 2
            store: page.store
        }
        NamePage {
            Layout.fillWidth: true
            visible: tabs.currentIndex === 3
            store: page.store
        }
        DiagPage {
            Layout.fillWidth: true
            visible: tabs.currentIndex === 4
            store: page.store
        }
    }

    function switchSection(step) {
        tabs.currentIndex = (tabs.currentIndex + tabs.count + step) % tabs.count;
    }
    Keys.onShortcutOverride: function (event) {
        if (event.modifiers & Qt.ControlModifier
                && [Qt.Key_PageDown, Qt.Key_PageUp, Qt.Key_Tab, Qt.Key_Backtab].indexOf(event.key) >= 0) {
            event.accepted = true;
        }
    }
    Keys.onPressed: function (event) {
        if (!(event.modifiers & Qt.ControlModifier)) return;
        if (event.key === Qt.Key_PageDown || (event.key === Qt.Key_Tab && !(event.modifiers & Qt.ShiftModifier))) {
            page.switchSection(1);
            event.accepted = true;
        } else if (event.key === Qt.Key_PageUp || event.key === Qt.Key_Backtab) {
            page.switchSection(-1);
            event.accepted = true;
        }
    }

    Connections {
        target: kcm
        function onLoadRequested() { alerts.load(); }
        function onSaveRequested() { alerts.save(); }
        function onSaveReturned() { alerts.update(); }
        function onDefaultsRequested() { alerts.defaults(); }
    }
    // The daemon may register after the page opened: read the settings again then.
    Connections {
        target: page.store
        function onPresentChanged() {
            if (page.store.present) absent.startError = "";
            if (page.store.present && !alerts.saving) alerts.load();
        }
    }

    // Responsiveness probe for tests/e2e/kcm.py (module argument "heartbeat"): lateness of a 16 ms GUI timer.
    Timer {
        id: heartbeat
        property double last: 0
        property double worst: 0
        property double since: 0
        interval: 16
        repeat: true
        onTriggered: {
            const now = Date.now();
            if (last > 0) worst = Math.max(worst, now - last - interval);
            last = now;
            if (now - since >= 500) {
                console.warn("akm-heartbeat " + JSON.stringify({ ts_ms: now, max_late_ms: worst }));
                since = now;
                worst = 0;
            }
        }
    }

    function args() {
        return (kcm.args || []).map(function (x) { return String(x).toLowerCase(); });
    }
    function showTabOfArgs() {
        const names = ["state", "keys", "notifications", "name", "diagnostics"];
        const a = args();
        for (let i = 0; i < names.length; ++i) {
            if (a.indexOf(names[i]) >= 0) tabs.currentIndex = i;
        }
    }
    Connections {
        target: kcm
        function onArgsChanged() { page.showTabOfArgs(); }
    }

    Component.onCompleted: {
        showTabOfArgs();
        heartbeat.running = args().indexOf("heartbeat") >= 0;
        page.store.refreshAll();
        alerts.load();
    }
}
