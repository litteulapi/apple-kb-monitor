// The declared battery chemistry ([battery] chemistry, used by the estimate), in words.
import QtQuick

QtObject {
    property string code: ""

    readonly property string label: code === "alkaline" ? i18n("alkaline")
        : code === "nimh" ? i18n("NiMH")
        : code === "lithium" ? i18n("lithium")
        : code === "" ? "" : i18n("not declared")
}
