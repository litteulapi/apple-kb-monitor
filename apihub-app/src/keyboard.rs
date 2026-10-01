//! Apple Wireless Keyboard HID telemetry reader.
//!
//! Pure Rust — reads HID Feature Reports via ioctl, no subprocess.
//! Supports BCM2042-based keyboards (A1314 in ISO/ANSI/JIS variants).
//!
//! # Calibration contract (shared with the Python CLI `apple-kb-monitor`, #80)
//!
//! Report 0x5A carries 4 big-endian u16 thresholds in mV, bytes 1..9, for the
//! battery levels [100 %, 75 %, 50 %, 25 %]. A curve is usable only if it is
//! strictly decreasing and the last value is non-zero (`calibration_valid`);
//! otherwise the default `[2900, 2450, 2350, 2000]` is used. Between two
//! thresholds the percentage is interpolated linearly; at/above the first it is
//! 100, below the 4th it decays linearly to 0 mV = 0 %; the voltage is
//! `adc * 3.3 / 1023` from report 0xF5 (big-endian u16). The Python side must
//! apply the same validation and fallback. This value is a diagnostic: the
//! displayed battery percentage comes from the kernel (`power.rs`).
//!
//! # Wake monitor (#79)
//!
//! Input report 0x13 is a vendor wake event. A single process-wide monitor
//! thread is started lazily (by `read_keyboard()` or `spawn_wake_monitor()`),
//! also when no keyboard is present yet, and it re-locates the hidraw node
//! forever. The result is exposed by `last_wake()` / `wake_count()` and in
//! `KbReport::wake`.

use std::sync::{Arc, Mutex};

// ── HID Report IDs ────────────────────────────────────────────────────────

/// Battery precise — pre-rounding ADC value
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
/// Device name chunk 1
pub const HID_NAME_1: u8 = 0x51;
/// Device name chunk 2
pub const HID_NAME_2: u8 = 0x52;
/// Device name chunk 3
pub const HID_NAME_3: u8 = 0x53;
/// Connection parameters — BT interval + latency
pub const HID_CONNECTION_PARAMS: u8 = 0x46;
/// Device identity — BCM2042 internal identity key (NOT the BT MAC)
pub const HID_DEVICE_IDENTITY: u8 = 0x4C;
/// Device state — 1=OK, 0=LOW
pub const HID_DEVICE_STATE: u8 = 0x09;

// ── Apple vendor/product IDs ──────────────────────────────────────────────

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
    ModelInfo { pid, model, chip, family }
}

