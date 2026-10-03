import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.kirigami as Kirigami
import "Signal.js" as Sig
import "History.js" as Hist
import "FnMode.js" as Fn

// Event-driven view of the daemon apple-kb-monitord (session bus,
// com.agenceapi.AppleKbMonitor1). No subprocess and no fast timer: DaemonLink
// receives StateChanged through a native D-Bus signal watcher and the applet
// re-reads GetState (in-memory snapshot, read-only, never touches the keyboard).
PlasmoidItem {
    id: root

    readonly property string busName: "com.agenceapi.AppleKbMonitor1"
    readonly property string objectPath: "/com/agenceapi/AppleKbMonitor1"

    // ── State (battery -1 / voltage 0 / rssi NaN / "" = unknown) ──
    readonly property bool daemonRunning: link.registered
    property bool connected: false
    property int batteryPercent: -1
    // Measured battery voltage (V), reports 0x46/0xFF; 0 = unknown.
    property real voltage: 0
    // Percentage interpolated on the unit's own discharge curve: an ESTIMATE, -1 = unknown.
    property real curvePercent: -1
    // Relative BR/EDR RSSI in dB (0 = ideal reception range), NOT dBm (#174).
    property real rssi: NaN
    // "excellent" | "good" | "weak" | "" (unknown)
    property string rssiQuality: ""
    // Charge estimated from the voltage and the declared chemistry (#178):
    // [hypothèse], -1 = none. The keyboard's own indication is batteryPercent.
    property real estimatePct: -1
    property real estimateLow: -1
    property real estimateHigh: -1
    property string estimateChem: ""
    property bool newBatteries: false
    // Unix time of the last reading (#179): the keyboard's percentage only
    // steps down when it reconnects, so the age of the reading is shown.
    property real lastUpdate: 0
    property string kbModel: ""
    // Name shown to the user: alias set on this computer, else own name (#141).
    property string kbName: ""
    property string kbMac: ""
    property string renameError: ""
    property string fwVersion: ""
    // Firmware check against the table embedded in the daemon (#227):
    // up_to_date / update_available / unknown, never a flash offer.
    property string fwStatus: "unknown"
    property string fwLatest: ""
    // Percentage as macOS shows it (#213), -1 = not available.
    property real applePct: -1
    property string thresholdsText: ""
    readonly property string fwText: fwVersion === "" ? "" : (
        fwStatus === "up_to_date" ? i18n("%1 — up to date (latest public version known to Apple)", fwVersion)
        : fwStatus === "update_available" ? i18n("%1 — update available from Apple (latest known: %2)", fwVersion, fwLatest)
        : i18n("%1 — unknown (model or version not in the table)", fwVersion))
    property string batteryType: ""
    property string remaining: ""
    property string lastError: ""
    property string windowHint: ""
    // The last full read missed a report (GetState: keyboard.incomplete).
    property bool readIncomplete: false

    // ── Sparkline of the last 7 days (#97): [t, %] points, bounded ──
    property var sparkPct: []
    property real sparkFrom: 0
    property real sparkTo: 1

    // ── History page (#120): 7, 30 or 90 days ──
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

    // ── Fn mode (#97): hid_apple.fnmode from the daemon, -1 = unknown ──
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
    // What the button does: the mode it switches to.
    readonly property string fnToggleText: Fn.next(fnMode) === 2
        ? i18n("Switch to F1–F12 first") : i18n("Switch to media keys first")

    // ── Diagnostic page (#120) ──
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

    // ── Pip-Boy panel (docs/DA-PIPBOY.md): the data of the window's tabs ──
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
    property string fwSource: ""
    property real installedAt: 0
    // Autonomy forecast of the daemon (#83): empty_at in Unix s, 0 = none.
    property real forecastEmptyAt: 0
    property real forecastRate: 0
    // Disconnections of the link (#105), -1 = nothing recorded.
    property int discHour: -1
    property int discDay: -1
    property int disc7d: -1
    property var discByDay: []
    property bool linkUnstable: false
    property string kbError: ""
    // Diagnose() of the daemon (#120): checks run on demand only.
    property var diagChecks: []
    property int diagPassed: 0
    property int diagTotal: 0
    property bool diagRunning: false
    property string diagError: ""
    // rssi_helper check of the last Diagnose(): 1 ok, 0 failed, -1 unknown.
    property int rssiHelperOk: -1
    // VT323, the face of the window (SIL OFL 1.1), registered for the popup.
    readonly property bool pipFontReady: vt.status === FontLoader.Ready
    FontLoader {
        id: vt
        source: Qt.resolvedUrl("../fonts/VT323-Regular.ttf")
    }

    readonly property bool hasBattery: connected && batteryPercent >= 0
    readonly property bool hasRssi: connected && !isNaN(rssi)

    // Short human text used by tooltip, compact label and screen readers.
    readonly property string batteryText: batteryPercent >= 0 ? i18n("%1%", batteryPercent) : "—"
    readonly property string rssiText: hasRssi
        ? i18n("%1 (%2)", qualityLabel(rssiQuality), Sig.rawRssi(rssi))
        : "—"
    readonly property bool hasEstimate: connected && estimatePct >= 0 && !newBatteries
    readonly property string estimateText: newBatteries
        ? i18n("new batteries, no estimate yet")
        : (estimatePct >= 0
            ? i18n("≈ %1% (%2 to %3%), %4", estimatePct.toFixed(0), estimateLow.toFixed(0),
                   estimateHigh.toFixed(0), estimateChem)
            : "")
    // Time of the last reading (no timer: re-evaluated on each state change).
    readonly property string updatedText: lastUpdate > 0
        ? Qt.formatTime(new Date(lastUpdate * 1000), Qt.locale().timeFormat(Locale.ShortFormat))
        : "—"
    readonly property string stateText: connected
        ? i18n("Connected")
        : (daemonRunning ? i18n("Keyboard not connected") : i18n("Monitor stopped"))
    readonly property string accessibleSummary: connected
        ? i18n("Apple keyboard, battery %1 percent", batteryPercent >= 0 ? batteryPercent : i18n("unknown"))
        : i18n("Apple keyboard, %1", stateText)

    // Visible in the notification area while the daemon is on the bus.
    Plasmoid.status: daemonRunning ? PlasmaCore.Types.ActiveStatus : PlasmaCore.Types.PassiveStatus
    Plasmoid.title: i18n("ApiHub")
    Plasmoid.icon: "apihub-scarab"

    // No preferredRepresentation: the system tray (Plasma 6, SystemTrayState
    // .setActiveApplet) only opens the popup of applets that leave it unset;
    // with it set, a click toggled `expanded` but the popup never showed.
    compactRepresentation: CompactRepresentation {}
    fullRepresentation: FullRepresentation {}

    toolTipTextFormat: Text.PlainText
    toolTipMainText: connected ? (kbName !== "" ? kbName : kbModel) : stateText
    toolTipSubText: connected
        ? [hasBattery ? i18n("Keyboard indication %1", batteryText) : "",
           (hasEstimate || newBatteries) ? i18n("Estimate: %1", estimateText) : "",
           applePct >= 0 ? i18n("Apple display: %1%", Math.round(applePct)) : "",
           voltage > 0 ? i18n("%1 V", voltage.toFixed(2)) : "",
           hasRssi ? i18n("Signal: %1", rssiText) : "",
           remaining,
           fwText !== "" ? i18n("Firmware: %1", fwText) : ""].filter(function (s) { return s !== ""; }).join("\n")
        : (daemonRunning ? i18n("Waiting for the Apple keyboard…")
                         : i18n("apple-kb-monitord is not on the session bus"))

    // ── Right-click menu: the daemon's menu (Tray.MenuItems), #268 ──
    // Only the entries that do something; the information rows are in the
    // popup. Rebuilt on every state change, so labels and states follow.
    readonly property var menuIds: [11, 12, 13, 14, 15, 16, 23, 24, 25, 19, 22]
    Component {
        id: menuAction
        PlasmaCore.Action {
            property int itemId: 0
            onTriggered: link.activateMenuItem(itemId, "")
        }
    }
    Component {
        id: menuSeparator
        PlasmaCore.Action { isSeparator: true }
    }
    function applyMenu(json) {
        let items;
        try {
            items = JSON.parse(json);
        } catch (e) {
            console.warn("apple-kb-monitor: bad MenuItems reply", e);
            return;
        }
        const old = Plasmoid.contextualActions;
        const list = [];
        let group = -1;
        for (let i = 0; i < items.length; ++i) {
            const it = items[i];
            if (root.menuIds.indexOf(it.id) < 0 || !it.visible) continue;
            // Groups: window/information actions, link actions, Fn mode.
            const g = it.id >= 23 ? 1 : (it.id === 19 || it.id === 22 ? 2 : 0);
            if (group >= 0 && g !== group) list.push(menuSeparator.createObject(root));
            group = g;
            const a = menuAction.createObject(root, { itemId: it.id, text: it.label, enabled: it.enabled });
            if (it.checked !== null) {
                a.checkable = true;
                a.checked = it.checked;
            }
            list.push(a);
        }
        Plasmoid.contextualActions = list;
        for (let j = 0; j < old.length; ++j) old[j].destroy();
    }

    DaemonLink {
        id: link
        busName: root.busName
        objectPath: root.objectPath
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
        onWindowFailed: function (message) {
            root.windowHint = i18n("Could not open the ApiHub window.");
        }
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
            // A failed read only hides the row; a failed change is said.
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

    // The popup opens: read what only it shows (history, Fn mode, link).
    // Nothing of this is polled while it is closed.
    onExpandedChanged: if (root.expanded) root.fetchDetails()

    Component.onCompleted: {
        // The widget is the notification-area icon (#268): a click opens and
        // closes its popup, like the volume or network ones, and its
        // right-click menu is the daemon's menu. The daemon withdraws its own
        // icon while this claim holds, so there is one icon, not two.
        link.claimId = String(Plasmoid.id);
        if (link.registered) fetchData();
    }

    // Removed from the panel or the tray: give the icon back to the daemon.
    Component.onDestruction: link.releaseTray()

    function batteryTypeOf(v) {
        if (v <= 0) return "";
        if (v >= 3.1) return i18n("Lithium (fresh)");
        if (v >= 2.85) return i18n("Alkaline (fresh)");
        if (v >= 2.5) return i18n("Alkaline or NiMH");
        if (v >= 2.3) return i18n("NiMH (likely)");
        if (v >= 2.0) return i18n("Depleted");
        return i18n("Critical, replace");
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
        root.curvePercent = (b.percentage_interpolated === null || b.percentage_interpolated === undefined) ? -1 : Number(b.percentage_interpolated);
        root.rssi = Sig.rssiOf(kb ? kb.radio : null);
        root.rssiQuality = (kb && kb.radio && kb.radio.rssi_quality) ? kb.radio.rssi_quality : Sig.qualityOf(root.rssi);
        var est = b.charge_estimate || null;
        root.estimatePct = est ? Number(est.pct) : -1;
        root.estimateLow = est ? Number(est.low) : -1;
        root.estimateHigh = est ? Number(est.high) : -1;
        root.estimateChem = est ? String(est.chemistry) : "";
        root.newBatteries = !!b.new_batteries;
        root.lastUpdate = Number(d.last_update || 0);
        root.kbModel = (kb && kb.device && kb.device.model) ? kb.device.model : i18n("Apple Keyboard");
        root.kbName = d.name || ((kb && kb.device && (kb.device.alias || kb.device.name)) || "");
        root.kbMac = (kb && kb.device && kb.device.mac) ? kb.device.mac : "";
        root.fwVersion = (kb && kb.firmware && kb.firmware.version) ? kb.firmware.version : "";
        root.fwStatus = (kb && kb.firmware && kb.firmware.status) ? String(kb.firmware.status) : "unknown";
        root.fwLatest = (kb && kb.firmware && kb.firmware.latest_known) ? String(kb.firmware.latest_known) : "";
        root.applePct = (b.apple_display_pct === null || b.apple_display_pct === undefined) ? -1 : Number(b.apple_display_pct);
        var th = b.thresholds || null;
        root.thresholdsText = th ? i18n("Full %1 / Low %2 / Critical %3 / Empty %4 mV", th.full_mv, th.low_mv, th.critical_mv, th.empty_mv) : "";
        root.batteryType = batteryTypeOf(root.voltage);
        root.remaining = d.remaining_display || "";
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
        root.fwSource = (kb && kb.firmware && kb.firmware.source) ? String(kb.firmware.source) : "";
        root.installedAt = Number(d.batteries_installed_at || 0);
        var fc = d.forecast || null;
        root.forecastEmptyAt = fc ? Number(fc.empty_at || 0) : 0;
        root.forecastRate = fc ? Number(fc.rate_pct_per_day || 0) : 0;
        var lq = d.link_quality || null;
        root.discHour = lq ? Number(lq.disconnects_last_hour) : -1;
        root.discDay = lq ? Number(lq.disconnects_last_day) : -1;
        root.disc7d = lq ? Number(lq.disconnects_7d) : -1;
        root.discByDay = lq && lq.disconnects_by_day ? lq.disconnects_by_day : [];
        root.linkUnstable = !!(lq && lq.unstable);
        root.kbError = d.kb_error ? String(d.kb_error) : "";
        // Popup opened before the first state: the address is known only now.
        if (root.expanded && root.fnMode < 0 && root.kbMac !== "" && !root.fnBusy) link.fetchFnMode(root.kbMac);
    }

    // History, Fn mode, daemon version and link state: on demand only.
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
        root.historySpanDays = Hist.spanDays(h);
        root.historyFrom = h.tMin;
        root.historyTo = h.tMax;
        root.historyPctMin = isNaN(h.pctMin) ? 0 : h.pctMin;
        root.historyPctMax = isNaN(h.pctMax) ? 100 : h.pctMax;
        root.historyVoltMin = isNaN(h.voltMin) ? 0 : h.voltMin;
        root.historyVoltMax = isNaN(h.voltMax) ? 1 : h.voltMax;
        root.historyPct = h.pct;
        root.historyVolt = h.volt;
    }

    // History page: load `days` (1, 7, 30 or 90) from the daemon.
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

    // Fn mode button: media keys first <-> F1–F12 first. The daemon asks for
    // the administrator authentication (polkit); nothing is written here.
    function toggleFnMode() {
        var next = Fn.next(root.fnMode);
        if (next < 0 || root.kbMac === "" || root.fnBusy) return;
        root.fnError = "";
        root.fnBusy = true;
        link.setFnMode(root.kbMac, next);
    }

    // DIAG tab: the daemon runs its checks (each bounded to 3 s, never the
    // keyboard). Only on demand, never polled.
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
        root.rssiHelperOk = -1;
        for (var i = 0; i < checks.length; i++)
            if (checks[i].id === "rssi_helper") root.rssiHelperOk = checks[i].ok ? 1 : 0;
    }

    function requestRefresh() {
        root.diagHint = "";
        link.refresh();
    }

    function requestReconnect() {
        root.diagHint = "";
        link.reconnect();
    }

    // Rename on this computer (BlueZ alias, nothing is written into the
    // keyboard). An empty name restores the keyboard's own name.
    function renameKeyboard(name) {
        if (root.kbMac === "") return;
        root.renameError = "";
        link.setAlias(root.kbMac, name);
    }

    function clear() {
        root.connected = false;
        root.batteryPercent = -1;
        root.voltage = 0;
        root.curvePercent = -1;
        root.rssi = NaN;
        root.rssiQuality = "";
        root.estimatePct = -1;
        root.applePct = -1;
        root.newBatteries = false;
        root.remaining = "";
        root.readIncomplete = false;
        root.sparkPct = [];
        root.historyPct = [];
        root.historyVolt = [];
        root.historyCount = 0;
        root.historyLoading = false;
        root.fnMode = -1;
        root.fnBusy = false;
        root.linkHealth = "";
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
    }

    function fetchData() {
        if (!link.registered) {
            clear();
            return;
        }
        link.fetch();
        link.fetchMenu();
    }

    // ── Open the ApiHub window ──
    // org.freedesktop.Application.Activate on com.agenceapi.AppleKbMonitor:
    // the name is D-Bus activatable (dbus/com.agenceapi.AppleKbMonitor.service),
    // so the window is started when closed and raised when open, whichever
    // tray (daemon or apihub-app) is in use (#149).
    function openWindow() {
        root.windowHint = "";
        link.activateWindow();
    }
}
