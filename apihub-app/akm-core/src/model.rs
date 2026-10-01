//! Supported Apple wireless keyboards and sysfs `uevent` parsing.

/// Apple USB vendor ID (`USB_VENDOR_ID_APPLE` in the kernel).
pub const APPLE_USB_VID: u32 = 0x05ac;
/// Apple Bluetooth SIG vendor ID (`BT_VENDOR_ID_APPLE`): Magic Keyboards
/// announce themselves with this vendor over Bluetooth.
pub const APPLE_BT_VID: u32 = 0x004c;

/// Hardware family, which decides what telemetry is safe to request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// BCM2042 aluminium wireless keyboards (A1255, A1314 2009/2011): answer
    /// the vendor Feature Reports (0xEA, 0xF5, ...).
    Bcm2042,
    /// Magic Keyboard 2015+ (BCM20733 / Apple silicon era): battery comes from
    /// the kernel only, never poll undeclared reports.
    MagicKeyboard,
    Unknown,
}

/// One supported keyboard model.
#[derive(Debug, Clone, Copy)]
pub struct ModelInfo {
    pub pid: u32,
    pub model: &'static str,
    pub chip: &'static str,
    pub family: Family,
}

const fn m(pid: u32, model: &'static str, chip: &'static str, family: Family) -> ModelInfo {
    ModelInfo {
        pid,
        model,
        chip,
        family,
    }
}

/// Wireless Apple keyboards, checked against the kernel's
/// `drivers/hid/hid-ids.h` and `hid-apple.c`. Wired models (ALU_ANSI 0x0220,
/// ALU_REVB 0x024f..) and internal ones (GEYSER4 0x0229) are deliberately absent.
pub const APPLE_MODELS: &[ModelInfo] = &[
    m(
        0x022c,
        "Apple Wireless Keyboard (A1255, aluminum, ANSI)",
        "BCM2042",
        Family::Bcm2042,
    ),
    m(
        0x022d,
        "Apple Wireless Keyboard (A1255, aluminum, ISO)",
        "BCM2042",
        Family::Bcm2042,
    ),
    m(
        0x022e,
        "Apple Wireless Keyboard (A1255, aluminum, JIS)",
        "BCM2042",
        Family::Bcm2042,
    ),
    m(
        0x0239,
        "Apple Wireless Keyboard (A1314, 2009, ANSI)",
        "BCM2042",
        Family::Bcm2042,
    ),
    m(
        0x023a,
        "Apple Wireless Keyboard (A1314, 2009, ISO)",
        "BCM2042",
        Family::Bcm2042,
    ),
    m(
        0x023b,
        "Apple Wireless Keyboard (A1314, 2009, JIS)",
        "BCM2042",
        Family::Bcm2042,
    ),
    m(
        0x0255,
        "Apple Wireless Keyboard (A1314, aluminum, ANSI)",
        "BCM2042",
        Family::Bcm2042,
    ),
    m(
        0x0256,
        "Apple Wireless Keyboard (A1314, aluminum, ISO)",
        "BCM2042",
        Family::Bcm2042,
    ),
    m(
        0x0257,
        "Apple Wireless Keyboard (A1314, aluminum, JIS)",
        "BCM2042",
        Family::Bcm2042,
    ),
    m(
        0x0267,
        "Apple Magic Keyboard 2015 (A1644)",
        "BCM20733",
        Family::MagicKeyboard,
    ),
    m(
        0x026c,
        "Apple Magic Keyboard with Numeric Keypad 2015 (A1843)",
        "BCM20733",
        Family::MagicKeyboard,
    ),
    m(
        0x029c,
        "Apple Magic Keyboard 2021 (A2450)",
        "Apple",
        Family::MagicKeyboard,
    ),
    m(
        0x029a,
        "Apple Magic Keyboard with Touch ID 2021 (A2449)",
        "Apple",
        Family::MagicKeyboard,
    ),
    m(
        0x029f,
        "Apple Magic Keyboard with Touch ID and Numeric Keypad 2021 (A2520)",
        "Apple",
        Family::MagicKeyboard,
    ),
    m(
        0x0320,
        "Apple Magic Keyboard 2024",
        "Apple",
        Family::MagicKeyboard,
    ),
    m(
        0x0321,
        "Apple Magic Keyboard with Touch ID 2024",
        "Apple",
        Family::MagicKeyboard,
    ),
    m(
        0x0322,
        "Apple Magic Keyboard with Touch ID and Numeric Keypad 2024",
        "Apple",
        Family::MagicKeyboard,
    ),
];

