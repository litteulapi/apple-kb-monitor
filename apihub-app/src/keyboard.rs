//! Apple Wireless Keyboard HID telemetry reader.
//!
//! Pure Rust — reads HID Feature Reports via ioctl, no subprocess.
//! Supports BCM2042-based keyboards (A1314 in ISO/ANSI/JIS variants).

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

/// Apple USB vendor ID (uppercase hex as it appears in sysfs uevent)
pub const APPLE_VENDOR_ID: &str = "05AC";

/// All known Apple Wireless Keyboard product IDs (BT HID)
pub const APPLE_PIDS: &[(&str, &str, &str)] = &[
    // (PID, model, chip)
    ("0220", "Apple Wireless Keyboard (A1016, white)", "BCM2042"),
    ("0229", "Apple Wireless Keyboard (A1255, aluminum, ANSI)", "BCM2042"),
    ("022C", "Apple Wireless Keyboard (A1255, aluminum, JIS)", "BCM2042"),
    ("0255", "Apple Wireless Keyboard (A1314, aluminum, ANSI)", "BCM2042"),
    ("0256", "Apple Wireless Keyboard (A1314, aluminum, ISO)", "BCM2042"),
    ("0257", "Apple Wireless Keyboard (A1314, aluminum, JIS)", "BCM2042"),
    ("024F", "Apple Magic Keyboard (A1644, ANSI)", "BCM20733"),
    ("0250", "Apple Magic Keyboard (A1644, ISO)", "BCM20733"),
    ("0267", "Apple Magic Keyboard with Touch ID (A2449, ANSI)", "BCM20733"),
    ("026C", "Apple Magic Keyboard with Touch ID (A2449, ISO)", "BCM20733"),
];

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
pub struct KbReport {
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

/// Find the first Apple keyboard hidraw device by scanning sysfs uevent.
/// Matches all 10 known Apple Wireless/Magic Keyboard PIDs.
pub fn find_apple_hidraw() -> Option<String> {
    let rd = std::fs::read_dir("/sys/class/hidraw").ok()?;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let device_path = entry.path().join("device/uevent");
        if let Ok(uevent) = std::fs::read_to_string(&device_path) {
            if uevent.contains(APPLE_VENDOR_ID) {
                for &(pid, _, _) in APPLE_PIDS {
                    if uevent.contains(pid) {
                        return Some(format!("/dev/{}", name));
                    }
                }
            }
        }
    }
    None
}

// ── Battery helpers ───────────────────────────────────────────────────────

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
        *fd_lock = None;
    }
    let path = find_apple_hidraw()?;
    let c_path = std::ffi::CString::new(path.as_str()).ok()?;
    let raw_fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
    if raw_fd < 0 { return None; }
    *fd_lock = Some((raw_fd, path.clone()));
    Some((raw_fd, path))
}

