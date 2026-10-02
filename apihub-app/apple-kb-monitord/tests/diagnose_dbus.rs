//! #120 on a private bus: `Diagnose()` returns the checks of the Diag tab as
//! JSON. The probe is a fixture: no program of the machine is run, no real
//! daemon, no keyboard.

use std::path::PathBuf;
use std::sync::Arc;

use akm_core::Watch;
use apple_kb_monitord::actor::Mailbox;
use apple_kb_monitord::diagnose::{Probe, Run};
use apple_kb_monitord::service;
use zbus::blocking::Connection;

const INNER: &str = "AKM_DIAGNOSE_INNER";

struct Fixture(PathBuf);

impl Probe for Fixture {
    fn run(&self, bin: &str, _args: &[&str]) -> Run {
        match bin {
            "bluetoothctl" => Run::Ok("bluetoothctl: 5.87".into()),
            _ => Run::Failed(3, String::new()),
        }
    }
    fn path(&self, absolute: &str) -> PathBuf {
        self.0.join(absolute.trim_start_matches('/'))
    }
    fn apple_hidraw(&self) -> Option<String> {
        Some("/dev/hidraw9".into())
    }
    fn readable(&self, _path: &str) -> Result<(), String> {
        Ok(())
    }
}

fn inner() {
    let root = std::env::temp_dir().join(format!("akm-diagbus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let params = root.join("sys/module/hid_apple/parameters");
    std::fs::create_dir_all(&params).unwrap();
    std::fs::write(params.join("fnmode"), "2\n").unwrap();

    let mut so = service::ServeOptions::new(Arc::new(Watch::new()), Mailbox::new(), None);
    so.diag = Arc::new(Fixture(root.clone()));
    let _server = service::serve_with(so).expect("serve");

    let c = Connection::session().unwrap();
    let json: String = c
        .call_method(
            Some(service::BUS_NAME),
            service::OBJECT_PATH,
            Some(service::INTERFACE),
            "Diagnose",
            &(),
        )
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["daemon_version"], env!("CARGO_PKG_VERSION"));
    let checks = v["checks"].as_array().unwrap();
    assert_eq!(v["total"], checks.len());
    let ok = |id: &str| {
        let c = checks.iter().find(|c| c["id"] == id).expect(id);
        assert!(c["label"].as_str().is_some_and(|l| !l.is_empty()), "{id}");
        assert!(c["detail"].as_str().is_some_and(|d| !d.is_empty()), "{id}");
        c["ok"].as_bool().unwrap()
    };
    assert!(ok("daemon") && ok("dbus") && ok("bluetoothctl") && ok("hidraw") && ok("fnmode"));
    assert!(ok("keymap"), "no hwdb = kernel mapping");
    assert!(!ok("service") && !ok("udev") && !ok("rssi_helper"));
    assert_eq!(v["passed"], 6);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn diagnose_on_the_bus() {
    if std::env::var_os(INNER).is_some() {
        inner();
        return;
    }
    apple_kb_monitord::testbus::rerun_test("diagnose_on_the_bus", INNER, &[("LC_ALL", "C")]);
}
