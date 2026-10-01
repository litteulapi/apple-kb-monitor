//! `akm-core` against the files captured on the real A1314 ISO keyboard
//! (`tests/fixtures/a1314_iso`, read-only capture of 2026-10-01): no keyboard,
//! no D-Bus needed.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use akm_core::decode::{build_report, Fixture, HidSource};
use akm_core::hidraw::{
    find_apple_hidraw_in, find_apple_keyboard_mac_in, hid_uevent_for_mac_in, report_from_sysfs_in,
};
use akm_core::model::{family_from_uevent, Family};
use akm_core::power::{kernel_battery_in, BatteryStatus};
use akm_core::report::KbWake;

const MAC: &str = "04:DB:56:CA:42:EE";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn read(rel: &str) -> String {
    fs::read_to_string(fixtures().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

static N: AtomicU32 = AtomicU32::new(0);

/// Fake `/sys` rebuilt from the captured files.
struct FakeSys(PathBuf);
impl FakeSys {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "akm-a1314-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&root);
        let put = |rel: &str, c: &str| {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, c).unwrap();
        };
        put(
            "bus/hid/devices/0005:05AC:0256.0014/uevent",
            &read("a1314_iso/hid_device.uevent"),
        );
        let ps = "class/power_supply/hid-04:db:56:ca:42:ee-battery-71";
        for attr in [
            "capacity",
            "status",
            "present",
            "online",
            "model_name",
            "scope",
            "type",
        ] {
            put(
                &format!("{ps}/{attr}"),
                &read(&format!("a1314_iso/ps_{attr}")),
            );
        }
        put(
            "class/hidraw/hidraw7/device/uevent",
            &read("a1314_iso/hid_device.uevent"),
        );
        FakeSys(root)
    }
}
impl Drop for FakeSys {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn captured_uevent_is_a_bcm2042_a1314_iso() {
    let u = read("a1314_iso/hid_device.uevent");
    assert_eq!(family_from_uevent(&u), Family::Bcm2042);
    assert_eq!(akm_core::model::mac_from_uevent(&u).as_deref(), Some(MAC));
}

#[test]
fn kernel_battery_from_captured_power_supply_is_90() {
    let sys = FakeSys::new();
    let b = kernel_battery_in(&sys.0, MAC).expect("kernel battery");
    assert_eq!(b.percent, 90);
    assert_eq!(b.status, BatteryStatus::Discharging);
    assert_eq!(
        kernel_battery_in(&sys.0, &MAC.to_lowercase())
            .unwrap()
            .percent,
        90
    );
}

#[test]
fn sysfs_only_report_without_hidraw() {
    let sys = FakeSys::new();
    assert!(hid_uevent_for_mac_in(&sys.0, MAC).is_some());
    let r = report_from_sysfs_in(&sys.0, MAC).expect("report");
    assert_eq!(r.battery_pct(), Some(90.0));
    assert_eq!(r.device.name.as_deref(), Some("Clavier de maria #1"));
    assert!(r
        .device
        .model
        .as_deref()
        .unwrap()
        .contains("A1314, aluminum, ISO"));
    assert!(r.bluetooth.connected);
    assert!(r.battery.voltage.is_none(), "no invented voltage");
    assert!(report_from_sysfs_in(&sys.0, "AA:BB:CC:DD:EE:FF").is_none());
    // Discovery without BlueZ: MAC from the HID bus, node from class/hidraw.
    assert_eq!(find_apple_keyboard_mac_in(&sys.0).as_deref(), Some(MAC));
    assert_eq!(
        find_apple_hidraw_in(&sys.0).as_deref(),
        Some("/dev/hidraw7")
    );
}

#[test]
fn read_keyboard_on_fixture_frames() {
    let sys = FakeSys::new();
    let uevent = read("a1314_iso/hid_device.uevent");
    // Real vendor frames of the same keyboard (tests/live/re, 0x4C redacted).
    let dump = fs::read_to_string(fixtures().join("../live/re/a1314_iso_frames.hex")).unwrap();
    let src = Fixture::from_hex_dump(&dump).unwrap();
    assert_eq!(src.feature(0x47).unwrap(), vec![0x47, 99]);
    let kernel = kernel_battery_in(&sys.0, MAC);
    let r = build_report(&uevent, kernel, &src, KbWake::default()).expect("keyboard answers");
    // The kernel capacity of the capture (90) wins over 0x47 and 0xEA.
    assert_eq!(r.battery_pct(), Some(90.0));
    assert_eq!(r.battery.percentage, Some(90.0));
    assert_eq!(r.battery.adc_raw, Some(900));
    // #139: real voltage from 0x46 (LE) = 2991 mV, coherent with 0xFF.
    assert_eq!(r.battery.voltage_mv, Some(2991));
    assert_eq!(r.battery.voltage_filtered_mv, Some(2953));
    assert!(!r.battery.voltage_doubtful);
    assert_eq!(r.battery.percentage_estimate, Some(99.9));
    assert_eq!(r.firmware.version.as_deref(), Some("0x0050"));
    assert_eq!(r.raw.get("0x46").map(String::as_str), Some("af0b"));
    assert_eq!(r.raw.get("0xff").map(String::as_str), Some("0baf01"));
    assert!(!r.incomplete);
    assert_eq!(r.device.mac.as_deref(), Some(MAC));
    // Keyboard asleep: probe 0xEA unanswered -> no report at all.
    let asleep = Fixture::from_hex_dump(&read("synthetic/get_feature_0x47_battery90.hex")).unwrap();
    assert!(build_report(&uevent, kernel, &asleep, KbWake::default()).is_none());
}