/// Wireless Apple keyboards, checked against the kernel's
/// `drivers/hid/hid-ids.h` and `hid-apple.c`. Wired models (ALU_ANSI 0x0220,
/// ALU_REVB 0x024f..) and internal ones (GEYSER4 0x0229) are deliberately absent.
pub const APPLE_MODELS: &[ModelInfo] = &[
    m(0x022c, "Apple Wireless Keyboard (A1255, aluminum, ANSI)", "BCM2042", Family::Bcm2042),
    m(0x022d, "Apple Wireless Keyboard (A1255, aluminum, ISO)", "BCM2042", Family::Bcm2042),
    m(0x022e, "Apple Wireless Keyboard (A1255, aluminum, JIS)", "BCM2042", Family::Bcm2042),
    m(0x0239, "Apple Wireless Keyboard (A1314, 2009, ANSI)", "BCM2042", Family::Bcm2042),
    m(0x023a, "Apple Wireless Keyboard (A1314, 2009, ISO)", "BCM2042", Family::Bcm2042),
    m(0x023b, "Apple Wireless Keyboard (A1314, 2009, JIS)", "BCM2042", Family::Bcm2042),
    m(0x0255, "Apple Wireless Keyboard (A1314, aluminum, ANSI)", "BCM2042", Family::Bcm2042),
    m(0x0256, "Apple Wireless Keyboard (A1314, aluminum, ISO)", "BCM2042", Family::Bcm2042),
    m(0x0257, "Apple Wireless Keyboard (A1314, aluminum, JIS)", "BCM2042", Family::Bcm2042),
    m(0x0267, "Apple Magic Keyboard 2015 (A1644)", "BCM20733", Family::MagicKeyboard),
    m(0x026c, "Apple Magic Keyboard with Numeric Keypad 2015 (A1843)", "BCM20733", Family::MagicKeyboard),
    m(0x029c, "Apple Magic Keyboard 2021 (A2450)", "Apple", Family::MagicKeyboard),
    m(0x029a, "Apple Magic Keyboard with Touch ID 2021 (A2449)", "Apple", Family::MagicKeyboard),
    m(0x029f, "Apple Magic Keyboard with Touch ID and Numeric Keypad 2021 (A2520)", "Apple", Family::MagicKeyboard),
    m(0x0320, "Apple Magic Keyboard 2024", "Apple", Family::MagicKeyboard),
    m(0x0321, "Apple Magic Keyboard with Touch ID 2024", "Apple", Family::MagicKeyboard),
    m(0x0322, "Apple Magic Keyboard with Touch ID and Numeric Keypad 2024", "Apple", Family::MagicKeyboard),
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
fn parse_hid_id(uevent: &str) -> Option<(u32, u32)> {
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

// ── ADC reference values ──────────────────────────────────────────────────

/// ADC max value (10-bit: 2^10 - 1 = 1023, not 1024)
pub const ADC_MAX: u32 = 1023;
/// ADC reference voltage (V)
pub const ADC_VREF: f64 = 3.3;
/// Default calibration curve [100%, 75%, 50%, 25%] in mV
pub const DEFAULT_CALIBRATION_MV: [u16; 4] = [2900, 2450, 2350, 2000];

// ── HIDIOCGFEATURE ioctl constant ─────────────────────────────────────────

/// HIDIOCGFEATURE = _IOWR('H', 0x07, 256) — read HID Feature Report
const HIDIOCGFEATURE: libc::c_ulong = 0xC1004807;

// ── Data structs ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct KbDevice {
    pub model: Option<String>,
    pub name: Option<String>,
    pub mac: Option<String>,
    pub chip: Option<String>,
    pub driver: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct KbBattery {
    pub percentage: Option<f64>,
    pub percentage_fine: Option<f64>,
    pub percentage_interpolated: Option<f64>,
    pub voltage: Option<f64>,
    pub adc_raw: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct KbBluetooth {
    pub connected: bool,
    pub paired: bool,
    pub rssi_dbus: Option<i32>,
    pub tx_power_dbus: Option<i32>,
    pub conn_interval_ms: Option<f64>,
    pub slave_latency: Option<u8>,
    pub supervision_timeout_s: Option<f64>,
    pub identity_key: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct KbRadio {
    pub rssi_dbm: Option<i32>,
    pub tx_power_dbm: Option<i32>,
}

#[derive(Debug, Clone, Default)]
pub struct KbFirmware {
    pub version: Option<String>,
    pub build: Option<u32>,
    pub adc_ref: Option<u16>,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // exposed for main.rs / UI (#79)
pub struct KbWake {
    /// Seconds since the last wake event (input report 0x13), if any was seen.
    pub last_age_s: Option<f64>,
    /// Number of wake events since the app started.
    pub count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct KbReport {
    #[allow(dead_code)] // read by main.rs (#79)
    pub wake: KbWake,
    pub device: KbDevice,
    pub battery: KbBattery,
    pub bluetooth: KbBluetooth,
    pub radio: KbRadio,
    pub firmware: KbFirmware,
}

// ── HID helpers ───────────────────────────────────────────────────────────

pub fn hid_read_feature(fd: libc::c_int, report_id: u8) -> Option<Vec<u8>> {
    let mut buf = [0u8; 256];
    buf[0] = report_id;
    let ret = unsafe { libc::ioctl(fd, HIDIOCGFEATURE, buf.as_mut_ptr()) };
    if ret > 0 {
        Some(buf[..ret as usize].to_vec())
    } else {
        None
    }
}

/// Identify an Apple keyboard from the `HID_ID=bus:vendor:product` line of a
/// hidraw uevent. Returns (model, chip).
pub fn apple_model_from_uevent(uevent: &str) -> Option<(&'static str, &'static str)> {
    model_from_uevent(uevent).map(|mi| (mi.model, mi.chip))
}

/// Find the first Apple keyboard hidraw device by scanning sysfs uevent.
/// Matches every model of `APPLE_MODELS` (USB 05AC and Bluetooth 004C vendors).
pub fn find_apple_hidraw() -> Option<String> {
    let rd = std::fs::read_dir("/sys/class/hidraw").ok()?;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let device_path = entry.path().join("device/uevent");
        if let Ok(uevent) = std::fs::read_to_string(&device_path) {
            if apple_model_from_uevent(&uevent).is_some() {
                return Some(format!("/dev/{}", name));
            }
        }
    }
    None
}

// ── Battery helpers ───────────────────────────────────────────────────────

/// A calibration curve is usable only if it is strictly decreasing and non-zero
/// ([100%, 75%, 50%, 25%] thresholds). A garbled report (zeros, unordered)
/// would otherwise yield meaningless percentages.
pub fn calibration_valid(t: &[u16; 4]) -> bool {
    t[3] > 0 && t[0] > t[1] && t[1] > t[2] && t[2] > t[3]
}

/// Interpolate battery % from voltage using the BCM2042 calibration curve.
/// Thresholds: [100%, 75%, 50%, 25%] in mV, linear interpolation between segments.
pub fn interpolate_battery(voltage_v: f64, thresholds_mv: &[u16; 4]) -> f64 {
    let mv = (voltage_v * 1000.0) as i32;
    let levels_pct = [100.0, 75.0, 50.0, 25.0, 0.0];
    let levels_mv = [
        thresholds_mv[0] as i32,
        thresholds_mv[1] as i32,
        thresholds_mv[2] as i32,
        thresholds_mv[3] as i32,
        0,
    ];
    if !calibration_valid(thresholds_mv) { return interpolate_battery(voltage_v, &DEFAULT_CALIBRATION_MV); }
    if mv >= levels_mv[0] { return 100.0; }
    if mv <= 0 { return 0.0; }
    for i in 0..4 {
        if mv >= levels_mv[i + 1] {
            let hi_mv = levels_mv[i] as f64;
            let lo_mv = levels_mv[i + 1] as f64;
            if hi_mv == lo_mv { return levels_pct[i]; }
            let frac = (mv as f64 - lo_mv) / (hi_mv - lo_mv);
            return levels_pct[i + 1] + frac * (levels_pct[i] - levels_pct[i + 1]);
        }
    }
    0.0
}

/// Detect battery chemistry from voltage (2xAA cells in series).
pub fn detect_battery_type(voltage: f64) -> &'static str {
    if voltage >= 3.1 { "Lithium (fresh)" }
    else if voltage >= 2.85 { "Alkaline (fresh)" }
    else if voltage >= 2.5 { "Alkaline or NiMH" }
    else if voltage >= 2.3 { "NiMH (likely)" }
    else if voltage >= 2.0 { "Depleted" }
    else { "Critical — replace" }
}

// ── Main reader ───────────────────────────────────────────────────────────

/// Persistent HID fd + path — opened once, reused forever.
static HID_FD: Mutex<Option<(libc::c_int, String)>> = Mutex::new(None);

fn get_hid_fd() -> Option<(libc::c_int, String)> {
    let mut fd_lock = HID_FD.lock().ok()?;
    if let Some((fd, ref path)) = *fd_lock {
        let ret = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if ret >= 0 { return Some((fd, path.clone())); }
        // fd already invalid at kernel level: nothing to close.
        *fd_lock = None;
    }
    let path = find_apple_hidraw()?;
    let c_path = std::ffi::CString::new(path.as_str()).ok()?;
    let raw_fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if raw_fd < 0 { return None; }
    *fd_lock = Some((raw_fd, path.clone()));
    Some((raw_fd, path))
}

/// The kernel power_supply is the source of truth for the percentage (same
/// node UPower reads). Raw HID values stay available as diagnostics.
fn apply_kernel_battery(report: &mut KbReport) {
    let Some(mac) = report.device.mac.as_deref() else { return };
    if let Some(b) = crate::power::kernel_battery(mac) {
        report.battery.percentage = Some(b.percent as f64);
    }
}

/// Real BT MAC from `HID_UNIQ=` (the 0x4C report holds an internal identity, not the MAC).
fn mac_from_uevent(uevent: &str) -> Option<String> {
    let mac = uevent.lines().find_map(|l| l.strip_prefix("HID_UNIQ="))?.trim().to_uppercase();
    (mac.contains(':') && mac.len() >= 17).then_some(mac)
}

/// Read keyboard telemetry via HID Feature Reports.
/// Uses persistent fd. Reads only essential reports to minimize BT traffic
/// and avoid triggering HIDP timeouts that cause disconnections.
pub fn read_keyboard() -> Option<KbReport> {
    // Start the wake monitor even if the keyboard is absent right now.
    let wake = ensure_wake_monitor();
    let (fd_val, path) = get_hid_fd()?;

    let mut report = KbReport {
        wake: KbWake {
            last_age_s: wake.last().map(|t| t.elapsed().as_secs_f64()),
            count: wake.count(),
        },
        ..Default::default()
    };

    // Identify the model first: BCM2042 vendor reports (0xEA, 0xF5, ...) are
    // undeclared in the HID descriptor and only exist on that family.
    let uevent = std::fs::read_to_string(
        std::path::Path::new("/sys/class/hidraw")
            .join(path.trim_start_matches("/dev/"))
            .join("device/uevent")
    ).unwrap_or_default();
    let info = model_from_uevent(&uevent);
    let fam = parse_hid_id(&uevent).map_or(Family::Unknown, |(v, p)| family(v, p));
    if let Some(mi) = info {
        report.device.model = Some(mi.model.to_string());
        report.device.chip = Some(mi.chip.to_string());
    }
    report.device.driver = Some("hid-apple".to_string());
    report.device.mac = mac_from_uevent(&uevent);
    apply_kernel_battery(&mut report);
    if fam != Family::Bcm2042 {
        // Magic Keyboard / unknown: no raw HID telemetry. The connection is
        // implied by the open hidraw node; battery comes from the kernel.
        report.bluetooth.connected = true;
        return Some(report);
    }

    // First: try battery precise (0xEA) as a connectivity probe.
    // If this fails, the keyboard is disconnected — don't hammer with more reads.
    let probe = hid_read_feature(fd_val, HID_BATTERY_PRECISE);
    if probe.is_none() {
        // Keyboard not responding — invalidate persistent fd so we reopen next time
        if let Ok(mut fd_lock) = HID_FD.lock() {
            if let Some((old_fd, _)) = fd_lock.take() {
                unsafe { libc::close(old_fd); }
            }
        }
        return None;
    }
    if let Some(ref buf) = probe {
        if buf.len() >= 2 {
            report.battery.percentage_fine = Some(buf[1] as f64);
        }
    }

    // Battery standard (0x47) — firmware-rounded
    if let Some(buf) = hid_read_feature(fd_val, HID_BATTERY_STANDARD) {
        if buf.len() >= 2 && report.battery.percentage.is_none() {
            report.battery.percentage = Some(buf[1] as f64);
        }
    }

    // ADC raw voltage (0xF5) — 10-bit, 3.3V reference
    if let Some(buf) = hid_read_feature(fd_val, HID_ADC_RAW) {
        if buf.len() >= 3 {
            let adc = ((buf[1] as u32) << 8) | buf[2] as u32;
            report.battery.adc_raw = Some(adc);
            report.battery.voltage = Some(adc as f64 * ADC_VREF / ADC_MAX as f64);
        }
    }

    // Calibration curve (0x5A) — 4 x u16 mV thresholds [100%, 75%, 50%, 25%]
    let mut calib = DEFAULT_CALIBRATION_MV;
    if let Some(buf) = hid_read_feature(fd_val, HID_CALIBRATION) {
        if buf.len() >= 9 {
            let mut c = [0u16; 4];
            for i in 0..4 {
                let off = 1 + i * 2;
                c[i] = ((buf[off] as u16) << 8) | buf[off + 1] as u16;
            }
            if calibration_valid(&c) {
                calib = c;
            }
        }
    }

    // Interpolated battery % from voltage + calibration curve
    if let Some(voltage) = report.battery.voltage {
        report.battery.percentage_interpolated = Some(
            interpolate_battery(voltage, &calib).round()
        );
    }

    // Firmware (0x4F) — high nibble = major, low nibble = minor
    if let Some(buf) = hid_read_feature(fd_val, HID_FIRMWARE_VERSION) {
        if buf.len() >= 2 {
            let major = buf[1] >> 4;
            let minor = buf[1] & 0x0F;
            report.firmware.version = Some(format!("{}.{}", major, minor));
        }
    }

    // Firmware build (0xFF) — u16 build + u8 flag
    if let Some(buf) = hid_read_feature(fd_val, HID_FIRMWARE_BUILD) {
        if buf.len() >= 3 {
            let build = ((buf[1] as u32) << 8) | buf[2] as u32;
            report.firmware.build = Some(build);
        }
    }

    // Device name (0x51 + 0x52 + 0x53) — 3 chunks of 8 bytes
    let mut name_bytes = Vec::new();
    for rid in [HID_NAME_1, HID_NAME_2, HID_NAME_3] {
        if let Some(buf) = hid_read_feature(fd_val, rid) {
            // Each chunk is NUL-padded: cut at the first NUL so padding never
            // ends up in the middle of the assembled name.
            let chunk = &buf[1..];
            let end = chunk.iter().position(|&b| b == 0).unwrap_or(chunk.len());
            name_bytes.extend_from_slice(&chunk[..end]);
        }
    }
    let name = String::from_utf8_lossy(&name_bytes).trim().to_string();
    if !name.is_empty() {
        report.device.name = Some(name);
    }

    // Connection params (0x46) — byte[1]=interval, byte[2]=latency
    if let Some(buf) = hid_read_feature(fd_val, HID_CONNECTION_PARAMS) {
        if buf.len() >= 3 {
            report.bluetooth.connected = true;
            let interval = buf[1] as f64 * 1.25; // × 1.25ms per BT spec
            let latency = buf[2];
            report.bluetooth.conn_interval_ms = Some(interval);
            report.bluetooth.slave_latency = Some(latency);
        }
    }

    // Supervision timeout (0x49) — LE u16 × 10ms
    if let Some(buf) = hid_read_feature(fd_val, 0x49) {
        if buf.len() >= 3 {
            let timeout = ((buf[2] as u16) << 8) | buf[1] as u16; // LE
            report.bluetooth.supervision_timeout_s = Some(timeout as f64 * 0.01);
        }
    }

    // ADC reference (0xF4) — factory calibration constant
    if let Some(buf) = hid_read_feature(fd_val, 0xF4) {
        if buf.len() >= 3 {
            report.firmware.adc_ref = Some(((buf[1] as u16) << 8) | buf[2] as u16);
        }
    }

    // Device identity (0x4C) — internal BCM2042 identity, NOT the BT MAC
    if let Some(buf) = hid_read_feature(fd_val, HID_DEVICE_IDENTITY) {
        if buf.len() > 7 {
            let key_hex: String = buf[1..].iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(":");
            report.bluetooth.identity_key = Some(key_hex);
        }
    }

    // Device state (0x09) — 1=OK, 0=LOW
    if let Some(buf) = hid_read_feature(fd_val, HID_DEVICE_STATE) {
        if buf.len() >= 2 && buf[1] == 0 {
            // Device reports LOW state — override percentage if it's above threshold
            if report.battery.percentage_interpolated.unwrap_or(100.0) > 15.0 {
                report.battery.percentage_interpolated = Some(10.0);
            }
        }
    }

    // fd stays open (persistent) for next call
    Some(report)
}

// ── Wake event monitor (Input Report 0x13) ──────────────────────────────

/// Shared wake state: instant of the last event and a running counter.
#[derive(Default)]
struct WakeState {
    last: Mutex<Option<std::time::Instant>>,
    count: std::sync::atomic::AtomicU64,
}

impl WakeState {
    fn record(&self) {
        if let Ok(mut l) = self.last.lock() {
            *l = Some(std::time::Instant::now());
        }
        self.count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn last(&self) -> Option<std::time::Instant> {
        self.last.lock().ok().and_then(|l| *l)
    }
    fn count(&self) -> u64 {
        self.count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

static WAKE: std::sync::OnceLock<Arc<WakeState>> = std::sync::OnceLock::new();

/// Start the process-wide wake monitor (idempotent) and return its state.
/// The thread keeps looking for the hidraw node, so it works when the keyboard
/// is absent at launch and survives disconnect/reconnect (node number changes).
fn ensure_wake_monitor() -> Arc<WakeState> {
    WAKE.get_or_init(|| {
        let st = Arc::new(WakeState::default());
        let lw = st.clone();
        let spawned = std::thread::Builder::new()
            .name("kb-wake-monitor".into())
            .spawn(move || loop {
                if let Some(path) = find_apple_hidraw() {
                    wake_loop(&path, &lw);
                }
                // Device gone (keyboard off / re-paired) or not present yet.
                std::thread::sleep(std::time::Duration::from_secs(5));
            });
        if let Err(e) = spawned {
            eprintln!("[keyboard] cannot spawn wake monitor: {}", e);
        }
        st
    })
    .clone()
}

/// Kept for callers that start the monitor explicitly; the path is no longer
/// needed (the monitor discovers the node itself). Safe to call repeatedly.
pub fn spawn_wake_monitor(_hidraw_path: &str) {
    ensure_wake_monitor();
}

#[allow(dead_code)] // public API for main.rs (#79)
/// Instant of the last wake event (input report 0x13), if one was seen.
pub fn last_wake() -> Option<std::time::Instant> {
    WAKE.get().and_then(|w| w.last())
}

#[allow(dead_code)] // public API for main.rs (#79)
/// Number of wake events seen since the monitor started.
pub fn wake_count() -> u64 {
    WAKE.get().map_or(0, |w| w.count())
}

/// True if an input report is the vendor wake/connection event (report 0x13).
fn is_wake_report(report: &[u8]) -> bool {
    report.first() == Some(&0x13)
}

/// Read input reports until the device disappears or errors. Closes the fd.
fn wake_loop(path: &str, lw: &WakeState) {
    let c_path = match std::ffi::CString::new(path) {
        Ok(p) => p,
        Err(_) => return,
    };
    let fd = unsafe {
        libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC)
    };
    if fd < 0 { return; }

    let mut buf = [0u8; 64];
    loop {
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let ret = unsafe { libc::poll(&mut pfd, 1, 2000) };
        if ret < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) { continue; }
            break;
        }
        if ret == 0 { continue; }
        // POLLHUP/POLLERR/POLLNVAL: hidraw node is gone. Without this exit the
        // loop spins at 100% CPU because poll returns immediately forever.
        if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 { break; }

        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            let e = std::io::Error::last_os_error().raw_os_error();
            if e == Some(libc::EAGAIN) || e == Some(libc::EINTR) { continue; }
            break;
        }
        if n == 0 { break; }

        if is_wake_report(&buf[..n as usize]) {
            lw.record();
        }
    }
    unsafe { libc::close(fd); }
}

// ── LED (state + control) ───────────────────────────────────────────────
//
// keyd grabs the physical keyboard (EVIOCGRAB), and the kernel then drops
// EV_LED events injected through another handle (input_inject_event). keyd
// forwards EV_LED received on its *virtual keyboard* to every grabbed device,
// so that is the write target when keyd runs; otherwise the Apple evdev is
// written directly. Apple keyboards are recognised by HID_ID (vendor/product),
// never by the user-editable device name.

use std::path::{Path, PathBuf};

/// Name keyd gives its uinput keyboard.
pub const KEYD_VIRTUAL_KEYBOARD: &str = "keyd virtual keyboard";

/// Why an LED write did not happen. Never silent: callers get the reason.
#[derive(Debug)]
pub enum LedError {
    /// Neither keyd's virtual keyboard nor an Apple evdev was found.
    NoTarget,
    /// The evdev could not be opened for writing (permissions: needs the uaccess ACL).
    Open(PathBuf, std::io::Error),
    /// The write failed or was short.
    Write(PathBuf, std::io::Error),
}

impl std::fmt::Display for LedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LedError::NoTarget => write!(f, "no keyd virtual keyboard nor Apple evdev found"),
            LedError::Open(p, e) => write!(f, "cannot open {}: {}", p.display(), e),
            LedError::Write(p, e) => write!(f, "cannot write {}: {}", p.display(), e),
        }
    }
}

