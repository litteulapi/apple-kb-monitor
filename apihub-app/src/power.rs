//! Kernel `power_supply` battery reader for Bluetooth HID keyboards.
//!
//! The kernel (hid-input.c quirks / hid-apple.c) already exposes the battery of
//! Apple keyboards as `/sys/class/power_supply/hid-<mac>-battery[-<id>]`, and
//! UPower reads the very same node. This module is the single source of truth
//! for the battery percentage; raw HID reports are diagnostics only.
//!
//! Design: pure parsers (`parse_capacity`, `parse_status`, `mac_matches_name`)
//! are separated from the I/O (`kernel_battery_in`), and every function that
//! touches the filesystem takes the sysfs root so it can run on a fake tree.
//! No `unsafe`, no subprocess.

use std::fs;
use std::path::{Path, PathBuf};

/// Charging state as reported by the power_supply `status` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryStatus {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown,
}

/// One kernel battery sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryReading {
    /// 0..=100
    pub percent: u8,
    pub status: BatteryStatus,
}

/// Parse the `capacity` attribute: an integer 0..=100, surrounding whitespace allowed.
pub fn parse_capacity(raw: &str) -> Option<u8> {
    let v: u8 = raw.trim().parse().ok()?;
    (v <= 100).then_some(v)
}

/// Parse the `status` attribute (case-insensitive; anything else is `Unknown`).
pub fn parse_status(raw: &str) -> BatteryStatus {
    match raw.trim().to_ascii_lowercase().as_str() {
        "charging" => BatteryStatus::Charging,
        "discharging" => BatteryStatus::Discharging,
        "full" => BatteryStatus::Full,
        "not charging" => BatteryStatus::NotCharging,
        _ => BatteryStatus::Unknown,
    }
}

/// Normalise a MAC to lowercase `aa:bb:cc:dd:ee:ff`; `None` if malformed.
/// Strict on purpose: the value is used to build filesystem paths.
pub fn normalize_mac(mac: &str) -> Option<String> {
    let mac = mac.trim();
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    if !parts.iter().all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit())) {
        return None;
    }
    Some(mac.to_ascii_lowercase())
}

/// True if a power_supply directory name is the battery of `mac_lower`:
/// `hid-<mac>-battery` (old kernels) or `hid-<mac>-battery-<digits>` (new).
pub fn mac_matches_name(name: &str, mac_lower: &str) -> bool {
    let prefix = format!("hid-{}-battery", mac_lower);
    let Some(rest) = name.to_ascii_lowercase().strip_prefix(&prefix).map(str::to_owned) else {
        return false;
    };
    rest.is_empty()
        || rest
            .strip_prefix('-')
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

/// Locate the power_supply directory of the keyboard with this MAC.
///
/// 1. Preferred (robust to naming changes): HID device whose `HID_UNIQ` equals
///    the MAC, then its `power_supply/*` child.
/// 2. Fallback: scan `class/power_supply` for `hid-<mac>-battery[-<id>]`.
pub fn find_power_supply(sysfs_root: &Path, mac: &str) -> Option<PathBuf> {
    let mac_lower = normalize_mac(mac)?;

    if let Ok(rd) = fs::read_dir(sysfs_root.join("bus/hid/devices")) {
        let mut devs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        devs.sort();
        for dev in devs {
            let uevent = fs::read_to_string(dev.join("uevent")).unwrap_or_default();
            let uniq = uevent.lines().find_map(|l| l.strip_prefix("HID_UNIQ="));
            if !uniq.is_some_and(|u| u.trim().eq_ignore_ascii_case(&mac_lower)) {
                continue;
            }
            if let Ok(ps) = fs::read_dir(dev.join("power_supply")) {
                let mut names: Vec<_> = ps.flatten().map(|e| e.file_name()).collect();
                names.sort();
                if let Some(n) = names.into_iter().next() {
                    return Some(sysfs_root.join("class/power_supply").join(n));
                }
            }
        }
    }

    let rd = fs::read_dir(sysfs_root.join("class/power_supply")).ok()?;
    let mut names: Vec<_> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| mac_matches_name(n, &mac_lower))
        .collect();
    names.sort();
    names
        .into_iter()
        .next()
        .map(|n| sysfs_root.join("class/power_supply").join(n))
}

/// Read a battery from an explicit power_supply directory.
/// `None` if the device is marked absent, or `capacity` is unreadable/invalid
/// (the kernel returns an I/O error when the keyboard does not answer).
pub fn read_power_supply(dir: &Path) -> Option<BatteryReading> {
    if let Ok(p) = fs::read_to_string(dir.join("present")) {
        if p.trim() == "0" {
            return None;
        }
    }
    let percent = parse_capacity(&fs::read_to_string(dir.join("capacity")).ok()?)?;
    let status = fs::read_to_string(dir.join("status"))
        .map(|s| parse_status(&s))
        .unwrap_or(BatteryStatus::Unknown);
    Some(BatteryReading { percent, status })
}

/// Same as [`kernel_battery`] with a configurable sysfs root (for tests).
pub fn kernel_battery_in(sysfs_root: &Path, mac: &str) -> Option<BatteryReading> {
    read_power_supply(&find_power_supply(sysfs_root, mac)?)
}

