// Store.qml on a private bus whose activation directory holds a fake
// com.agenceapi.AppleKbMonitor1.service (store_timeouts.sh): no read of the
// module may D-Bus-activate a stopped daemon.
import QtQuick
import "../ui"

Item {
    id: test

    Store { id: store }

    Component.onCompleted: {
        store.refreshAll();
        store.fetchVersion();
        store.fetchKeymap();
        store.getConfig(function () {});
        store.setAlias("AA:BB:CC:DD:EE:F1", "x", function () {});
        store.reconnect(function () {});
        store.keymapCall("Keymap", "", [], 1000, function () {});
    }
    Timer {
        interval: 2000; running: true
        onTriggered: {
            if (store.present) {
                console.log("FAIL store-activation: the daemon was activated");
                Qt.exit(1);
            }
            console.log("PASS store-activation");
            Qt.exit(0);
        }
    }
}
