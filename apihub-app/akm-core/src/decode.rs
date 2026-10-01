//! Decoding of the BCM2042 vendor Feature Reports, independent of the device
//! (docs/REVUE-ARCHITECTURE-GLOBALE.md §4.6): anything implementing
//! [`HidSource`] can be decoded — the real `/dev/hidrawN`
//! ([`crate::hidraw::Hidraw`]) or a [`Fixture`] made of recorded frames.

use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

use crate::calibration::{
    estimate_percentage_mv, parse_calibration, MAX_SOURCE_GAP_MV, PLAUSIBLE_MV,
};
use crate::model::{family_from_uevent, mac_from_uevent, model_from_uevent, Family};
use crate::power::BatteryReading;
use crate::report::{KbReport, KbWake};

// ── HID Report IDs ────────────────────────────────────────────────────────
//
// Only 0x09 is a declared Feature report; 0x47 is a declared Input report
// (Battery Strength) that the kernel reads as Feature. Every other id is
// undeclared vendor data: its meaning comes from reverse engineering only
// (docs/AUDIT-DECODAGE-HID.md). Marking: [mesuré] observed on the A1314 ISO
// on 2026-10-01, [source] external reference, [hypothèse] not demonstrated.

/// Connectivity probe. [mesuré] 1 byte, 98 while 0x47 = kernel = 99: the
/// former "pre-rounding percentage" claim is refuted (#136). Kept raw.
pub const HID_PROBE: u8 = 0xEA;
/// Battery Strength (Generic Device Controls 0x06 / 0x20), 0..100.
/// [source] `hid-input.c` quirk PERCENT|FEATURE; [mesuré] = kernel capacity.
pub const HID_BATTERY_STRENGTH: u8 = 0x47;
/// u16 big-endian, uninterpreted. [mesuré] constant (900) across a battery
/// change: NOT a voltage (#139).
pub const HID_ADC_RAW: u8 = 0xF5;
/// 4 x u16 big-endian, strictly decreasing [mesuré]; [hypothèse] mV
/// thresholds for [100 %, 75 %, 50 %, 25 %].
pub const HID_CALIBRATION: u8 = 0x5A;
/// u16 little-endian. [mesuré] = DID version of the BlueZ modalias (0x0050).
pub const HID_VERSION: u8 = 0x4F;
/// Uninterpreted. [mesuré] varies between two reads (2982 then 2991 BE): NOT
/// a firmware build number (#131).
pub const HID_RAW_FF: u8 = 0xFF;
/// Device name chunks 1..3 [mesuré] (= `HID_NAME` and BlueZ).
pub const HID_NAME_1: u8 = 0x51;
pub const HID_NAME_2: u8 = 0x52;
pub const HID_NAME_3: u8 = 0x53;
/// Uninterpreted. NOT LE connection parameters: the link is BR/EDR
/// [mesuré] and the 1.25 ms interval / slave latency are LE-only [source] (#132).
pub const HID_RAW_46: u8 = 0x46;
/// Uninterpreted. NOT an LE supervision timeout (#132).
pub const HID_RAW_49: u8 = 0x49;
/// u16 big-endian, uninterpreted [mesuré] (= 0x5B bytes 1..3).
pub const HID_RAW_F4: u8 = 0xF4;
/// Byte 1 = 0x03, bytes 2..8 reversed = address of the paired host adapter
/// (= `HID_PHYS`) [mesuré]; the 12 following bytes are unidentified
/// [hypothèse: pairing record] and never published by default (#123, #133).
pub const HID_PAIRED_HOST: u8 = 0x4C;
/// Declared Feature (vendor 0xFF01 usage 0x0B, logical 0..1) [mesuré].
/// "0 = LOW" is not demonstrated: exposed raw, never rewrites a percentage.
pub const HID_STATE: u8 = 0x09;

/// Every Feature Report a full BCM2042 read requests, in order.
pub const BCM2042_READ_ORDER: [u8; 14] = [
    HID_PROBE,
    HID_BATTERY_STRENGTH,
    HID_ADC_RAW,
    HID_CALIBRATION,
    HID_VERSION,
    HID_RAW_FF,
    HID_NAME_1,
    HID_NAME_2,
    HID_NAME_3,
    HID_RAW_46,
    HID_RAW_49,
    HID_RAW_F4,
    HID_PAIRED_HOST,
    HID_STATE,
];

