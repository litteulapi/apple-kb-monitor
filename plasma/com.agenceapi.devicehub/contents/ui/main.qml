import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.kirigami as Kirigami
import "Signal.js" as Sig

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

    preferredRepresentation: compactRepresentation
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

    DaemonLink {
        id: link
        busName: root.busName
        objectPath: root.objectPath
        onRegisteredChanged: {
            if (registered) root.fetchData(); else root.clear();
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
    }

    Component.onCompleted: {
        // Take over the notification area: the daemon withdraws its own icon
        // (#253). Setting the id sends the claim when the daemon is on the bus.
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
    }

    function fetchData() {
        if (!link.registered) {
            clear();
            return;
        }
        link.fetch();
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
