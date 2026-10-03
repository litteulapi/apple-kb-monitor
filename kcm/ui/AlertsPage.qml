// SPDX-License-Identifier: GPL-2.0-or-later
// Notifications and alerts: the daemon's config.toml, saved by the KCM
// "Apply" button (only the changed keys are rewritten; comments and unknown
// keys are kept). The daemon reads the file when it starts.
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
    property var loaded: ({})        // values read from the file (defaults filled in)
    property bool ready: false
    property bool saving: false
    property bool savedOnce: false
    property var parseWarnings: []

    spacing: Kirigami.Units.smallSpacing

    function val(k, m) {
        return m[k] !== undefined ? m[k] : defaultsMap[k];
    }
    // The value the daemon takes for `k` in the parsed file, else the default.
    function readKey(k, p) {
        const v = Toml.typed(p, k, kindsMap[k]);
        return v !== undefined ? v : defaultsMap[k];
    }
    function parseThresholds(t) {
        const parts = String(t).split(/[ ,;]+/).filter(function (x) { return x !== ""; });
        if (parts.length === 0 || parts.length > 8) return null;
        const out = [];
        for (let i = 0; i < parts.length; ++i) {
            if (!/^\d{1,2}$/.test(parts[i])) return null;
            const n = Number(parts[i]);
            if (n < 1 || n > 95) return null;
            if (out.indexOf(n) < 0) out.push(n);
        }
        out.sort(function (a, b) { return b - a; });
        return out;
    }
    // What the form says now.
    function current() {
        return {
            "alerts.enabled": enabled.checked,
            "alerts.thresholds": parseThresholds(thresholds.text),
            "alerts.critical": critical.value,
            "alerts.hysteresis": hysteresis.value,
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
    function fill(m) {
        enabled.checked = val("alerts.enabled", m);
        const th = val("alerts.thresholds", m);
        thresholds.text = Array.isArray(th) ? th.join(", ") : "";
        critical.value = Math.round(val("alerts.critical", m));
        hysteresis.value = Math.round(val("alerts.hysteresis", m));
        chemistry.currentIndex = Math.max(0, chemistries.indexOf(String(val("battery.chemistry", m)).toLowerCase()));
        connection.checked = val("notifications.connection", m);
        replaced.checked = val("notifications.battery_replaced", m);
        powerdevil.checked = val("notifications.defer_to_powerdevil", m);
        applePct.checked = val("display.apple_percent", m);
        willShutdown.checked = val("apple.will_shutdown", m);
    }
    function update() {
        if (!ready) return;
        const c = current();
        let dirty = false;
        let defaults = true;
        for (const k in defaultsMap) {
            // invalid entry: keep Apply enabled, save refuses (kcm.cpp keeps
            // it enabled after the refusal, #285)
            if (c[k] === null) { dirty = true; continue; }
            if (!same(c[k], loaded[k])) dirty = true;
            if (!same(c[k], defaultsMap[k])) defaults = false;
        }
        kcm.needsSave = dirty;
        kcm.representsDefaults = defaults;
    }

    function load() {
        ready = false;
        root.store.readConfig(function (ok, text, error) {
            const m = {};
            const p = ok ? Toml.parse(text) : { values: {}, kinds: {}, lines: {}, warnings: [] };
            const warnings = p.warnings.slice();
            for (const k in defaultsMap) {
                m[k] = readKey(k, p);
                if (p.kinds[k] !== undefined && Toml.typed(p, k, kindsMap[k]) === undefined)
                    warnings.push(k + (p.lines[k] ? " (line " + p.lines[k] + ")" : ""));
            }
            loaded = m;
            parseWarnings = warnings;
            fill(m);
            ready = true;
            update();
        });
    }
    function save() {
        if (saving) return;
        // #284: a file that could not be read whole is never written over.
        if (!root.store.configLoaded) {
            result.type = Kirigami.MessageType.Error;
            result.text = i18n("Not saved: the settings file could not be read (%1), so it is not written over. Fix the file or move it away, then reopen this page.", root.store.configError);
            result.visible = true;
            return;
        }
        const c = current();
        if (c["alerts.thresholds"] === null) {
            result.type = Kirigami.MessageType.Error;
            result.text = i18n("Not saved: the thresholds must be whole numbers between 1 and 95, separated by commas.");
            result.visible = true;
            return;
        }
        let text = root.store.configText;
        try {
            for (const k in defaultsMap) {
                if (!same(c[k], loaded[k])) {
                    const dot = k.indexOf(".");
                    text = Toml.set(text, k.slice(0, dot), k.slice(dot + 1), c[k]);
                }
            }
        } catch (e) {
            // #258: a form of TOML this editor does not rewrite safely
            // ([[table]], inline table, quoted key): nothing is written.
            result.type = Kirigami.MessageType.Error;
            result.text = i18n("Not saved: config.toml uses a form this page cannot edit safely (%1). Edit the file by hand.", String(e.message || e));
            result.visible = true;
            return;
        }
        saving = true;
        root.store.writeConfig(text, function (ok, error) {
            saving = false;
            if (ok) {
                loaded = c;
                savedOnce = true;
                update();
                result.type = Kirigami.MessageType.Positive;
                result.text = root.store.present
                    ? i18n("Saved. The service reads these settings when it starts: restart it to use them now.")
                    : i18n("Saved. They will be used when the service starts.");
            } else {
                result.type = Kirigami.MessageType.Error;
                result.text = error === "changed"
                    ? i18n("Not saved: the settings file was changed by another program since this page read it. Nothing was written; use Reset to read it again.")
                    : i18n("Not saved: %1", error);
            }
            result.visible = true;
        });
    }
    function defaults() {
        fill(defaultsMap);
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
            onToggled: root.update()
        }
        QQC2.TextField {
            id: thresholds
            objectName: "alerts_thresholds"
            Kirigami.FormData.label: i18n("Thresholds (%):")
            enabled: enabled.checked
            placeholderText: "30, 15, 5"
            inputMethodHints: Qt.ImhFormattedNumbersOnly
            Accessible.name: i18n("Alert thresholds in percent, separated by commas")
            onTextEdited: root.update()
            color: root.parseThresholds(text) === null ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
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
            onToggled: root.update()
        }
        QQC2.SpinBox {
            id: critical
            objectName: "alerts_critical"
            Kirigami.FormData.label: i18n("Critical at or below (%):")
            enabled: enabled.checked
            from: 0
            to: 95
            Accessible.name: i18n("Critical threshold in percent")
            onValueModified: root.update()
        }
        QQC2.SpinBox {
            id: hysteresis
            objectName: "alerts_hysteresis"
            Kirigami.FormData.label: i18n("Re-arm after (points):")
            enabled: enabled.checked
            from: 1
            to: 20
            Accessible.name: i18n("Points above a threshold before it can notify again")
            onValueModified: root.update()
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
            onActivated: root.update()
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
            onToggled: root.update()
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
            onToggled: root.update()
        }
        QQC2.Switch {
            id: replaced
            objectName: "alerts_replaced"
            Kirigami.FormData.label: i18n("New batteries:")
            text: i18n("Notify when new batteries are detected")
            onToggled: root.update()
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
            onToggled: root.update()
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