impl std::error::Error for LedError {}

/// Where an LED event goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedTarget {
    /// keyd's virtual keyboard: keyd relays EV_LED to the grabbed keyboard.
    KeydVirtual(PathBuf),
    /// Apple keyboard evdev, written directly (keyd not running).
    AppleDirect(PathBuf),
}

impl LedTarget {
    pub fn path(&self) -> &Path {
        match self {
            LedTarget::KeydVirtual(p) | LedTarget::AppleDirect(p) => p,
        }
    }
}

/// Walk up from a sysfs node to the HID device and return its uevent text.
fn hid_uevent_above(node: &Path) -> Option<String> {
    let real = std::fs::canonicalize(node).ok()?;
    for dir in real.ancestors().skip(1) {
        if let Ok(u) = std::fs::read_to_string(dir.join("uevent")) {
            if u.lines().any(|l| l.starts_with("HID_ID=")) {
                return Some(u);
            }
        }
    }
    None
}

/// True if this sysfs input/led node belongs to a supported Apple keyboard.
fn node_is_apple_keyboard(node: &Path) -> bool {
    hid_uevent_above(node).is_some_and(|u| model_from_uevent(&u).is_some())
}

/// Sorted `name -> path` entries of a sysfs class directory.
fn class_entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    v.sort();
    v
}