/// Reports copied verbatim (payload hex) into [`KbReport::raw`].
const RAW_REPORTS: [u8; 10] = [
    HID_PROBE,
    HID_BATTERY_STRENGTH,
    HID_ADC_RAW,
    HID_CALIBRATION,
    HID_VERSION,
    HID_RAW_FF,
    HID_RAW_46,
    HID_RAW_49,
    HID_RAW_F4,
    HID_STATE,
];

/// Limits of one vendor read (#134).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Total time allowed for the whole read; no request starts after it.
    pub budget: Duration,
    /// A failed request that took at least this long is a link timeout.
    /// [mesuré] undeclared ids are refused in milliseconds, while a HIDP
    /// timeout of BlueZ answers EIO after 3.4-3.6 s.
    pub slow_failure: Duration,
    /// Publish the unidentified bytes of report 0x4C in `raw` (opt-in,
    /// diagnostics only, never by default: #123).
    pub reveal_pairing_bytes: bool,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            budget: Duration::from_secs(5),
            slow_failure: Duration::from_secs(1),
            reveal_pairing_bytes: false,
        }
    }
}

/// A source of HID reports (GET_REPORT). The returned buffer starts with the
/// report id.
pub trait HidSource {
    /// GET Feature `report_id`.
    fn feature(&self, report_id: u8) -> io::Result<Vec<u8>>;
    /// GET Input `report_id` (Apple's second half of the battery read, `0x30`,
    /// R2 #251). A source without Input reads answers `Unsupported`.
    fn input(&self, report_id: u8) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("no GET Input on this source (report {report_id:#04x})"),
        ))
    }
}

/// Recorded frames, for tests without hardware. Unknown ids answer `NotFound`
/// (like an undeclared report on the real device).
#[derive(Debug, Clone, Default)]
pub struct Fixture {
    reports: HashMap<u8, Vec<u8>>,
    inputs: HashMap<u8, Vec<u8>>,
}

impl Fixture {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one Feature frame (`bytes[0]` is the report id).
    pub fn with(mut self, bytes: &[u8]) -> Self {
        if let Some(&id) = bytes.first() {
            self.reports.insert(id, bytes.to_vec());
        }
        self
    }

    /// Add one Input frame answered to GET Input (`bytes[0]` is the report id).
    pub fn with_input(mut self, bytes: &[u8]) -> Self {
        if let Some(&id) = bytes.first() {
            self.inputs.insert(id, bytes.to_vec());
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
            if let Some(c) = hex.chars().find(|c| !c.is_ascii_hexdigit()) {
                return Err(format!(
                    "line {}: non-hexadecimal character {c:?}",
                    n + 1
                ));
            }
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
        self.reports.get(&report_id).cloned().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no report {report_id:#04x}"),
            )
        })
    }

    fn input(&self, report_id: u8) -> io::Result<Vec<u8>> {
        self.inputs.get(&report_id).cloned().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no input report {report_id:#04x}"),
            )
        })
    }
}

/// True when an error means the link is gone or timing out: stop the read
/// instead of sending more GET_REPORT to a keyboard falling asleep.
fn is_link_failure(e: &io::Error, took: Duration, opts: &DecodeOptions) -> bool {
    use io::ErrorKind as K;
    took >= opts.slow_failure
        || matches!(
            e.kind(),
            K::TimedOut
                | K::NotConnected
                | K::BrokenPipe
                | K::ConnectionAborted
                | K::ConnectionReset
        )
        || matches!(
            e.raw_os_error(),
            Some(
                libc::ENODEV
                    | libc::ENXIO
                    | libc::EBADF
                    | libc::ETIMEDOUT
                    | libc::EHOSTDOWN
                    | libc::ENOTCONN
                    | libc::ESHUTDOWN
            )
        )
}

/// Sequential reader that stops at the first link failure or when the time
/// budget is spent.
struct Reader<'a> {
    src: &'a dyn HidSource,
    opts: DecodeOptions,
    start: Instant,
    sent: u32,
    stopped: bool,
}

