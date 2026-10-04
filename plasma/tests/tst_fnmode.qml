// SetFnMode with no reply from the bus (polkit answered late): the widget watches FnMode
// instead of reporting a failure (fake_services.py, mode 3), and follows its PropertiesChanged.
import QtQuick
import "../com.agenceapi.devicehub/contents/ui"

Item {
    id: t
    property bool set3: false
    property bool followed: false
    function done() {
        if (!set3 || !followed) return;
        console.log("PASS fnmode-late");
        Qt.exit(0);
    }
    DaemonLink {
        id: link
        deviceMac: "AA:BB:CC:DD:EE:F1"
        onFnModeSet: function (mode) {
            if (mode !== 3) { console.log("FAIL fnmode-late: set " + mode); Qt.exit(1); }
            t.set3 = true;
            t.done();
        }
        // PropertiesChanged of the keyboard is followed, whoever changed the mode.
        onFnModeReceived: function (mode) {
            if (mode === 3) { t.followed = true; t.done(); }
        }
        onFnModeFailed: function (m) { console.log("FAIL fnmode-late: " + m); Qt.exit(1); }
    }
    Timer { interval: 1000; running: true; onTriggered: link.setFnMode("AA:BB:CC:DD:EE:F1", 3) }
    Timer { interval: 12000; running: true; onTriggered: { console.log("FAIL fnmode-late: no outcome"); Qt.exit(1); } }
}
