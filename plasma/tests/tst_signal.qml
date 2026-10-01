import QtQuick
import "../com.agenceapi.devicehub/contents/ui/Signal.js" as Sig

// Pure-function test of the RSSI logic of the widget (#174), no D-Bus.
Item {
    function check(name, ok) {
        if (!ok) { console.log("FAIL signal: " + name); Qt.exit(1); }
    }
    Component.onCompleted: {
        // 0 is the IDEAL range, a real value (the widget used to read it as invalid or "0 dBm").
        check("zero is valid", Sig.rssiOf({ rssi_rel_db: 0 }) === 0);
        check("positive is legal", Sig.rssiOf({ rssi_rel_db: 3 }) === 3);
        check("old name", Sig.rssiOf({ rssi_dbm: -2 }) === -2);
        check("new name wins", Sig.rssiOf({ rssi_rel_db: -1, rssi_dbm: -9 }) === -1);
        check("127 unknown", isNaN(Sig.rssiOf({ rssi_rel_db: 127 })));
        check("null unknown", isNaN(Sig.rssiOf({ rssi_rel_db: null })));
        check("absent unknown", isNaN(Sig.rssiOf({})) && isNaN(Sig.rssiOf(null)));
        check("out of range", isNaN(Sig.rssiOf({ rssi_rel_db: -300 })));
        check("excellent", Sig.qualityOf(0) === "excellent" && Sig.qualityOf(4) === "excellent");
        check("good", Sig.qualityOf(-1) === "good" && Sig.qualityOf(-5) === "good");
        check("weak", Sig.qualityOf(-6) === "weak" && Sig.qualityOf(-40) === "weak");
        check("unknown quality", Sig.qualityOf(NaN) === "");
        check("raw text", Sig.rawRssi(0) === "0" && Sig.rawRssi(-3) === "−3" && Sig.rawRssi(2) === "+2");
        console.log("PASS signal");
        Qt.exit(0);
    }
}
