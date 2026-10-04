// Why the signal is not measured, from `radio.rssi_error.code`: same words as the menu (akm_core::rssi).
import QtQuick

QtObject {
    property string code: ""

    readonly property string why: code === "helper_missing" ? i18n("rssi-helper not installed")
        : i18n("see akmctl doctor")
    readonly property string shortText: i18n("not measured (%1)", why)
    readonly property string reason: code === "helper_missing" ? i18n("the rssi-helper utility is not installed")
        : code === "helper_failed" ? i18n("the rssi-helper utility failed (missing capability, Bluetooth refused)")
        : code === "timeout" ? i18n("the rssi-helper utility does not answer in time")
        : code === "unavailable" ? i18n("the Bluetooth adapter gives no measure for this link")
        : i18n("the signal cannot be measured")
    readonly property string fix: code === "helper_missing" ? i18n("reinstall the apple-kb-monitor package")
        : code === "helper_failed" ? i18n("reinstall the apple-kb-monitor package, then run akmctl doctor")
        : code === "timeout" ? i18n("check the Bluetooth service: systemctl status bluetooth")
        : code === "unavailable" ? i18n("bring the keyboard closer, or reconnect it")
        : i18n("run akmctl doctor")
}
