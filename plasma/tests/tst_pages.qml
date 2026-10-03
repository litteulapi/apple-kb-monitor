import QtQuick
import QtQuick.Layouts
import "../com.agenceapi.devicehub/contents/ui"
import "../com.agenceapi.devicehub/contents/ui/History.js" as Hist

// The Pip-Boy popup (tabs STAT RADIO KEYS DATA DIAG, #97 #120) is
// instantiated offscreen against a stand-in for the applet: every property
// and function the tabs use exists here, so a binding on a missing name shows
// up as a ReferenceError in the log (run-widget-tests.sh fails on it). Each
// tab is opened in the popup and also loaded alone. No D-Bus.
Item {
    id: root
    width: 420
    height: 700

    property bool expanded: true
    property bool daemonRunning: true
    property bool connected: true
    property int batteryPercent: 87
    property string batteryText: "87%"
    property string accessibleSummary: "Apple keyboard, battery 87 percent"
    property string toolTipSubText: ""
    property bool hasBattery: true
    property real voltage: 2.987
    property real curvePercent: 71.2
    property string batteryType: "Alkaline (fresh)"
    property string rssiText: "good (−3)"
    property bool hasEstimate: true
    property bool newBatteries: false
    property string estimateText: "≈ 70% (60 to 80%), alkaline"
    property string updatedText: "12:34"
    property string stateText: "Connected"
    property string kbModel: "Apple Wireless Keyboard (A1314)"
    property string kbName: "Clavier de alice"
    property string renameError: ""
    property string fwVersion: "0x0050"
    property string fwStatus: "up_to_date"
    property string fwText: "0x0050 — up to date"
    property real applePct: 90
    property string thresholdsText: "Full 3000 / Low 2300 / Critical 2100 / Empty 2000 mV"
    property string remaining: "about 60 days"
    property string lastError: ""
    property string windowHint: ""
    property bool readIncomplete: true

    property var sparkPct: []
    property real sparkFrom: 0
    property real sparkTo: 1
    property int historyDays: 7
    property var historyPct: []
    property var historyVolt: []
    property int historyCount: 0
    property int historySpanDays: 0
    property real historyFrom: 0
    property real historyTo: 1
    property real historyPctMin: 0
    property real historyPctMax: 100
    property real historyVoltMin: 0
    property real historyVoltMax: 1
    property bool historyLoading: false
    property string historyError: ""

    property int fnMode: 1
    property bool fnBusy: false
    property string fnError: ""
    property string fnModeText: "media keys first"
    property string fnToggleText: "Switch to F1–F12 first"

    property string daemonVersion: "3.1.0"
    property string linkHealth: "connected"
    property string linkText: "connected (attempts: 0, failures: 0)"
    property string linkError: ""
    property string diagHint: ""
    property int linkAttempts: 0
    property int linkFailures: 0
    property string kbMac: "AA:BB:CC:DD:EE:F1"
    property real rssi: -3
    property string rssiQuality: "good"
    property bool hasRssi: true
    property real lastUpdate: 1790000000
    property string estimateChem: "alkaline"
    property bool capsLock: true
    property bool numLock: false
    property bool paired: true
    property string pairedHost: "AA:BB:CC:DD:EE:F2"
    property string kbChip: "BCM2042"
    property string kbDriver: "hid-apple"
    property real txPower: 4
    property real rssiAt: 1790000000
    property real wakeAge: 120
    property string thresholdLevel: "ok"
    property var thresholds: ({ full_mv: 2954, low_mv: 2506, critical_mv: 2404, empty_mv: 2054 })
    property string fwSource: "table"
    property string fwLatest: "0x0050"
    property real installedAt: 1780000000
    property real forecastEmptyAt: Date.now() / 1000 + 41 * 86400
    property real forecastRate: 0.8
    property int discHour: 0
    property int discDay: 2
    property int disc7d: 5
    property var discByDay: [0, 1, 0, 2, 0, 0, 2]
    property bool linkUnstable: false
    property string kbError: ""
    property var diagChecks: []
    property int diagPassed: 0
    property int diagTotal: 0
    property bool diagRunning: false
    property string diagError: ""
    property int rssiHelperOk: -1
    property int diagnoses: 0

    property int toggles: 0
    property int shown: 0
    property int refreshes: 0
    property int reconnects: 0
    function fetchDetails() {}
    function toggleFnMode() { toggles++; }
    function showHistory(days) { historyDays = days; shown++; }
    function requestRefresh() { refreshes++; }
    function requestReconnect() { reconnects++; }
    function renameKeyboard(name) {}
    function openWindow() {}
    function runDiagnose() { diagnoses++; }

    FontLoader {
        id: vt
        source: "../com.agenceapi.devicehub/contents/fonts/VT323-Regular.ttf"
    }

    Loader {
        id: popup
        anchors.fill: parent
        source: "../com.agenceapi.devicehub/contents/ui/FullRepresentation.qml"
    }
    TabStat { id: stat; applet: root; width: 400 }
    TabRadio { id: radio; applet: root; width: 400 }
    TabKeys { id: keys; applet: root; width: 400 }
    TabData { id: data; applet: root; width: 400 }
    TabDiag { id: diag; applet: root; width: 400 }
    LineChart {
        id: chart
        width: 300
        height: 60
        primary: root.historyPct
        secondary: root.historyVolt
        tMin: root.historyFrom
        tMax: root.historyTo
    }

    function check(name, ok) {
        if (!ok) { console.log("FAIL pages: " + name); Qt.exit(1); }
    }

    Timer {
        interval: 300
        running: true
        onTriggered: {
            root.check("popup loaded (status " + popup.status + ")", popup.status === Loader.Ready);
            root.check("VT323 font loaded", vt.status === FontLoader.Ready && vt.font.family === "VT323");
            var tabs = [stat, radio, keys, data, diag];
            for (var t = 0; t < tabs.length; t++)
                root.check("tab " + t + " has a size", tabs[t].implicitHeight > 200);
            // Every tab opens in the popup; DATA loads the history, DIAG runs the checks.
            for (var o = 4; o >= 0; o--) {
                popup.item.openTab(o);
                root.check("popup shows tab " + o, popup.item.tab === o);
            }
            root.check("DATA asked for the history", root.shown === 1);
            root.check("DIAG ran the checks", root.diagnoses === 1);
            // Diagnose result: count and log.
            root.diagChecks = [{ id: "binary", label: "Daemon binary", ok: true, detail: "apple-kb-monitord 3.1.0" },
                               { id: "rssi_helper", label: "RSSI helper", ok: false, detail: "missing" }];
            root.diagPassed = 1;
            root.diagTotal = 2;
            root.check("diag log listed", diag.implicitHeight > 300);
            // 90 days of readings reach the chart as a bounded series.
            var now = Date.now() / 1000, all = [];
            for (var i = 0; i < 20000; i++)
                all.push({ ts: now - 90 * 86400 * (1 - i / 20000), pct: 100 - i / 400, voltage: 3 - i / 40000 });
            var h = Hist.series(all, now - 90 * 86400, now, Hist.POINTS_MAX);
            root.historyDays = 90;
            root.historyCount = h.count;
            root.historySpanDays = Hist.spanDays(h);
            root.historyFrom = h.tMin;
            root.historyTo = h.tMax;
            root.historyVoltMin = h.voltMin;
            root.historyVoltMax = h.voltMax;
            root.historyPct = h.pct;
            root.historyVolt = h.volt;
            root.sparkPct = h.pct;
            root.check("chart gets the bounded series", chart.primary.length <= Hist.POINTS_MAX && chart.primary.length > 100);
            root.check("DATA shows the chart", data.hasChart && data.hasVoltage);
            // Daemon gone, then keyboard gone: the pages still evaluate.
            root.connected = false;
            root.daemonRunning = false;
            root.daemonRunning = true;
            root.historyError = "org.freedesktop.DBus.Error.NoReply";
            root.linkHealth = "unreachable";
            // No signal while connected: RADIO says why.
            root.connected = true;
            root.hasRssi = false;
            root.rssi = NaN;
            root.discHour = -1;
            root.discByDay = [];
            root.thresholds = null;
            root.fnMode = -1;
            done.start();
        }
    }
    Timer {
        id: done
        interval: 300
        onTriggered: {
            console.log("PASS pages");
            Qt.exit(0);
        }
    }
}