/// keyd's virtual keyboard evdev (`/dev/input/eventN`), if keyd is running.
pub fn find_keyd_virtual_evdev_in(sys: &Path, dev: &Path) -> Option<PathBuf> {
    class_entries(&sys.join("class/input")).into_iter().find_map(|(name, p)| {
        if !name.starts_with("event") {
            return None;
        }
        let n = std::fs::read_to_string(p.join("device/name")).ok()?;
        (n.trim() == KEYD_VIRTUAL_KEYBOARD).then(|| dev.join("input").join(&name))
    })
}

/// Apple keyboard evdev, recognised by HID_ID of its HID parent.
pub fn find_apple_evdev_in(sys: &Path, dev: &Path) -> Option<PathBuf> {
    class_entries(&sys.join("class/input")).into_iter().find_map(|(name, p)| {
        (name.starts_with("event") && node_is_apple_keyboard(&p))
            .then(|| dev.join("input").join(&name))
    })
}

/// Pick the LED write target: keyd virtual keyboard first, Apple evdev as fallback.
pub fn led_target_in(sys: &Path, dev: &Path) -> Option<LedTarget> {
    find_keyd_virtual_evdev_in(sys, dev)
        .map(LedTarget::KeydVirtual)
        .or_else(|| find_apple_evdev_in(sys, dev).map(LedTarget::AppleDirect))
}

