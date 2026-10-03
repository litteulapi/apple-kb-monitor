import QtQuick
import QtQuick.Layouts
import "Pip.js" as Pip
import "Signal.js" as Sig

// RADIO: signal in words and bars on a graduated scale with a needle (dB
// relative to the ideal range, NOT dBm, #174), or why there is no signal;
// facts of the link; link quality (disconnections over 1 h / 24 h / 7 days);
// reconnection by the daemon.
ColumnLayout {
    id: page
    property var applet
    spacing: Pip.GAP * 1.5

    function num(v, digits) {
        return Number(v).toLocaleString(Qt.locale(), "f", digits);
    }
    function ago(s) {
        if (s < 0) return "";
        if (s < 90) return i18n("%1 s ago", Math.round(s));
        if (s < 5400) return i18n("%1 min ago", Math.round(s / 60));
        if (s < 129600) return i18n("%1 h ago", Math.round(s / 3600));
        return i18n("%1 days ago", Math.round(s / 86400));
    }
    readonly property string qualityWord: applet.rssiQuality === "excellent" ? i18n("EXCELLENT")
        : applet.rssiQuality === "good" ? i18n("GOOD") : applet.rssiQuality === "weak" ? i18n("WEAK") : ""

    PipPanel {
        title: i18n("Signal")
        RowLayout {
            Layout.fillWidth: true
            PipText {
                Layout.fillWidth: true
                text: applet.hasRssi ? page.qualityWord + " (" + Sig.rawRssi(applet.rssi) + ")"
                    : applet.connected ? i18n("SIGNAL UNAVAILABLE") : "---"
                font.pixelSize: applet.hasRssi ? Pip.HERO / 2 : Pip.VALUE
                color: applet.hasRssi ? Pip.qualityColor(applet.rssiQuality) : (applet.connected ? Pip.AMBER : Pip.GREEN_MID)
                glow: applet.connected
            }
            PipBars {
                Layout.alignment: Qt.AlignBottom
                lit: Pip.qualityBars(applet.rssiQuality)
                color: Pip.qualityColor(applet.rssiQuality)
                barHeight: 30
            }
        }

        // Scale -20..+5 dB with the weak / good / excellent zones and a needle.
        Item {
            id: dial
            Layout.fillWidth: true
            visible: applet.hasRssi
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
                    // Zone boundaries.
                    ctx.strokeStyle = Pip.GREEN_FRAME;
                    [-5, 0].forEach(function (b) {
                        var bx = Math.round(dial.px(b)) + 0.5;
                        ctx.beginPath();
                        ctx.moveTo(bx, 20);
                        ctx.lineTo(bx, y);
                        ctx.stroke();
                    });
                    // Needle with its halo.
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
            visible: applet.hasRssi
            label: i18n("Measured")
            value: applet.rssiAt > 0 ? i18n("at %1", Qt.formatTime(new Date(applet.rssiAt * 1000), "HH:mm:ss")) : ""
        }
        PipText {
            Layout.fillWidth: true
            visible: applet.hasRssi
            text: i18n("dB relative to the ideal reception range of the adapter (0 = ideal), not dBm.")
            font.pixelSize: Pip.SMALL
            color: Pip.GREEN_MID
        }

        // No signal while connected: say why and how to fix it.
        PipText {
            Layout.fillWidth: true
            visible: applet.connected && !applet.hasRssi
            text: applet.rssiHelperOk === 0
                ? i18n("The RSSI helper is not installed (/usr/lib/apple-kb-monitor/rssi-helper, checked by DIAG): reinstall the package.")
                : i18n("The monitor gets no measurement from its RSSI helper. Usual cause: your user is not in the akm group (the monitor's log then says \"rssi-helper permission denied\").")
            color: Pip.AMBER
        }
        Rectangle {
            Layout.fillWidth: true
            visible: applet.connected && !applet.hasRssi && applet.rssiHelperOk !== 0
            implicitHeight: fix.implicitHeight + 12
            color: Pip.BG
            border.color: Pip.GREEN_FRAME
            border.width: 1
            PipText {
                id: fix
                x: 8
                y: 6
                width: parent.width - 16
                text: "sudo usermod -aG akm $USER"
                font.pixelSize: Pip.BODY
            }
        }
        PipText {
            Layout.fillWidth: true
            visible: applet.connected && !applet.hasRssi && applet.rssiHelperOk !== 0
            text: i18n("Then close the session and open it again: the group is read at login.")
            font.pixelSize: Pip.SMALL
            color: Pip.GREEN_MID
        }
        PipText {
            Layout.fillWidth: true
            visible: !applet.connected
            text: i18n("No measurement: the keyboard is not connected.")
            color: Pip.GREEN_MID
        }
    }

    PipPanel {
        title: i18n("Link")
        PipKv {
            label: i18n("Connected")
            value: applet.daemonRunning ? (applet.connected ? i18n("Yes") : i18n("No")) : ""
            valueColor: applet.connected ? Pip.PHOSPHOR : Pip.RED
        }
        PipKv {
            label: i18n("Paired")
            value: applet.connected ? (applet.paired ? i18n("Yes") : i18n("No")) : ""
        }
        PipKv {
            label: i18n("TX power")
            value: applet.connected && !isNaN(applet.txPower) ? i18n("%1 dBm", applet.txPower) : ""
        }
        PipKv {
            label: i18n("Last wake-up")
            value: applet.connected && applet.wakeAge >= 0 ? page.ago(applet.wakeAge) : ""
        }
        PipKv {
            label: i18n("Paired host")
            value: applet.pairedHost
        }
        PipKv {
            label: i18n("Monitor's link")
            value: applet.linkHealth === "" ? ""
                : (applet.linkHealth === "connected" ? i18n("connected")
                    : applet.linkHealth === "dormant" ? i18n("asleep")
                    : applet.linkHealth === "unreachable" ? i18n("unreachable") : applet.linkHealth)
            valueColor: applet.linkHealth === "unreachable" ? Pip.RED
                : applet.linkHealth === "dormant" ? Pip.AMBER : Pip.PHOSPHOR
        }
        PipKv {
            visible: applet.linkHealth !== ""
            label: i18n("Attempts / failures")
            value: i18n("%1 / %2", applet.linkAttempts, applet.linkFailures)
            valueColor: applet.linkFailures > 0 ? Pip.AMBER : Pip.PHOSPHOR
        }
        PipKv {
            visible: applet.linkError !== ""
            label: i18n("Last error")
            value: applet.linkError
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
                model: [[i18n("1 h"), applet.discHour], [i18n("24 h"), applet.discDay], [i18n("7 days"), applet.disc7d]]
                delegate: PipCell {
                    required property var modelData
                    label: modelData[0]
                    value: modelData[1] < 0 ? "" : String(modelData[1])
                    valueColor: modelData[1] === 0 ? Pip.PHOSPHOR : (applet.linkUnstable ? Pip.RED : Pip.AMBER)
                }
            }
        }
        // One bar per day over 7 days, oldest first.
        Item {
            id: week
            Layout.fillWidth: true
            visible: applet.discByDay.length > 0
            implicitHeight: 64
            readonly property int peak: Math.max.apply(null, [1].concat(applet.discByDay))
            Row {
                anchors.fill: parent
                anchors.bottomMargin: 20
                spacing: 6
                Repeater {
                    model: applet.discByDay
                    delegate: Item {
                        required property int index
                        required property var modelData
                        width: (week.width - 6 * Math.max(0, applet.discByDay.length - 1)) / Math.max(1, applet.discByDay.length)
                        height: parent.height
                        Rectangle {
                            anchors.bottom: parent.bottom
                            width: parent.width
                            height: Math.max(2, parent.height * Number(modelData) / week.peak)
                            color: Number(modelData) === 0 ? Pip.GREEN_FRAME : Pip.AMBER
                        }
                        PipText {
                            anchors.horizontalCenter: parent.horizontalCenter
                            y: parent.height + 2
                            text: index === applet.discByDay.length - 1 ? i18n("today") : i18n("D-%1", applet.discByDay.length - 1 - index)
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
            visible: applet.disc7d < 0
            text: i18n("No disconnection recorded by the monitor yet.")
            color: Pip.GREEN_MID
            font.pixelSize: Pip.SMALL
        }
    }

    PipPanel {
        title: i18n("Restore the link")
        PipButton {
            text: i18n("Reconnect")
            enabled: applet.daemonRunning
            onClicked: applet.requestReconnect()
        }
        PipText {
            Layout.fillWidth: true
            visible: applet.diagHint !== ""
            text: applet.diagHint
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
