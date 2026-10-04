//! HID report descriptor walk and wake report decoding, kept for the descriptor fixtures' tests.

/// Vendor input report of the BCM2042 keyboards: bit 0 `FF01:000A`, bit 1 `FF01:000C`.
pub const WAKE_REPORT_ID: u8 = 0x13;

/// What a HID report descriptor declares, as far as selection needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RdescInfo {
    /// An application collection with usage Generic Desktop / Keyboard (`0x0001_0006`).
    pub keyboard: bool,
    /// Report IDs carrying at least one Input main item (0 = no report ID).
    pub input_reports: Vec<u8>,
}

impl RdescInfo {
    #[must_use]
    pub fn has_input_report(&self, id: u8) -> bool {
        self.input_reports.contains(&id)
    }
}

/// Minimal HID report descriptor walk (short items; long items skipped).
#[must_use]
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
            (1, 0x0) => usage_page = val,                 // Usage Page
            (1, 0x8) => report_id = val.to_le_bytes()[0], // Report ID
            (2, 0x0) if size == 4 => usages.push(val),    // extended Usage
            (2, 0x0) => usages.push((usage_page << 16) | val),
            (0, 0x8) => {
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

#[must_use]
pub fn decode_wake(report: &[u8]) -> Option<WakeBits> {
    match report {
        [WAKE_REPORT_ID, bits, ..] if bits & 0x03 != 0 => Some(WakeBits {
            ready: bits & 0x01 != 0,
            conn_request: bits & 0x02 != 0,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
}