/// Look a (vendor, product) pair up. Both Apple vendors (USB 0x05AC and
/// Bluetooth 0x004C) are accepted for every model.
pub fn lookup_model(vid: u32, pid: u32) -> Option<&'static ModelInfo> {
    if vid != APPLE_USB_VID && vid != APPLE_BT_VID {
        return None;
    }
    APPLE_MODELS.iter().find(|mi| mi.pid == pid)
}

/// Family of a (vendor, product) pair; `Unknown` if not a supported keyboard.
pub fn family(vid: u32, pid: u32) -> Family {
    lookup_model(vid, pid).map_or(Family::Unknown, |mi| mi.family)
}

/// Parse `HID_ID=bus:vendor:product` out of a uevent (exact hex match on
/// fields, never a substring of MAC / name / modalias).
pub fn parse_hid_id(uevent: &str) -> Option<(u32, u32)> {
    let line = uevent.lines().find_map(|l| l.strip_prefix("HID_ID="))?;
    let mut parts = line.trim().split(':');
    let _bus = parts.next()?;
    let vid = u32::from_str_radix(parts.next()?, 16).ok()?;
    let pid = u32::from_str_radix(parts.next()?, 16).ok()?;
    Some((vid, pid))
}

/// Full model info of the keyboard described by a hidraw/HID uevent.
pub fn model_from_uevent(uevent: &str) -> Option<&'static ModelInfo> {
    let (vid, pid) = parse_hid_id(uevent)?;
    lookup_model(vid, pid)
}

/// Family of the keyboard described by a uevent (`Unknown` if none).
pub fn family_from_uevent(uevent: &str) -> Family {
    parse_hid_id(uevent).map_or(Family::Unknown, |(v, p)| family(v, p))
}

/// Identify an Apple keyboard from a uevent. Returns (model, chip).
pub fn apple_model_from_uevent(uevent: &str) -> Option<(&'static str, &'static str)> {
    model_from_uevent(uevent).map(|mi| (mi.model, mi.chip))
}

/// Real BT MAC (upper-case) from `HID_UNIQ=` (the 0x4C report holds an
/// internal identity, not the MAC).
pub fn mac_from_uevent(uevent: &str) -> Option<String> {
    let mac = uevent
        .lines()
        .find_map(|l| l.strip_prefix("HID_UNIQ="))?
        .trim()
        .to_uppercase();
    (mac.contains(':') && mac.len() >= 17).then_some(mac)
}

/// User-visible name from `HID_NAME=`.
pub fn name_from_uevent(uevent: &str) -> Option<String> {
    let n = uevent
        .lines()
        .find_map(|l| l.strip_prefix("HID_NAME="))?
        .trim();
    (!n.is_empty()).then(|| n.to_string())
}

/// Is this BlueZ Modalias an Apple device (USB 05AC or Bluetooth SIG 004C)?
/// Vendor only: AirPods, mice, trackpads and iPhones match too. Use
/// [`is_keyboard_device`] to select a keyboard.
pub fn is_apple_modalias(m: &str) -> bool {
    let m = m.to_ascii_lowercase();
    m.starts_with("usb:v05ac") || m.starts_with("bluetooth:v004c")
}

/// (vendor, product) of a BlueZ / kernel modalias: `usb:v05ACp0256d0050`,
/// `bluetooth:v004Cp029Cd0001`.
pub fn parse_modalias(m: &str) -> Option<(u32, u32)> {
    let (_, rest) = m.split_once(':')?;
    let rest = rest.strip_prefix(['v', 'V'])?;
    let vid = u32::from_str_radix(rest.get(..4)?, 16).ok()?;
    let rest = rest.get(4..)?.strip_prefix(['p', 'P'])?;
    let pid = u32::from_str_radix(rest.get(..4)?, 16).ok()?;
    Some((vid, pid))
}

/// Supported keyboard model behind a modalias (`None`: AirPods, mice, ...).
pub fn model_from_modalias(m: &str) -> Option<&'static ModelInfo> {
    parse_modalias(m).and_then(|(v, p)| lookup_model(v, p))
}

/// Bluetooth Class of Device: major class Peripheral (0x05) with the keyboard
/// bit (minor bit 6) set — keyboards and keyboard/pointer combos.
pub fn is_keyboard_class(cod: u32) -> bool {
    (cod >> 8) & 0x1f == 0x05 && cod & 0x40 != 0
}