impl Reader<'_> {
    fn get(&mut self, id: u8) -> Option<Vec<u8>> {
        if self.stopped {
            return None;
        }
        // The probe is always sent; the budget applies to the requests after it.
        if self.sent > 0 && self.start.elapsed() >= self.opts.budget {
            self.stopped = true;
            return None;
        }
        self.sent += 1;
        let t = Instant::now();
        match self.src.feature(id) {
            Ok(b) if !b.is_empty() => Some(b),
            Ok(_) => None,
            Err(e) => {
                if is_link_failure(&e, t.elapsed(), &self.opts) {
                    self.stopped = true;
                }
                None
            }
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bytes 2..8 of report 0x4C, reversed, as a Bluetooth address.
fn paired_host_addr(buf: &[u8]) -> Option<String> {
    let addr = buf.get(2..8)?;
    Some(
        addr.iter()
            .rev()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// [`decode_bcm2042_with`] with the default limits.
pub fn decode_bcm2042(src: &dyn HidSource, report: &mut KbReport) -> bool {
    decode_bcm2042_with(src, report, &DecodeOptions::default())
}

/// Decode the BCM2042 vendor reports into `report`. Returns `false` when the
/// probe (0xEA) does not answer: the keyboard is not reachable, nothing else
/// is requested. After the probe, the read stops at the first link failure
/// (timeout, device gone) or when `opts.budget` is spent, and the report is
/// marked `incomplete` (#134).
pub fn decode_bcm2042_with(
    src: &dyn HidSource,
    report: &mut KbReport,
    opts: &DecodeOptions,
) -> bool {
    let mut rd = Reader {
        src,
        opts: *opts,
        start: Instant::now(),
        sent: 0,
        stopped: false,
    };
    let mut frames: HashMap<u8, Vec<u8>> = HashMap::new();
    let Some(probe) = rd.get(HID_PROBE) else {
        return false;
    };
    frames.insert(HID_PROBE, probe);
    for &id in &BCM2042_READ_ORDER[1..] {
        match rd.get(id) {
            Some(b) => {
                frames.insert(id, b);
            }
            None if rd.stopped => break,
            None => {}
        }
    }
    report.incomplete = rd.stopped;

    for id in RAW_REPORTS {
        if let Some(b) = frames.get(&id) {
            report.raw.insert(format!("{id:#04x}"), hex(&b[1..]));
        }
    }

    if let Some(buf) = frames.get(&HID_BATTERY_STRENGTH) {
        if let Some(&pct) = buf.get(1).filter(|&&p| p <= 100) {
            if report.battery.percentage.is_none() {
                report.battery.percentage = Some(f64::from(pct));
            }
        }
    }

    if let Some(buf) = frames.get(&HID_ADC_RAW).filter(|b| b.len() >= 3) {
        report.battery.adc_raw = Some(u32::from(u16::from_be_bytes([buf[1], buf[2]])));
    }

    decode_voltage(&frames, &mut report.battery);

    // No curve read from the device: no estimate (the old default had no source).
    let calib = frames
        .get(&HID_CALIBRATION)
        .and_then(|b| parse_calibration(b));
    // [hypothèse] the firmware computes on the filtered voltage (0x49).
    let basis = report
        .battery
        .voltage_filtered_mv
        .or(report.battery.voltage_mv);
    if let (Some(mv), Some(c)) = (basis, calib) {
        let est = estimate_percentage_mv(f64::from(mv), &c);
        report.battery.percentage_estimate = est;
        report.battery.percentage_interpolated = est;
    }

    if let Some(buf) = frames.get(&HID_VERSION).filter(|b| b.len() >= 2) {
        let v = u16::from_le_bytes([buf[1], buf.get(2).copied().unwrap_or(0)]);
        report.firmware.version = Some(format!("0x{v:04X}"));
    }

    // Device name: 3 NUL-padded chunks; cut each at its first NUL.
    let mut name_bytes = Vec::new();
    for rid in [HID_NAME_1, HID_NAME_2, HID_NAME_3] {
        if let Some(buf) = frames.get(&rid) {
            let chunk = &buf[1..];
            let end = chunk.iter().position(|&b| b == 0).unwrap_or(chunk.len());
            name_bytes.extend_from_slice(&chunk[..end]);
        }
    }
    let name = String::from_utf8_lossy(&name_bytes).trim().to_string();
    if !name.is_empty() {
        report.device.name = Some(name);
    }

    if let Some(buf) = frames.get(&HID_PAIRED_HOST) {
        report.bluetooth.paired_host_addr = paired_host_addr(buf);
        if opts.reveal_pairing_bytes {
            report
                .raw
                .insert(format!("{HID_PAIRED_HOST:#04x}"), hex(&buf[1..]));
        }
    }

    // An answering probe means the link is up.
    report.bluetooth.connected = true;
    true
}

/// u16 from `buf[1..3]`, kept only if it is a plausible cell voltage.
fn plausible_mv(v: u16) -> Option<u32> {
    PLAUSIBLE_MV.contains(&v).then_some(u32::from(v))
}

/// Cell voltage: 0x46 (u16 LE) [mesuré], cross-checked against 0xFF bytes 1-2
/// (u16 BE, same value) [mesuré]; 0x49 (u16 LE) is the filtered voltage
/// [hypothèse]. A gap above 20 mV between 0x46 and 0xFF flags the sample as
/// doubtful and keeps 0x46.
fn decode_voltage(frames: &HashMap<u8, Vec<u8>>, bat: &mut crate::report::KbBattery) {
    let v46 = frames
        .get(&HID_RAW_46)
        .filter(|b| b.len() >= 3)
        .and_then(|b| plausible_mv(u16::from_le_bytes([b[1], b[2]])));
    let vff = frames
        .get(&HID_RAW_FF)
        .filter(|b| b.len() >= 3)
        .and_then(|b| plausible_mv(u16::from_be_bytes([b[1], b[2]])));
    bat.voltage_filtered_mv = frames
        .get(&HID_RAW_49)
        .filter(|b| b.len() >= 3)
        .and_then(|b| plausible_mv(u16::from_le_bytes([b[1], b[2]])));
    bat.voltage_doubtful = match (v46, vff) {
        (Some(a), Some(b)) => a.abs_diff(b) > u32::from(MAX_SOURCE_GAP_MV),
        _ => false,
    };
    bat.voltage_mv = v46.or(vff);
    bat.voltage = bat.voltage_mv.map(|mv| f64::from(mv) / 1000.0);
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
    // Compatibility mirror (kernel > 0x47), never 0xEA.
    report.battery.percentage_fine = report.battery.percentage;
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
    use std::cell::{Cell, RefCell};

    const UEVENT: &str = "DRIVER=apple\nHID_ID=0005:000005AC:00000256\nHID_NAME=Clavier de maria #1\nHID_UNIQ=04:db:56:ca:42:ee\n";

    /// [mesuré] Real frames of the A1314 ISO (2026-10-01, exact ioctl
    /// lengths), 0x4C redacted to zeros.
    const REAL_FRAMES: &str = include_str!("../../../tests/live/re/a1314_iso_frames.hex");

    /// Host address bytes of 0x4C (= `HID_PHYS` 6c:94:66:52:7c:0d, documented in
    /// the audit), unknown bytes kept redacted, then poisoned with a marker.
    const REAL_4C_HOST_ONLY: [u8; 20] = [
        0x4C, 0x03, 0x0D, 0x7C, 0x52, 0x66, 0x94, 0x6C, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    fn real() -> Fixture {
        Fixture::from_hex_dump(REAL_FRAMES).unwrap()
    }

    fn full_fixture() -> Fixture {
        real()
            .with(b"\x51Apple Wi\x00")
            .with(b"\x52reless K\x00")
            .with(b"\x53eyboard\x00")
    }

    #[test]
    fn hex_dump_rejects_non_ascii_without_panicking() {
        // #220: "é" is 2 bytes, so an even length used to slice mid-character.
        for bad in ["a\u{e9}a", "\u{e9}\u{e9}", "4\u{20ac}", "47 5\u{e9}"] {
            let e = Fixture::from_hex_dump(bad).unwrap_err();
            assert!(e.starts_with("line 1: non-hexadecimal character"), "{e}");
        }
        let e = Fixture::from_hex_dump("47 50\n46 g0").unwrap_err();
        assert!(e.contains("line 2") && e.contains("'g'"), "{e}");
        assert_eq!(Fixture::from_hex_dump("47 50 # \u{e9}t\u{e9}\n").unwrap().len(), 1);
    }

    #[test]
    fn decodes_the_real_a1314_frames() {
        let f = real();
        assert_eq!(f.len(), 14);
        let mut r = KbReport::default();
        assert!(decode_bcm2042(&f, &mut r));
        assert!(!r.incomplete);
        // 0x47 = 99 = kernel capacity [mesuré]; 0xEA (98) is not a percentage.
        assert_eq!(r.battery.percentage, Some(99.0));
        assert_eq!(r.battery_pct(), Some(99.0));
        assert_eq!(r.raw.get("0xea").map(String::as_str), Some("62"));
        // #139: voltage = 0x46 LE (2991 mV), cross-checked with 0xFF BE; 0xF5
        // (900) is kept raw and never converted.
        assert_eq!(r.battery.adc_raw, Some(900));
        assert_eq!(r.battery.voltage_mv, Some(2991));
        assert!((r.battery.voltage.unwrap() - 2.991).abs() < 1e-9);
        assert_eq!(r.battery.voltage_filtered_mv, Some(2953));
        assert!(!r.battery.voltage_doubtful);
        // 0x49 = 2953 mV on 2954/2506/... -> 99.9 % (estimate); 0x47 stays 99.
        assert_eq!(r.battery.percentage_estimate, Some(99.9));
        assert_eq!(r.battery.percentage_interpolated, Some(99.9));
        assert_eq!(r.firmware.version.as_deref(), Some("0x0050"));
        assert_eq!(r.device.name.as_deref(), Some("Clavier de maria #1"));
        // #131 / #132: kept raw, never interpreted.
        assert_eq!(r.raw.get("0xff").map(String::as_str), Some("0baf01"));
        assert_eq!(r.raw.get("0x46").map(String::as_str), Some("af0b"));
        assert_eq!(r.raw.get("0x49").map(String::as_str), Some("890b"));
        assert_eq!(r.raw.get("0xf4").map(String::as_str), Some("06cc"));
        assert_eq!(r.raw.get("0x09").map(String::as_str), Some("010000"));
        assert_eq!(
            r.raw.get("0x5a").map(String::as_str),
            Some("0b8a09ca09640806")
        );
        let json = serde_json::to_string(&r).unwrap();
        for gone in [
            "conn_interval",
            "slave_latency",
            "supervision_timeout",
            "identity_key",
            "\"build\"",
            "adc_ref",
        ] {
            assert!(!json.contains(gone), "{gone} still published");
        }
        assert!(r.bluetooth.connected);
    }

    #[test]
    fn report_0x4c_exposes_only_the_paired_host_address() {
        let mut frame = REAL_4C_HOST_ONLY;
        frame[8..].fill(0xA5); // stand-in for the 12 unidentified bytes
        let f = real().with(&frame);
        let mut r = KbReport::default();
        decode_bcm2042(&f, &mut r);
        assert_eq!(
            r.bluetooth.paired_host_addr.as_deref(),
            Some("6C:94:66:52:7C:0D")
        );
        assert!(!r.raw.contains_key("0x4c"));
        let json = serde_json::to_string(&r).unwrap().to_lowercase();
        assert!(
            !json.contains("a5a5"),
            "unidentified 0x4C bytes leaked: {json}"
        );
        // Opt-in diagnostics only.
        let mut r = KbReport::default();
        let opts = DecodeOptions {
            reveal_pairing_bytes: true,
            ..DecodeOptions::default()
        };
        decode_bcm2042_with(&f, &mut r, &opts);
        assert!(r.raw["0x4c"].ends_with("a5a5"));
        // A short frame never yields a bogus address.
        let mut r = KbReport::default();
        decode_bcm2042(&real().with(&[0x4C, 3, 1, 2]), &mut r);
        assert!(r.bluetooth.paired_host_addr.is_none());
    }

    /// Records the requests; fails with `err` (after `delay`) on request `fail_at`.
    struct Script {
        inner: Fixture,
        fail_at: u32,
        err: fn() -> io::Error,
        delay: Duration,
        seen: RefCell<Vec<u8>>,
    }
    impl HidSource for Script {
        fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
            self.seen.borrow_mut().push(id);
            if self.seen.borrow().len() as u32 == self.fail_at {
                std::thread::sleep(self.delay);
                return Err((self.err)());
            }
            self.inner.feature(id)
        }
    }

    fn script(fail_at: u32, err: fn() -> io::Error, delay: Duration) -> Script {
        Script {
            inner: real(),
            fail_at,
            err,
            delay,
            seen: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn timeout_on_third_request_stops_the_read() {
        let s = script(
            3,
            || io::Error::from(io::ErrorKind::TimedOut),
            Duration::ZERO,
        );
        let mut r = KbReport::default();
        assert!(decode_bcm2042(&s, &mut r));
        assert_eq!(s.seen.borrow().len(), 3, "no 4th request after a timeout");
        assert!(r.incomplete);
        assert_eq!(r.battery.percentage, Some(99.0));
        assert!(r.battery.adc_raw.is_none());
    }

    #[test]
    fn slow_eio_is_a_link_timeout_but_fast_eio_is_a_refused_report() {
        let eio = || io::Error::from_raw_os_error(libc::EIO);
        let opts = DecodeOptions {
            slow_failure: Duration::from_millis(20),
            ..DecodeOptions::default()
        };
        // [mesuré] BlueZ HIDP timeout: EIO after 3.4 s (scaled down here).
        let slow = script(4, eio, Duration::from_millis(30));
        let mut r = KbReport::default();
        decode_bcm2042_with(&slow, &mut r, &opts);
        assert_eq!(slow.seen.borrow().len(), 4);
        assert!(r.incomplete);
        // A report refused at once: skipped, the read goes on.
        let fast = script(4, eio, Duration::ZERO);
        let mut r = KbReport::default();
        decode_bcm2042_with(&fast, &mut r, &opts);
        assert_eq!(fast.seen.borrow().len(), BCM2042_READ_ORDER.len());
        assert!(!r.incomplete);
        assert!(!r.raw.contains_key("0x5a"));
    }

    #[test]
    fn device_gone_stops_the_read() {
        let s = script(
            2,
            || io::Error::from_raw_os_error(libc::ENODEV),
            Duration::ZERO,
        );
        let mut r = KbReport::default();
        assert!(decode_bcm2042(&s, &mut r));
        assert_eq!(s.seen.borrow().len(), 2);
        assert!(r.incomplete);
    }

    #[test]
    fn total_time_budget_is_bounded() {
        let s = script(0, || io::Error::from(io::ErrorKind::Other), Duration::ZERO);
        let opts = DecodeOptions {
            budget: Duration::ZERO,
            ..DecodeOptions::default()
        };
        let mut r = KbReport::default();
        assert!(decode_bcm2042_with(&s, &mut r, &opts));
        assert_eq!(
            *s.seen.borrow(),
            vec![HID_PROBE],
            "only the probe once the budget is spent"
        );
        assert!(r.incomplete);
    }

    #[test]
    fn probe_failure_stops_reading() {
        struct Counting(Cell<u32>);
        impl HidSource for Counting {
            fn feature(&self, _id: u8) -> io::Result<Vec<u8>> {
                self.0.set(self.0.get() + 1);
                Err(io::Error::from(io::ErrorKind::TimedOut))
            }
        }
        let c = Counting(Cell::new(0));
        assert!(!decode_bcm2042(&c, &mut KbReport::default()));
        assert_eq!(c.0.get(), 1, "only the probe may be sent");
    }

    #[test]
    fn state_report_never_rewrites_a_percentage() {
        let f = real().with(&[0x09, 0, 0, 0]);
        let mut r = KbReport::default();
        decode_bcm2042(&f, &mut r);
        assert_eq!(r.battery.percentage_estimate, Some(99.9));
        assert_eq!(r.raw.get("0x09").map(String::as_str), Some("000000"));
    }

    #[test]
    fn no_estimate_without_a_valid_curve() {
        let f = real().with(&[0x5A, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut r = KbReport::default();
        decode_bcm2042(&f, &mut r);
        assert!(r.battery.voltage.is_some());
        assert!(r.battery.percentage_interpolated.is_none());
        assert!(r.battery.percentage_estimate.is_none());
    }

    #[test]
    fn voltage_sources_and_coherence() {
        // 0x46 absent: fallback on 0xFF bytes 1-2 (BE).
        let mut f = real();
        f = f.with(&[0x46]);
        let mut r = KbReport::default();
        decode_bcm2042(&f, &mut r);
        assert_eq!(r.battery.voltage_mv, Some(2991), "0xFF fallback");
        assert!(!r.battery.voltage_doubtful);
        // 0x46 and 0xFF diverge by 50 mV: doubtful, 0x46 kept.
        let f = real().with(&[0x46, 0xE1, 0x0B]); // 3041 mV
        let mut r = KbReport::default();
        decode_bcm2042(&f, &mut r);
        assert_eq!(r.battery.voltage_mv, Some(3041));
        assert!(r.battery.voltage_doubtful);
        // 20 mV gap is still coherent.
        let f = real().with(&[0x46, 0xC3, 0x0B]); // 3011 mV
        let mut r = KbReport::default();
        decode_bcm2042(&f, &mut r);
        assert!(!r.battery.voltage_doubtful);
        // Implausible value dropped; no 0x49 -> basis is 0x46.
        let f = real()
            .with(&[0x46, 0, 0])
            .with(&[0xFF, 0, 0, 0])
            .with(&[0x49]);
        let mut r = KbReport::default();
        decode_bcm2042(&f, &mut r);
        assert_eq!(r.battery.voltage_mv, None);
        assert_eq!(r.battery.voltage, None);
        assert_eq!(r.battery.percentage_estimate, None);
        let f = real().with(&[0x49]);
        let mut r = KbReport::default();
        decode_bcm2042(&f, &mut r);
        assert_eq!(r.battery.voltage_filtered_mv, None);
        assert_eq!(r.battery.percentage_estimate, Some(100.0), "0x46 = 2991");
    }

    #[test]
    fn estimate_never_nan_and_clamped() {
        for v in [[0x46u8, 0xFF, 0xFF], [0x46, 0x10, 0x00], [0x46, 0x00, 0x11]] {
            let f = real().with(&v).with(&[0x49]);
            let mut r = KbReport::default();
            decode_bcm2042(&f, &mut r);
            if let Some(p) = r.battery.percentage_estimate {
                assert!(p.is_finite() && (0.0..=100.0).contains(&p));
            }
        }
    }

    #[test]
    fn out_of_range_strength_is_not_a_percentage() {
        let f = real().with(&[0x47, 200]);
        let mut r = KbReport::default();
        decode_bcm2042(&f, &mut r);
        assert!(r.battery.percentage.is_none());
        assert_eq!(r.raw.get("0x47").map(String::as_str), Some("c8"));
    }

    #[test]
    fn build_report_prefers_kernel_capacity() {
        let k = BatteryReading {
            percent: 90,
            status: BatteryStatus::Discharging,
        };
        let r = build_report(UEVENT, Some(k), &full_fixture(), KbWake::default()).unwrap();
        assert_eq!(r.battery_pct(), Some(90.0));
        assert_eq!(r.battery.percentage, Some(90.0));
        assert_eq!(r.battery.percentage_fine, Some(90.0), "mirror, never 0xEA");
        // Without kernel: 0x47 (99), not 0xEA (98).
        let r = build_report(UEVENT, None, &full_fixture(), KbWake::default()).unwrap();
        assert_eq!(r.battery_pct(), Some(99.0));
        assert_eq!(r.battery.percentage_fine, Some(99.0));
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
        let k = BatteryReading {
            percent: 55,
            status: BatteryStatus::Discharging,
        };
        let r = build_report(u, Some(k), &Panics, KbWake::default()).unwrap();
        assert_eq!(r.battery_pct(), Some(55.0));
        assert!(r.bluetooth.connected);
    }

    #[test]
    fn hex_dump_parsing() {
        let f = Fixture::from_hex_dump(
            "# dump\nea 5a\n475a000000\n\n5a 0b 54 09 92 09 2e 07 d0 # calib\n",
        )
        .unwrap();
        assert_eq!(f.len(), 3);
        assert_eq!(f.feature(0xEA).unwrap(), vec![0xEA, 0x5A]);
        assert!(f.feature(0x99).is_err());
        assert!(Fixture::from_hex_dump("abc").is_err());
        assert!(Fixture::from_hex_dump("zz").is_err());
    }
}
