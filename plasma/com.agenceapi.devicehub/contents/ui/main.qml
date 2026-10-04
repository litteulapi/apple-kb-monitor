import QtQuick
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.kcmutils as KCMUtils
import "Signal.js" as Sig
import "History.js" as Hist
import "FnMode.js" as Fn
import "Menu.js" as Menu
import "Pip.js" as Pip

PlasmoidItem {
    id: root

    readonly property string busName: "com.agenceapi.AppleKbMonitor1"
    readonly property string objectPath: "/com/agenceapi/AppleKbMonitor1"

    readonly property bool daemonRunning: link.registered
    property bool connected: false
    property int batteryPercent: -1
    property real voltage: 0
    // Relative BR/EDR RSSI in dB (0 = ideal reception range), NOT dBm.
    property real rssi: NaN
    property string rssiQuality: ""
    // Charge estimated from the voltage and the declared chemistry: [assumption], -1 = none.
    property real estimatePct: -1
    property real estimateLow: -1
    property real estimateHigh: -1
    property string estimateChem: ""
    property bool newBatteries: false
    property real lastUpdate: 0
    property string kbModel: ""
    property string kbName: ""
    property string kbMac: ""
    property string renameError: ""
    property string fwVersion: ""
    property string fwStatus: "unknown"
    property string fwLatest: ""
    property real applePct: -1
    property string thresholdsText: ""
    readonly property string fwText: fwVersion === "" ? "" : (
        fwStatus === "up_to_date" ? i18n("%1 — up to date (latest public version known to Apple)", fwVersion)
        : fwStatus === "update_available" ? i18n("%1 — update available from Apple (latest known: %2)", fwVersion, fwLatest)
        : i18n("%1 — unknown (model or version not in the table)", fwVersion))
    // One word for the chemistry, the one the estimate uses: never a guess from the voltage.
    readonly property Chemistry chemistry: Chemistry { code: root.estimateChem }
    readonly property string batteryType: chemistry.label
    // Time left from the daemon's forecast, re-evaluated at each reading.
    readonly property string remaining: {
        var reading = lastUpdate;
        var days = Pip.daysLeft({ empty_at: forecastEmptyAt }, Date.now() / 1000);
        return days < 0 ? "" : i18n("Autonomy: %1", days >= 1.5
            ? i18np("≈ %1 day", "≈ %1 days", Math.round(days))
            : i18n("≈ %1 h", Math.round(days * 24)));
    }
    property string lastError: ""
    property bool readIncomplete: false

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

    property int fnMode: -1
    property bool fnBusy: false
    property string fnError: ""
    readonly property string fnModeText: {
        var k = Fn.kind(fnMode);
        return k === "media" ? i18n("media keys first")
            : k === "fkeys" ? i18n("F1–F12 first")
            : k === "off" ? i18n("Fn key has no effect")
            : k === "nofkeys" ? i18n("F-keys disabled") : "";
    }
    readonly property string fnToggleText: Fn.next(fnMode) === 2
        ? i18n("Switch to F1–F12 first") : i18n("Switch to media keys first")

    property string daemonVersion: ""
    property string linkHealth: ""
    property int linkAttempts: 0
    property int linkFailures: 0
    property string linkError: ""
    property string diagHint: ""
    readonly property string linkText: linkHealth === "" ? "" : i18n("%1 (attempts: %2, failures: %3)",
        linkHealth === "connected" ? i18n("connected")
            : linkHealth === "dormant" ? i18n("asleep")
            : linkHealth === "unreachable" ? i18n("unreachable") : linkHealth,
        linkAttempts, linkFailures)

    property bool capsLock: false
    property bool numLock: false
    property bool paired: false
    property string pairedHost: ""
    property string kbChip: ""
    property string kbDriver: ""
    property real txPower: NaN
    property real rssiAt: 0
    property real wakeAge: -1
    property string thresholdLevel: ""
    property var thresholds: null
    property real installedAt: 0
    property real forecastEmptyAt: 0
    property real forecastRate: 0
    property real forecastSpanDays: 0
    property int discHour: -1
    property int discDay: -1
    property int disc7d: -1
    property var discByDay: []
    property bool linkUnstable: false
    property var diagChecks: []
    property int diagPassed: 0
    property int diagTotal: 0
    property bool diagRunning: false
    property string diagError: ""
    property string rssiIssue: ""
    FontLoader {
        id: vt
        source: Qt.resolvedUrl("../fonts/VT323-Regular.ttf")
    }

    readonly property bool hasBattery: connected && batteryPercent >= 0
    readonly property bool hasRssi: connected && !isNaN(rssi)

    readonly property string batteryText: batteryPercent >= 0 ? i18n("%1%", batteryPercent) : "—"
    readonly property string rssiText: hasRssi
        ? i18n("%1 (%2)", qualityLabel(rssiQuality), Sig.rawRssi(rssi))
        : "—"
    readonly property bool hasEstimate: connected && estimatePct >= 0 && !newBatteries
    readonly property string estimateText: newBatteries
        ? i18n("new batteries, no estimate yet")
        : (estimatePct >= 0
            ? i18n("≈ %1% (%2 to %3%), %4", estimatePct.toFixed(0), estimateLow.toFixed(0),
                   estimateHigh.toFixed(0), chemistry.label)
            : "")
    readonly property string updatedText: lastUpdate > 0
        ? Qt.formatTime(new Date(lastUpdate * 1000), Qt.locale().timeFormat(Locale.ShortFormat))
        : "—"
    readonly property string stateText: connected
        ? i18n("Connected")
        : (daemonRunning ? i18n("Keyboard not connected") : i18n("Monitor stopped"))
    readonly property string accessibleSummary: connected
        ? (batteryPercent >= 0 ? i18n("Apple keyboard, battery %1 percent", batteryPercent) : i18n("Apple keyboard, battery level unknown"))
        : i18n("Apple keyboard, %1", stateText)

    Plasmoid.status: daemonRunning ? PlasmaCore.Types.ActiveStatus : PlasmaCore.Types.PassiveStatus
    Plasmoid.title: "ApiHub"
    Plasmoid.icon: "apihub-scarab"

    compactRepresentation: CompactRepresentation { applet: root }
    fullRepresentation: FullRepresentation { applet: root }

    toolTipTextFormat: Text.PlainText
    toolTipMainText: connected ? (kbName !== "" ? kbName : kbModel) : stateText
    toolTipSubText: connected
        ? [hasBattery ? i18n("Keyboard indication %1", batteryText) : "",
           (hasEstimate || newBatteries) ? i18n("Estimate: %1", estimateText) : "",
           applePct >= 0 ? i18n("Apple display: %1%", Math.round(applePct)) : "",
           voltage > 0 ? i18n("%1 V", Pip.num(voltage, 2)) : "",
           hasRssi ? i18n("Signal: %1", rssiText) : "",
           remaining,
           fwText !== "" ? i18n("Firmware: %1", fwText) : ""].filter(function (s) { return s !== ""; }).join("\n")
        : (daemonRunning ? i18n("Waiting for the Apple keyboard…")
                         : i18n("apple-kb-monitord is not on the session bus"))

    // The applet's "Configure…" entry opens the System Settings module.
    PlasmaCore.Action {
        id: configureAction
        text: i18nc("@action:inmenu", "&Configure Apple Keyboard…")
        icon.name: "configure"
        onTriggered: root.openSettings("")
    }

    Component {
        id: menuAction
        PlasmaCore.Action {
            property int itemId: 0
            onTriggered: link.activateMenuItem(itemId)
        }
    }
    // exclusive by default (QActionGroup::ExclusionPolicy::Exclusive)
    PlasmaCore.ActionGroup {
        id: fnGroup
    }
    Component {
        id: menuSeparator
        PlasmaCore.Action { isSeparator: true }
    }
    function applyMenu(json) {
        let entries;
        try {
            entries = Menu.build(json);
        } catch (e) {
            console.warn("apple-kb-monitor: bad MenuItems reply", e);
            return;
        }
        const cur = Plasmoid.contextualActions;
        if (Menu.sameShape(cur, entries)) {
            // an open menu holds these actions: destroying them would empty it
            for (let i = 0; i < entries.length; ++i) {
                const e = entries[i];
                if (e.separator) continue;
                cur[i].text = e.text;
                cur[i].enabled = e.enabled;
                Menu.applyCheck(cur[i], e, fnGroup);
            }
            return;
        }
        const list = entries.map(function (e) {
            if (e.separator) return menuSeparator.createObject(root);
            const a = menuAction.createObject(root, { itemId: e.id, text: e.text, enabled: e.enabled });
            Menu.applyCheck(a, e, fnGroup);
            return a;
        });
        const old = Menu.swap(Plasmoid, "contextualActions", list);
        for (let j = 0; j < old.length; ++j) old[j].destroy();
    }

    DaemonLink {
        id: link
        busName: root.busName
        objectPath: root.objectPath
        deviceMac: root.kbMac
        onRegisteredChanged: {
            if (registered) root.fetchData(); else root.clear();
            if (registered && root.expanded) root.fetchDetails();
        }
        onMenuReceived: function (json) {
            root.applyMenu(json);
        }
        onMenuItemFailed: function (message) {
            console.warn("apple-kb-monitor: menu action refused", message);
        }
        onStateReceived: function (json) {
            try {
                root.apply(json);
            } catch (e) {
                console.warn("apple-kb-monitor: bad GetState reply", e);
                root.clear();
            }
        }
        onFailed: function (message) {
            console.warn("apple-kb-monitor: GetState failed", message);
            root.clear();
        }
        onAliasSet: function (name) {
            root.kbName = name;
            root.fetchData();
        }
        onAliasFailed: function (message) {
            root.renameError = message;
        }
        onPanelRequested: root.expanded = true
        onHistoryReceived: function (tag, json) {
            root.applyHistory(tag, json);
        }
        onHistoryFailed: function (tag, message) {
            if (tag !== "page") return;
            root.historyLoading = false;
            root.historyError = message;
        }
        onFnModeReceived: function (mode) {
            root.fnMode = mode;
        }
        onFnModeSet: function (mode) {
            root.fnBusy = false;
            root.fnMode = mode;
        }
        onFnModeFailed: function (message) {
            if (root.fnBusy) root.fnError = message;
            root.fnBusy = false;
        }
        onVersionReceived: function (version) {
            root.daemonVersion = version;
        }
        onLinkStatusReceived: function (json) {
            root.applyLinkStatus(json);
        }
        onRefreshDone: function (ok, message) {
            root.diagHint = ok ? i18n("New reading requested.")
                               : i18n("Reading not requested: %1", message);
        }
        onDiagnoseReceived: function (json) {
            root.applyDiagnose(json);
        }
        onDiagnoseFailed: function (message) {
            root.diagRunning = false;
            root.diagError = message;
        }
        onReconnectDone: function (ok, message) {
            root.diagHint = ok
                ? i18n("Reconnection requested. Press a key on the keyboard if it is asleep.")
                : i18n("Reconnection not requested: %1", message !== "" ? message : i18n("refused by the monitor"));
            link.fetchLinkStatus();
        }
    }

    onExpandedChanged: if (root.expanded) root.fetchDetails()

    Component.onCompleted: {
        Plasmoid.setInternalAction("configure", configureAction);
        if (link.registered) fetchData();
    }


    function qualityLabel(q) {
        if (q === "excellent") return i18n("excellent");
        if (q === "good") return i18n("good");
        if (q === "weak") return i18n("weak");
        return "—";
    }

    function apply(json) {
        var d = JSON.parse(json);
        var kb = d.keyboard || null;
        var b = kb ? (kb.battery || {}) : {};
        var pct = b.percentage;
        root.connected = !!d.connected && kb !== null;
        root.batteryPercent = (pct === null || pct === undefined) ? -1 : Math.round(pct);
        root.voltage = b.voltage || 0;
        root.rssi = Sig.rssiOf(kb ? kb.radio : null);
        root.rssiIssue = kb && kb.radio && kb.radio.rssi_error ? String(kb.radio.rssi_error.code || "other") : "";
        root.rssiQuality = (kb && kb.radio && kb.radio.rssi_quality) ? kb.radio.rssi_quality : Sig.qualityOf(root.rssi);
        var est = b.charge_estimate || null;
        root.estimatePct = est ? Number(est.pct) : -1;
        root.estimateLow = est ? Number(est.low) : -1;
        root.estimateHigh = est ? Number(est.high) : -1;
        root.estimateChem = est ? String(est.chemistry) : "";
        root.newBatteries = !!b.new_batteries;
        root.lastUpdate = Number(d.last_update || 0);
        root.kbModel = (kb && kb.device && kb.device.model) ? kb.device.model : i18n("Apple Keyboard");
        root.kbName = (kb && kb.device && (kb.device.alias || kb.device.name)) || "";
        root.kbMac = (kb && kb.device && kb.device.mac) ? kb.device.mac : "";
        root.fwVersion = (kb && kb.firmware && kb.firmware.version) ? kb.firmware.version : "";
        root.fwStatus = (kb && kb.firmware && kb.firmware.status) ? String(kb.firmware.status) : "unknown";
        root.fwLatest = (kb && kb.firmware && kb.firmware.latest_known) ? String(kb.firmware.latest_known) : "";
        root.applePct = (b.apple_display_pct === null || b.apple_display_pct === undefined) ? -1 : Number(b.apple_display_pct);
        var th = b.thresholds || null;
        root.thresholdsText = th ? i18n("Full %1 / Low %2 / Critical %3 / Empty %4 mV", th.full_mv, th.low_mv, th.critical_mv, th.empty_mv) : "";
        root.lastError = d.last_error || "";
        root.readIncomplete = !!(kb && kb.incomplete);
        root.capsLock = !!d.caps_lock;
        root.numLock = !!d.num_lock;
        var bt = kb ? (kb.bluetooth || {}) : {};
        root.paired = !!bt.paired;
        root.pairedHost = bt.paired_host_addr ? String(bt.paired_host_addr) : "";
        root.kbChip = (kb && kb.device && kb.device.chip) ? String(kb.device.chip) : "";
        root.kbDriver = (kb && kb.device && kb.device.driver) ? String(kb.device.driver) : "";
        var tx = kb && kb.radio ? kb.radio.tx_power_dbm : null;
        root.txPower = (tx === null || tx === undefined) ? NaN : Number(tx);
        root.rssiAt = Number(d.rssi_at || 0);
        var wk = kb && kb.wake ? kb.wake.last_age_s : null;
        root.wakeAge = (wk === null || wk === undefined) ? -1 : Number(wk);
        root.thresholdLevel = b.threshold_level ? String(b.threshold_level) : "";
        root.thresholds = th;
        root.installedAt = Number(d.batteries_installed_at || 0);
        var fc = d.forecast || null;
        root.forecastEmptyAt = fc ? Number(fc.empty_at || 0) : 0;
        root.forecastRate = fc ? Number(fc.rate_pct_per_day || 0) : 0;
        root.forecastSpanDays = fc ? Number(fc.span_s || 0) / 86400 : 0;
        var lq = d.link_quality || null;
        root.discHour = lq ? Number(lq.disconnects_last_hour) : -1;
        root.discDay = lq ? Number(lq.disconnects_last_day) : -1;
        root.disc7d = lq ? Number(lq.disconnects_7d) : -1;
        root.discByDay = lq && lq.disconnects_by_day ? lq.disconnects_by_day : [];
        root.linkUnstable = !!(lq && lq.unstable);
        if (root.expanded && root.fnMode < 0 && root.kbMac !== "" && !root.fnBusy) link.fetchFnMode(root.kbMac);
    }

    function fetchDetails() {
        if (!link.registered) return;
        link.fetchHistory(Date.now() / 1000 - 7 * 86400, "spark");
        if (root.kbMac !== "") link.fetchFnMode(root.kbMac);
        link.fetchVersion();
        link.fetchLinkStatus();
    }

    function applyHistory(tag, json) {
        var now = Date.now() / 1000;
        if (tag === "spark") {
            var s = Hist.parse(json, now - 7 * 86400, now, 120);
            root.sparkFrom = s.tMin;
            root.sparkTo = s.tMax;
            root.sparkPct = s.count >= 2 ? s.pct : [];
            return;
        }
        var h = Hist.parse(json, now - root.historyDays * 86400, now, Hist.POINTS_MAX);
        root.historyLoading = false;
        root.historyError = "";
        root.historyCount = h.count;
        root.historyFrom = h.tMin;
        root.historyTo = h.tMax;
        root.historyVoltMin = isNaN(h.voltMin) ? 0 : h.voltMin;
        root.historyVoltMax = isNaN(h.voltMax) ? 1 : h.voltMax;
        root.historyPct = h.pct;
        root.historyVolt = h.volt;
    }

    function showHistory(days) {
        root.historyDays = days;
        if (!link.registered) return;
        root.historyLoading = true;
        root.historyError = "";
        link.fetchHistory(Date.now() / 1000 - days * 86400, "page");
    }

    function applyLinkStatus(json) {
        var mine = null;
        try {
            var all = JSON.parse(json);
            for (var i = 0; i < all.length; i++) {
                if (root.kbMac === "" || String(all[i].mac).toUpperCase() === root.kbMac.toUpperCase()) {
                    mine = all[i];
                    break;
                }
            }
        } catch (e) {
            mine = null;
        }
        root.linkHealth = mine ? String(mine.health || "") : "";
        root.linkAttempts = mine ? Number(mine.attempts || 0) : 0;
        root.linkFailures = mine ? Number(mine.failures || 0) : 0;
        root.linkError = mine ? String(mine.last_error || "") : "";
    }

    function toggleFnMode() {
        setFnModeTo(Fn.next(root.fnMode));
    }

    function setFnModeTo(mode) {
        if (mode < 0 || root.kbMac === "" || root.fnBusy) return;
        root.fnError = "";
        root.fnBusy = true;
        link.setFnMode(root.kbMac, mode);
    }

    // DIAG tab: the daemon runs its checks (each bounded to 3 s, never the keyboard).
    function runDiagnose() {
        if (!link.registered || root.diagRunning) return;
        root.diagError = "";
        root.diagRunning = true;
        link.diagnose();
    }

    function applyDiagnose(json) {
        root.diagRunning = false;
        var d;
        try {
            d = JSON.parse(json);
        } catch (e) {
            root.diagError = String(e);
            return;
        }
        var checks = d.checks || [];
        root.diagChecks = checks;
        root.diagPassed = Number(d.passed || 0);
        root.diagTotal = Number(d.total || checks.length);
        if (d.daemon_version) root.daemonVersion = String(d.daemon_version);
    }

    function requestRefresh() {
        root.diagHint = "";
        link.refresh();
    }

    function requestReconnect() {
        root.diagHint = "";
        link.reconnect();
    }

    function renameKeyboard(name) {
        if (root.kbMac === "") return;
        root.renameError = "";
        link.setAlias(root.kbMac, name);
    }

    function clear() {
        root.connected = false;
        root.batteryPercent = -1;
        root.voltage = 0;
        root.rssi = NaN;
        root.rssiIssue = "";
        root.rssiQuality = "";
        root.estimatePct = -1;
        root.applePct = -1;
        root.newBatteries = false;
        root.readIncomplete = false;
        root.sparkPct = [];
        root.historyPct = [];
        root.historyVolt = [];
        root.historyCount = 0;
        root.historyLoading = false;
        root.fnMode = -1;
        root.fnBusy = false;
        root.linkHealth = "";
        root.linkError = "";
        root.linkAttempts = 0;
        root.linkFailures = 0;
        root.pairedHost = "";
        root.lastError = "";
        root.diagChecks = [];
        root.diagPassed = 0;
        root.diagTotal = 0;
        root.diagError = "";
        root.daemonVersion = "";
        root.diagHint = "";
        root.capsLock = false;
        root.numLock = false;
        root.forecastEmptyAt = 0;
        root.discHour = -1;
        root.discDay = -1;
        root.disc7d = -1;
        root.discByDay = [];
        root.linkUnstable = false;
        root.txPower = NaN;
        root.rssiAt = 0;
        root.wakeAge = -1;
        root.fnError = "";
        root.renameError = "";
        root.historyError = "";
        root.forecastRate = 0;
        root.forecastSpanDays = 0;
        root.thresholds = null;
        root.thresholdsText = "";
        root.installedAt = 0;
        root.lastUpdate = 0;
    }

    function fetchData() {
        if (!link.registered) {
            clear();
            return;
        }
        link.fetch();
        link.fetchMenu();
    }

    function openSettings(page) {
        KCMUtils.KCMLauncher.openSystemSettings("kcm_applekeyboard", page ? [page] : []);
    }
}