/// Read keyboard telemetry via HID Feature Reports.
/// Uses persistent fd. Reads only essential reports to minimize BT traffic
/// and avoid triggering HIDP timeouts that cause disconnections.
pub fn read_keyboard() -> Option<KbReport> {
    let (fd_val, path) = get_hid_fd()?;

    let mut report = KbReport::default();

    // First: try battery precise (0xEA) as a connectivity probe.
    // If this fails, the keyboard is disconnected — don't hammer with more reads.
    let probe = hid_read_feature(fd_val, HID_BATTERY_PRECISE);
    if probe.is_none() {
        // Keyboard not responding — invalidate persistent fd so we reopen next time
        if let Ok(mut fd_lock) = HID_FD.lock() { *fd_lock = None; }
        return None;
    }
    if let Some(ref buf) = probe {
        if buf.len() >= 2 {
            report.battery.percentage_fine = Some(buf[1] as f64);
        }
    }

    // Battery standard (0x47) — firmware-rounded
    if let Some(buf) = hid_read_feature(fd_val, HID_BATTERY_STANDARD) {
        if buf.len() >= 2 {
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
            for i in 0..4 {
                let off = 1 + i * 2;
                calib[i] = ((buf[off] as u16) << 8) | buf[off + 1] as u16;
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
            name_bytes.extend_from_slice(&buf[1..]);
        }
    }
    let name = String::from_utf8_lossy(&name_bytes).trim_end_matches('\0').to_string();
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

    // Real BT MAC from sysfs HID_UNIQ in uevent
    // (0x4C identity report contains BCM2042 internal ID, not BT MAC)
    let hidraw_name = path.trim_start_matches("/dev/");
    let uevent_path = format!("/sys/class/hidraw/{}/device/uevent", hidraw_name);
    if let Ok(uevent) = std::fs::read_to_string(&uevent_path) {
        for line in uevent.lines() {
            if let Some(mac) = line.strip_prefix("HID_UNIQ=") {
                let mac = mac.trim().to_uppercase();
                if mac.contains(':') && mac.len() >= 17 {
                    report.device.mac = Some(mac);
                }
            }
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

    // Identify model from PID (via sysfs uevent)
    let uevent = std::fs::read_to_string(
        std::path::Path::new("/sys/class/hidraw")
            .join(path.trim_start_matches("/dev/"))
            .join("device/uevent")
    ).unwrap_or_default();
    let (model, chip) = {
        let mut found = ("Apple Wireless Keyboard", "BCM2042");
        for &(pid, name, c) in APPLE_PIDS {
            if uevent.contains(pid) {
                found = (name, c);
                break;
            }
        }
        found
    };
    report.device.model = Some(model.to_string());
    report.device.chip = Some(chip.to_string());
    report.device.driver = Some("hid-apple".to_string());

    // fd stays open (persistent) for next call
    Some(report)
}

// ── Wake event monitor (Input Report 0x13) ──────────────────────────────

/// Spawn a thread that monitors HID Input Report 0x13 (vendor wake/connection events).
/// Returns a shared timestamp of the last wake event.
pub fn spawn_wake_monitor(hidraw_path: &str) -> Arc<Mutex<Option<std::time::Instant>>> {
    let last_wake: Arc<Mutex<Option<std::time::Instant>>> = Arc::new(Mutex::new(None));
    let lw = last_wake.clone();
    let path = hidraw_path.to_string();

    std::thread::spawn(move || {
        let c_path = match std::ffi::CString::new(path.as_str()) {
            Ok(p) => p,
            Err(_) => return,
        };
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if fd < 0 { return; }

        let mut buf = [0u8; 64];
        loop {
            // poll with 2s timeout
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            let ret = unsafe { libc::poll(&mut pfd, 1, 2000) };
            if ret <= 0 { continue; }

            let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 { continue; }

            // Report 0x13 = vendor wake event (FF01 usage page)
            if buf[0] == 0x13 {
                if let Ok(mut lw) = lw.lock() {
                    *lw = Some(std::time::Instant::now());
                }
            }
        }
    });

    last_wake
}

// ── LED state reader ────────────────────────────────────────────────────

/// Read CapsLock and NumLock LED state from sysfs.
/// Returns (capslock_on, numlock_on).
pub fn read_led_state() -> (bool, bool) {
    let mut caps = false;
    let mut num = false;

    if let Ok(rd) = std::fs::read_dir("/sys/class/leds") {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let bri_path = entry.path().join("brightness");
            if let Ok(val) = std::fs::read_to_string(&bri_path) {
                let on = val.trim() != "0";
                if name.contains("capslock") { caps = on; }
                if name.contains("numlock") { num = on; }
            }
        }
    }
    (caps, num)
}

// ── LED control via evdev ───────────────────────────────────────────────

/// Find the Apple keyboard evdev path (e.g. /dev/input/event13)
pub fn find_apple_evdev() -> Option<String> {
    if let Ok(rd) = std::fs::read_dir("/sys/class/input") {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("event") { continue; }
            let name_path = entry.path().join("device/name");
            if let Ok(dev_name) = std::fs::read_to_string(&name_path) {
                if dev_name.trim().contains("Apple") && dev_name.trim().contains("Keyboard") {
                    return Some(format!("/dev/input/{}", name));
                }
            }
        }
    }
    None
}

/// Set a keyboard LED via evdev (EV_LED input_event).
/// led: 0=NumLock, 1=CapsLock, 2=ScrollLock
/// value: true=on, false=off
pub fn set_led(led: u16, value: bool) {
    if let Some(evdev) = find_apple_evdev() {
        if let Ok(c_path) = std::ffi::CString::new(evdev.as_str()) {
            let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_WRONLY) };
            if fd >= 0 {
                // struct input_event: tv_sec(8) + tv_usec(8) + type(2) + code(2) + value(4) = 24
                let mut event = [0u8; 24];
                // type = EV_LED = 0x11
                event[16] = 0x11;
                event[17] = 0x00;
                // code = led
                event[18] = led as u8;
                event[19] = (led >> 8) as u8;
                // value = 0 or 1
                event[20] = if value { 1 } else { 0 };
                unsafe { libc::write(fd, event.as_ptr() as *const libc::c_void, 24); }
                unsafe { libc::close(fd); }
            }
        }
    }
}

/// Flash CapsLock LED N times (for notifications).
pub fn flash_capslock(times: u8) {
    std::thread::spawn(move || {
        for _ in 0..times {
            set_led(1, true);
            std::thread::sleep(std::time::Duration::from_millis(300));
            set_led(1, false);
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    });
}
