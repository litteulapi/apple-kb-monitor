// Store.qml with no daemon on the bus (store_timeouts.sh): an absent daemon
// leaves no file or key-table error, the module's header reports it alone.
import QtQuick
import "../ui"

Item {
    id: test

    function fail(why) {
        console.log("FAIL store-absent: " + why);
        Qt.exit(1);
    }

    Store { id: store }

    Component.onCompleted: {
        store.getConfig(function (ok, values, error) {
            if (ok || store.configLoaded) test.fail("config loaded without a daemon");
            if (error !== "" || store.configError !== "") test.fail("configError = " + store.configError);
        });
        store.fetchKeys();
    }
    Timer {
        interval: 1500; running: true
        onTriggered: {
            if (store.present) test.fail("a daemon is on the test bus");
            if (store.keyTableBusy) test.fail("key table still busy");
            if (store.keyTableError !== "") test.fail("keyTableError = " + store.keyTableError);
            const masked = store.maskMacs('{"mac":"aa:bb:cc:dd:ee:f1","paired_host_addr":"11:22:33:44:55:66"} /dev_AA_BB_CC_DD_EE_F1 x0:11:22:33:44:55');
            if (masked !== '{"mac":"AA:BB:XX:XX:XX:F1","paired_host_addr":"11:22:XX:XX:XX:66"} /dev_AA_BB_XX_XX_XX_F1 x0:11:22:33:44:55')
                test.fail("maskMacs: " + masked);
            console.log("PASS store-absent");
            Qt.exit(0);
        }
    }
}
