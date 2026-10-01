import QtQuick
import "../com.agenceapi.devicehub/contents/ui"

// Headless test of DaemonLink against fake_services.py (see run-widget-tests.sh).
Item {
    id: t
    property int states: 0
    property bool activated: false
    property string alias: ""
    property real pct: -1
    property real volt: 0

    DaemonLink {
        id: link
        claimId: "42"
        onRegisteredChanged: if (registered) fetch()
        onStateReceived: function (json) {
            var kb = JSON.parse(json).keyboard.battery;
            t.states++;
            // The widget reads `percentage` (never percentage_fine) and the measured voltage.
            t.pct = kb.percentage;
            t.volt = kb.voltage;
        }
        onWindowActivated: t.activated = true
        onAliasSet: function (n) { t.alias = n; }
        onAliasFailed: function (m) { console.log("FAIL alias: " + m); Qt.exit(1); }
        onWindowFailed: function (m) { console.log("FAIL window: " + m); Qt.exit(1); }
        onFailed: function (m) { console.log("FAIL getstate: " + m); Qt.exit(1); }
    }

    Timer {
        interval: 3500
        running: true
        onTriggered: {
            link.activateWindow();
            link.setAlias("AA:BB", "Mon clavier");
            link.releaseTray();
            finish.start();
        }
    }
    Timer {
        id: finish
        interval: 800
        onTriggered: {
            // 1 initial fetch + >= 2 StateChanged-driven fetches in 3.5 s (signal every 0.7 s)
            var ok = t.states >= 3 && t.activated && t.alias === "Mon clavier" && t.pct === 87 && Math.abs(t.volt - 2.987) < 1e-9;
            console.log("RESULT states=" + t.states + " activated=" + t.activated + " pct=" + t.pct + " alias=" + t.alias + " volt=" + t.volt);
            console.log(ok ? "PASS" : "FAIL");
            Qt.exit(ok ? 0 : 1);
        }
    }
}
