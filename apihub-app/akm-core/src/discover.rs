//! Keyboard discovery and selection (#124, #129).
//!
//! An Apple vendor ID is not enough to call a device a keyboard: AirPods,
//! Magic Mouse, Magic Trackpad and iPhones share it. A device is a keyboard
//! when its (vendor, product) is in the model table ([`crate::model`]) **and**,
//! when the kernel exposes it, its HID report descriptor declares a
//! Generic Desktop / Keyboard application collection.
//!
//! Several keyboards may be connected at once: [`select_keyboard_in`] picks the
//! one with the requested MAC, else the first one (sorted by hidraw node).
//!
//! The wake monitor (input report 0x13) only makes sense for keyboards whose
//! descriptor declares that report (BCM2042 A1255/A1314): [`wake_supported`].

use std::path::{Path, PathBuf};

use crate::model::{mac_from_uevent, model_from_uevent, Family, ModelInfo};

/// Vendor input report of the BCM2042 keyboards: bit 0 `FF01:000A`
/// (device ready), bit 1 `FF01:000C` (connection request).
pub const WAKE_REPORT_ID: u8 = 0x13;

/// What a HID report descriptor declares, as far as selection needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RdescInfo {
    /// An application collection with usage Generic Desktop / Keyboard (0x0001_0006).
    pub keyboard: bool,
    /// Report IDs carrying at least one Input main item (0 = no report ID).
    pub input_reports: Vec<u8>,
}

impl RdescInfo {
    pub fn has_input_report(&self, id: u8) -> bool {
        self.input_reports.contains(&id)
    }
}

/// Minimal HID report descriptor walk (short items; long items skipped).
pub fn parse_rdesc(d: &[u8]) -> RdescInfo {
    let mut info = RdescInfo::default();
    let mut usage_page: u32 = 0;
    let mut report_id: u8 = 0;
    let mut usages: Vec<u32> = Vec::new();
    let mut i = 0;
    while i < d.len() {
        let b = d[i];
        if b == 0xFE {
            // Long item: bDataSize, bLongItemTag, data.
            let n = d.get(i + 1).copied().unwrap_or(0) as usize;
            i += 3 + n;
            continue;
        }
        let size = match b & 0x03 {
            3 => 4,
            s => s as usize,
        };
        let Some(raw) = d.get(i + 1..i + 1 + size) else {
            break;
        };
        let val = raw
            .iter()
            .enumerate()
            .fold(0u32, |acc, (k, &x)| acc | (u32::from(x) << (8 * k)));
        let kind = (b >> 2) & 0x03;
        let tag = b >> 4;
        match (kind, tag) {
            (1, 0x0) => usage_page = val,              // Usage Page
            (1, 0x8) => report_id = val as u8,         // Report ID
            (2, 0x0) if size == 4 => usages.push(val), // extended Usage
            (2, 0x0) => usages.push((usage_page << 16) | val),
            (0, 0x8) => {
                // Input
                if !info.input_reports.contains(&report_id) {
                    info.input_reports.push(report_id);
                }
                usages.clear();
            }
            (0, 0xA) => {
                // Collection: 1 = Application
                if val == 1 && usages.first() == Some(&0x0001_0006) {
                    info.keyboard = true;
                }
                usages.clear();
            }
            (0, _) => usages.clear(), // Output, Feature, End Collection
            _ => {}
        }
        i += 1 + size;
    }
    info
}

/// Decoded wake report (0x13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WakeBits {
    /// Bit 0, `FF01:000A`: device ready.
    pub ready: bool,
    /// Bit 1, `FF01:000C`: connection request.
    pub conn_request: bool,
}

/// Decode an input report as a wake event. `None` for any other report and
/// for an all-zero 0x13 (release of the 1-bit fields, not an event).
pub fn decode_wake(report: &[u8]) -> Option<WakeBits> {
    match report {
        [WAKE_REPORT_ID, bits, ..] if bits & 0x03 != 0 => Some(WakeBits {
            ready: bits & 0x01 != 0,
            conn_request: bits & 0x02 != 0,
        }),
        _ => None,
    }
}

/// One Apple keyboard seen through `/sys/class/hidraw`.
#[derive(Debug, Clone)]
pub struct KeyboardNode {
    /// `/dev/hidrawN`.
    pub hidraw: String,
    /// sysfs directory of the hidraw class entry.
    pub sysfs: PathBuf,
    pub model: &'static ModelInfo,
    /// Real BT MAC (upper case), `None` over USB.
    pub mac: Option<String>,
    /// Parsed report descriptor, `None` if sysfs did not expose it.
    pub rdesc: Option<RdescInfo>,
}

