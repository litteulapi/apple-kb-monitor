//! Decoding of the BCM2042 vendor Feature Reports, independent of the device
//! (docs/REVUE-ARCHITECTURE-GLOBALE.md §4.6): anything implementing
//! [`HidSource`] can be decoded — the real `/dev/hidrawN`
//! ([`crate::hidraw::Hidraw`]) or a [`Fixture`] made of recorded frames.

use std::collections::HashMap;
use std::io;

use crate::calibration::{adc_to_voltage, interpolate_battery, parse_calibration, DEFAULT_CALIBRATION_MV};
use crate::model::{family_from_uevent, mac_from_uevent, model_from_uevent, Family};
use crate::power::BatteryReading;
use crate::report::{KbReport, KbWake};

// ── HID Report IDs ────────────────────────────────────────────────────────

/// Battery precise — pre-rounding value (also the connectivity probe)
pub const HID_BATTERY_PRECISE: u8 = 0xEA;
/// Battery standard — firmware-rounded percentage
pub const HID_BATTERY_STANDARD: u8 = 0x47;
/// ADC raw voltage — 10-bit, 3.3 V reference
pub const HID_ADC_RAW: u8 = 0xF5;
/// Calibration curve — 4 x u16 mV thresholds [100%, 75%, 50%, 25%]
pub const HID_CALIBRATION: u8 = 0x5A;
/// Firmware version — high nibble = major, low nibble = minor
pub const HID_FIRMWARE_VERSION: u8 = 0x4F;
/// Firmware build — u16 build number + u8 flag
pub const HID_FIRMWARE_BUILD: u8 = 0xFF;
/// Device name chunks 1..3
pub const HID_NAME_1: u8 = 0x51;
pub const HID_NAME_2: u8 = 0x52;
pub const HID_NAME_3: u8 = 0x53;
/// Connection parameters — BT interval + latency
pub const HID_CONNECTION_PARAMS: u8 = 0x46;
/// Supervision timeout — LE u16 x 10 ms
pub const HID_SUPERVISION_TIMEOUT: u8 = 0x49;
/// ADC reference — factory calibration constant
pub const HID_ADC_REF: u8 = 0xF4;
/// Device identity — BCM2042 internal identity key (NOT the BT MAC)
pub const HID_DEVICE_IDENTITY: u8 = 0x4C;
/// Device state — 1=OK, 0=LOW
pub const HID_DEVICE_STATE: u8 = 0x09;

/// Every Feature Report a full BCM2042 read requests, in order.
pub const BCM2042_READ_ORDER: [u8; 14] = [
    HID_BATTERY_PRECISE,
    HID_BATTERY_STANDARD,
    HID_ADC_RAW,
    HID_CALIBRATION,
    HID_FIRMWARE_VERSION,
    HID_FIRMWARE_BUILD,
    HID_NAME_1,
    HID_NAME_2,
    HID_NAME_3,
    HID_CONNECTION_PARAMS,
    HID_SUPERVISION_TIMEOUT,
    HID_ADC_REF,
    HID_DEVICE_IDENTITY,
    HID_DEVICE_STATE,
];

/// A source of HID Feature Reports. The returned buffer starts with the report id.
pub trait HidSource {
    fn feature(&self, report_id: u8) -> io::Result<Vec<u8>>;
}

/// Recorded frames, for tests without hardware. Unknown ids answer `NotFound`
/// (like an undeclared report on the real device).
#[derive(Debug, Clone, Default)]
pub struct Fixture {
    reports: HashMap<u8, Vec<u8>>,
}

impl Fixture {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one frame (`bytes[0]` is the report id).
    pub fn with(mut self, bytes: &[u8]) -> Self {
        if let Some(&id) = bytes.first() {
            self.reports.insert(id, bytes.to_vec());
        }
        self
    }