/// brightness file of the Apple keyboard's own `input*::<suffix>` LED
/// (`suffix` = "capslock" / "numlock"), not just any LED of that name.
pub fn apple_led_brightness_in(sys: &Path, suffix: &str) -> Option<PathBuf> {
    let want = format!("::{}", suffix);
    class_entries(&sys.join("class/leds")).into_iter().find_map(|(name, p)| {
        (name.ends_with(&want) && node_is_apple_keyboard(&p.join("device")))
            .then(|| p.join("brightness"))
    })
}

fn read_led_in(sys: &Path, suffix: &str) -> bool {
    apple_led_brightness_in(sys, suffix)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|v| v.trim() != "0")
}

/// Read CapsLock and NumLock LED state of the Apple keyboard from sysfs.
/// Returns (capslock_on, numlock_on); false if the Apple keyboard has no such LED.
pub fn read_led_state() -> (bool, bool) {
    let sys = Path::new("/sys");
    (read_led_in(sys, "capslock"), read_led_in(sys, "numlock"))
}

/// `struct input_event` (x86_64: 24 bytes) carrying EV_LED.
pub fn build_led_event(led: u16, value: bool) -> [u8; 24] {
    let mut ev = [0u8; 24]; // tv_sec(8) + tv_usec(8) + type(2) + code(2) + value(4)
    ev[16..18].copy_from_slice(&0x11u16.to_ne_bytes()); // EV_LED
    ev[18..20].copy_from_slice(&led.to_ne_bytes());
    ev[20..24].copy_from_slice(&(value as i32).to_ne_bytes());
    ev
}

