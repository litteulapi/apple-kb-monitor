// Store.qml deadlines against fake_slow_daemon.py (store_timeouts.sh).
import QtQuick
import "../ui"

Item {
    id: test
    property bool setDone: false
    property bool setOk: false
    property bool failed: false

    function fail(why) {
        failed = true;
        console.log("FAIL store: " + why);
        Qt.exit(1);
    }

    Store { id: store }

    // GetState answers after 8 s: "timeout" at 5 s, and the late reply is ignored.
    Timer {
        interval: 6500; running: true
        onTriggered: {
            if (store.stateError !== "timeout") test.fail("stateError at 6.5 s = " + store.stateError);
            store.setParam("fnmode", 2, function (ok, msg) {
                test.setDone = true;
                test.setOk = ok;
                if (!ok) test.fail("setParam: " + msg);
            });
        }
    }
    // RunAkmctl never answers; the parameter appears in KeyTable: success well before the bus gives up.
    Timer {
        interval: 13000; running: true
        onTriggered: {
            if (store.state !== null) test.fail("late GetState reply was applied");
            if (store.stateError !== "timeout") test.fail("stateError at 13 s = " + store.stateError);
            if (!test.setDone || !test.setOk) test.fail("setParam not settled by its probe");
            if (test.failed) return;
            console.log("PASS store");
            Qt.exit(0);
        }
    }
}
