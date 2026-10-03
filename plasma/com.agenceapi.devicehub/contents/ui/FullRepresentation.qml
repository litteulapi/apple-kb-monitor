import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.extras as PlasmaExtras
import "Pip.js" as Pip

// The popup of the notification-area icon, drawn as the screen of the
// ApiHub window (Pip-Boy art direction, docs/DA-PIPBOY.md): terminal header,
// tabs STAT RADIO KEYS DATA DIAG, alerts, status bar. Every function of the
// window that the daemon offers over D-Bus is here; the rest (table of the
// special keys, manual mapping) opens the window.
PlasmaExtras.Representation {
    id: full

    // Size of a Plasma applet popup; the tab body scrolls when taller.
    Layout.preferredWidth: Kirigami.Units.gridUnit * 26
    Layout.minimumWidth: Kirigami.Units.gridUnit * 22
    Layout.preferredHeight: Kirigami.Units.gridUnit * 36
    Layout.minimumHeight: Kirigami.Units.gridUnit * 24
    // The Pip-Boy screen fills the popup edge to edge, whatever the theme.
    collapseMarginsHint: true

    // 0 STAT, 1 RADIO, 2 KEYS, 3 DATA, 4 DIAG.
    property int tab: 0
    readonly property var tabNames: ["STAT", "RADIO", "KEYS", "DATA", "DIAG"]

    focus: true
    Accessible.role: Accessible.Pane
    Accessible.name: root.accessibleSummary

    // What a tab shows is read when it is opened, never polled.
    function openTab(i) {
        if (i < 0 || i > 4) return;
        full.tab = i;
        if (i === 3 && root.historyCount === 0 && !root.historyLoading) root.showHistory(root.historyDays);
        if (i === 4) {
            root.fetchDetails();
            if (root.diagChecks.length === 0) root.runDiagnose();
        }
        if (i === 1 && root.diagChecks.length === 0 && root.connected && isNaN(root.rssi)) root.runDiagnose();
    }

    // Window keys (DA-PIPBOY §6): 1-5 open a tab (physical keys, so the
    // unshifted AZERTY row works too), arrows switch tabs and scroll,
    // Escape goes back to STAT, F5 reads again what the tab shows.
    Keys.onPressed: function (event) {
        var digits = [Qt.Key_1, Qt.Key_2, Qt.Key_3, Qt.Key_4, Qt.Key_5];
        var azerty = [Qt.Key_Ampersand, Qt.Key_Eacute, Qt.Key_QuoteDbl, Qt.Key_Apostrophe, Qt.Key_ParenLeft];
        var i = digits.indexOf(event.key);
        if (i < 0) i = azerty.indexOf(event.key);
        if (i >= 0) {
            full.openTab(i);
            event.accepted = true;
        } else if (event.key === Qt.Key_Right) {
            full.openTab((full.tab + 1) % 5);
            event.accepted = true;
        } else if (event.key === Qt.Key_Left) {
            full.openTab((full.tab + 4) % 5);
            event.accepted = true;
        } else if (event.key === Qt.Key_Down || event.key === Qt.Key_PageDown) {
            pages.itemAt(full.tab).scrollBy(event.key === Qt.Key_Down ? 40 : pages.height * 0.8);
            event.accepted = true;
        } else if (event.key === Qt.Key_Up || event.key === Qt.Key_PageUp) {
            pages.itemAt(full.tab).scrollBy(event.key === Qt.Key_Up ? -40 : -pages.height * 0.8);
            event.accepted = true;
        } else if (event.key === Qt.Key_Escape && full.tab !== 0) {
            full.openTab(0);
            event.accepted = true;
        } else if (event.key === Qt.Key_F5) {
            if (full.tab === 3) root.showHistory(root.historyDays);
            else if (full.tab === 4) root.runDiagnose();
            else root.fetchDetails();
            event.accepted = true;
        }
    }

    contentItem: Item {
        id: screen
        implicitWidth: Kirigami.Units.gridUnit * 26
        implicitHeight: Kirigami.Units.gridUnit * 36

        // CRT tube: lighter in the middle, background at the edges.
        Rectangle {
            anchors.fill: parent
            gradient: Gradient {
                GradientStop { position: 0; color: Pip.BG }
                GradientStop { position: 0.5; color: Pip.BG_PANEL }
                GradientStop { position: 1; color: Pip.BG }
            }
        }
        // Bezel: frame line and four phosphor corner brackets.
        Rectangle {
            id: bezel
            anchors.fill: parent
            anchors.margins: 4
            color: "transparent"
            border.color: Pip.GREEN_FRAME
            border.width: 1
            Repeater {
                model: [[0, 0, 1, 1], [1, 0, -1, 1], [0, 1, 1, -1], [1, 1, -1, -1]]
                delegate: Item {
                    required property var modelData
                    x: modelData[0] * bezel.width
                    y: modelData[1] * bezel.height
                    Rectangle {
                        x: modelData[2] > 0 ? 0 : -14
                        y: modelData[3] > 0 ? 0 : -2
                        width: 14
                        height: 2
                        color: Pip.PHOSPHOR
                    }
                    Rectangle {
                        x: modelData[2] > 0 ? 0 : -2
                        y: modelData[3] > 0 ? 0 : -14
                        width: 2
                        height: 14
                        color: Pip.PHOSPHOR
                    }
                }
            }
        }

        ColumnLayout {
            id: shell
            anchors.fill: parent
            anchors.margins: Pip.GUTTER
            anchors.topMargin: Pip.GUTTER - 4
            anchors.bottomMargin: Pip.GUTTER - 4
            spacing: 4

            // ── Header ──
            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: false
                PipText {
                    Layout.fillWidth: true
                    text: "ROBCO INDUSTRIES (TM) TERMLINK PROTOCOL"
                    font.pixelSize: Pip.SMALL
                    color: Pip.GREEN_MID
                    wrapMode: Text.NoWrap
                    elide: Text.ElideRight
                }
                PipText {
                    text: root.daemonVersion !== "" ? "V" + root.daemonVersion : ""
                    font.pixelSize: Pip.SMALL
                    color: Pip.GREEN_MID
                    wrapMode: Text.NoWrap
                }
            }
            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: false
                spacing: Pip.GAP
                PipText {
                    Layout.fillWidth: true
                    text: "> " + i18n("Apple keyboard monitor").toUpperCase()
                    font.pixelSize: Pip.TITLE
                    glow: true
                    wrapMode: Text.NoWrap
                    elide: Text.ElideRight
                    Accessible.role: Accessible.Heading
                }
                Rectangle {
                    width: 10
                    height: 10
                    color: full.statusColor
                }
                PipText {
                    text: full.statusWord
                    color: full.statusColor
                    font.pixelSize: Pip.BODY
                    glow: true
                    wrapMode: Text.NoWrap
                }
            }
            // Double rule of the header.
            Column {
                Layout.fillWidth: true
                spacing: 2
                Rectangle { width: parent.width; height: 1; color: Pip.GREEN_FRAME }
                Rectangle { width: parent.width; height: 1; color: Pip.GREEN_FRAME }
            }

            // ── Tabs: the active one is framed and opens the rule ──
            Item {
                Layout.fillWidth: true
                Layout.topMargin: 4
                implicitHeight: Pip.TARGET + 6
                Rectangle {
                    anchors.bottom: parent.bottom
                    width: parent.width
                    height: 1
                    color: Pip.PHOSPHOR
                }
                Row {
                    id: tabRow
                    height: parent.height
                    Repeater {
                        model: full.tabNames
                        delegate: Item {
                            id: tabItem
                            required property int index
                            required property string modelData
                            readonly property bool active: full.tab === index
                            width: tabRow.parent.width / 5
                            height: tabRow.height
                            Accessible.role: Accessible.PageTab
                            Accessible.name: modelData
                            Accessible.selected: active
                            Accessible.onPressAction: full.openTab(index)
                            Rectangle {
                                anchors.fill: parent
                                anchors.bottomMargin: tabItem.active ? -1 : 0
                                visible: tabItem.active || tabMouse.containsMouse
                                color: tabItem.active ? Pip.BG : Qt.rgba(0.08, 1, 0, 0.10)
                                border.color: tabItem.active ? Pip.PHOSPHOR : "transparent"
                                border.width: 1
                                // Opens the rule under the active tab.
                                Rectangle {
                                    visible: tabItem.active
                                    anchors.bottom: parent.bottom
                                    x: 1
                                    width: parent.width - 2
                                    height: 2
                                    color: Pip.BG
                                }
                            }
                            Row {
                                anchors.centerIn: parent
                                spacing: 3
                                PipText {
                                    anchors.baseline: name.baseline
                                    text: String(tabItem.index + 1)
                                    font.pixelSize: Pip.SMALL
                                    color: tabItem.active ? Pip.PHOSPHOR : Pip.GREEN_MID
                                    wrapMode: Text.NoWrap
                                    Accessible.ignored: true
                                }
                                PipText {
                                    id: name
                                    text: tabItem.modelData
                                    font.pixelSize: Pip.TITLE
                                    color: tabItem.active ? Pip.PHOSPHOR : Pip.GREEN_MID
                                    glow: tabItem.active
                                    wrapMode: Text.NoWrap
                                    Accessible.ignored: true
                                }
                            }
                            MouseArea {
                                id: tabMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: {
                                    full.openTab(tabItem.index);
                                    full.forceActiveFocus();
                                }
                            }
                        }
                    }
                }
            }

            // ── Alerts, most urgent first; nothing when all is well ──
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: false
                Layout.topMargin: 4
                // Hidden when empty, so a removed alert leaves no gap.
                visible: full.alerts.length > 0
                spacing: 4
                Repeater {
                    model: full.alerts
                    delegate: PipAlert {
                        required property var modelData
                        text: modelData[0]
                        tint: modelData[1]
                    }
                }
            }

            // ── The tabs ──
            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.topMargin: 4
                Repeater {
                    id: pages
                    model: [tabStat, tabRadio, tabKeys, tabData, tabDiag]
                    delegate: PipScroll {
                        required property int index
                        required property var modelData
                        anchors.fill: parent
                        visible: full.tab === index
                        Loader {
                            width: parent.width
                            sourceComponent: modelData
                        }
                    }
                }
            }

            // ── Status bar: name, masked address, firmware, time of the reading ──
            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: Pip.GREEN_FRAME
            }
            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: false
                spacing: Pip.GAP
                PipText {
                    Layout.fillWidth: true
                    text: root.kbName !== "" ? root.kbName.toUpperCase() : (root.kbModel !== "" ? root.kbModel.toUpperCase() : "---")
                    font.pixelSize: Pip.BODY
                    color: Pip.PHOSPHOR
                    maximumLineCount: 2
                    elide: Text.ElideRight
                }
                PipButton {
                    text: i18n("Open window")
                    onClicked: root.openWindow()
                }
            }
            PipText {
                Layout.fillWidth: true
                text: Pip.maskMac(root.kbMac) + " | FW " + (root.fwVersion !== "" ? root.fwVersion : "---")
                      + " | " + i18n("READ %1", root.lastUpdate > 0 ? root.updatedText : "--:--")
                font.pixelSize: Pip.SMALL
                color: Pip.GREEN_MID
            }
            PipText {
                Layout.fillWidth: true
                visible: root.windowHint !== ""
                text: root.windowHint
                color: Pip.RED
                font.pixelSize: Pip.SMALL
            }
        }

        // Scanlines and vignette over everything (static, drawn on resize).
        Canvas {
            anchors.fill: parent
            onWidthChanged: requestPaint()
            onHeightChanged: requestPaint()
            Accessible.ignored: true
            onPaint: {
                var ctx = getContext("2d");
                ctx.reset();
                ctx.fillStyle = Qt.rgba(0.008, 0.07, 0.024, 0.26);
                for (var y = 0; y < height; y += Pip.SCANLINE_PITCH)
                    ctx.fillRect(0, y, width, 1);
                var depth = Math.min(36, width / 3, height / 3);
                var edges = [[0, 0, width, depth, 0, 0, 0, depth],
                             [0, height - depth, width, depth, 0, height, 0, height - depth],
                             [0, 0, depth, height, 0, 0, depth, 0],
                             [width - depth, 0, depth, height, width, 0, width - depth, 0]];
                for (var i = 0; i < 4; i++) {
                    var e = edges[i];
                    var g = ctx.createLinearGradient(e[4], e[5], e[6], e[7]);
                    g.addColorStop(0, Qt.rgba(0.008, 0.07, 0.024, 0.42));
                    g.addColorStop(1, Qt.rgba(0.008, 0.07, 0.024, 0));
                    ctx.fillStyle = g;
                    ctx.fillRect(e[0], e[1], e[2], e[3]);
                }
            }
        }
    }

    readonly property string statusWord: !root.daemonRunning ? i18n("NO DAEMON")
        : root.connected ? i18n("ONLINE") : i18n("OFFLINE")
    readonly property color statusColor: !root.daemonRunning ? Pip.AMBER
        : root.connected ? Pip.PHOSPHOR : Pip.RED

    // [text, colour], from the most to the least urgent (DA-PIPBOY §6).
    readonly property var alerts: {
        var a = [];
        if (!root.daemonRunning) {
            a.push([i18n("The monitor apple-kb-monitord is not on the session bus: nothing can be read."), Pip.AMBER]);
            return a;
        }
        if (root.lastError !== "") a.push([i18n("Reading error: %1", root.lastError), Pip.RED]);
        if (!root.connected) a.push([i18n("Keyboard disconnected. Press a key to wake it up, or reconnect it from RADIO."), Pip.RED]);
        else if (root.batteryPercent < 0) a.push([i18n("Waiting for the keyboard data…"), Pip.AMBER]);
        if (root.connected && (root.thresholdLevel === "critical" || root.thresholdLevel === "empty"))
            a.push([i18n("Batteries critical: replace them now."), Pip.RED]);
        else if (root.connected && root.thresholdLevel === "low")
            a.push([i18n("Batteries low: plan to replace them."), Pip.AMBER]);
        if (root.linkUnstable) a.push([i18n("Unstable link: %1 disconnections in the last hour.", root.discHour), Pip.AMBER]);
        return a;
    }

    Component { id: tabStat; TabStat { applet: root } }
    Component { id: tabRadio; TabRadio { applet: root } }
    Component { id: tabKeys; TabKeys { applet: root } }
    Component { id: tabData; TabData { applet: root } }
    Component { id: tabDiag; TabDiag { applet: root } }
}