/// Should a BlueZ device be treated as one of our keyboards? Its modalias
/// must be in the model table, and its Class of Device, when BlueZ knows it,
/// must say keyboard (#124: an Apple vendor ID alone is not enough).
pub fn is_keyboard_device(modalias: &str, class: Option<u32>) -> bool {
    model_from_modalias(modalias).is_some() && class.is_none_or(is_keyboard_class)
}

/// UPower object path that may be a keyboard battery. Generic: it does not
/// say WHICH keyboard (a `mouse_hid_...` or gamepad path no longer matches, #171);
/// prefer [`upower_path_matches_mac`] when the followed MAC is known.
pub fn is_keyboard_upower_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains("/devices/keyboard_") || p.contains("/devices/battery_hid")
}

/// Does this UPower object path belong to the device `mac` (`AA:BB:CC:DD:EE:FF`)?
/// UPower embeds the address with underscores (`hid_aa_bb_cc_dd_ee_ff_battery`).
pub fn upower_path_matches_mac(path: &str, mac: &str) -> bool {
    let tail = path.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    let want = mac.to_ascii_lowercase().replace(':', "_");
    want.len() == 17 && tail.contains(&want)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uevent_matches_exact_vendor_and_product() {
        let u =
            "DRIVER=hid-generic\nHID_ID=0005:000005AC:00000255\nHID_NAME=Apple Wireless Keyboard\n";
        let (m, c) = apple_model_from_uevent(u).unwrap();
        assert!(m.contains("A1314") && m.contains("ANSI"));
        assert_eq!(c, "BCM2042");
    }

    #[test]
    fn uevent_rejects_non_apple_and_substring_false_positives() {
        assert!(apple_model_from_uevent("HID_ID=0003:0000046D:00000255\n").is_none());
        let u = "HID_ID=0005:000005AC:00000999\nHID_UNIQ=aa:bb:02:55:cc:dd\nHID_PHYS=05AC0255\n";
        assert!(apple_model_from_uevent(u).is_none());
        assert!(apple_model_from_uevent("").is_none());
        assert!(apple_model_from_uevent("HID_ID=garbage\n").is_none());
        assert!(apple_model_from_uevent("HID_ID=0005:000005AC\n").is_none());
    }

    #[test]
    fn real_a1314_iso_uevent() {
        let u = "DRIVER=hid-generic\nHID_ID=0005:000005AC:00000256\nHID_NAME=Clavier de maria #1\nHID_PHYS=44:af:28:00:00:01\nHID_UNIQ=04:db:56:ca:42:ee\n";
        let mi = model_from_uevent(u).unwrap();
        assert_eq!(mi.pid, 0x0256);
        assert_eq!(mi.family, Family::Bcm2042);
        assert!(mi.model.contains("A1314") && mi.model.contains("ISO"));
        assert_eq!(mac_from_uevent(u).as_deref(), Some("04:DB:56:CA:42:EE"));
        assert_eq!(name_from_uevent(u).as_deref(), Some("Clavier de maria #1"));
        assert_eq!(family_from_uevent(u), Family::Bcm2042);
    }

    #[test]
    fn magic_keyboard_bluetooth_vendor_004c() {
        let mi = model_from_uevent("HID_ID=0005:0000004C:0000029C\n").unwrap();
        assert_eq!(mi.family, Family::MagicKeyboard);
        assert_eq!(family(0x004c, 0x0267), Family::MagicKeyboard);
        assert_eq!(family(0x05ac, 0x0321), Family::MagicKeyboard);
        assert!(lookup_model(0x004c, 0x029a)
            .unwrap()
            .model
            .contains("Touch ID"));
        assert!(lookup_model(0x004c, 0x0322).is_some());
    }

    #[test]
    fn corrected_pids_match_kernel_hid_ids() {
        for pid in [0x0220, 0x0229, 0x024f, 0x0250] {
            assert!(lookup_model(APPLE_USB_VID, pid).is_none(), "{:#06x}", pid);
        }
        assert!(lookup_model(APPLE_USB_VID, 0x022c)
            .unwrap()
            .model
            .contains("ANSI"));
        assert!(lookup_model(APPLE_USB_VID, 0x022d)
            .unwrap()
            .model
            .contains("ISO"));
        assert!(lookup_model(APPLE_USB_VID, 0x022e)
            .unwrap()
            .model
            .contains("JIS"));
        assert!(lookup_model(APPLE_USB_VID, 0x0267)
            .unwrap()
            .model
            .contains("2015"));
        assert!(lookup_model(APPLE_USB_VID, 0x026c)
            .unwrap()
            .model
            .contains("Numeric"));
    }

    #[test]
    fn bcm2042_gating_by_family() {
        let bcm: Vec<u32> = (0x022c..=0x022e)
            .chain(0x0239..=0x023b)
            .chain(0x0255..=0x0257)
            .collect();
        for mi in APPLE_MODELS {
            assert_eq!(
                mi.family == Family::Bcm2042,
                bcm.contains(&mi.pid),
                "{:#06x}",
                mi.pid
            );
        }
        for pid in &bcm {
            assert_eq!(family(APPLE_USB_VID, *pid), Family::Bcm2042);
        }
        assert_eq!(family(0x046d, 0x0256), Family::Unknown);
        assert_eq!(family(APPLE_USB_VID, 0x9999), Family::Unknown);
    }

    #[test]
    fn model_table_has_no_duplicate_pid() {
        for (i, a) in APPLE_MODELS.iter().enumerate() {
            assert!(
                APPLE_MODELS[i + 1..].iter().all(|b| b.pid != a.pid),
                "{:#06x}",
                a.pid
            );
        }
    }

    #[test]
    fn modalias_and_path_filters() {
        assert!(is_apple_modalias("usb:v05ACp0256d0050"));
        assert!(is_apple_modalias("bluetooth:v004Cp029Cd0001"));
        assert!(!is_apple_modalias("usb:v046Dp0001d0001"));
        assert_eq!(
            parse_modalias("usb:v05ACp0256d0050"),
            Some((0x05ac, 0x0256))
        );
        assert_eq!(
            parse_modalias("bluetooth:v004Cp029Cd0206"),
            Some((0x004c, 0x029c))
        );
        assert_eq!(parse_modalias("bluetooth:v004C"), None);
        assert_eq!(parse_modalias("garbage"), None);
        assert_eq!(parse_modalias(""), None);
        assert!(is_keyboard_upower_path(
            "/org/freedesktop/UPower/devices/keyboard_hid_04o_db"
        ));
        assert!(!is_keyboard_upower_path(
            "/org/freedesktop/UPower/devices/battery_BAT0"
        ));
        // #171: other HID devices are not keyboards.
        let mouse = "/org/freedesktop/UPower/devices/mouse_hid_ec_2e_ee_a1_b2_c3_battery";
        assert!(!is_keyboard_upower_path(mouse));
        assert!(!is_keyboard_upower_path(
            "/org/freedesktop/UPower/devices/gaming_input_hid_aa_bb_battery"
        ));
        let kb = "/org/freedesktop/UPower/devices/keyboard_hid_04_DB_56_CA_42_EE_battery";
        assert!(upower_path_matches_mac(kb, "04:DB:56:CA:42:EE"));
        assert!(!upower_path_matches_mac(kb, "EC:2E:EE:A1:B2:C3"));
        assert!(!upower_path_matches_mac(mouse, "04:DB:56:CA:42:EE"));
        assert!(!upower_path_matches_mac(kb, ""));
    }

    #[test]
    fn only_keyboards_pass_the_device_filter() {
        // Keyboards (BCM2042 over the USB vendor, Magic Keyboard over the BT one).
        assert!(is_keyboard_device("usb:v05ACp0256d0050", Some(0x002540)));
        assert!(is_keyboard_device("bluetooth:v004Cp029Cd0206", None));
        assert!(is_keyboard_device("usb:v05ACp0255d0050", Some(0x0005c0))); // combo
                                                                            // Apple, not keyboards (#124).
        for m in [
            "bluetooth:v004Cp200Ed0001", // AirPods
            "bluetooth:v004Cp0269d0001", // Magic Mouse 2
            "bluetooth:v004Cp0265d0001", // Magic Trackpad 2
            "bluetooth:v004Cp0324d0001", // Magic Trackpad 2 USB-C
            "usb:v05ACp030Dd0001",       // Magic Mouse 1
            "usb:v05ACp030Ed0001",       // Magic Trackpad 1
            "bluetooth:v004Cp1234d0001", // phone-ish
            "usb:v046Dp0256d0001",       // Logitech, same PID
        ] {
            assert!(!is_keyboard_device(m, None), "{m}");
        }
        // Table PID but a non-keyboard class (headset 0x240404, mouse 0x002580).
        assert!(!is_keyboard_device("usb:v05ACp0256d0050", Some(0x240404)));
        assert!(!is_keyboard_device("usb:v05ACp0256d0050", Some(0x002580)));
        assert!(is_keyboard_class(0x002540));
        assert!(!is_keyboard_class(0x7a020c)); // phone
    }
}
