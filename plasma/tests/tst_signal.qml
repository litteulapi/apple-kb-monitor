import QtQuick
import "../com.agenceapi.devicehub/contents/ui/Signal.js" as Sig
import "../com.agenceapi.devicehub/contents/ui/Pip.js" as Pip

Item {
    function check(name, ok) {
        if (!ok) { console.log("FAIL signal: " + name); Qt.exit(1); }
    }
    Component.onCompleted: {
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
        check("num is locale fixed-point", Pip.num(2.5, 2) === "2" + Qt.locale().decimalPoint + "50");
        console.log("PASS signal");
        Qt.exit(0);
    }
}
