// SPDX-License-Identifier: GPL-2.0-or-later
// System Settings → Input Devices → Keyboard → Apple Keyboard (#250).
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM

KCM.SimpleKCM {
    id: page

    readonly property Store store: Store {
        bridge: kcm.bridge
    }

    header: ColumnLayout {
        spacing: 0

        Kirigami.InlineMessage {
            id: absent
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.smallSpacing
            visible: page.store.known && !page.store.present
            type: Kirigami.MessageType.Warning
            property bool starting: false
            text: i18n("The keyboard service (apple-kb-monitord) is not running: the state, the name and the reconnection are unavailable. The keys and the notifications can still be set.")
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
                            if (!ok) {
                                absent.text = i18n("The service could not be started: %1", msg || i18n("unknown error"));
                            }
                        });
                    }
                }
            ]
        }

        QQC2.TabBar {
            id: tabs
            Layout.fillWidth: true
            Accessible.name: i18n("Sections of the Apple keyboard settings")
            // Ctrl+PgDown / Ctrl+PgUp (and Ctrl+Tab) switch tabs, see Keys below
            QQC2.TabButton { text: i18nc("@title:tab", "State"); focusPolicy: Qt.StrongFocus }
            QQC2.TabButton { text: i18nc("@title:tab", "Keys"); focusPolicy: Qt.StrongFocus }
            QQC2.TabButton { text: i18nc("@title:tab", "Notifications"); focusPolicy: Qt.StrongFocus }
            QQC2.TabButton { text: i18nc("@title:tab", "Name"); focusPolicy: Qt.StrongFocus }
            QQC2.TabButton { text: i18nc("@title:tab", "Diagnostics"); focusPolicy: Qt.StrongFocus }
        }
    }

    // Only the current section is visible; hidden items take no room in the
    // layout, so each section scrolls on its own length.
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

    // Keyboard: next / previous section with Ctrl+PgDown / Ctrl+PgUp (and
    // Ctrl+Tab / Ctrl+Shift+Tab), from anywhere in the module. A QML Shortcut
    // would never fire here: the module lives in a QQuickWidget whose window
    // is not the active one, so the key events are handled as they bubble up.
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

    Component.onCompleted: {
        // Tab given on the command line: kcmshell6 kcm_applekeyboard --args keys
        const names = ["state", "keys", "notifications", "name", "diagnostics"];
        const a = (kcm.args || []).map(function (x) { return String(x).toLowerCase(); });
        for (let i = 0; i < names.length; ++i) {
            if (a.indexOf(names[i]) >= 0) tabs.currentIndex = i;
        }
        page.store.refreshAll();
        alerts.load();
    }
}
