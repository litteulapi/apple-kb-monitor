import QtQuick
import org.kde.plasma.core as PlasmaCore
import "../com.agenceapi.devicehub/contents/ui/Menu.js" as Menu

Item {
    id: t
    property list<QtObject> acts
    property bool failed: false
    Component { id: obj; QtObject { property int n: 0 } }
    Component { id: act; PlasmaCore.Action {} }
    PlasmaCore.ActionGroup { id: fnGroup }
    function check(name, ok) {
        if (!ok) { console.log("FAIL menu: " + name); failed = true; Qt.exit(1); }
    }
    Component.onCompleted: {
        var json = JSON.stringify([
            { id: 1, label: "Header", enabled: true, visible: true, checked: null },
            { id: 12, label: "Refresh", enabled: true, visible: true, checked: null },
            { id: 18, label: "Fn mode", enabled: true, visible: true, checked: null },
            { id: 19, label: "Media", enabled: true, visible: true, checked: false },
            { id: 22, label: "F1", enabled: true, visible: true, checked: true },
            { id: 24, label: "Disconnect", enabled: false, visible: true, checked: null },
            { id: 25, label: "Forget", enabled: true, visible: false, checked: null }]);
        var m = Menu.build(json);
        check("entries", m.length === 6 && m[0].id === 12 && m[1].separator && m[2].id === 19
              && m[3].checked === true && m[4].separator && m[5].id === 24 && !m[5].enabled);
        var sep = { isSeparator: true }, a12 = { isSeparator: false, itemId: 12 }, a19 = { isSeparator: false, itemId: 19 };
        check("same shape", Menu.sameShape([a12, sep, a19], [{ id: 12 }, { separator: true }, { id: 19 }]));
        check("other order", !Menu.sameShape([a19, sep, a12], [{ id: 12 }, { separator: true }, { id: 19 }]));
        check("other length", !Menu.sameShape([a12], [{ id: 12 }, { separator: true }, { id: 19 }]));
        check("separator moved", !Menu.sameShape([a12, a19, sep], [{ id: 12 }, { separator: true }, { id: 19 }]));
        // the two Fn modes are radio items: the checked one stays checked when chosen again
        var media = act.createObject(t), f1 = act.createObject(t), refresh = act.createObject(t);
        Menu.applyCheck(media, m[2], fnGroup);
        Menu.applyCheck(f1, m[3], fnGroup);
        Menu.applyCheck(refresh, m[0], fnGroup);
        check("Fn modes grouped", media.actionGroup === fnGroup && f1.actionGroup === fnGroup && refresh.actionGroup === null && !refresh.checkable);
        f1.trigger();
        check("checked mode stays checked", f1.checked && !media.checked);
        media.trigger();
        check("one mode at a time", media.checked && !f1.checked);
        t.acts = [obj.createObject(t, { n: 1 })];
        for (var round = 0; round < 2; ++round) {
            var old = Menu.swap(t, "acts", [obj.createObject(t, { n: 2 }), obj.createObject(t, { n: 3 })]);
            for (var j = 0; j < old.length; ++j) old[j].destroy();
        }
        Qt.callLater(function () {
            check("new actions survive the swap", t.acts.length === 2 && t.acts[0] !== null
                  && t.acts[1] !== null && t.acts[0].n === 2 && t.acts[1].n === 3);
            if (!t.failed) console.log("PASS menu");
            Qt.exit(0);
        });
    }
}
