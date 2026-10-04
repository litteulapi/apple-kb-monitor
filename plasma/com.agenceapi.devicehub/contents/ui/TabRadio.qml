pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip
import "Signal.js" as Sig

ColumnLayout {
    id: page
    required property var applet
    readonly property SignalIssue signalIssue: SignalIssue { code: page.applet ? page.applet.rssiIssue : "" }
    property bool compact: false
    spacing: Pip.GAP * 1.5

    function ago(s) {
        if (s < 0) return "";
        if (s < 90) return i18np("%1 s ago", "%1 s ago", Math.round(s));
        if (s < 5400) return i18np("%1 min ago", "%1 min ago", Math.round(s / 60));
        if (s < 129600) return i18np("%1 h ago", "%1 h ago", Math.round(s / 3600));
        return i18np("%1 day ago", "%1 days ago", Math.round(s / 86400));
    }
    readonly property string qualityWord: applet.rssiQuality === "excellent" ? i18n("EXCELLENT")
        : applet.rssiQuality === "good" ? i18n("GOOD") : applet.rssiQuality === "weak" ? i18n("WEAK") : ""

    PipPanel {
        title: i18n("Signal")
        RowLayout {
            Layout.fillWidth: true
            PipText {
                Layout.fillWidth: true
                text: page.applet.hasRssi ? page.qualityWord + " (" + Sig.rawRssi(page.applet.rssi) + ")"
                    : page.applet.connected ? i18n("Not measured").toLocaleUpperCase() : "---"
                font.pixelSize: page.applet.hasRssi ? Pip.HERO / 2 : Pip.VALUE
                color: page.applet.hasRssi ? Pip.qualityColor(page.applet.rssiQuality) : (page.applet.connected ? Pip.AMBER : Pip.GREEN_MID)
                glow: page.applet.connected
            }
            PipBars {
                Layout.alignment: Qt.AlignBottom
                lit: Pip.qualityBars(page.applet.rssiQuality)
                color: Pip.qualityColor(page.applet.rssiQuality)
                barHeight: 30
            }
        }

        Item {
            id: dial
            Layout.fillWidth: true
            visible: page.applet.hasRssi
            implicitHeight: 74
            readonly property real x0: 14
            readonly property real w: width - 28
            function px(db) { return x0 + Pip.scaleX(db) * w; }
            Repeater {
                model: [[(-20 + -5) / 2, i18n("WEAK")], [-2.5, i18n("GOOD")], [2.5, i18n("EXCELLENT")]]
                delegate: PipText {
                    required property var modelData
                    x: dial.px(modelData[0]) - implicitWidth / 2
                    y: 0
                    text: modelData[1]
                    font.pixelSize: Pip.SMALL
                    color: Pip.GREEN_MID
                    wrapMode: Text.NoWrap
                }
            }
            Canvas {
                id: scaleCanvas
                anchors.fill: parent
                onWidthChanged: requestPaint()
                Connections {
                    target: page.applet
                    function onRssiChanged() { scaleCanvas.requestPaint(); }
                }
                onPaint: {
                    var ctx = getContext("2d");
                    ctx.reset();
                    var y = 44;
                    ctx.strokeStyle = Pip.GREEN_MID;
                    ctx.lineWidth = 1;
                    ctx.beginPath();
                    ctx.moveTo(dial.x0, y + 0.5);
                    ctx.lineTo(dial.x0 + dial.w, y + 0.5);
                    ctx.stroke();
                    for (var db = Pip.SCALE_MIN; db <= Pip.SCALE_MAX; db++) {
                        var x = Math.round(dial.px(db)) + 0.5;
                        var major = db % 5 === 0;
                        ctx.beginPath();
                        ctx.moveTo(x, y - (major ? 8 : 4));
                        ctx.lineTo(x, y);
                        ctx.stroke();
                    }
                    ctx.strokeStyle = Pip.GREEN_FRAME;
                    [-5, 0].forEach(function (b) {
                        var bx = Math.round(dial.px(b)) + 0.5;
                        ctx.beginPath();
                        ctx.moveTo(bx, 20);
                        ctx.lineTo(bx, y);
                        ctx.stroke();
                    });
                    if (!isNaN(page.applet.rssi)) {
                        var nx = dial.px(page.applet.rssi);
                        ctx.fillStyle = Qt.rgba(0.08, 1, 0, 0.25);
                        ctx.fillRect(nx - 3, 22, 6, y - 20);
                        ctx.fillStyle = Pip.PHOSPHOR;
                        ctx.fillRect(nx - 1, 22, 2, y - 20);
                        ctx.beginPath();
                        ctx.moveTo(nx - 6, 18);
                        ctx.lineTo(nx + 6, 18);
                        ctx.lineTo(nx, 26);
                        ctx.closePath();
                        ctx.fill();
                    }
                }
            }
            Repeater {
                model: [-20, -15, -10, -5, 0, 5]
                delegate: PipText {
                    required property var modelData
                    x: dial.px(modelData) - implicitWidth / 2
                    y: 50
                    text: modelData > 0 ? "+" + modelData : String(modelData)
                    font.pixelSize: Pip.SMALL
                    color: Pip.GREEN_MID
                    wrapMode: Text.NoWrap
                }
            }
        }
        PipKv {
            visible: page.applet.hasRssi
            label: i18n("Measured")
            value: page.applet.rssiAt > 0 ? i18n("at %1", Qt.formatTime(new Date(page.applet.rssiAt * 1000), Qt.locale(), Locale.LongFormat)) : ""
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.hasRssi
            text: i18n("dB relative to the ideal reception range of the adapter (0 = ideal), not dBm.")
            font.pixelSize: Pip.SMALL
            color: Pip.GREEN_MID
        }

        PipText {
            Layout.fillWidth: true
            visible: page.applet.connected && !page.applet.hasRssi
            text: page.signalIssue.reason
            color: Pip.AMBER
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.connected && !page.applet.hasRssi
            text: i18n("Correction: %1", page.signalIssue.fix)
            font.pixelSize: Pip.SMALL
            color: Pip.GREEN_MID
        }
        PipText {
            Layout.fillWidth: true
            visible: !page.applet.connected
            text: i18n("No measurement: the keyboard is not connected.")
            color: Pip.GREEN_MID
        }
    }

    PipPanel {
        title: i18n("Link")
        PipKv {
            label: i18n("Connected")
            value: page.applet.daemonRunning ? (page.applet.connected ? i18n("Yes") : i18n("No")) : ""
            valueColor: page.applet.connected ? Pip.PHOSPHOR : Pip.RED
        }
        PipKv {
            label: i18n("Paired")
            value: page.applet.connected ? (page.applet.paired ? i18n("Yes") : i18n("No")) : ""
        }
        PipKv {
            label: i18n("TX power")
            value: page.applet.connected && !isNaN(page.applet.txPower) ? i18n("%1 dBm", page.applet.txPower) : ""
        }
        PipKv {
            label: i18n("Last wake-up")
            value: page.applet.connected && page.applet.wakeAge >= 0 ? page.ago(page.applet.wakeAge) : ""
        }
        PipKv {
            label: i18n("Paired host")
            value: page.applet.pairedHost !== "" ? Pip.maskMac(page.applet.pairedHost) : ""
        }
        PipKv {
            label: i18n("Monitor's link")
            value: page.applet.linkHealth === "" ? ""
                : (page.applet.linkHealth === "connected" ? i18n("connected")
                    : page.applet.linkHealth === "dormant" ? i18n("asleep")
                    : page.applet.linkHealth === "unreachable" ? i18n("unreachable") : page.applet.linkHealth)
            valueColor: page.applet.linkHealth === "unreachable" ? Pip.RED
                : page.applet.linkHealth === "dormant" ? Pip.AMBER : Pip.PHOSPHOR
        }
        PipKv {
            visible: page.applet.linkHealth !== ""
            label: i18n("Attempts / failures")
            value: i18n("%1 / %2", page.applet.linkAttempts, page.applet.linkFailures)
            valueColor: page.applet.linkFailures > 0 ? Pip.AMBER : Pip.PHOSPHOR
        }
        PipKv {
            visible: page.applet.linkError !== ""
            label: i18n("Last error")
            value: page.applet.linkError
            valueColor: Pip.RED
        }
    }

    PipPanel {
        title: i18n("Link quality")
        PipText {
            Layout.fillWidth: true
            text: i18n("Disconnections")
            font.pixelSize: Pip.SMALL
            color: Pip.GREEN_MID
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Pip.GAP
            Repeater {
                model: [[i18n("1 h"), page.applet.discHour], [i18n("24 h"), page.applet.discDay], [i18n("7 days"), page.applet.disc7d]]
                delegate: PipCell {
                    required property var modelData
                    label: modelData[0]
                    value: modelData[1] < 0 ? "" : String(modelData[1])
                    valueColor: modelData[1] === 0 ? Pip.PHOSPHOR : (page.applet.linkUnstable ? Pip.RED : Pip.AMBER)
                }
            }
        }
        Item {
            id: week
            Layout.fillWidth: true
            visible: page.applet.discByDay.length > 0
            implicitHeight: 64
            readonly property int peak: Math.max.apply(null, [1].concat(page.applet.discByDay))
            Row {
                anchors.fill: parent
                anchors.bottomMargin: 20
                spacing: 6
                Repeater {
                    model: page.applet.discByDay
                    delegate: Item {
                        id: day
                        required property int index
                        required property var modelData
                        width: (week.width - 6 * Math.max(0, page.applet.discByDay.length - 1)) / Math.max(1, page.applet.discByDay.length)
                        height: parent.height
                        Rectangle {
                            anchors.bottom: parent.bottom
                            width: parent.width
                            height: Math.max(2, parent.height * Number(day.modelData) / week.peak)
                            color: Number(day.modelData) === 0 ? Pip.GREEN_FRAME : Pip.AMBER
                        }
                        PipText {
                            anchors.horizontalCenter: parent.horizontalCenter
                            y: parent.height + 2
                            text: day.index === page.applet.discByDay.length - 1 ? i18n("today") : i18n("D-%1", page.applet.discByDay.length - 1 - day.index)
                            font.pixelSize: Pip.SMALL - 2
                            color: Pip.GREEN_MID
                            wrapMode: Text.NoWrap
                        }
                    }
                }
            }
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.disc7d < 0
            text: i18n("No disconnection recorded by the monitor yet.")
            color: Pip.GREEN_MID
            font.pixelSize: Pip.SMALL
        }
    }

    PipPanel {
        title: i18n("Restore the link")
        PipButton {
            text: i18n("Reconnect")
            enabled: page.applet.daemonRunning
            onClicked: page.applet.requestReconnect()
        }
        PipText {
            Layout.fillWidth: true
            visible: page.applet.diagHint !== ""
            text: page.applet.diagHint
            font.pixelSize: Pip.SMALL
        }
        PipText {
            Layout.fillWidth: true
            text: i18n("Pairing lost or refused? Pair it again from a terminal (a confirmation is asked): akmctl repair")
            font.pixelSize: Pip.SMALL
            color: Pip.GREEN_MID
        }
    }
}
