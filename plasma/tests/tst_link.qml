import QtQuick
import "../com.agenceapi.devicehub/contents/ui"
import "../com.agenceapi.devicehub/contents/ui/History.js" as Hist
import "../com.agenceapi.devicehub/contents/ui/FnMode.js" as Fn

Item {
    id: t
    property int stateCount: 0
    property bool activated: false
    property string alias: ""
    property real pct: -1
    property real volt: 0
    property int histCount: -1
    property int histVolts: -1
    property string histTag: ""
    property int fnBefore: -1
    property int fnAfter: -1
    property string version: ""
    property string health: ""
    property int refreshed: 0
    property int reconnected: 0
    property int checks: 0

    DaemonLink {
        id: link
        onRegisteredChanged: if (registered) fetch()
        onStateReceived: function (json) {
            var kb = JSON.parse(json).keyboard.battery;
            t.stateCount++;
            // The widget reads `percentage` (never percentage_fine) and the measured voltage.
            t.pct = kb.percentage;
            t.volt = kb.voltage;
        }
        onPanelRequested: t.activated = true
        onHistoryReceived: function (tag, json) {
            var now = Date.now() / 1000;
            var s = Hist.parse(json, now - 7 * 86400, now, Hist.POINTS_MAX);
            t.histTag = tag;
            t.histCount = s.count;
            t.histVolts = s.volt.length;
        }
        onHistoryFailed: function (tag, m) { console.log("FAIL history: " + m); Qt.exit(1); }
        onFnModeReceived: function (mode) {
            if (t.fnBefore < 0) {
                t.fnBefore = mode;
                link.setFnMode("AA:BB:CC:DD:EE:F1", Fn.next(mode));
            } else {
                t.fnAfter = mode;
            }
        }
        onFnModeSet: function (mode) { link.fetchFnMode("aa:bb:cc:dd:ee:f1"); }
        onFnModeFailed: function (m) { console.log("FAIL fnmode: " + m); Qt.exit(1); }
        onVersionReceived: function (v) { t.version = v; }
        onLinkStatusReceived: function (json) { t.health = JSON.parse(json)[0].health; }
        onRefreshDone: function (ok, m) { if (ok) t.refreshed++; }
        onReconnectDone: function (ok, m) { if (ok) t.reconnected++; }
        onDiagnoseReceived: function (json) { t.checks = JSON.parse(json).total; }
        onDiagnoseFailed: function (m) { console.log("FAIL diagnose: " + m); Qt.exit(1); }
        onAliasSet: function (n) { t.alias = n; }
        onAliasFailed: function (m) { console.log("FAIL alias: " + m); Qt.exit(1); }
        onFailed: function (m) { console.log("FAIL getstate: " + m); Qt.exit(1); }
    }

    Timer {
        interval: 3500
        running: true
        onTriggered: {
            link.setAlias("AA:BB", "My keyboard");
            link.fetchHistory(Date.now() / 1000 - 7 * 86400, "spark");
            link.fetchFnMode("AA:BB:CC:DD:EE:F1");
            link.fetchVersion();
            link.fetchLinkStatus();
            link.refresh();
            link.reconnect();
            link.diagnose();
            link.setFnMode("../../x", 2);
            finish.start();
        }
    }
    Timer {
        id: finish
        interval: 800
        onTriggered: {
            var ok = t.stateCount >= 3 && t.activated && t.alias === "My keyboard" && t.pct === 87 && Math.abs(t.volt - 2.987) < 1e-9;
            // 3 readings in the 7 days, 2 with a measured voltage; Fn 1 -> 2.
            var more = t.histTag === "spark" && t.histCount === 3 && t.histVolts === 2
                && t.fnBefore === 1 && t.fnAfter === 2 && t.version === "9.8.7"
                && t.health === "unreachable" && t.refreshed === 1 && t.reconnected === 1 && t.checks === 9;
            console.log("RESULT states=" + t.stateCount + " activated=" + t.activated + " pct=" + t.pct + " alias=" + t.alias + " volt=" + t.volt);
            console.log("RESULT history=" + t.histCount + "/" + t.histVolts + " tag=" + t.histTag + " fn=" + t.fnBefore + "->" + t.fnAfter
                + " version=" + t.version + " link=" + t.health + " refresh=" + t.refreshed + " reconnect=" + t.reconnected + " checks=" + t.checks);
            ok = ok && more;
            console.log(ok ? "PASS" : "FAIL");
            Qt.exit(ok ? 0 : 1);
        }
    }
}
