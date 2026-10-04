pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.extras as PlasmaExtras
import "Pip.js" as Pip

PlasmaExtras.Representation {
    id: full
    required property var applet

    Layout.preferredWidth: Kirigami.Units.gridUnit * 26
    Layout.minimumWidth: Kirigami.Units.gridUnit * 22
    Layout.preferredHeight: Kirigami.Units.gridUnit * 34
    Layout.minimumHeight: Kirigami.Units.gridUnit * 20
    collapseMarginsHint: true

    property int tab: 0
    readonly property bool compact: screen.height < Kirigami.Units.gridUnit * 30
    readonly property var tabNames: [i18n("State"), i18n("Radio"), i18n("Keys"), i18n("Data"), i18n("Diag")].map(function (t) { return t.toLocaleUpperCase(); })

    focus: true
    Accessible.role: Accessible.Pane
    Accessible.name: full.applet.accessibleSummary

    // What a tab shows is read when it is opened, never polled.
    function openTab(i) {
        if (i < 0 || i > 4) return;
        full.tab = i;
        if (i === 3 && full.applet.historyCount === 0 && !full.applet.historyLoading) full.applet.showHistory(full.applet.historyDays);
        if (i === 4) {
            full.applet.fetchDetails();
            if (full.applet.diagChecks.length === 0) full.applet.runDiagnose();
        }
        if (i === 1 && full.applet.diagChecks.length === 0 && full.applet.connected && isNaN(full.applet.rssi)) full.applet.runDiagnose();
    }

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
            (pages.itemAt(full.tab) as PipScroll).scrollBy(event.key === Qt.Key_Down ? 40 : pages.height * 0.8);
            event.accepted = true;
        } else if (event.key === Qt.Key_Up || event.key === Qt.Key_PageUp) {
            (pages.itemAt(full.tab) as PipScroll).scrollBy(event.key === Qt.Key_Up ? -40 : -pages.height * 0.8);
            event.accepted = true;
        } else if (event.key === Qt.Key_Escape && full.tab !== 0) {
            full.openTab(0);
            event.accepted = true;
        } else if (event.key === Qt.Key_F5) {
            if (full.tab === 3) full.applet.showHistory(full.applet.historyDays);
            else if (full.tab === 4) full.applet.runDiagnose();
            else full.applet.fetchDetails();
            event.accepted = true;
        }
    }

    contentItem: Item {
        id: screen
        implicitWidth: Kirigami.Units.gridUnit * 26
        implicitHeight: Kirigami.Units.gridUnit * 36

        Rectangle {
            anchors.fill: parent
            gradient: Gradient {
                GradientStop { position: 0; color: Pip.BG }
                GradientStop { position: 0.5; color: Pip.BG_PANEL }
                GradientStop { position: 1; color: Pip.BG }
            }
        }
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
                    id: corner
                    required property var modelData
                    x: modelData[0] * bezel.width
                    y: modelData[1] * bezel.height
                    Rectangle {
                        x: corner.modelData[2] > 0 ? 0 : -14
                        y: corner.modelData[3] > 0 ? 0 : -2
                        width: 14
                        height: 2
                        color: Pip.PHOSPHOR
                    }
                    Rectangle {
                        x: corner.modelData[2] > 0 ? 0 : -2
                        y: corner.modelData[3] > 0 ? 0 : -14
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

            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: false
                visible: !full.compact
                PipText {
                    Layout.fillWidth: true
                    text: "APIHUB — AGENCE API"
                    font.pixelSize: Pip.SMALL
                    color: Pip.GREEN_MID
                    wrapMode: Text.NoWrap
                    elide: Text.ElideRight
                }
                PipText {
                    text: full.applet.daemonVersion !== "" ? "V" + full.applet.daemonVersion : ""
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
                    text: "> " + i18n("Apple keyboard monitor").toLocaleUpperCase()
                    font.pixelSize: Pip.TITLE
                    glow: true
                    wrapMode: Text.NoWrap
                    elide: Text.ElideRight
                    Accessible.role: Accessible.Heading
                }
                Rectangle {
                    implicitWidth: 10
                    implicitHeight: 10
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
            Column {
                Layout.fillWidth: true
                spacing: 2
                Rectangle { width: parent.width; height: 1; color: Pip.GREEN_FRAME }
                Rectangle { width: parent.width; height: 1; color: Pip.GREEN_FRAME }
            }

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

            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: false
                Layout.topMargin: 4
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

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.topMargin: 4
                Repeater {
                    id: pages
                    model: [tabStat, tabRadio, tabKeys, tabData, tabDiag]
                    delegate: PipScroll {
                        id: tabPage
                        required property int index
                        required property var modelData
                        anchors.fill: parent
                        visible: full.tab === index
                        Loader {
                            width: parent.width
                            sourceComponent: tabPage.modelData
                        }
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 1
                color: Pip.GREEN_FRAME
            }
            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: false
                spacing: Pip.GAP
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    PipText {
                        Layout.fillWidth: true
                        text: full.kbLabel
                        font.pixelSize: full.compact ? Pip.SMALL : Pip.BODY
                        color: Pip.PHOSPHOR
                        maximumLineCount: full.compact ? 1 : 2
                        elide: Text.ElideRight
                    }
                    PipText {
                        Layout.fillWidth: true
                        visible: full.compact
                        text: full.facts
                        font.pixelSize: Pip.SMALL
                        color: Pip.GREEN_MID
                        maximumLineCount: 1
                        elide: Text.ElideRight
                    }
                }
                PipButton {
                    text: i18n("Settings")
                    onClicked: full.applet.openSettings("")
                }
            }
            PipText {
                Layout.fillWidth: true
                visible: !full.compact
                text: Pip.maskMac(full.applet.kbMac) + " | " + full.facts
                font.pixelSize: Pip.SMALL
                color: Pip.GREEN_MID
            }
        }

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

    readonly property string kbLabel: full.applet.kbName !== "" ? full.applet.kbName.toLocaleUpperCase()
        : (full.applet.kbModel !== "" ? full.applet.kbModel.toLocaleUpperCase() : "---")
    readonly property string facts: "FW " + (full.applet.fwVersion !== "" ? full.applet.fwVersion : "---")
        + " | " + i18n("READ %1", full.applet.lastUpdate > 0 ? full.applet.updatedText : "--:--")
    readonly property string statusWord: !full.applet.daemonRunning ? i18n("NO DAEMON")
        : full.applet.connected ? i18n("ONLINE") : i18n("OFFLINE")
    readonly property color statusColor: !full.applet.daemonRunning ? Pip.AMBER
        : full.applet.connected ? Pip.PHOSPHOR : Pip.RED

    readonly property var alerts: {
        var a = [];
        if (!full.applet.daemonRunning) {
            a.push([i18n("The monitor apple-kb-monitord is not on the session bus: nothing can be read."), Pip.AMBER]);
            return a;
        }
        if (full.applet.lastError !== "") a.push([i18n("Reading error: %1", full.applet.lastError), Pip.RED]);
        if (!full.applet.connected) a.push([i18n("Keyboard disconnected. Press a key to wake it up, or reconnect it from RADIO."), Pip.RED]);
        else if (full.applet.batteryPercent < 0) a.push([i18n("Waiting for the keyboard data…"), Pip.AMBER]);
        if (full.applet.connected && (full.applet.thresholdLevel === "critical" || full.applet.thresholdLevel === "empty"))
            a.push([i18n("Batteries critical: replace them now."), Pip.RED]);
        else if (full.applet.connected && full.applet.thresholdLevel === "low")
            a.push([i18n("Batteries low: plan to replace them."), Pip.AMBER]);
        if (full.applet.linkUnstable) a.push([i18np("Unstable link: %1 disconnection in the last hour.", "Unstable link: %1 disconnections in the last hour.", full.applet.discHour), Pip.AMBER]);
        return a;
    }

    Component { id: tabStat; TabStat { applet: full.applet; compact: full.compact } }
    Component { id: tabRadio; TabRadio { applet: full.applet; compact: full.compact } }
    Component { id: tabKeys; TabKeys { applet: full.applet; compact: full.compact } }
    Component { id: tabData; TabData { applet: full.applet; compact: full.compact } }
    Component { id: tabDiag; TabDiag { applet: full.applet; compact: full.compact } }
}