impl KeyboardNode {
    /// Does this keyboard send the 0x13 wake report? From the descriptor when
    /// readable, else from the family (only BCM2042 declares it).
    pub fn wake_supported(&self) -> bool {
        match &self.rdesc {
            Some(r) => r.has_input_report(WAKE_REPORT_ID),
            None => self.model.family == Family::Bcm2042,
        }
    }
}

/// Every Apple keyboard under a sysfs root, sorted by hidraw node. A device
/// whose descriptor is readable but declares no keyboard collection is
/// rejected even if its PID is in the table.
pub fn list_keyboards_in(sys: &Path) -> Vec<KeyboardNode> {
    let mut entries: Vec<_> = std::fs::read_dir(sys.join("class/hidraw"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    entries.sort();
    entries
        .into_iter()
        .filter_map(|(name, p)| {
            let uevent = std::fs::read_to_string(p.join("device/uevent")).ok()?;
            let model = model_from_uevent(&uevent)?;
            let rdesc = std::fs::read(p.join("device/report_descriptor"))
                .ok()
                .filter(|d| !d.is_empty())
                .map(|d| parse_rdesc(&d));
            if rdesc.as_ref().is_some_and(|r| !r.keyboard) {
                return None;
            }
            Some(KeyboardNode {
                hidraw: format!("/dev/{name}"),
                sysfs: p,
                model,
                mac: mac_from_uevent(&uevent),
                rdesc,
            })
        })
        .collect()
}

/// The keyboard to work on: the one with `mac` if given (and present),
/// else the first one.
pub fn select_keyboard_in(sys: &Path, mac: Option<&str>) -> Option<KeyboardNode> {
    let all = list_keyboards_in(sys);
    match mac {
        Some(m) => all.into_iter().find(|k| {
            k.mac
                .as_deref()
                .is_some_and(|km| km.eq_ignore_ascii_case(m))
        }),
        None => all.into_iter().next(),
    }
}

/// [`select_keyboard_in`] on `/sys`.
pub fn select_keyboard(mac: Option<&str>) -> Option<KeyboardNode> {
    select_keyboard_in(Path::new("/sys"), mac)
}

/// Should the wake monitor run for this keyboard? `false` when no keyboard
/// is present.
pub fn wake_supported_in(sys: &Path, mac: Option<&str>) -> bool {
    select_keyboard_in(sys, mac).is_some_and(|k| k.wake_supported())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn fixture(rel: &str) -> Vec<u8> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(rel);
        let txt = std::fs::read_to_string(&p).unwrap();
        let clean: String = txt.split_whitespace().collect();
        (0..clean.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn a1314_descriptor_is_a_keyboard_with_wake_report() {
        let r = parse_rdesc(&fixture("a1314_iso/report_descriptor.hex"));
        assert!(r.keyboard);
        for id in [0x01, 0x11, 0x12, 0x13, 0x47] {
            assert!(r.has_input_report(id), "{id:#04x}");
        }
    }

    #[test]
    fn magic_keyboard_2021_descriptor_has_no_wake_report() {
        let r = parse_rdesc(&fixture("models/mk2021_0005_004c_029c_bt.hex"));
        assert!(r.keyboard);
        assert!(!r.has_input_report(WAKE_REPORT_ID));
        assert!(r.has_input_report(0x90));
    }

    #[test]
    fn mouse_descriptor_is_not_a_keyboard() {
        // Generic Desktop / Mouse application collection with one input.
        let mouse = [
            0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x85, 0x12, 0x09, 0x01, 0xA1, 0x00, 0x05, 0x09,
            0x19, 0x01, 0x29, 0x02, 0x15, 0x00, 0x25, 0x01, 0x95, 0x02, 0x75, 0x01, 0x81, 0x02,
            0xC0, 0xC0,
        ];
        let r = parse_rdesc(&mouse);
        assert!(!r.keyboard);
        assert_eq!(r.input_reports, vec![0x12]);
        assert_eq!(parse_rdesc(&[]), RdescInfo::default());
        assert_eq!(parse_rdesc(&[0x05]), RdescInfo::default()); // truncated
    }

    #[test]
    fn wake_bits_decoded_and_zero_ignored() {
        assert_eq!(
            decode_wake(&[0x13, 0x01]),
            Some(WakeBits {
                ready: true,
                conn_request: false
            })
        );
        assert_eq!(
            decode_wake(&[0x13, 0x02]),
            Some(WakeBits {
                ready: false,
                conn_request: true
            })
        );
        assert_eq!(
            decode_wake(&[0x13, 0x03]),
            Some(WakeBits {
                ready: true,
                conn_request: true
            })
        );
        assert_eq!(decode_wake(&[0x13, 0x00]), None);
        assert_eq!(decode_wake(&[0x13, 0xFC]), None); // padding bits only
        assert_eq!(decode_wake(&[0x13]), None);
        assert_eq!(decode_wake(&[0x01, 0x03]), None);
        assert_eq!(decode_wake(&[]), None);
    }

    static T: AtomicU32 = AtomicU32::new(0);

    struct Sys(PathBuf);
    impl Sys {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "akm-discover-{}-{}",
                std::process::id(),
                T.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Sys(p)
        }
        fn hidraw(&self, node: &str, hid_id: &str, mac: &str, rdesc: Option<&[u8]>) {
            let d = self.0.join("class/hidraw").join(node).join("device");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("uevent"),
                format!("HID_ID={hid_id}\nHID_NAME=x\nHID_UNIQ={mac}\n"),
            )
            .unwrap();
            if let Some(r) = rdesc {
                std::fs::write(d.join("report_descriptor"), r).unwrap();
            }
        }
    }
    impl Drop for Sys {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn non_keyboards_never_selected() {
        let s = Sys::new();
        s.hidraw(
            "hidraw0",
            "0005:000005AC:0000030D",
            "aa:bb:cc:dd:ee:01",
            None,
        ); // Magic Mouse 1
        s.hidraw(
            "hidraw1",
            "0005:0000004C:00000269",
            "aa:bb:cc:dd:ee:02",
            None,
        ); // Magic Mouse 2
        s.hidraw(
            "hidraw2",
            "0005:0000004C:00000265",
            "aa:bb:cc:dd:ee:03",
            None,
        ); // Trackpad 2
        assert!(list_keyboards_in(&s.0).is_empty());
        assert!(select_keyboard_in(&s.0, None).is_none());
        let a1314 = fixture("a1314_iso/report_descriptor.hex");
        s.hidraw(
            "hidraw3",
            "0005:000005AC:00000256",
            "04:db:56:ca:42:ee",
            Some(&a1314),
        );
        let k = select_keyboard_in(&s.0, None).unwrap();
        assert_eq!(k.hidraw, "/dev/hidraw3");
        assert_eq!(k.mac.as_deref(), Some("04:DB:56:CA:42:EE"));
        assert!(k.wake_supported());
    }

    #[test]
    fn table_pid_with_non_keyboard_descriptor_is_rejected() {
        let s = Sys::new();
        let mouse = [0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x81, 0x02, 0xC0];
        s.hidraw(
            "hidraw0",
            "0005:000005AC:00000256",
            "aa:bb:cc:dd:ee:01",
            Some(&mouse),
        );
        assert!(list_keyboards_in(&s.0).is_empty());
    }

    #[test]
    fn several_keyboards_selected_by_mac() {
        let s = Sys::new();
        let mk = fixture("models/mk2021_0005_004c_029c_bt.hex");
        let a1314 = fixture("a1314_iso/report_descriptor.hex");
        s.hidraw(
            "hidraw0",
            "0005:0000004C:0000029C",
            "aa:bb:cc:dd:ee:02",
            Some(&mk),
        );
        s.hidraw(
            "hidraw1",
            "0005:000005AC:00000256",
            "04:db:56:ca:42:ee",
            Some(&a1314),
        );
        assert_eq!(list_keyboards_in(&s.0).len(), 2);
        let first = select_keyboard_in(&s.0, None).unwrap();
        assert_eq!(first.model.pid, 0x029c);
        assert!(!first.wake_supported());
        let a = select_keyboard_in(&s.0, Some("04:DB:56:CA:42:EE")).unwrap();
        assert_eq!(a.hidraw, "/dev/hidraw1");
        assert!(wake_supported_in(&s.0, Some("04:db:56:ca:42:ee")));
        assert!(!wake_supported_in(&s.0, Some("AA:BB:CC:DD:EE:02")));
        assert!(select_keyboard_in(&s.0, Some("11:22:33:44:55:66")).is_none());
        assert!(!wake_supported_in(&s.0, Some("11:22:33:44:55:66")));
    }

    #[test]
    fn unreadable_descriptor_falls_back_to_family() {
        let s = Sys::new();
        s.hidraw(
            "hidraw0",
            "0005:0000004C:00000267",
            "aa:bb:cc:dd:ee:02",
            None,
        );
        let k = select_keyboard_in(&s.0, None).unwrap();
        assert!(!k.wake_supported());
        s.hidraw(
            "hidraw1",
            "0005:000005AC:00000255",
            "aa:bb:cc:dd:ee:03",
            None,
        );
        assert!(wake_supported_in(&s.0, Some("AA:BB:CC:DD:EE:03")));
    }
}