    /// Parse a dump: one frame per line, hex bytes (spaces optional), `#` comments.
    /// Example line: `ea 5a` or `5a0b5409920 92e07d0`.
    pub fn from_hex_dump(text: &str) -> Result<Self, String> {
        let mut f = Self::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let hex: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            if !hex.len().is_multiple_of(2) {
                return Err(format!("line {}: odd number of hex digits", n + 1));
            }
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|e| format!("line {}: {e}", n + 1))?;
            f = f.with(&bytes);
        }
        Ok(f)
    }

    pub fn len(&self) -> usize {
        self.reports.len()
    }

    pub fn is_empty(&self) -> bool {
        self.reports.is_empty()
    }
}

impl HidSource for Fixture {
    fn feature(&self, report_id: u8) -> io::Result<Vec<u8>> {
        self.reports
            .get(&report_id)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no report {report_id:#04x}")))
    }
}

fn get(src: &dyn HidSource, id: u8) -> Option<Vec<u8>> {
    src.feature(id).ok().filter(|b| !b.is_empty())
}

/// Decode the BCM2042 vendor reports into `report`. Returns `false` when the
/// probe (0xEA) does not answer: the keyboard is not reachable, nothing else
/// is requested (avoids HIDP timeouts that cause disconnections).
pub fn decode_bcm2042(src: &dyn HidSource, report: &mut KbReport) -> bool {
    let Some(probe) = get(src, HID_BATTERY_PRECISE) else {
        return false;
    };
    if probe.len() >= 2 {
        report.battery.percentage_fine = Some(f64::from(probe[1]));
    }

    if let Some(buf) = get(src, HID_BATTERY_STANDARD) {
        if buf.len() >= 2 && report.battery.percentage.is_none() {
            report.battery.percentage = Some(f64::from(buf[1]));
        }
    }

    if let Some(buf) = get(src, HID_ADC_RAW) {
        if buf.len() >= 3 {
            let adc = u32::from(u16::from_be_bytes([buf[1], buf[2]]));
            report.battery.adc_raw = Some(adc);
            report.battery.voltage = Some(adc_to_voltage(adc));
        }
    }

    let calib = get(src, HID_CALIBRATION)
        .and_then(|b| parse_calibration(&b))
        .unwrap_or(DEFAULT_CALIBRATION_MV);
    if let Some(voltage) = report.battery.voltage {
        report.battery.percentage_interpolated = Some(interpolate_battery(voltage, &calib).round());
    }

    if let Some(buf) = get(src, HID_FIRMWARE_VERSION) {
        if buf.len() >= 2 {
            report.firmware.version = Some(format!("{}.{}", buf[1] >> 4, buf[1] & 0x0F));
        }
    }

    if let Some(buf) = get(src, HID_FIRMWARE_BUILD) {
        if buf.len() >= 3 {
            report.firmware.build = Some(u32::from(u16::from_be_bytes([buf[1], buf[2]])));
        }
    }

    // Device name: 3 NUL-padded chunks; cut each at its first NUL.
    let mut name_bytes = Vec::new();
    for rid in [HID_NAME_1, HID_NAME_2, HID_NAME_3] {
        if let Some(buf) = get(src, rid) {
            let chunk = &buf[1..];
            let end = chunk.iter().position(|&b| b == 0).unwrap_or(chunk.len());
            name_bytes.extend_from_slice(&chunk[..end]);
        }
    }
    let name = String::from_utf8_lossy(&name_bytes).trim().to_string();
    if !name.is_empty() {
        report.device.name = Some(name);
    }

    if let Some(buf) = get(src, HID_CONNECTION_PARAMS) {
        if buf.len() >= 3 {
            report.bluetooth.connected = true;
            report.bluetooth.conn_interval_ms = Some(f64::from(buf[1]) * 1.25);
            report.bluetooth.slave_latency = Some(buf[2]);
        }
    }

    if let Some(buf) = get(src, HID_SUPERVISION_TIMEOUT) {
        if buf.len() >= 3 {
            let timeout = u16::from_le_bytes([buf[1], buf[2]]);
            report.bluetooth.supervision_timeout_s = Some(f64::from(timeout) * 0.01);
        }
    }

    if let Some(buf) = get(src, HID_ADC_REF) {
        if buf.len() >= 3 {
            report.firmware.adc_ref = Some(u16::from_be_bytes([buf[1], buf[2]]));
        }
    }

    if let Some(buf) = get(src, HID_DEVICE_IDENTITY) {
        if buf.len() > 7 {
            let key: Vec<String> = buf[1..].iter().map(|b| format!("{b:02X}")).collect();
            report.bluetooth.identity_key = Some(key.join(":"));
        }
    }

    if let Some(buf) = get(src, HID_DEVICE_STATE) {
        // Device reports LOW: clamp an optimistic interpolated percentage.
        if buf.len() >= 2 && buf[1] == 0 && report.battery.percentage_interpolated.unwrap_or(100.0) > 15.0 {
            report.battery.percentage_interpolated = Some(10.0);
        }
    }
    // An answering probe means the link is up even without report 0x46.
    report.bluetooth.connected = true;
    true
}

/// Build a full report from the HID `uevent` of the device, the kernel
/// battery and a HID source. `None` when a BCM2042 does not answer the probe.
pub fn build_report(
    uevent: &str,
    kernel: Option<BatteryReading>,
    src: &dyn HidSource,
    wake: KbWake,
) -> Option<KbReport> {
    let mut report = report_from_uevent(uevent, kernel);
    report.wake = wake;
    if family_from_uevent(uevent) != Family::Bcm2042 {
        // Magic Keyboard / unknown: no raw HID telemetry. The connection is
        // implied by the open hidraw node; battery comes from the kernel.
        report.bluetooth.connected = true;
        return Some(report);
    }
    if !decode_bcm2042(src, &mut report) {
        return None;
    }
    if let Some(k) = kernel {
        // Kernel capacity is the source of truth for the displayed value.
        report.battery.percentage_fine = Some(f64::from(k.percent));
    }
    Some(report)
}

/// Identity + kernel battery only (no HID I/O): used when the hidraw node is
/// not readable but the kernel still exposes the battery.
pub fn report_from_uevent(uevent: &str, kernel: Option<BatteryReading>) -> KbReport {
    let mut report = KbReport::default();
    if let Some(mi) = model_from_uevent(uevent) {
        report.device.model = Some(mi.model.to_string());
        report.device.chip = Some(mi.chip.to_string());
    }
    report.device.driver = Some("hid-apple".to_string());
    report.device.mac = mac_from_uevent(uevent);
    report.device.name = crate::model::name_from_uevent(uevent);
    if let Some(k) = kernel {
        report.battery.percentage = Some(f64::from(k.percent));
        report.battery.percentage_fine = Some(f64::from(k.percent));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::BatteryStatus;

    const UEVENT: &str = "DRIVER=apple\nHID_ID=0005:000005AC:00000256\nHID_NAME=Clavier de maria #1\nHID_UNIQ=04:db:56:ca:42:ee\n";

    fn full_fixture() -> Fixture {
        Fixture::new()
            .with(&[0xEA, 98])
            .with(&[0x47, 100])
            .with(&[0xF5, 0x03, 0x68])
            .with(&[0x5A, 0x0B, 0x54, 0x09, 0x92, 0x09, 0x2E, 0x07, 0xD0])
            .with(&[0x4F, 0x50])
            .with(&[0xFF, 0x0C, 0x32, 0x01])
            .with(b"\x51Apple Wi\x00")
            .with(b"\x52reless K\x00")
            .with(b"\x53eyboard\x00")
            .with(&[0x46, 55, 12])
            .with(&[0x49, 0xD0, 0x07])
            .with(&[0xF4, 0x01, 0x23])
            .with(&[0x4C, 1, 2, 3, 4, 5, 6, 7])
            .with(&[0x09, 1])
    }

    #[test]
    fn decodes_every_report() {
        let mut r = KbReport::default();
        assert!(decode_bcm2042(&full_fixture(), &mut r));
        assert_eq!(r.battery.percentage_fine, Some(98.0));
        assert_eq!(r.battery.percentage, Some(100.0));
        assert_eq!(r.battery.adc_raw, Some(0x0368));
        assert!((r.battery.voltage.unwrap() - 2.813).abs() < 0.001);
        assert_eq!(r.battery.percentage_interpolated, Some(95.0));
        assert_eq!(r.firmware.version.as_deref(), Some("5.0"));
        assert_eq!(r.firmware.build, Some(0x0C32));
        assert_eq!(r.firmware.adc_ref, Some(0x0123));
        assert_eq!(r.device.name.as_deref(), Some("Apple Wireless Keyboard"));
        assert_eq!(r.bluetooth.conn_interval_ms, Some(68.75));
        assert_eq!(r.bluetooth.slave_latency, Some(12));
        assert_eq!(r.bluetooth.supervision_timeout_s, Some(20.0));
        assert_eq!(r.bluetooth.identity_key.as_deref(), Some("01:02:03:04:05:06:07"));
        assert!(r.bluetooth.connected);
    }

    #[test]
    fn probe_failure_stops_reading() {
        struct Counting(std::cell::Cell<u32>);
        impl HidSource for Counting {
            fn feature(&self, _id: u8) -> io::Result<Vec<u8>> {
                self.0.set(self.0.get() + 1);
                Err(io::Error::from(io::ErrorKind::TimedOut))
            }
        }
        let c = Counting(std::cell::Cell::new(0));
        assert!(!decode_bcm2042(&c, &mut KbReport::default()));
        assert_eq!(c.0.get(), 1, "only the probe may be sent");
    }

    #[test]
    fn device_low_state_clamps_interpolation() {
        let f = full_fixture().with(&[0x09, 0]);
        let mut r = KbReport::default();
        decode_bcm2042(&f, &mut r);
        assert_eq!(r.battery.percentage_interpolated, Some(10.0));
    }

    #[test]
    fn garbled_calibration_falls_back_to_default() {
        let f = full_fixture().with(&[0x5A, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut r = KbReport::default();
        decode_bcm2042(&f, &mut r);
        assert_eq!(r.battery.percentage_interpolated, Some(95.0));
    }

    #[test]
    fn build_report_prefers_kernel_capacity() {
        let k = BatteryReading { percent: 90, status: BatteryStatus::Discharging };
        let r = build_report(UEVENT, Some(k), &full_fixture(), KbWake::default()).unwrap();
        assert_eq!(r.battery_pct(), Some(90.0));
        assert_eq!(r.battery.percentage, Some(90.0));
        assert_eq!(r.device.mac.as_deref(), Some("04:DB:56:CA:42:EE"));
        assert!(r.device.model.as_deref().unwrap().contains("A1314"));
        // HID name wins over the uevent name.
        assert_eq!(r.device.name.as_deref(), Some("Apple Wireless Keyboard"));
        assert!(build_report(UEVENT, Some(k), &Fixture::new(), KbWake::default()).is_none());
    }

    #[test]
    fn magic_keyboard_never_polls_vendor_reports() {
        struct Panics;
        impl HidSource for Panics {
            fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
                panic!("report {id:#x} requested on a Magic Keyboard")
            }
        }
        let u = "HID_ID=0005:0000004C:0000029C\nHID_UNIQ=aa:bb:cc:dd:ee:ff\n";
        let k = BatteryReading { percent: 55, status: BatteryStatus::Discharging };
        let r = build_report(u, Some(k), &Panics, KbWake::default()).unwrap();
        assert_eq!(r.battery_pct(), Some(55.0));
        assert!(r.bluetooth.connected);
    }

    #[test]
    fn hex_dump_parsing() {
        let f = Fixture::from_hex_dump("# dump\nea 5a\n475a000000\n\n5a 0b 54 09 92 09 2e 07 d0 # calib\n").unwrap();
        assert_eq!(f.len(), 3);
        assert_eq!(f.feature(0xEA).unwrap(), vec![0xEA, 0x5A]);
        assert!(f.feature(0x99).is_err());
        assert!(Fixture::from_hex_dump("abc").is_err());
        assert!(Fixture::from_hex_dump("zz").is_err());
    }
}