fn write_led_event(target: &Path, led: u16, value: bool) -> Result<(), LedError> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(target)
        .map_err(|e| LedError::Open(target.to_path_buf(), e))?;
    f.write_all(&build_led_event(led, value))
        .map_err(|e| LedError::Write(target.to_path_buf(), e))
}

/// Set a keyboard LED (0=NumLock, 1=CapsLock, 2=ScrollLock) through keyd's
/// virtual keyboard, or directly on the Apple evdev if keyd is absent.
/// Returns the target used, or an explicit error (never a silent no-op).
pub fn set_led(led: u16, value: bool) -> Result<LedTarget, LedError> {
    let target = led_target_in(Path::new("/sys"), Path::new("/dev")).ok_or(LedError::NoTarget)?;
    write_led_event(target.path(), led, value)?;
    Ok(target)
}

/// Flash CapsLock LED N times (for notifications). Stops and logs on the first error.
pub fn flash_capslock(times: u8) {
    std::thread::spawn(move || {
        // Restore the Apple keyboard's real CapsLock state afterwards.
        let was_on = read_led_state().0;
        for _ in 0..times {
            for v in [!was_on, was_on] {
                if let Err(e) = set_led(1, v) {
                    eprintln!("[keyboard] LED flash aborted: {}", e);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uevent_matches_exact_vendor_and_product() {
        let u = "DRIVER=hid-generic\nHID_ID=0005:000005AC:00000255\nHID_NAME=Apple Wireless Keyboard\n";
        let (m, c) = apple_model_from_uevent(u).unwrap();
        assert!(m.contains("A1314") && m.contains("ANSI"));
        assert_eq!(c, "BCM2042");
    }

    #[test]
    fn uevent_rejects_non_apple_and_substring_false_positives() {
        // Non-Apple vendor whose product id happens to be 0255.
        assert!(apple_model_from_uevent("HID_ID=0003:0000046D:00000255\n").is_none());
        // Apple vendor, unknown product, "0255" only appears in the MAC.
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
    }

    #[test]
    fn magic_keyboard_bluetooth_vendor_004c() {
        let mi = model_from_uevent("HID_ID=0005:0000004C:0000029C\n").unwrap();
        assert_eq!(mi.family, Family::MagicKeyboard);
        assert_eq!(family(0x004c, 0x0267), Family::MagicKeyboard);
        assert_eq!(family(0x05ac, 0x0321), Family::MagicKeyboard);
        // Touch ID 2021 is 0x029a, 2024 numpad is 0x0322
        assert!(lookup_model(0x004c, 0x029a).unwrap().model.contains("Touch ID"));
        assert!(lookup_model(0x004c, 0x0322).is_some());
    }

    #[test]
    fn corrected_pids_match_kernel_hid_ids() {
        // Previously wrong entries: wired / internal devices, not wireless keyboards.
        for pid in [0x0220, 0x0229, 0x024f, 0x0250] {
            assert!(lookup_model(APPLE_USB_VID, pid).is_none(), "{:#06x}", pid);
        }
        // 0x022c is the ANSI A1255 (not JIS); ISO 0x022d, JIS 0x022e.
        assert!(lookup_model(APPLE_USB_VID, 0x022c).unwrap().model.contains("ANSI"));
        assert!(lookup_model(APPLE_USB_VID, 0x022d).unwrap().model.contains("ISO"));
        assert!(lookup_model(APPLE_USB_VID, 0x022e).unwrap().model.contains("JIS"));
        // 0x0267 is the 2015 Magic Keyboard, 0x026c the numpad model.
        assert!(lookup_model(APPLE_USB_VID, 0x0267).unwrap().model.contains("2015"));
        assert!(lookup_model(APPLE_USB_VID, 0x026c).unwrap().model.contains("Numeric"));
    }

    #[test]
    fn bcm2042_gating_by_family() {
        let bcm: Vec<u32> = (0x022c..=0x022e).chain(0x0239..=0x023b).chain(0x0255..=0x0257).collect();
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
            assert!(APPLE_MODELS[i + 1..].iter().all(|b| b.pid != a.pid), "{:#06x}", a.pid);
        }
    }

    #[test]
    fn calibration_validation() {
        assert!(calibration_valid(&DEFAULT_CALIBRATION_MV));
        assert!(!calibration_valid(&[0, 0, 0, 0]));
        assert!(!calibration_valid(&[2000, 2450, 2350, 2900]));
        assert!(!calibration_valid(&[2900, 2900, 2350, 2000]));
    }

    #[test]
    fn interpolation_bounds_and_midpoints() {
        let c = DEFAULT_CALIBRATION_MV;
        assert_eq!(interpolate_battery(3.3, &c), 100.0);
        assert_eq!(interpolate_battery(2.9, &c), 100.0);
        assert_eq!(interpolate_battery(2.45, &c), 75.0);
        assert_eq!(interpolate_battery(2.35, &c), 50.0);
        assert_eq!(interpolate_battery(2.0, &c), 25.0);
        assert_eq!(interpolate_battery(0.0, &c), 0.0);
        assert_eq!(interpolate_battery(-1.0, &c), 0.0);
        let mid = interpolate_battery(2.675, &c); // halfway 2450..2900
        assert!((mid - 87.5).abs() < 0.01, "{}", mid);
        // below the 25% threshold it decays linearly to 0
        assert!((interpolate_battery(1.0, &c) - 12.5).abs() < 0.01);
    }

    #[test]
    fn interpolation_survives_garbled_calibration() {
        for bad in [[0u16; 4], [100, 200, 300, 400], [2900, 2900, 2900, 2900]] {
            for mv in [0.0, 1.0, 2.2, 2.6, 3.3] {
                let p = interpolate_battery(mv, &bad);
                assert!((0.0..=100.0).contains(&p), "{:?} {} -> {}", bad, mv, p);
                assert_eq!(p, interpolate_battery(mv, &DEFAULT_CALIBRATION_MV));
            }
        }
    }

    #[test]
    fn battery_type_thresholds() {
        assert_eq!(detect_battery_type(3.2), "Lithium (fresh)");
        assert_eq!(detect_battery_type(2.9), "Alkaline (fresh)");
        assert_eq!(detect_battery_type(1.0), "Critical — replace");
    }

    #[test]
    fn wake_state_records_events() {
        let w = WakeState::default();
        assert!(w.last().is_none());
        assert_eq!(w.count(), 0);
        w.record();
        w.record();
        assert_eq!(w.count(), 2);
        assert!(w.last().unwrap().elapsed().as_secs() < 5);
    }

    #[test]
    fn wake_report_detection() {
        assert!(is_wake_report(&[0x13, 1, 2]));
        assert!(!is_wake_report(&[0x12]));
        assert!(!is_wake_report(&[]));
    }

    #[test]
    fn hid_read_feature_on_invalid_fd_is_none() {
        assert!(hid_read_feature(-1, HID_BATTERY_PRECISE).is_none());
    }

    // ── LED tests on a fake sysfs ─────────────────────────────────────────

    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU32, Ordering};

    static T: AtomicU32 = AtomicU32::new(0);

    struct Fake(PathBuf);
    impl Fake {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("kbled-test-{}-{}", std::process::id(), T.fetch_add(1, Ordering::SeqCst)));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Fake(p)
        }
        fn put(&self, rel: &str, c: &str) {
            let f = self.0.join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, c).unwrap();
        }
        /// HID device + input node + evdev + capslock LED, wired with symlinks like real sysfs.
        fn add_hid_keyboard(&self, hid: &str, hid_id: &str, input: &str, event: &str, name: &str) {
            let hdir = format!("devices/virtual/uhid/{hid}");
            self.put(&format!("{hdir}/uevent"), &format!("HID_ID={hid_id}\nHID_NAME={name}\n"));
            self.put(&format!("{hdir}/input/{input}/name"), name);
            self.put(&format!("{hdir}/input/{input}/{event}/dev"), "13:64\n");
            self.put(&format!("{hdir}/input/{input}/{input}::capslock/brightness"), "1\n");
            self.put(&format!("{hdir}/input/{input}/{input}::numlock/brightness"), "0\n");
            std::fs::create_dir_all(self.0.join("class/input")).unwrap();
            std::fs::create_dir_all(self.0.join("class/leds")).unwrap();
            let abs = |r: &str| self.0.join(r);
            symlink(abs(&format!("{hdir}/input/{input}/{event}")), abs(&format!("class/input/{event}"))).unwrap();
            symlink(abs(&format!("{hdir}/input/{input}")), abs(&format!("class/input/{input}"))).unwrap();
            // event's device -> input node, as in real sysfs
            symlink(abs(&format!("{hdir}/input/{input}")), abs(&format!("{hdir}/input/{input}/{event}/device"))).unwrap();
            for l in ["capslock", "numlock"] {
                let led = abs(&format!("{hdir}/input/{input}/{input}::{l}"));
                symlink(abs(&format!("{hdir}/input/{input}")), led.join("device")).unwrap();
                symlink(&led, abs(&format!("class/leds/{input}::{l}"))).unwrap();
            }
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn led_event_layout() {
        let ev = build_led_event(1, true);
        assert_eq!(&ev[..16], &[0u8; 16]);
        assert_eq!(u16::from_ne_bytes([ev[16], ev[17]]), 0x11);
        assert_eq!(u16::from_ne_bytes([ev[18], ev[19]]), 1);
        assert_eq!(i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]), 1);
        assert_eq!(build_led_event(0, false)[20..], [0, 0, 0, 0]);
    }

    #[test]
    fn apple_evdev_found_by_hid_id_not_by_name() {
        let f = Fake::new();
        // User-renamed Apple keyboard + a non-Apple one named like an Apple one.
        f.add_hid_keyboard("0005:05AC:0256.0014", "0005:000005AC:00000256", "input7", "event7", "Clavier de maria #1");
        f.add_hid_keyboard("0003:046D:C31C.0001", "0003:0000046D:0000C31C", "input3", "event3", "Apple Keyboard Lookalike");
        let dev = Path::new("/dev");
        assert_eq!(find_apple_evdev_in(&f.0, dev), Some(PathBuf::from("/dev/input/event7")));
    }

    #[test]
    fn led_target_prefers_keyd_virtual_keyboard() {
        let f = Fake::new();
        f.add_hid_keyboard("0005:05AC:0256.0014", "0005:000005AC:00000256", "input7", "event7", "Clavier de maria #1");
        let dev = Path::new("/dev");
        assert_eq!(led_target_in(&f.0, dev), Some(LedTarget::AppleDirect(PathBuf::from("/dev/input/event7"))));
        // keyd appears
        f.put("devices/virtual/input/input99/name", "keyd virtual keyboard\n");
        std::fs::create_dir_all(f.0.join("devices/virtual/input/input99/event99")).unwrap();
        symlink(f.0.join("devices/virtual/input/input99/event99"), f.0.join("class/input/event99")).unwrap();
        symlink(f.0.join("devices/virtual/input/input99"), f.0.join("devices/virtual/input/input99/event99/device")).unwrap();
        assert_eq!(led_target_in(&f.0, dev), Some(LedTarget::KeydVirtual(PathBuf::from("/dev/input/event99"))));
    }

    #[test]
    fn led_target_none_without_keyboard() {
        let f = Fake::new();
        assert_eq!(led_target_in(&f.0, Path::new("/dev")), None);
    }

    #[test]
    fn led_state_reads_the_apple_keyboard_led_only() {
        let f = Fake::new();
        f.add_hid_keyboard("0003:046D:C31C.0001", "0003:0000046D:0000C31C", "input3", "event3", "Other");
        f.put("devices/virtual/uhid/0003:046D:C31C.0001/input/input3/input3::capslock/brightness", "1\n");
        f.add_hid_keyboard("0005:05AC:0256.0014", "0005:000005AC:00000256", "input7", "event7", "Apple");
        f.put("devices/virtual/uhid/0005:05AC:0256.0014/input/input7/input7::capslock/brightness", "0\n");
        f.put("devices/virtual/uhid/0005:05AC:0256.0014/input/input7/input7::numlock/brightness", "1\n");
        assert!(!read_led_in(&f.0, "capslock")); // the other keyboard's lit LED is ignored
        assert!(read_led_in(&f.0, "numlock"));
    }

    #[test]
    fn write_led_event_writes_24_bytes_and_reports_errors() {
        let f = Fake::new();
        let file = f.0.join("evdev");
        std::fs::write(&file, b"").unwrap();
        write_led_event(&file, 1, true).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), build_led_event(1, true).to_vec());
        let err = write_led_event(&f.0.join("missing/dir/event"), 1, true).unwrap_err();
        assert!(matches!(err, LedError::Open(..)));
        assert!(err.to_string().contains("cannot open"));
    }
}
