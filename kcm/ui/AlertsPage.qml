// SPDX-License-Identifier: GPL-2.0-or-later
// Notifications and alerts: the daemon's config.toml, saved by the KCM
// "Apply" button. Only the keys the user changed are rewritten, and every
// other byte of the file is kept (#284, #286); the values are shown as the
// file writes them, with the daemon's bounds. The daemon reads the file
// when it starts.
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import "Toml.js" as Toml

ColumnLayout {
    id: root
    objectName: "alertsPage"

    required property Store store

    // Defaults of akm-core/src/config.rs.
    readonly property var defaultsMap: ({
        "alerts.enabled": true,
        "alerts.thresholds": [30, 15, 5],
        "alerts.critical": 5,
        "alerts.hysteresis": 3,
        "battery.chemistry": "alkaline",
        "notifications.connection": true,
        "notifications.battery_replaced": true,
        "notifications.defer_to_powerdevil": true,
        "display.apple_percent": true,
        "apple.will_shutdown": true
    })
    // TOML type the daemon takes for each key (serde in config.rs): a value
    // of another type is ignored there, so it is ignored here too (#12).
    readonly property var kindsMap: ({
        "alerts.enabled": "boolean",
        "alerts.thresholds": "integers",
        "alerts.critical": "integer",
        "alerts.hysteresis": "float",
        "battery.chemistry": "string",
        "notifications.connection": "boolean",
        "notifications.battery_replaced": "boolean",
        "notifications.defer_to_powerdevil": "boolean",
        "display.apple_percent": "boolean",
        "apple.will_shutdown": "boolean"
    })
    readonly property var chemistries: ["alkaline", "nimh", "lithium", "unknown"]
    // Bounds of akm-core (config.rs, alerts.rs): thresholds 1..=99 (others
    // dropped), critical 0..=99 (else the default), hysteresis any finite
    // number, used clamped to 1..=20 (#286).
    readonly property int pctMax: 99
    readonly property real hysteresisMin: 1
    readonly property real hysteresisMax: 20
    property var raw: ({})           // what the file says, per key (typed, else undefined)
    property var loaded: ({})        // what the daemon uses, per key (defaults filled in)
    property var touched: ({})       // keys the user changed since the file was read
    property var rangeNotes: []      // file values the daemon does not take as written
    property int criticalMin: 0      // the spin box shows an out-of-range critical as it is
    property int criticalMax: pctMax
    property bool ready: false
    property bool saving: false
    property bool savedOnce: false
    property var parseWarnings: []

    spacing: Kirigami.Units.smallSpacing

    function val(k, m) {
        return m[k] !== undefined ? m[k] : defaultsMap[k];
    }
    // The value the daemon takes for `k` in the parsed file, else undefined.
    function readKey(k, p) {
        return Toml.typed(p, k, kindsMap[k]);
    }
    // Chemistry name as akm-core's Chemistry::parse reads it, else null.
    function chemistryOf(s) {
        const t = String(s).trim().toLowerCase();
        const alias = { "alkaline": "alkaline", "alcaline": "alkaline", "nimh": "nimh", "ni-mh": "nimh",
                        "lithium": "lithium", "unknown": "unknown", "inconnue": "unknown" };
        return alias[t] !== undefined ? alias[t] : null;
    }
    // What the daemon uses for a value of `k` (akm-core semantics): the value
    // compared to decide whether anything changed.
    function effective(k, v) {
        if (v === undefined || v === null) return defaultsMap[k];
        if (k === "alerts.thresholds") {
            const out = [];
            for (let i = 0; i < v.length; ++i) {
                const n = v[i];
                if (n >= 1 && n <= pctMax && out.indexOf(n) < 0) out.push(n);
            }
            out.sort(function (a, b) { return b - a; });
            return out;
        }
        if (k === "alerts.critical") return v >= 0 && v <= pctMax ? v : defaultsMap[k];
        if (k === "alerts.hysteresis") return Math.min(hysteresisMax, Math.max(hysteresisMin, v));
        if (k === "battery.chemistry") return chemistryOf(v) || defaultsMap[k];
        return v;
    }
    // Thresholds typed in the field: whole numbers 1-99, else null.
    function parseThresholds(t) {
        const parts = String(t).split(/[ ,;]+/).filter(function (x) { return x !== ""; });
        if (parts.length === 0) return null;
        const out = [];
        for (let i = 0; i < parts.length; ++i) {
            if (!/^\d{1,2}$/.test(parts[i])) return null;
            const n = Number(parts[i]);
            if (n < 1 || n > pctMax) return null;
            out.push(n);
        }
        return out;
    }
    // Hysteresis typed in the field ("2.5" or "2,5"): 1 to 20, else null.
    function parseHysteresis(t) {
        const s = String(t).trim().replace(",", ".");
        if (!/^\d{1,2}(\.\d{1,3})?$/.test(s)) return null;
        const n = Number(s);
        return n >= hysteresisMin && n <= hysteresisMax ? n : null;
    }
    function hysteresisText(v) {
        return String(v);
    }
    function validCritical(v) {
        return v >= 0 && v <= pctMax;
    }
    // What the form says now, per key; null for an entry that is not valid.
    function current() {
        const th = parseThresholds(thresholds.text);
        return {
            "alerts.enabled": enabled.checked,
            // written highest first, without duplicates, as the daemon reads it
            "alerts.thresholds": th === null ? null : effective("alerts.thresholds", th),
            "alerts.critical": validCritical(critical.value) ? critical.value : null,
            "alerts.hysteresis": parseHysteresis(hysteresis.text),
            "battery.chemistry": chemistries[chemistry.currentIndex] || "alkaline",
            "notifications.connection": connection.checked,
            "notifications.battery_replaced": replaced.checked,
            "notifications.defer_to_powerdevil": powerdevil.checked,
            "display.apple_percent": applePct.checked,
            "apple.will_shutdown": willShutdown.checked
        };
    }
    function same(a, b) {
        return JSON.stringify(a) === JSON.stringify(b);
    }
    // Shows `m` (raw file values, or the defaults) as written: an untouched
    // value is never rounded, sorted or bounded on screen (#286).
    function fill(m) {
        enabled.checked = val("alerts.enabled", m);
        const th = val("alerts.thresholds", m);
        thresholds.text = Array.isArray(th) ? th.join(", ") : "";
        const c = val("alerts.critical", m);
        criticalMin = Math.max(-2147483647, Math.min(0, c));
        criticalMax = Math.min(2147483647, Math.max(pctMax, c));
        critical.value = c;
        hysteresis.text = hysteresisText(val("alerts.hysteresis", m));
        chemistry.currentIndex = Math.max(0, chemistries.indexOf(effective("battery.chemistry", val("battery.chemistry", m))));
        connection.checked = val("notifications.connection", m);
        replaced.checked = val("notifications.battery_replaced", m);
        powerdevil.checked = val("notifications.defer_to_powerdevil", m);
        applePct.checked = val("display.apple_percent", m);
        willShutdown.checked = val("apple.will_shutdown", m);
    }
    // The user changed `k` (a control's own edit signal, never a fill()).
    function touch(k) {
        const t = Object.assign({}, touched);
        t[k] = true;
        touched = t;
        update();
    }
    function update() {
        if (!ready) return;
        const c = current();
        let dirty = false;
        let defaults = true;
        for (const k in defaultsMap) {
            if (touched[k]) {
                // invalid entry: Apply stays enabled, save refuses (#285)
                if (c[k] === null) { dirty = true; defaults = false; continue; }
                if (!same(effective(k, c[k]), loaded[k])) dirty = true;
                if (!same(effective(k, c[k]), defaultsMap[k])) defaults = false;
            } else if (!same(loaded[k], defaultsMap[k])) {
                defaults = false;
            }
        }
        kcm.needsSave = dirty;
        kcm.representsDefaults = defaults;
    }
    // One note per value of the file the daemon does not take as written.
    function notesOf(r) {
        const notes = [];
        const th = r["alerts.thresholds"];
        if (th !== undefined && !same(th.filter(function (n) { return n >= 1 && n <= pctMax; }), th))
            notes.push("thresholds = [" + th.join(", ") + "]");
        const c = r["alerts.critical"];
        if (c !== undefined && !validCritical(c)) notes.push("critical = " + c);
        const h = r["alerts.hysteresis"];
        if (h !== undefined && (h < hysteresisMin || h > hysteresisMax)) notes.push("hysteresis = " + h);
        const ch = r["battery.chemistry"];
        if (ch !== undefined && chemistryOf(ch) === null) notes.push("chemistry = \"" + ch + "\"");
        return notes;
    }

    function load() {
        ready = false;
        root.store.readConfig(function (ok, text, error) {
            const r = {};
            const m = {};
            const p = ok ? Toml.parse(text) : { values: {}, kinds: {}, lines: {}, warnings: [] };
            const warnings = p.warnings.slice();
            for (const k in defaultsMap) {
                r[k] = readKey(k, p);
                m[k] = effective(k, r[k]);
                if (p.kinds[k] !== undefined && r[k] === undefined)
                    warnings.push(k + (p.lines[k] ? " (line " + p.lines[k] + ")" : ""));
            }
            raw = r;
            loaded = m;
            touched = ({});
            rangeNotes = notesOf(r);
            parseWarnings = warnings;
            fill(r);
            ready = true;
            update();
        });
    }
    function refuse(text) {
        result.type = Kirigami.MessageType.Error;
        result.text = text;
        result.visible = true;
    }
    function save() {
        if (saving) return;
        // #284: a file that could not be read whole is never written over.
        if (!root.store.configLoaded) {
            refuse(i18n("Not saved: the settings file could not be read (%1), so it is not written over. Fix the file or move it away, then reopen this page.", root.store.configError));
            return;
        }
        const c = current();
        // only what the user changed is checked and written (#286)
        if (touched["alerts.thresholds"] && c["alerts.thresholds"] === null) {
            refuse(i18n("Not saved: the thresholds must be whole numbers between 1 and 99, separated by commas."));
            return;
        }
        if (touched["alerts.critical"] && c["alerts.critical"] === null) {
            refuse(i18n("Not saved: the critical level must be a whole number between 0 and 99."));
            return;
        }
        if (touched["alerts.hysteresis"] && c["alerts.hysteresis"] === null) {
            refuse(i18n("Not saved: the re-arm margin must be a number between 1 and 20 (for example 3 or 2.5)."));
            return;
        }
        const written = [];
        let text = root.store.configText;
        try {
            for (const k in defaultsMap) {
                if (touched[k] && !same(effective(k, c[k]), loaded[k])) {
                    const dot = k.indexOf(".");
                    text = Toml.set(text, k.slice(0, dot), k.slice(dot + 1), c[k]);
                    written.push(k);
                }
            }
        } catch (e) {
            // #258: a form of TOML this editor does not rewrite safely
            // ([[table]], inline table, quoted key): nothing is written.
            refuse(i18n("Not saved: config.toml uses a form this page cannot edit safely (%1). Edit the file by hand.", String(e.message || e)));
            return;
        }
        saving = true;
        root.store.writeConfig(text, function (ok, error) {
            saving = false;
            if (ok) {
                const r = Object.assign({}, raw);
                const m = Object.assign({}, loaded);
                for (let i = 0; i < written.length; ++i) {
                    r[written[i]] = c[written[i]];
                    m[written[i]] = effective(written[i], c[written[i]]);
                }
                raw = r;
                loaded = m;
                touched = ({});
                rangeNotes = notesOf(r);
                savedOnce = true;
                update();
                result.type = Kirigami.MessageType.Positive;
                result.text = root.store.present
                    ? i18n("Saved. The service reads these settings when it starts: restart it to use them now.")
                    : i18n("Saved. They will be used when the service starts.");
                result.visible = true;
            } else {
                refuse(error === "changed"
                    ? i18n("Not saved: the settings file was changed by another program since this page read it. Nothing was written; use Reset to read it again.")
                    : i18n("Not saved: %1", error));
            }
        });
    }
    // "Defaults": every value is shown at its default and counts as changed.
    function defaults() {
        fill(defaultsMap);
        const t = {};
        for (const k in defaultsMap) t[k] = true;
        touched = t;
        update();
    }

    Kirigami.InlineMessage {
        id: result
        objectName: "alertsResult"
        Layout.fillWidth: true
        Layout.margins: Kirigami.Units.smallSpacing
        visible: false
        showCloseButton: true
        actions: [
            Kirigami.Action {
                id: restartAction
                visible: root.savedOnce && root.store.present && result.type === Kirigami.MessageType.Positive
                text: i18n("Restart the service")
                icon.name: "view-refresh"
                onTriggered: {
                    root.store.restartDaemon(function (ok, msg) {
                        result.type = ok ? Kirigami.MessageType.Positive : Kirigami.MessageType.Error;
                        result.text = ok ? i18n("Service restarted with the new settings.") : i18n("The service could not be restarted: %1", msg);
                        root.savedOnce = false;
                    });
                }
            }
        ]
    }
    Kirigami.InlineMessage {
        objectName: "alertsFileWarning"
        Layout.fillWidth: true
        Layout.margins: Kirigami.Units.smallSpacing
        visible: root.store.configError !== "" || root.parseWarnings.length > 0
        type: Kirigami.MessageType.Warning
        text: root.store.configError !== ""
            ? i18n("The settings file could not be read (%1): the defaults are shown and nothing can be saved, so that the file is never written over.", root.store.configError)
            : i18n("Some lines of the settings file are not understood (%1); they are kept as they are.", root.parseWarnings.join(", "))
    }

    Kirigami.InlineMessage {
        objectName: "alertsRangeWarning"
        Layout.fillWidth: true
        Layout.margins: Kirigami.Units.smallSpacing
        visible: root.rangeNotes.length > 0
        type: Kirigami.MessageType.Warning
        text: i18n("The service does not take these values of the settings file as they are written: %1. They are shown as written and kept unless you change them.", root.rangeNotes.join(", "))
    }

    Kirigami.FormLayout {
        Layout.fillWidth: true
        // #284: nothing to edit when the file could not be read
        enabled: root.ready && !root.saving && root.store.configLoaded

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Low battery")
        }
        QQC2.Switch {
            id: enabled
            objectName: "alerts_enabled"
            Kirigami.FormData.label: i18n("Alerts:")
            text: i18n("Notify when the battery is low")
            onToggled: root.touch("alerts.enabled")
        }
        QQC2.TextField {
            id: thresholds
            objectName: "alerts_thresholds"
            Kirigami.FormData.label: i18n("Thresholds (%):")
            enabled: enabled.checked
            placeholderText: "30, 15, 5"
            inputMethodHints: Qt.ImhFormattedNumbersOnly
            Accessible.name: i18n("Alert thresholds in percent, separated by commas")
            onTextEdited: root.touch("alerts.thresholds")
            color: root.parseThresholds(text) === null ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
            Accessible.description: i18n("Whole numbers from 1 to 99")
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            wrapMode: Text.Wrap
            font: Kirigami.Theme.smallFont
            text: i18n("One notification per threshold crossed, on the estimated charge.")
        }
        QQC2.Switch {
            id: powerdevil
            objectName: "alerts_powerdevil"
            Kirigami.FormData.label: i18n("With KDE:")
            enabled: enabled.checked
            text: i18n("When KDE already warns about this keyboard, send only one reminder")
            onToggled: root.touch("notifications.defer_to_powerdevil")
        }
        QQC2.SpinBox {
            id: critical
            objectName: "alerts_critical"
            Kirigami.FormData.label: i18n("Critical at or below (%):")
            enabled: enabled.checked
            // 0-99 as the daemon; a value of the file outside is shown as it is
            from: root.criticalMin
            to: root.criticalMax
            Accessible.name: i18n("Critical threshold in percent")
            onValueModified: root.touch("alerts.critical")
        }
        QQC2.TextField {
            id: hysteresis
            objectName: "alerts_hysteresis"
            Kirigami.FormData.label: i18n("Re-arm after (points):")
            enabled: enabled.checked
            // a decimal number, as the daemon reads it (1 to 20)
            inputMethodHints: Qt.ImhFormattedNumbersOnly
            placeholderText: "3"
            Accessible.name: i18n("Points above a threshold before it can notify again")
            Accessible.description: i18n("A number from 1 to 20, decimals allowed")
            onTextEdited: root.touch("alerts.hysteresis")
            color: root.parseHysteresis(text) === null ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Batteries")
        }
        QQC2.ComboBox {
            id: chemistry
            objectName: "alerts_chemistry"
            Kirigami.FormData.label: i18n("Battery type:")
            model: [i18nc("battery chemistry", "alkaline"), i18nc("battery chemistry", "NiMH (rechargeable)"), i18nc("battery chemistry", "lithium"), i18nc("battery chemistry", "unknown")]
            Accessible.name: i18n("Battery type")
            onActivated: root.touch("battery.chemistry")
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            wrapMode: Text.Wrap
            font: Kirigami.Theme.smallFont
            text: i18n("Used for the estimated charge and its range; the keyboard's own indication does not depend on it.")
        }
        QQC2.Switch {
            id: applePct
            objectName: "alerts_applePct"
            Kirigami.FormData.label: i18n("Display:")
            text: i18n("Also show the percentage as macOS shows it")
            onToggled: root.touch("display.apple_percent")
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Other notifications")
        }
        QQC2.Switch {
            id: connection
            objectName: "alerts_connection"
            Kirigami.FormData.label: i18n("Connection:")
            text: i18n("Notify on disconnection and reconnection")
            onToggled: root.touch("notifications.connection")
        }
        QQC2.Switch {
            id: replaced
            objectName: "alerts_replaced"
            Kirigami.FormData.label: i18n("New batteries:")
            text: i18n("Notify when new batteries are detected")
            onToggled: root.touch("notifications.battery_replaced")
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18nc("@title:group", "Shutdown")
        }
        QQC2.Switch {
            id: willShutdown
            objectName: "alerts_willShutdown"
            Kirigami.FormData.label: i18n("WillShutdown:")
            text: i18n("Tell the keyboard when the computer shuts down or restarts")
            onToggled: root.touch("apple.will_shutdown")
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            wrapMode: Text.Wrap
            font: Kirigami.Theme.smallFont
            text: i18n("What macOS sends at every shutdown (one message, nothing else is written). Off = the keyboard is left untouched.")
        }

        Kirigami.Separator {
            Kirigami.FormData.isSection: true
        }
        QQC2.Label {
            Kirigami.FormData.label: i18n("File:")
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            text: root.store.bridge ? root.store.bridge.configPath : ""
            font.family: "monospace"
            elide: Text.ElideMiddle
        }
    }
}
