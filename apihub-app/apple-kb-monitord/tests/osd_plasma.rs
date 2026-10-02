//! #100 / #101 on a private bus: the daemon's OSD sink calls
//! `org.kde.osdService.showText` of a fake plasmashell. Nothing reaches the
//! real desktop.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use apple_kb_monitord::notify::Lang;
use apple_kb_monitord::osd::{Enabled, Osd, PlasmaOsd, OSD_INTERFACE, OSD_PATH, OSD_SERVICE};

const INNER: &str = "AKM_OSD_INNER";

struct FakeOsd(Mutex<Sender<(String, String)>>);

#[zbus::interface(name = "org.kde.osdService")]
impl FakeOsd {
    #[zbus(name = "showText")]
    fn show_text(&self, icon: String, text: String) {
        let _ = self.0.lock().unwrap().send((icon, text));
    }
}

fn inner() {
    assert_eq!(
        (OSD_SERVICE, OSD_PATH, OSD_INTERFACE),
        (
            "org.kde.plasmashell",
            "/org/kde/osdService",
            "org.kde.osdService"
        )
    );
    let osd = Osd::new(
        Arc::new(PlasmaOsd::default()),
        Lang::Fr,
        Enabled {
            fn_mode: true,
            caps_lock: true,
        },
    );
    // No plasmashell on the bus: nothing happens, nothing fails.
    osd.caps(false);
    assert!(osd.caps(true));

    let (tx, rx) = mpsc::channel();
    let _shell = zbus::blocking::ConnectionBuilder::session()
        .unwrap()
        .name(OSD_SERVICE)
        .unwrap()
        .serve_at(OSD_PATH, FakeOsd(Mutex::new(tx)))
        .unwrap()
        .build()
        .unwrap();
    let wait = Duration::from_secs(5);

    // Caps Lock released, then the Fn mode changes.
    assert!(osd.caps(false));
    assert_eq!(
        rx.recv_timeout(wait).unwrap(),
        (
            "input-keyboard".to_string(),
            "Verr. Maj d\u{e9}sactiv\u{e9}".to_string()
        )
    );
    assert!(osd.caps(true));
    assert_eq!(
        rx.recv_timeout(wait).unwrap(),
        (
            "input-caps-on".to_string(),
            "Verr. Maj activ\u{e9}".to_string()
        )
    );
    osd.fn_mode(Some(1));
    assert!(osd.fn_mode(Some(2)));
    assert_eq!(
        rx.recv_timeout(wait).unwrap().1,
        "Mode Fn\u{a0}: F1\u{2013}F12 d'abord"
    );
    // Unchanged: no call at all.
    assert!(!osd.fn_mode(Some(2)) && !osd.caps(true));
    assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
}

#[test]
fn osd_goes_to_plasma_on_the_session_bus() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    apple_kb_monitord::testbus::rerun_test("osd_goes_to_plasma_on_the_session_bus", INNER, &[]);
}