/// Battery of the keyboard with this Bluetooth MAC, from the kernel.
/// Source of truth for the percentage. Reading `capacity` makes the kernel
/// issue a HID GET_REPORT (same as UPower does every 30 s); do not call it in
/// a tight loop.
pub fn kernel_battery(mac: &str) -> Option<BatteryReading> {
    kernel_battery_in(Path::new("/sys"), mac)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    /// Self-cleaning temp dir (no extra dependency).
    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "kbpower-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn write(&self, rel: &str, content: &str) {
            let f = self.0.join(rel);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, content).unwrap();
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const MAC: &str = "04:DB:56:CA:42:EE";

    #[test]
    fn capacity_parsing() {
        assert_eq!(parse_capacity("90\n"), Some(90));
        assert_eq!(parse_capacity(" 0 "), Some(0));
        assert_eq!(parse_capacity("100"), Some(100));
        assert_eq!(parse_capacity("101"), None);
        assert_eq!(parse_capacity("-1"), None);
        assert_eq!(parse_capacity(""), None);
        assert_eq!(parse_capacity("abc"), None);
    }

    #[test]
    fn status_parsing() {
        assert_eq!(parse_status("Discharging\n"), BatteryStatus::Discharging);
        assert_eq!(parse_status("charging"), BatteryStatus::Charging);
        assert_eq!(parse_status("Full"), BatteryStatus::Full);
        assert_eq!(parse_status("Not charging"), BatteryStatus::NotCharging);
        assert_eq!(parse_status("whatever"), BatteryStatus::Unknown);
        assert_eq!(parse_status(""), BatteryStatus::Unknown);
    }

    #[test]
    fn mac_normalisation_is_strict() {
        assert_eq!(normalize_mac(MAC).as_deref(), Some("04:db:56:ca:42:ee"));
        assert_eq!(normalize_mac("04:db:56:ca:42:ee").as_deref(), Some("04:db:56:ca:42:ee"));
        assert!(normalize_mac("04:db:56:ca:42").is_none());
        assert!(normalize_mac("04:db:56:ca:42:zz").is_none());
        assert!(normalize_mac("../../etc:passwd:x:y:z:1").is_none());
        assert!(normalize_mac("").is_none());
    }

    #[test]
    fn name_matching() {
        let m = "04:db:56:ca:42:ee";
        assert!(mac_matches_name("hid-04:db:56:ca:42:ee-battery", m));
        assert!(mac_matches_name("hid-04:db:56:ca:42:ee-battery-71", m));
        assert!(mac_matches_name("HID-04:DB:56:CA:42:EE-BATTERY-71", m));
        assert!(!mac_matches_name("hid-04:db:56:ca:42:ee-battery-", m));
        assert!(!mac_matches_name("hid-04:db:56:ca:42:ee-battery-x1", m));
        assert!(!mac_matches_name("hid-04:db:56:ca:42:ef-battery-71", m));
        assert!(!mac_matches_name("BAT0", m));
    }

    #[test]
    fn reads_by_name_fallback() {
        let t = Tmp::new();
        let d = "class/power_supply/hid-04:db:56:ca:42:ee-battery-71";
        t.write(&format!("{d}/capacity"), "90\n");
        t.write(&format!("{d}/status"), "Discharging\n");
        t.write(&format!("{d}/present"), "1\n");
        // Decoy for another keyboard and the laptop battery.
        t.write("class/power_supply/hid-aa:bb:cc:dd:ee:ff-battery-71/capacity", "12\n");
        t.write("class/power_supply/BAT0/capacity", "55\n");
        let r = kernel_battery_in(&t.0, MAC).unwrap();
        assert_eq!(r, BatteryReading { percent: 90, status: BatteryStatus::Discharging });
        assert_eq!(kernel_battery_in(&t.0, "aa:bb:cc:dd:ee:ff").unwrap().percent, 12);
    }

    #[test]
    fn reads_via_hid_parent_with_unusual_name() {
        let t = Tmp::new();
        t.write(
            "bus/hid/devices/0005:05AC:0256.0014/uevent",
            "HID_ID=0005:000005AC:00000256\nHID_UNIQ=04:db:56:ca:42:ee\n",
        );
        // Name the fallback would never match.
        t.write("bus/hid/devices/0005:05AC:0256.0014/power_supply/kbd-battery/x", "");
        t.write("class/power_supply/kbd-battery/capacity", "73\n");
        t.write("class/power_supply/kbd-battery/status", "Charging\n");
        let r = kernel_battery_in(&t.0, MAC).unwrap();
        assert_eq!(r, BatteryReading { percent: 73, status: BatteryStatus::Charging });
    }

    #[test]
    fn missing_status_is_unknown_and_bad_capacity_is_none() {
        let t = Tmp::new();
        let d = "class/power_supply/hid-04:db:56:ca:42:ee-battery";
        t.write(&format!("{d}/capacity"), "40\n");
        assert_eq!(kernel_battery_in(&t.0, MAC).unwrap().status, BatteryStatus::Unknown);
        t.write(&format!("{d}/capacity"), "garbage\n");
        assert!(kernel_battery_in(&t.0, MAC).is_none());
    }

    #[test]
    fn absent_device_and_missing_tree_give_none() {
        let t = Tmp::new();
        assert!(kernel_battery_in(&t.0, MAC).is_none());
        let d = "class/power_supply/hid-04:db:56:ca:42:ee-battery-71";
        t.write(&format!("{d}/capacity"), "90\n");
        t.write(&format!("{d}/present"), "0\n");
        assert!(kernel_battery_in(&t.0, MAC).is_none());
        assert!(kernel_battery_in(&t.0, "not-a-mac").is_none());
    }
}
