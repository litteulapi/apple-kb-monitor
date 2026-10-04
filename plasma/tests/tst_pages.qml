import QtQuick
import QtQuick.Layouts
import "../com.agenceapi.devicehub/contents/ui"
import "../com.agenceapi.devicehub/contents/ui/History.js" as Hist
import "../com.agenceapi.devicehub/contents/ui/Pip.js" as Pip

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
    property string batteryType: "alkaline"
    property string rssiText: "good (−3)"
    property bool hasEstimate: true
    property bool newBatteries: false
    property string estimateText: "≈ 70% (60 to 80%), alkaline"
    property string updatedText: "12:34"
    property string stateText: "Connected"
    property string kbModel: "Apple Wireless Keyboard (A1314)"
    property string kbName: "Alice's keyboard"
    property string renameError: ""
    property string fwVersion: "0x0050"
    property string fwStatus: "up_to_date"
    property string fwText: "0x0050 — up to date"
    property real applePct: 90
    property string thresholdsText: "Full 3000 / Low 2300 / Critical 2100 / Empty 2000 mV"
    property string remaining: "about 60 days"
    property string lastError: ""
    property bool readIncomplete: true

    property var sparkPct: []
    property real sparkFrom: 0
    property real sparkTo: 1
    property int historyDays: 7
    property var historyPct: []
    property var historyVolt: []
    property int historyCount: 0
    property real historyFrom: 0
    property real historyTo: 1
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
    property string fwLatest: "0x0050"
    property real installedAt: 1780000000
    property real forecastEmptyAt: Date.now() / 1000 + 41 * 86400
    property real forecastRate: 0.8
    property real forecastSpanDays: 12.4
    property int discHour: 0
    property int discDay: 2
    property int disc7d: 5
    property var discByDay: [0, 1, 0, 2, 0, 0, 2]
    property bool linkUnstable: false
    property var diagChecks: []
    property int diagPassed: 0
    property int diagTotal: 0
    property bool diagRunning: false
    property string diagError: ""
    property string rssiIssue: ""
    property int diagnoses: 0

    property int toggles: 0
    property int shown: 0
    property int refreshes: 0
    property int reconnects: 0
    function fetchDetails() {}
    function toggleFnMode() { toggles++; }
    property int fnAsked: -1
    function setFnModeTo(m) { fnAsked = m; }
    function showHistory(days) { historyDays = days; shown++; }
    function requestRefresh() { refreshes++; }
    function requestReconnect() { reconnects++; }
    property var renames: []
    function renameKeyboard(name) { renames.push(name); }
    function openSettings(page) {}
    function runDiagnose() { diagnoses++; }

    FontLoader {
        id: vt
        source: "../com.agenceapi.devicehub/contents/fonts/VT323-Regular.ttf"
    }

    Loader {
        id: popup
        anchors.fill: parent
        Component.onCompleted: setSource("../com.agenceapi.devicehub/contents/ui/FullRepresentation.qml", { applet: root })
    }
    Chemistry { id: chem }
    PipSegments { id: segments; width: 200; fraction: 0.86 }
    CompactRepresentation { id: compact; applet: root; width: 32; height: 32 }
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
    function withText(item, text) {
        if (item.text === text) return item;
        var kids = item.children || [];
        for (var i = 0; i < kids.length; i++) {
            var r = withText(kids[i], text);
            if (r) return r;
        }
        return null;
    }
    function withProp(item, prop) {
        if (item[prop] !== undefined) return item;
        var kids = item.children || [];
        for (var i = 0; i < kids.length; i++) {
            var r = withProp(kids[i], prop);
            if (r) return r;
        }
        return null;
    }
    function kv(item, label) {
        if (item.label === label && item.value !== undefined) return item;
        var kids = item.children || [];
        for (var i = 0; i < kids.length; i++) {
            var r = kv(kids[i], label);
            if (r) return r;
        }
        return null;
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
            for (var o = 4; o >= 0; o--) {
                (popup.item as FullRepresentation).openTab(o);
                root.check("popup shows tab " + o, (popup.item as FullRepresentation).tab === o);
            }
            root.check("DATA asked for the history", root.shown === 1);
            root.check("DIAG ran the checks", root.diagnoses === 1);
            // the Fn button names the action, not the current state.
            root.check("Fn button names the action", root.withText(stat, "Switch to F1–F12 first") !== null
                       && root.withText(stat, "Fn: media keys first") === null);
            // each Fn choice asks for its own mode, also from mode 0 or 4.
            for (var fm of [0, 4]) {
                root.fnMode = fm;
                root.withText(keys, "F1–F12 first").clicked();
                root.check("F1–F12 first from mode " + fm + " asks 2 (" + root.fnAsked + ")", root.fnAsked === 2);
                root.withText(keys, "Media keys first").clicked();
                root.check("media first from mode " + fm + " asks 1 (" + root.fnAsked + ")", root.fnAsked === 1);
            }
            root.fnMode = 1;
            // the name field takes what the daemon accepts; Enter on blanks renames nothing.
            var nameField = root.withProp(stat, "maximumLength");
            root.check("name field bounded to 64 (" + (nameField ? nameField.maximumLength : "") + ")", nameField && nameField.maximumLength === 64);
            nameField.text = "   ";
            nameField.accepted();
            root.check("Enter on a blank name renames nothing", root.renames.length === 0);
            nameField.text = " Bob ";
            nameField.accepted();
            root.check("Enter renames with the trimmed name", root.renames.length === 1 && root.renames[0] === "Bob");
            // the percent sign follows the translated pattern.
            var af = [["%1%", "", "%"], ["%1 %", "", " %"], ["%%1", "%", ""]];
            for (var ai = 0; ai < af.length; ai++)
                root.check("affixes of " + af[ai][0], Pip.affixes(af[ai][0])[0] === af[ai][1] && Pip.affixes(af[ai][0])[1] === af[ai][2]);
            root.check("DATA axis through i18n", root.withText(data, "50%") !== null);
            var trend = root.kv(stat, "Trend");
            root.check("STAT gives the trend with its span (" + (trend ? trend.value : "") + ")", trend && trend.value.indexOf("over 12 days") > 0);
            var host = root.kv(radio, "Paired host");
            root.check("RADIO has the paired host", host !== null);
            root.check("RADIO masks the paired host (" + (host ? host.value : "") + ")", host && host.value === "AA:BB:XX:XX:XX:F2");
            var dm = { "M/d/yy": "M/d", "dd/MM/yyyy": "dd/MM", "dd.MM.yy": "dd.MM", "yyyy-MM-dd": "MM-dd", "yy. M. d.": "M. d.", "d MMM y": "d MMM" };
            for (var fmt in dm)
                root.check("axis date of " + fmt + " = " + Pip.dayMonthFormat(fmt), Pip.dayMonthFormat(fmt) === dm[fmt]);
            var addr = root.kv(data, "Address");
            root.check("DATA masks the address (" + (addr ? addr.value : "") + ")", addr && addr.value === "AA:BB:XX:XX:XX:F1");
            root.diagChecks = [{ id: "binary", label: "Daemon binary", ok: true, detail: "apple-kb-monitord 3.1.0" },
                               { id: "rssi_helper", label: "RSSI helper", ok: false, detail: "missing" }];
            root.diagPassed = 1;
            root.diagTotal = 2;
            root.check("diag log listed", diag.implicitHeight > 300);
            var now = Date.now() / 1000, all = [];
            for (var i = 0; i < 20000; i++)
                all.push({ ts: now - 90 * 86400 * (1 - i / 20000), pct: 100 - i / 400, voltage: 3 - i / 40000 });
            var h = Hist.series(all, now - 90 * 86400, now, Hist.POINTS_MAX);
            root.historyDays = 90;
            root.historyCount = h.count;
            root.historyFrom = h.tMin;
            root.historyTo = h.tMax;
            root.historyVoltMin = h.voltMin;
            root.historyVoltMax = h.voltMax;
            root.historyPct = h.pct;
            root.historyVolt = h.volt;
            root.sparkPct = h.pct;
            root.check("chart gets the bounded series", chart.primary.length <= Hist.POINTS_MAX && chart.primary.length > 100);
            root.check("DATA shows the chart", data.hasChart && data.hasVoltage);
            // the voltage is read on its own scale, never on the percent axis.
            root.check("voltage scale spans the curve", data.voltTicks.length === 5
                       && data.voltTicks[0] >= h.voltMax && data.voltTicks[4] <= h.voltMin);
            root.connected = false;
            root.daemonRunning = false;
            root.daemonRunning = true;
            root.historyError = "org.freedesktop.DBus.Error.NoReply";
            root.linkHealth = "unreachable";
            root.connected = true;
            root.hasRssi = false;
            root.rssi = NaN;
            root.discHour = -1;
            root.discByDay = [];
            root.thresholds = null;
            root.fnMode = -1;
            // a helper that is installed but not yet usable is never "not installed".
            // the chemistry shown is the declared one, never a voltage guess.
            chem.code = "nimh";
            root.check("declared NiMH", chem.label === "NiMH");
            chem.code = "unknown";
            root.check("undeclared chemistry", chem.label === "not declared");
            // the reason comes from rssi_error.code, never from a failed check.
            root.rssiIssue = "helper_failed";
            root.check("RADIO says the helper failed (" + radio.signalIssue.reason + ")",
                       radio.signalIssue.reason.indexOf("failed") > 0 && radio.signalIssue.reason.indexOf("not installed") < 0);
            // STAT and the menu say the same words.
            root.check("STAT short line", stat.signalIssue.shortText === "not measured (see akmctl doctor)");
            root.rssiIssue = "helper_missing";
            root.check("missing helper", radio.signalIssue.reason === "the rssi-helper utility is not installed"
                       && stat.signalIssue.shortText === "not measured (rssi-helper not installed)");
            // the gauge reads as the translated percentage, never "86 %" outside i18n.
            root.check("gauge name (" + segments.Accessible.name + ")", segments.Accessible.name === "86%");
            // the trend uses the English percent of the rest of the popup.
            root.check("trend percent (" + (trend ? trend.value : "") + ")", trend && /^−0[.,]8% per day/.test(trend.value));
            // a click on the icon of an open popup closes it: the popup hides itself at the press.
            root.expanded = true;
            compact.press();
            root.expanded = false;
            compact.click();
            root.check("click on the icon closes the open popup", !root.expanded);
            compact.press();
            compact.click();
            root.check("click on the icon opens the closed popup", root.expanded);
            root.check("middle button left to Plasma", compact.acceptedButtons === Qt.LeftButton);
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
